//! "Add to Steam" while Steam runs. Port of Heroic's
//! `addNonSteamGameViaSteamClient` / `removeNonSteamGameViaSteamClient`
//! (through [`SteamClient`], takes effect at once) and of the
//! `steam://addnonsteamgame` fallback of `steamUrlHandler.ts`, used when the
//! debugging interface is off.
//!
//! The URL handler cannot set a title or launch options, so a `.desktop`
//! wrapper is added instead (the approach of GloriousEggroll's
//! steam-add-nonsteam-game); Steam then writes the entry itself.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::steam::{self, install_art, remove_art, user_folders, wrapper_file_name, ArtFiles, GridIds, ShortcutSpec};
use crate::steam_client::{Artwork, ClientShortcut, NewShortcut, SteamClient};
use crate::GameId;

fn matches(s: &ClientShortcut, game: &GameId) -> bool {
    steam::matches_shortcut(&s.exe, "", &s.launch_options, game)
}

/// `config` folders of every Steam user that has one.
fn config_dirs(steam: &Path) -> Vec<PathBuf> {
    let userdata = steam.join("userdata");
    match user_folders(&userdata) {
        Ok(folders) => folders.iter().map(|f| userdata.join(f).join("config")).filter(|d| d.is_dir()).collect(),
        Err(e) => {
            log::warn!("{e}");
            Vec::new()
        }
    }
}

/// `prepareImagesForAllUsers`: the artwork into every user's `grid/`; the grid
/// folder of the last user, whose files the client is then given.
fn art_for_all_users(steam: &Path, ids: &GridIds, art: &ArtFiles, title: &str) -> Option<PathBuf> {
    let mut last = None;
    for config in config_dirs(steam) {
        for e in install_art(&config, ids, art) {
            log::warn!("steam artwork for {title}: {e}");
        }
        last = Some(config.join("grid"));
    }
    last
}

/// The grid file of each artwork type (`.png` or `.jpg`, whichever exists).
fn grid_files(grid: &Path, ids: &GridIds) -> Vec<(PathBuf, Artwork)> {
    [
        (format!("{}p", ids.other), Artwork::Grid),
        (format!("{}_hero", ids.other), Artwork::Hero),
        (format!("{}_logo", ids.other), Artwork::Logo),
        (ids.other.clone(), Artwork::WideGrid),
    ]
    .into_iter()
    .filter_map(|(stem, kind)| ["jpg", "png"].iter().map(|ext| grid.join(format!("{stem}.{ext}"))).find(|p| p.exists()).map(|p| (p, kind)))
    .collect()
}

/// Adds `game` through the running client and sets its artwork. `Ok(true)`
/// when Steam already had it (the artwork is still refreshed, like Heroic).
pub fn add_with_client(client: &SteamClient, steam: &Path, game: &GameId, spec: &ShortcutSpec, art: &ArtFiles) -> Result<bool> {
    let existing = client.shortcuts()?.into_iter().find(|s| matches(s, game)).map(|s| s.appid);
    let appid = match existing {
        Some(id) => id,
        None => client.add_shortcut(&NewShortcut {
            name: &game.title,
            exe: &spec.exe,
            start_dir: &spec.start_dir,
            launch_options: &spec.launch_options,
            icon: spec.icon.as_deref(),
        })?,
    };
    let ids = GridIds::from_appid(appid);
    if let Some(grid) = art_for_all_users(steam, &ids, art, &game.title) {
        for (file, kind) in grid_files(&grid, &ids) {
            if let Err(e) = client.set_artwork_file(appid, &file, kind) {
                log::warn!("Failed to set Steam artwork from {} with: {e:#}", file.display());
            }
        }
    }
    Ok(existing.is_some())
}

/// Removes `game` through the running client, with its artwork and our
/// wrapper in `wrapper_dir`. `Ok(false)` when Steam did not have it.
pub fn remove_with_client(client: &SteamClient, steam: &Path, game: &GameId, wrapper_dir: &Path) -> Result<bool> {
    let Some(existing) = client.shortcuts()?.into_iter().find(|s| matches(s, game)) else { return Ok(false) };
    client.remove_shortcut(existing.appid)?;
    remove_wrapper(wrapper_dir, game);
    // The id Steam gave it and the one a file-made entry would have.
    let ids = [GridIds::from_appid(existing.appid), GridIds::generate(&existing.exe, &existing.app_name)];
    for config in config_dirs(steam) {
        for id in &ids {
            for e in remove_art(&config, id) {
                log::warn!("steam artwork for {}: {e}", game.title);
            }
        }
    }
    Ok(true)
}

