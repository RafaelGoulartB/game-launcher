//! Shortcuts for games: `.desktop` files (Desktop and application menu) and
//! "Add to Steam". Glue between the library, Heroic's settings and the
//! `heronite-shortcuts` crate.
//!
//! Every function here is **blocking** (reads the library, writes files,
//! may download artwork): call it from a background thread (`cx.background_spawn`
//! or `std::thread`), never from the UI thread.
//!
//! # For other features
//!
//! After a game was added or installed, call
//!
//! ```ignore
//! let outcome = crate::system::shortcuts::on_game_added(&heroic_paths, "sideload", &app_name);
//! for m in &outcome.messages { /* show_toast(m.text, m.error) */ }
//! ```
//!
//! It honours `addDesktopShortcuts` / `addStartMenuShortcuts` /
//! `addSteamShortcuts` of Heroic's global settings (does nothing, quickly,
//! when all three are off). The game must already be in the library files
//! (the function reloads them to get title and artwork). Roots can be
//! redirected with `HERONITE_DESKTOP_DIR`, `HERONITE_APPLICATIONS_DIR` and
//! `HERONITE_STEAM_DIR`.

use std::path::{Path, PathBuf};

use heronite_data::game_config::load_global;
use heronite_data::{art, AppPaths, ArtContext, CollectionGame, DataPaths, Library};
use heronite_i18n::tr;
use heronite_shortcuts::desktop::{self, Roots};
use heronite_shortcuts::steam::{self, ArtFiles, ShortcutSpec};
use heronite_shortcuts::steam_client::{debugger_addr, SteamClient};
use heronite_shortcuts::steam_running::{self, UrlHandler};
use heronite_shortcuts::GameId;
use serde_json::Value;

/// One line for the user (a toast).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub text: String,
    pub error: bool,
}

/// What an operation did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub messages: Vec<Message>,
}

impl Outcome {
    fn ok(&mut self, text: impl Into<String>) {
        self.messages.push(Message { text: text.into(), error: false });
    }

    fn err(&mut self, text: impl Into<String>) {
        self.messages.push(Message { text: text.into(), error: true });
    }

    /// All messages as one toast (`error` when any failed).
    pub fn summary(&self) -> Option<Message> {
        if self.messages.is_empty() {
            return None;
        }
        Some(Message {
            text: self.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>().join("\n"),
            error: self.messages.iter().any(|m| m.error),
        })
    }
}

/// The three automatic-shortcut settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AutoSettings {
    pub desktop: bool,
    pub menu: bool,
    pub steam: bool,
}

impl AutoSettings {
    pub fn load(data_paths: &DataPaths) -> Self {
        let g = load_global(data_paths, &home());
        let flag = |k: &str| g.get(k).and_then(Value::as_bool).unwrap_or(false);
        Self { desktop: flag("addDesktopShortcuts"), menu: flag("addStartMenuShortcuts"), steam: flag("addSteamShortcuts") }
    }

    pub fn any(self) -> bool {
        self.desktop || self.menu || self.steam
    }
}