/// `isAddedToSteam` through the running client.
pub fn is_added_with_client(client: &SteamClient, game: &GameId) -> Result<bool> {
    Ok(client.shortcuts()?.iter().any(|s| matches(s, game)))
}

/// How to reach the `steam://` handler of the running client.
#[derive(Debug, Clone)]
pub struct UrlHandler {
    /// The `steam` command; it hands the URL to the running client and exits.
    pub steam_bin: PathBuf,
    /// Where the `.desktop` wrappers live (must outlive the shortcut).
    pub wrapper_dir: PathBuf,
    /// Steam's handler only accepts local paths while this file exists.
    pub sentinel: PathBuf,
    pub poll_attempts: u32,
    pub poll_interval: Duration,
}

impl UrlHandler {
    pub fn new(steam_bin: impl Into<PathBuf>, wrapper_dir: impl Into<PathBuf>) -> Self {
        Self {
            steam_bin: steam_bin.into(),
            wrapper_dir: wrapper_dir.into(),
            sentinel: std::env::temp_dir().join("addnonsteamgamefile"),
            poll_attempts: 20,
            poll_interval: Duration::from_millis(500),
        }
    }
}

const HANDLER_TIMEOUT: Duration = Duration::from_secs(15);

fn escape_desktop_value(v: &str) -> String {
    v.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
}

/// `encodeURIComponent`.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The wrapper Heroic's `addNonSteamGameViaUrlHandler` writes.
fn wrapper_content(game: &GameId, spec: &ShortcutSpec) -> String {
    let icon = spec.icon.as_deref().map(|i| format!("Icon={}\n", escape_desktop_value(i))).unwrap_or_default();
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={} {}\nPath={}\n{icon}Terminal=false\nNoDisplay=true\n",
        escape_desktop_value(&game.title),
        spec.exe,
        spec.launch_options,
        escape_desktop_value(&spec.start_dir.replace('"', "")),
    )
}

fn remove_wrapper(wrapper_dir: &Path, game: &GameId) {
    let file = wrapper_dir.join(wrapper_file_name(game));
    if file.exists() {
        if let Err(e) = fs::remove_file(&file) {
            log::warn!("Failed to remove {} with: {e}", file.display());
        }
    }
}