fn home() -> String {
    dirs::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Steam install folder: `HERONITE_STEAM_DIR`, else `defaultSteamPath`.
pub fn steam_path(data_paths: &DataPaths) -> PathBuf {
    if let Some(dir) = crate::env::value("HERONITE_STEAM_DIR") {
        return PathBuf::from(dir);
    }
    configured_steam_path(data_paths)
}

fn configured_steam_path(data_paths: &DataPaths) -> PathBuf {
    let g = load_global(data_paths, &home());
    steam::clean_steam_path(g.get("defaultSteamPath").and_then(Value::as_str).unwrap_or(""))
}

/// The executable shortcuts (and the autostart entry) point at.
pub(crate) fn own_exe() -> PathBuf {
    // In an AppImage the executable lives in a mount that is gone after the
    // app quits; the AppImage file itself is what shortcuts must start.
    if let Some(appimage) = std::env::var_os("APPIMAGE").filter(|p| !p.is_empty()) {
        return PathBuf::from(appimage);
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("heronite-rs"));
    // A replaced binary shows up as ".../heronite-rs (deleted)".
    let s = exe.to_string_lossy();
    PathBuf::from(s.strip_suffix(" (deleted)").unwrap_or(&s).to_string())
}

fn find_game(data_paths: &DataPaths, runner: &str, app_name: &str) -> Result<(Library, std::sync::Arc<CollectionGame>), String> {
    let lib = Library::load(data_paths).map_err(|e| tr!("Could not read the library: {error}", error = format!("{e:#}")))?;
    let game = lib
        .games
        .iter()
        .find(|g| g.app_name == app_name && (runner.is_empty() || g.runner == runner))
        .cloned()
        .ok_or_else(|| tr!("Game {name} was not found in the library", name = app_name))?;
    Ok((lib, game))
}

fn is_dlc(game: &CollectionGame) -> bool {
    game.game.install.is_dlc == Some(true)
}

fn game_id(game: &CollectionGame) -> GameId {
    GameId { runner: game.runner.clone(), app_name: game.app_name.clone(), title: game.title.clone() }
}

fn image_opts(data_paths: &DataPaths) -> heronite_images::Opts {
    heronite_images::Opts::new(data_paths.images_cache_dir(), Some(AppPaths::for_data(data_paths).thumbs_dir()))
}

/// Writes the first loadable image of `chain` as a `w`x`h` file (`cover`
/// crop). `png`: PNG, else JPEG q90.
fn render(chain: &[heronite_data::ImageRef], w: u32, h: u32, dest: &Path, png: bool, opts: &heronite_images::Opts) -> Option<PathBuf> {
    let chain = crate::images::to_chain(chain);
    if chain.is_empty() {
        return None;
    }
    let decoded = match heronite_images::load_cover(&chain, w, h, opts) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("shortcut artwork {}: {e}", dest.display());
            return None;
        }
    };
    let (width, height) = (decoded.width, decoded.height);
    let mut buf = decoded.bgra;
    heronite_images::fx::swap_rb(&mut buf);
    let img = image::RgbaImage::from_raw(width, height, buf)?;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).ok()?;
    }
    let tmp = dest.with_extension("tmp");
    let written = (|| -> Result<(), String> {
        let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut file = std::io::BufWriter::new(file);
        if png {
            img.write_to(&mut file, image::ImageFormat::Png).map_err(|e| e.to_string())
        } else {
            let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, 90).encode_image(&rgb).map_err(|e| e.to_string())
        }
    })();
    match written.and_then(|_| std::fs::rename(&tmp, dest).map_err(|e| e.to_string())) {
        Ok(()) => Some(dest.to_path_buf()),
        Err(e) => {
            log::warn!("writing {}: {e}", dest.display());
            let _ = std::fs::remove_file(&tmp);
            None
        }
    }
}

/// Icon for the `.desktop` files: Heroic's `icons/<appName>.(jpg|png)` when
/// it exists, else a 256x256 PNG made from the cover and kept there.
pub fn icon_file(data_paths: &DataPaths, lib: &Library, game: &CollectionGame) -> Option<PathBuf> {
    let dir = data_paths.icons_dir();
    for ext in ["jpg", "png"] {
        let p = dir.join(format!("{}.{ext}", game.app_name));
        if p.exists() {
            return Some(p);
        }
    }
    let shots = (!lib.settings.screenshots.folder.is_empty()).then(|| PathBuf::from(&lib.settings.screenshots.folder));
    let ctx = ArtContext::new(data_paths, shots.as_deref());
    render(&art::card_cover(&ctx, game), 256, 256, &dir.join(format!("{}.png", game.app_name)), true, &image_opts(data_paths))
}

/// Grid artwork for Steam, generated into the app cache.
fn steam_art(data_paths: &DataPaths, lib: &Library, game: &CollectionGame) -> ArtFiles {
    let shots = (!lib.settings.screenshots.folder.is_empty()).then(|| PathBuf::from(&lib.settings.screenshots.folder));
    let ctx = ArtContext::new(data_paths, shots.as_deref());
    let opts = image_opts(data_paths);
    let dir = AppPaths::for_data(data_paths).cache.join("steam-art").join(format!("{}_{}", game.runner, game.app_name));
    let cover_chain = art::card_cover(&ctx, game);
    let stage_chain = art::stage_art(&ctx, game);
    // Wide images come from the stage/hero chain, else the cover.
    let wide = if stage_chain.is_empty() { &cover_chain } else { &stage_chain };
    ArtFiles {
        cover: render(&cover_chain, 600, 900, &dir.join("cover.jpg"), false, &opts),
        header: render(wide, 920, 430, &dir.join("header.jpg"), false, &opts),
        hero: render(wide, 1920, 620, &dir.join("hero.jpg"), false, &opts),
        logo: None,
    }
}