/// Runs `steam <url>` and waits for it (bounded). It is not killed on
/// timeout: should the command have started Steam itself, killing it would
/// close the user's client.
fn hand_to_steam(bin: &Path, url: &str) -> Result<()> {
    let mut child = Command::new(bin)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("running {}", bin.display()))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("{} exited with {status}", bin.display());
            }
            return Ok(());
        }
        if started.elapsed() > HANDLER_TIMEOUT {
            bail!("{} did not finish in {} s", bin.display(), HANDLER_TIMEOUT.as_secs());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Adds `game` through `steam://addnonsteamgame/<wrapper>`, then waits for
/// Steam to flush the entry to learn its id and place the artwork. `Ok(None)`
/// when the entry did not show up in time (the shortcut still exists, without
/// artwork).
pub fn add_with_url_handler(h: &UrlHandler, steam: &Path, game: &GameId, spec: &ShortcutSpec, art: &ArtFiles) -> Result<Option<u32>> {
    fs::create_dir_all(&h.wrapper_dir).with_context(|| format!("creating {}", h.wrapper_dir.display()))?;
    let wrapper = h.wrapper_dir.join(wrapper_file_name(game));
    fs::write(&wrapper, wrapper_content(game, spec)).with_context(|| format!("writing {}", wrapper.display()))?;
    fs::write(&h.sentinel, "").with_context(|| format!("writing {}", h.sentinel.display()))?;
    let url = format!("steam://addnonsteamgame/{}", encode_uri_component(&wrapper.to_string_lossy()));
    hand_to_steam(&h.steam_bin, &url)?;
    log::info!("Added {} to Steam via the addnonsteamgame url handler.", game.title);

    for attempt in 0..h.poll_attempts {
        if let Some(appid) = steam::find_appid(steam, game) {
            art_for_all_users(steam, &GridIds::from_appid(appid), art, &game.title);
            return Ok(Some(appid));
        }
        if attempt + 1 < h.poll_attempts {
            std::thread::sleep(h.poll_interval);
        }
    }
    log::warn!("Could not find the Steam shortcut of {} to set its artwork.", game.title);
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steam_client::testing::start;
    use crate::vdf::{self, Document, Entry, Node};
    use serde_json::json;

    fn game() -> GameId {
        GameId { runner: "legendary".into(), app_name: "Quail".into(), title: "Cool Game".into() }
    }

    fn spec() -> ShortcutSpec {
        ShortcutSpec {
            exe: "\"/opt/heronite-rs/heronite-rs\"".into(),
            start_dir: "\"/opt/heronite-rs\"".into(),
            launch_options: "--no-gui \"heronite://launch?appName=Quail&runner=legendary\"".into(),
            icon: Some("/icons/Quail.png".into()),
            last_play_time: Some(1),
        }
    }

    fn steam_dir(root: &Path) -> PathBuf {
        let steam = root.join("Steam");
        fs::create_dir_all(steam.join("userdata/1234/config")).unwrap();
        fs::create_dir_all(steam.join("userdata/5678/config")).unwrap();
        steam
    }

    fn art(root: &Path) -> ArtFiles {
        let cover = root.join("cover.jpg");
        fs::write(&cover, b"\xff\xd8cover").unwrap();
        ArtFiles { cover: Some(cover), ..Default::default() }
    }

    #[test]
    fn uri_component_encoding_matches_javascript() {
        assert_eq!(encode_uri_component("/a b/é-_.!~*'()?&"), "%2Fa%20b%2F%C3%A9-_.!~*'()%3F%26");
    }

    #[test]
    fn client_add_sets_artwork_for_every_user() {
        let dir = tempfile::tempdir().unwrap();
        let steam = steam_dir(dir.path());
        let fake = start(Box::new(|expr| {
            if expr.contains("allApps") {
                json!([])
            } else if expr.contains("AddShortcut") {
                json!(3_000_000_001u64)
            } else {
                json!(true)
            }
        }));
        let client = SteamClient::new(&fake.addr);
        let already = add_with_client(&client, &steam, &game(), &spec(), &art(dir.path())).unwrap();
        assert!(!already);
        for user in ["1234", "5678"] {
            let grid = steam.join(format!("userdata/{user}/config/grid"));
            assert_eq!(fs::read(grid.join("3000000001p.jpg")).unwrap(), b"\xff\xd8cover");
            assert!(grid.join("3000000001_logo.jpg").exists());
        }
        let seen = fake.expressions.lock().unwrap().clone();
        assert!(seen[1].contains("AddShortcut(\"Cool Game\""));
        // Cover as the grid (0) and the transparent logo as a PNG (2).
        assert!(seen.iter().any(|e| e.contains("SetCustomArtworkForApp(3000000001,") && e.ends_with("\"jpg\", 0); return true })()")));
        assert!(seen.iter().any(|e| e.contains("\"png\", 2)")));
    }

    #[test]
    fn client_add_of_a_present_game_does_not_duplicate_it() {
        let dir = tempfile::tempdir().unwrap();
        let steam = steam_dir(dir.path());
        let fake = start(Box::new(|expr| {
            if expr.contains("allApps") {
                json!([{"appid": 3_000_000_001u64, "appName": "Cool Game", "exe": "", "launchOptions": "\"heronite://launch/legendary/Quail\""}])
            } else {
                json!(true)
            }
        }));
        let client = SteamClient::new(&fake.addr);
        assert!(add_with_client(&client, &steam, &game(), &spec(), &ArtFiles::default()).unwrap());
        assert!(is_added_with_client(&client, &game()).unwrap());
        assert!(!fake.expressions.lock().unwrap().iter().any(|e| e.contains("AddShortcut")));
    }

    #[test]
    fn client_remove_drops_the_shortcut_wrapper_and_artwork() {
        let dir = tempfile::tempdir().unwrap();
        let steam = steam_dir(dir.path());
        let wrappers = dir.path().join("wrappers");
        fs::create_dir_all(&wrappers).unwrap();
        fs::write(wrappers.join("legendary-Quail.desktop"), "x").unwrap();
        let grid = steam.join("userdata/1234/config/grid");
        fs::create_dir_all(&grid).unwrap();
        fs::write(grid.join("3000000001p.jpg"), "x").unwrap();
        let shortcut = format!("{}/legendary-Quail.desktop", wrappers.join("heronite-rs-steam-shortcuts").display());
        let fake = start(Box::new(move |expr| {
            if expr.contains("allApps") {
                json!([{"appid": 3_000_000_001u64, "appName": "Cool Game", "exe": shortcut, "launchOptions": ""}])
            } else {
                json!(true)
            }
        }));
        let client = SteamClient::new(&fake.addr);
        assert!(remove_with_client(&client, &steam, &game(), &wrappers).unwrap());
        assert!(!wrappers.join("legendary-Quail.desktop").exists());
        assert!(!grid.join("3000000001p.jpg").exists());
        assert!(fake.expressions.lock().unwrap().iter().any(|e| e.contains("RemoveShortcut(3000000001)")));

        let other = GameId { app_name: "Other".into(), ..game() };
        assert!(!remove_with_client(&client, &steam, &other, &wrappers).unwrap());
    }

    /// A `steam` that records its argument and an entry Steam would have written.
    #[test]
    fn url_handler_writes_the_wrapper_and_places_artwork() {
        let dir = tempfile::tempdir().unwrap();
        let steam = steam_dir(dir.path());
        let wrappers = dir.path().join("apps/heronite-rs-steam-shortcuts");
        let wrapper = wrappers.join("legendary-Quail.desktop");
        let log = dir.path().join("steam-args");
        let bin = dir.path().join("steam");
        fs::write(&bin, format!("#!/bin/sh\necho \"$@\" > '{}'\n", log.display())).unwrap();
        fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let entry = vec![
            Entry::new("appid", Node::Int(3_000_000_002)),
            Entry::new("AppName", Node::string("Cool Game")),
            Entry::new("Exe", Node::string("\"/opt/heronite-rs/heronite-rs\"")),
            Entry::new("ShortcutPath", Node::string(&wrapper.to_string_lossy())),
            Entry::new("LaunchOptions", Node::string("")),
        ];
        let mut doc = Document::empty_shortcuts();
        doc.get_mut("shortcuts").and_then(Node::as_obj_mut).unwrap().push(Entry::new("0", Node::Obj(entry)));
        fs::write(steam.join("userdata/1234/config/shortcuts.vdf"), vdf::serialize(&doc)).unwrap();

        let mut h = UrlHandler::new(&bin, &wrappers);
        h.sentinel = dir.path().join("addnonsteamgamefile");
        h.poll_attempts = 2;
        h.poll_interval = Duration::from_millis(10);
        let id = add_with_url_handler(&h, &steam, &game(), &spec(), &art(dir.path())).unwrap();
        assert_eq!(id, Some(3_000_000_002));
        assert!(h.sentinel.exists());
        let text = fs::read_to_string(&wrapper).unwrap();
        assert!(text.contains("Name=Cool Game\n"));
        assert!(text.contains("Exec=\"/opt/heronite-rs/heronite-rs\" --no-gui \"heronite://launch?appName=Quail&runner=legendary\"\n"));
        assert!(text.contains("Path=/opt/heronite-rs\n"));
        assert!(text.contains("NoDisplay=true"));
        let args = fs::read_to_string(&log).unwrap();
        assert_eq!(args.trim(), format!("steam://addnonsteamgame/{}", encode_uri_component(&wrapper.to_string_lossy())));
        assert!(steam.join("userdata/5678/config/grid/3000000002p.jpg").exists());
    }

    #[test]
    fn a_failing_steam_command_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let steam = steam_dir(dir.path());
        let mut h = UrlHandler::new("/bin/false", dir.path().join("w"));
        h.sentinel = dir.path().join("s");
        assert!(add_with_url_handler(&h, &steam, &game(), &spec(), &ArtFiles::default()).is_err());
    }
}