/// Desktop / application-menu shortcut files (`addShortcuts` without Steam).
/// `desktop`/`menu` pick the targets.
fn write_desktop(data_paths: &DataPaths, lib: &Library, game: &CollectionGame, desktop: bool, menu: bool, out: &mut Outcome) {
    let roots = Roots::detect();
    let icon = icon_file(data_paths, lib, game);
    match desktop::add(&roots, &game_id(game), &own_exe(), icon.as_deref(), desktop, menu) {
        Ok(w) => match (w.desktop.is_some(), w.menu.is_some()) {
            (true, true) => out.ok(tr!("Shortcuts were created on Desktop and Start Menu")),
            (true, false) => out.ok(tr!("Shortcuts were created on Desktop")),
            (false, true) => out.ok(tr!("Shortcuts were created on Start Menu")),
            (false, false) => {}
        },
        Err(e) => out.err(tr!("Could not create the shortcuts for {title}: {error}", title = game.title, error = format!("{e:#}"))),
    }
}

/// Context-menu "Add shortcuts": Desktop and application menu, whatever the
/// automatic settings say (Heroic's `fromMenu`).
pub fn add_shortcuts(data_paths: &DataPaths, runner: &str, app_name: &str) -> Outcome {
    let mut out = Outcome::default();
    match find_game(data_paths, runner, app_name) {
        Ok((lib, game)) => {
            if !is_dlc(&game) {
                write_desktop(data_paths, &lib, &game, true, true, &mut out);
            }
        }
        Err(e) => out.err(e),
    }
    out
}

/// Context-menu "Remove shortcuts".
pub fn remove_shortcuts(data_paths: &DataPaths, runner: &str, app_name: &str) -> Outcome {
    let mut out = Outcome::default();
    match find_game(data_paths, runner, app_name) {
        Ok((_, game)) => match desktop::remove(&Roots::detect(), &game.title) {
            Ok(_) => out.ok(tr!("Shortcuts were removed from Desktop and Start Menu")),
            Err(e) => out.err(tr!("Could not remove the shortcuts of {title}: {error}", title = game.title, error = format!("{e:#}"))),
        },
        Err(e) => out.err(e),
    }
    out
}

/// `shortcutsExists`: for the menu label.
pub fn shortcuts_exist(data_paths: &DataPaths, runner: &str, app_name: &str) -> bool {
    find_game(data_paths, runner, app_name).map(|(_, g)| desktop::exists(&Roots::detect(), &g.title)).unwrap_or(false)
}

fn last_play_secs(game: &CollectionGame) -> Option<u32> {
    let ts = game.last_played.as_deref()?;
    chrono::DateTime::parse_from_rfc3339(ts).ok().map(|d| d.timestamp().clamp(0, u32::MAX as i64) as u32)
}

fn steam_spec(game: &CollectionGame, icon: Option<&Path>) -> ShortcutSpec {
    let exe = own_exe();
    let start_dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    ShortcutSpec {
        exe: format!("\"{}\"", exe.display()),
        start_dir: format!("\"{}\"", start_dir.display()),
        launch_options: format!(
            "--no-gui \"{}\"",
            heronite_shortcuts::launch_url(crate::system::cli::SCHEME, &game.runner, &game.app_name)
        ),
        icon: icon.map(|p| p.to_string_lossy().into_owned()),
        last_play_time: last_play_secs(game),
    }
}

/// The running Steam's debugger: `HERONITE_STEAM_CEF` (`host:port`, tests
/// against a fake) or Steam's own address.
pub fn steam_client() -> SteamClient {
    SteamClient::new(crate::env::value("HERONITE_STEAM_CEF").unwrap_or_else(debugger_addr))
}

/// Where `.desktop` wrappers for the `steam://addnonsteamgame` fallback live.
fn wrapper_dir() -> PathBuf {
    Roots::detect().applications.join(steam::WRAPPER_DIR)
}

/// The `steam` command: `HERONITE_STEAM_BIN` (tests), `steam` on the PATH.
fn url_handler() -> UrlHandler {
    UrlHandler::new(crate::env::value("HERONITE_STEAM_BIN").unwrap_or_else(|| "steam".into()), wrapper_dir())
}

fn steam_running() -> &'static str {
    tr!("Steam is running. Close Steam and try again, or restart Steam once to let Heronite manage shortcuts while Steam is running.")
}

/// "Add to Steam". With Steam closed, `shortcuts.vdf` is edited (Steam picks
/// it up at its next start). With Steam running, Heroic's order: its debugging
/// interface, else the `steam://addnonsteamgame` handler (which also turns the
/// interface on for the next Steam start).
pub fn add_to_steam(data_paths: &DataPaths, runner: &str, app_name: &str) -> Outcome {
    let mut out = Outcome::default();
    let (lib, game) = match find_game(data_paths, runner, app_name) {
        Ok(v) => v,
        Err(e) => {
            out.err(e);
            return out;
        }
    };
    if is_dlc(&game) {
        return out;
    }
    let steam_dir = steam_path(data_paths);
    let running = steam::is_steam_running(&steam_dir);
    let client = steam_client();
    let live = running && client.is_available();
    if running && !live {
        steam::enable_client_api_on_next_start(&steam_dir);
        if steam::is_added(&steam_dir, &game_id(&game)) {
            out.ok(tr!("{title} was successfully added to Steam.", title = game.title));
            return out;
        }
    }
    let icon = icon_file(data_paths, &lib, &game);
    let art = steam_art(data_paths, &lib, &game);
    let spec = steam_spec(&game, icon.as_deref());
    if live {
        match steam_running::add_with_client(&client, &steam_dir, &game_id(&game), &spec, &art) {
            Ok(_) => out.ok(tr!("{title} was successfully added to Steam.", title = game.title)),
            Err(e) => out.err(tr!("Adding {title} to Steam failed with:\n{error}", title = game.title, error = format!("{e:#}"))),
        }
        return out;
    }
    if running {
        match steam_running::add_with_url_handler(&url_handler(), &steam_dir, &game_id(&game), &spec, &art) {
            Ok(_) => out.ok(tr!("{title} was successfully added to Steam.", title = game.title)),
            Err(e) => {
                log::error!("Failed to add {} to Steam via the url handler with: {e:#}", game.title);
                out.err(steam_running());
            }
        }
        return out;
    }
    match steam::add_game(&steam_dir, &game_id(&game), &spec, &art) {
        Ok(report) if report.errors.is_empty() => out.ok(tr!(
            "{title} was successfully added to Steam. A restart of Steam is required for changes to take effect.",
            title = game.title
        )),
        Ok(report) => {
            for e in &report.errors {
                log::error!("{e}");
            }
            out.ok(tr!(
                "{title} could not be added to all found Steam users. See logs for more info. A restart of Steam is required for changes to take effect.",
                title = game.title
            ));
        }
        Err(e) => out.err(tr!("Adding {title} to Steam failed with:\n{error}", title = game.title, error = format!("{e:#}"))),
    }
    out
}

/// "Remove from Steam": the file with Steam closed, the debugging interface
/// with Steam running (the URL handler cannot remove).
pub fn remove_from_steam(data_paths: &DataPaths, runner: &str, app_name: &str) -> Outcome {
    let mut out = Outcome::default();
    let (_, game) = match find_game(data_paths, runner, app_name) {
        Ok(v) => v,
        Err(e) => {
            out.err(e);
            return out;
        }
    };
    let steam_dir = steam_path(data_paths);
    if steam::is_steam_running(&steam_dir) {
        let client = steam_client();
        if !client.is_available() {
            steam::enable_client_api_on_next_start(&steam_dir);
            log::error!("Can't remove \"{}\" from Steam while Steam is running.", game.title);
            out.err(steam_running());
            return out;
        }
        match steam_running::remove_with_client(&client, &steam_dir, &game_id(&game), &wrapper_dir()) {
            Ok(true) => out.ok(tr!("{title} was successfully removed from Steam.", title = game.title)),
            Ok(false) => {}
            Err(e) => out.err(tr!("Removing {title} from Steam failed with:\n{error}", title = game.title, error = format!("{e:#}"))),
        }
        return out;
    }
    match steam::remove_game(&steam_dir, &game_id(&game)) {
        Ok(report) if report.errors.is_empty() => {
            if !report.users_done.is_empty() {
                out.ok(tr!(
                    "{title} was successfully removed from Steam. A restart of Steam is required for changes to take effect.",
                    title = game.title
                ));
            }
        }
        Ok(report) => {
            for e in &report.errors {
                log::error!("{e}");
            }
            out.err(tr!(
                "{title} could not be removed from all found Steam users. See logs for more info. A restart of Steam is required for changes to take effect.",
                title = game.title
            ));
        }
        Err(e) => out.err(tr!("Removing {title} from Steam failed with:\n{error}", title = game.title, error = format!("{e:#}"))),
    }
    out
}

/// `isAddedToSteam`: for the menu label. A running Steam with its debugging
/// interface is asked directly (its list is newer than the file).
pub fn is_added_to_steam(data_paths: &DataPaths, runner: &str, app_name: &str) -> bool {
    let Ok((_, game)) = find_game(data_paths, runner, app_name) else { return false };
    let steam_dir = steam_path(data_paths);
    if steam::is_steam_running(&steam_dir) {
        let client = steam_client();
        if client.is_available() {
            match steam_running::is_added_with_client(&client, &game_id(&game)) {
                Ok(added) => return added,
                Err(e) => log::warn!("Failed to list Steam shortcuts with: {e:#}"),
            }
        }
    }
    steam::is_added(&steam_dir, &game_id(&game))
}

/// Heroic's `addShortcuts(game)` after a game was installed or added: the
/// automatic settings decide what is created (Desktop, application menu,
/// Steam). Returns quickly when all three are off.
pub fn on_game_added(data_paths: &DataPaths, runner: &str, app_name: &str) -> Outcome {
    let settings = AutoSettings::load(data_paths);
    let mut out = Outcome::default();
    if !settings.any() {
        return out;
    }
    // DLC and add-ons are not in the library: nothing to do, nothing to report.
    let (lib, game) = match find_game(data_paths, runner, app_name) {
        Ok(v) => v,
        Err(e) => {
            log::info!("no shortcuts: {e}");
            return out;
        }
    };
    if is_dlc(&game) {
        return out;
    }
    log::info!("adding shortcuts for {}", game.title);
    if settings.steam {
        out.messages.extend(add_to_steam(data_paths, runner, app_name).messages);
    }
    if settings.desktop || settings.menu {
        write_desktop(data_paths, &lib, &game, settings.desktop, settings.menu, &mut out);
    }
    out
}

/// [`on_game_added`] on a background thread; `done` runs on the UI thread
/// with the outcome (usually: show `outcome.summary()` as a toast).
pub fn spawn_on_game_added<T: 'static>(
    cx: &mut gpui_kit::Context<T>,
    data_paths: &DataPaths,
    runner: &str,
    app_name: &str,
    done: impl FnOnce(&mut T, Outcome, &mut gpui_kit::Context<T>) + 'static,
) {
    let (data_paths, runner, app_name) = (data_paths.clone(), runner.to_string(), app_name.to_string());
    use gpui_kit::AppContext as _;
    cx.spawn(async move |this, cx| {
        let outcome = cx.background_spawn(async move { on_game_added(&data_paths, &runner, &app_name) }).await;
        if !outcome.messages.is_empty() {
            this.update(cx, |this, cx| done(this, outcome, cx)).ok();
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn summary_joins_messages() {
        let mut o = Outcome::default();
        assert!(o.summary().is_none());
        o.ok("a");
        o.err("b");
        let s = o.summary().unwrap();
        assert_eq!(s.text, "a\nb");
        assert!(s.error);
    }

    #[test]
    fn auto_settings_come_from_config_json() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path());
        assert!(!AutoSettings::load(&data_paths).any());
        fs::write(
            dir.path().join("config.json"),
            "{\n  \"defaultSettings\": {\n    \"addDesktopShortcuts\": true,\n    \"addSteamShortcuts\": true\n  },\n  \"version\": \"v0\"\n}",
        )
        .unwrap();
        let s = AutoSettings::load(&data_paths);
        assert!(s.desktop && s.steam && !s.menu);
    }

    #[test]
    fn on_game_added_does_nothing_when_settings_are_off() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path());
        assert_eq!(on_game_added(&data_paths, "sideload", "nope"), Outcome::default());
    }

    #[test]
    fn steam_path_override_and_quote_stripping() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path());
        fs::write(
            dir.path().join("config.json"),
            "{\n  \"defaultSettings\": {\n    \"defaultSteamPath\": \"/home/u/.steam/steam'\"\n  },\n  \"version\": \"v0\"\n}",
        )
        .unwrap();
        assert_eq!(configured_steam_path(&data_paths), PathBuf::from("/home/u/.steam/steam"));
    }

    /// End to end against a fixture library (`HERONITE_FIXTURE_SYSTEM=/tmp/heronite-fixture-system`,
    /// a copy of the data files) with every root in a tempdir: desktop files,
    /// icon, Steam shortcut + grid artwork, and their removal. Skipped without the fixture.
    #[test]
    fn end_to_end_on_a_fixture_library() {
        let Some(fixture) = crate::env::value("HERONITE_FIXTURE_SYSTEM").map(PathBuf::from) else {
            eprintln!("HERONITE_FIXTURE_SYSTEM not set; skipping");
            return;
        };
        let work = tempfile::tempdir().unwrap();
        // The fixture is copied so the icon / settings writes stay in the tempdir.
        let heroic_dir = work.path().join("heroic");
        fn copy(from: &Path, to: &Path) {
            fs::create_dir_all(to).unwrap();
            for e in fs::read_dir(from).unwrap().flatten() {
                let (src, dst) = (e.path(), to.join(e.file_name()));
                if src.is_dir() {
                    copy(&src, &dst);
                } else {
                    fs::copy(&src, &dst).unwrap();
                }
            }
        }
        copy(&fixture, &heroic_dir);
        let data_paths = DataPaths::at(&heroic_dir);
        let steam = work.path().join("Steam");
        fs::create_dir_all(steam.join("userdata/4242/config")).unwrap();
        std::env::set_var("HERONITE_DESKTOP_DIR", work.path().join("Desktop"));
        std::env::set_var("HERONITE_APPLICATIONS_DIR", work.path().join("apps"));
        std::env::set_var("HERONITE_STEAM_DIR", &steam);
        std::env::set_var("XDG_CACHE_HOME", work.path().join("cache"));

        let lib = Library::load(&data_paths).unwrap();
        let game = lib.games.iter().find(|g| g.installed && g.runner == "sideload").expect("an installed game").clone();
        let (runner, app) = (game.runner.clone(), game.app_name.clone());

        let out = add_shortcuts(&data_paths, &runner, &app);
        assert_eq!(out.messages.len(), 1, "{out:?}");
        assert!(!out.messages[0].error, "{out:?}");
        assert!(shortcuts_exist(&data_paths, &runner, &app));
        let file = work.path().join("apps").join(format!("{}.desktop", desktop::sanitize_filename(&game.title)));
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains(&format!("--launch {runner} ")), "{text}");
        assert!(work.path().join("Desktop").join(file.file_name().unwrap()).exists());

        let out = add_to_steam(&data_paths, &runner, &app);
        assert!(out.messages.iter().all(|m| !m.error), "{out:?}");
        assert!(is_added_to_steam(&data_paths, &runner, &app));
        let vdf = fs::read(steam.join("userdata/4242/config/shortcuts.vdf")).unwrap();
        let vdf = String::from_utf8_lossy(&vdf);
        assert!(vdf.contains(&format!("heronite://launch?appName={app}&runner={runner}")));

        let out = remove_from_steam(&data_paths, &runner, &app);
        assert!(out.messages.iter().all(|m| !m.error), "{out:?}");
        assert!(!is_added_to_steam(&data_paths, &runner, &app));
        let out = remove_shortcuts(&data_paths, &runner, &app);
        assert!(!out.messages[0].error);
        assert!(!shortcuts_exist(&data_paths, &runner, &app));

        // The automatic path: settings on -> files appear.
        heronite_data::game_config::set_global_setting(&data_paths, "/home/u", "addStartMenuShortcuts", serde_json::json!(true)).unwrap();
        let out = on_game_added(&data_paths, &runner, &app);
        assert!(out.messages.iter().all(|m| !m.error), "{out:?}");
        assert!(file.exists());
        assert!(!work.path().join("Desktop").join(file.file_name().unwrap()).exists());
    }
}
