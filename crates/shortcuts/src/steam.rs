//! "Add to Steam": non-Steam shortcuts in `userdata/<id>/config/shortcuts.vdf`
//! plus the artwork in `config/grid/`. Port of Heroic's
//! `shortcuts/nonesteamgame/{nonesteamgame,steamhelper,steamProcess}.ts` for
//! the case where Steam is **not running** (Steam reads `shortcuts.vdf` at
//! start and rewrites it on exit, so a change made meanwhile is lost). A
//! running Steam goes through [`crate::steam_running`].

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use heronite_i18n::tr;

use crate::vdf::{self, Document, Entry, Node};
use crate::GameId;

/// 1x1 transparent PNG Heroic writes as the logo, so Steam hides the title.
pub const TRANSPARENT_LOGO_PNG: [u8; 68] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
    0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x18, 0x95, 0x63,
    0x60, 0x00, 0x02, 0x00, 0x00, 0x05, 0x00, 0x01, 0xd6, 0x9f, 0x16, 0xcd, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42,
    0x60, 0x82,
];

/// URL schemes whose `launch` links identify a shortcut as this game's: the
/// current one and its earlier name (shortcuts made before the rename).
/// Shortcuts made by Heroic (`heroic://`) are Heroic's and left alone.
const SCHEMES: [&str; 2] = ["heronite", "heronite-rs"];

// Ids (steam-rom-manager's generate-app-id)

/// zlib's CRC-32.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// `appid` stored in `shortcuts.vdf` (and the short grid id).
pub fn shortcut_appid(exe: &str, app_name: &str) -> u32 {
    crc32(format!("{exe}{app_name}").as_bytes()) | 0x8000_0000
}

/// Grid file ids of a shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridIds {
    /// `generateAppId`: Big Picture grid (`<id>.jpg`).
    pub big_picture: String,
    /// `generateShortAppId`: every other grid image.
    pub other: String,
}

impl GridIds {
    pub fn from_appid(appid: u32) -> Self {
        Self { big_picture: (((appid as u64) << 32) | 0x0200_0000).to_string(), other: appid.to_string() }
    }

    pub fn generate(exe: &str, app_name: &str) -> Self {
        Self::from_appid(shortcut_appid(exe, app_name))
    }
}

/// `defaultSteamPath` with the quotes Heroic strips.
pub fn clean_steam_path(raw: &str) -> PathBuf {
    PathBuf::from(raw.replace('\'', ""))
}

/// Best-effort "is the Steam client running" (`isSteamRunning`): pid files
/// next to / above the install folder, and Steam Deck game mode counts as
/// running.
pub fn is_steam_running(steam: &Path) -> bool {
    if std::env::var("XDG_CURRENT_DESKTOP").as_deref() == Ok("gamescope") {
        return true;
    }
    let mut files = Vec::new();
    if let Some(parent) = steam.parent() {
        files.push(parent.join("steam.pid"));
    }
    files.push(steam.join("steam.pid"));
    files.into_iter().any(|file| fs::read_to_string(&file).ok().and_then(|s| s.trim().parse::<i32>().ok()).is_some_and(pid_alive))
}

fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 only checks that the process exists.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// User folders under `<steam>/userdata` (not `0` or `ac`); the error text is
/// Heroic's.
pub fn user_folders(userdata: &Path) -> Result<Vec<String>, String> {
    if !userdata.exists() {
        return Err(tr!("{path} does not exist. Can't add/remove game to/from Steam!", path = userdata.display()));
    }
    let mut folders: Vec<String> = fs::read_dir(userdata)
        .map_err(|e| format!("{}: {e}", userdata.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "0" && n != "ac")
        .collect();
    folders.sort();
    if folders.is_empty() {
        return Err(tr!("{path} does not contain a valid user directory!", path = userdata.display()));
    }
    Ok(folders)
}

// shortcuts.vdf

fn key_eq(e: &Entry, key: &str) -> bool {
    e.key.eq_ignore_ascii_case(key.as_bytes())
}

fn field(entry: &[Entry], key: &str) -> Option<String> {
    entry.iter().find(|e| key_eq(e, key)).and_then(|e| e.value.as_str())
}

fn read_doc(file: &Path) -> Result<Document> {
    let bytes = fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    vdf::parse(&bytes).with_context(|| format!("parsing {}", file.display()))
}

/// Suffix of the copy of `shortcuts.vdf` made before every write.
pub const BACKUP_SUFFIX: &str = ".heronite-bak";

/// Writes `shortcuts.vdf`: the current file is copied to
/// `shortcuts.vdf.heronite-bak` (overwritten each time) first, then the new
/// content goes to a unique temp file that is fsynced and renamed over it.
fn write_doc(file: &Path, doc: &Document) -> Result<()> {
    if file.is_file() {
        let backup = PathBuf::from(format!("{}{BACKUP_SUFFIX}", file.display()));
        if let Err(e) = fs::copy(file, &backup) {
            // Without a backup the write could lose the user's shortcuts.
            return Err(e).with_context(|| format!("backing up {}", file.display()));
        }
    }
    heronite_data::temps::write_atomic(file, &vdf::serialize(doc), None).with_context(|| format!("writing {}", file.display()))
}

/// `checkIfShortcutObjectIsValid`: problems found (empty = valid).
pub fn validate(doc: &Document) -> Vec<String> {
    let mut errors = Vec::new();
    match doc.get("shortcuts") {
        None => errors.push("Could not find entry \"shortcuts\"!".to_string()),
        Some(Node::Obj(entries)) => {
            for entry in entries {
                let Some(fields) = entry.value.as_obj() else {
                    errors.push("Entry \"shortcuts\" is not an array!".to_string());
                    break;
                };
                for key in ["AppName", "Exe", "LaunchOptions"] {
                    if !fields.iter().any(|f| key_eq(f, key)) {
                        errors.push(format!("One of the game entries is missing the {key} parameter!"));
                    }
                }
            }
        }
        Some(_) => errors.push("Entry \"shortcuts\" is not an array!".to_string()),
    }
    errors
}

/// Folder (under the application menu folder) of the `.desktop` wrappers
/// added through `steam://addnonsteamgame`.
pub const WRAPPER_DIR: &str = "heronite-rs-steam-shortcuts";

/// `<runner>-<appName>.desktop`, Heroic's `steamShortcutWrapperPath` name.
pub fn wrapper_file_name(game: &GameId) -> String {
    format!("{}-{}.desktop", game.runner, game.app_name)
}

/// `matchesHeroicGame`: a launch link of `game`, or a wrapper of `game` as
/// the executable or shortcut path.
pub fn matches_shortcut(exe: &str, shortcut_path: &str, launch_options: &str, game: &GameId) -> bool {
    let name = wrapper_file_name(game);
    let tail = format!("{WRAPPER_DIR}/{name}");
    exe.contains(&tail) || shortcut_path.contains(&tail) || matches_game(launch_options, game)
}

/// Does a shortcut point at `game`? Matches this app's launch links in
/// Heroic's forms (`appName=X&runner=Y`, `launch/X"`, `launch/Y/X"`).
pub fn matches_game(launch_options: &str, game: &GameId) -> bool {
    SCHEMES.iter().any(|scheme| launch_options.contains(&format!("{scheme}://launch")))
        && (launch_options.contains(&format!("appName={}&runner={}", game.app_name, game.runner))
            || launch_options.contains(&format!("launch/{}\"", game.app_name))
            || launch_options.contains(&format!("launch/{}/{}\"", game.runner, game.app_name)))
}

fn find_entry(shortcuts: &[Entry], game: &GameId) -> Option<usize> {
    shortcuts.iter().position(|e| {
        e.value.as_obj().is_some_and(|f| {
            let get = |k: &str| field(f, k).unwrap_or_default();
            matches_shortcut(&get("Exe"), &get("ShortcutPath"), &get("LaunchOptions"), game)
        })
    })
}

fn shortcuts_mut(doc: &mut Document) -> Option<&mut Vec<Entry>> {
    doc.get_mut("shortcuts").and_then(Node::as_obj_mut)
}

fn int(key: &str, v: u32) -> Entry {
    Entry::new(key, Node::Int(v))
}

fn text(key: &str, v: &str) -> Entry {
    Entry::new(key, Node::string(v))
}

/// What goes into a new shortcut. `exe` and `start_dir` are stored as given
/// (Heroic quotes them: `"/path/heronite-rs"`).
#[derive(Debug, Clone)]
pub struct ShortcutSpec {
    pub exe: String,
    pub start_dir: String,
    pub launch_options: String,
    pub icon: Option<String>,
    /// Unix seconds; now when `None`.
    pub last_play_time: Option<u32>,
}

/// An entry laid out the way Steam writes its own.
pub fn build_entry(title: &str, spec: &ShortcutSpec) -> Vec<Entry> {
    let last_played =
        spec.last_play_time.unwrap_or_else(|| SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0));
    vec![
        int("appid", shortcut_appid(&spec.exe, title)),
        text("AppName", title),
        text("Exe", &spec.exe),
        text("StartDir", &spec.start_dir),
        text("icon", spec.icon.as_deref().unwrap_or("")),
        text("ShortcutPath", ""),
        text("LaunchOptions", &spec.launch_options),
        int("IsHidden", 0),
        int("AllowDesktopConfig", 1),
        int("AllowOverlay", 1),
        int("OpenVR", 0),
        int("Devkit", 0),
        text("DevkitGameID", ""),
        int("DevkitOverrideAppID", 0),
        int("LastPlayTime", last_played),
        text("FlatpakAppID", ""),
        Entry::new("tags", Node::Obj(Vec::new())),
    ]
}

fn next_index(shortcuts: &[Entry]) -> String {
    let max = shortcuts.iter().filter_map(|e| e.key_str().parse::<u64>().ok()).max();
    max.map(|m| m + 1).unwrap_or(0).to_string()
}

fn renumber(shortcuts: &mut [Entry]) {
    if shortcuts.iter().all(|e| e.key_str().parse::<u64>().is_ok()) {
        for (i, e) in shortcuts.iter_mut().enumerate() {
            e.key = i.to_string().into_bytes();
        }
    }
}

const COVER_SUFFIX: &str = "p.jpg";
const HERO_SUFFIX: &str = "_hero.jpg";
const LOGO_SUFFIX: &str = "_logo.jpg";
const PICTURE_EXT: &str = ".jpg";

/// Image files (already JPEG/PNG) to place in a user's `grid/` folder.
#[derive(Debug, Clone, Default)]
pub struct ArtFiles {
    /// `<id>p.jpg`: vertical cover (600x900).
    pub cover: Option<PathBuf>,
    /// `<id>.jpg`: wide header.
    pub header: Option<PathBuf>,
    /// `<id>_hero.jpg`: hero background.
    pub hero: Option<PathBuf>,
    /// `<id>_logo.jpg`; the transparent logo is written when `None`.
    pub logo: Option<PathBuf>,
}

/// `checkImageExistsAlready`: `.png` or `.jpg` with the same stem.
fn image_exists(path: &Path) -> bool {
    path.with_extension("png").exists() || path.with_extension("jpg").exists()
}

/// `prepareImagesForSteam`: copies the artwork into `grid/` unless an image
/// with that name is already there. Failures are returned, not fatal.
pub fn install_art(config_dir: &Path, ids: &GridIds, art: &ArtFiles) -> Vec<String> {
    let grid = config_dir.join("grid");
    let mut errors = Vec::new();
    if let Err(e) = fs::create_dir_all(&grid) {
        return vec![format!("creating {}: {e}", grid.display())];
    }
    let targets = [
        (grid.join(format!("{}{COVER_SUFFIX}", ids.other)), art.cover.as_ref()),
        (grid.join(format!("{}{PICTURE_EXT}", ids.other)), art.header.as_ref()),
        (grid.join(format!("{}{HERO_SUFFIX}", ids.other)), art.hero.as_ref()),
        (grid.join(format!("{}{LOGO_SUFFIX}", ids.other)), art.logo.as_ref()),
    ];
    for (i, (dest, src)) in targets.into_iter().enumerate() {
        if image_exists(&dest) {
            continue;
        }
        let result = match src {
            Some(src) => fs::copy(src, &dest).map(|_| ()).map_err(|e| format!("copying {} to {}: {e}", src.display(), dest.display())),
            None if i == 3 => fs::write(&dest, TRANSPARENT_LOGO_PNG).map_err(|e| format!("writing {}: {e}", dest.display())),
            None => continue,
        };
        if let Err(e) = result {
            errors.push(e);
        }
    }
    errors
}

/// `removeImagesFromSteam`.
pub fn remove_art(config_dir: &Path, ids: &GridIds) -> Vec<String> {
    let grid = config_dir.join("grid");
    if !grid.exists() {
        return Vec::new();
    }
    let files = [
        format!("{}{COVER_SUFFIX}", ids.other),
        format!("{}{PICTURE_EXT}", ids.other),
        format!("{}{HERO_SUFFIX}", ids.other),
        format!("{}{PICTURE_EXT}", ids.big_picture),
        format!("{}{LOGO_SUFFIX}", ids.other),
    ];
    let mut errors = Vec::new();
    for name in files {
        let path = grid.join(name);
        if image_exists(&path) && path.exists() {
            if let Err(e) = fs::remove_file(&path) {
                errors.push(format!("Removing of {} failed with:\n{e}", path.display()));
            }
        }
    }
    errors
}

/// Result of [`add_game`] / [`remove_game`] when at least one user changed
/// (or already had the right state).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Users whose `shortcuts.vdf` now has the wanted state.
    pub users_done: Vec<String>,
    /// Per-user problems (corrupted file, write failure). Non-empty with
    /// `users_done` non-empty means "partially done".
    pub errors: Vec<String>,
    /// Add: the game was already there for every changed user.
    pub already: bool,
}

fn ensure_not_running(steam: &Path, adding: bool, title: &str) -> Result<()> {
    if is_steam_running(steam) {
        bail!(if adding {
            tr!(
                "Can't add \"{title}\" to Steam while Steam is running. Close Steam and try again (Steam overwrites shortcuts.vdf when it exits).",
                title
            )
        } else {
            tr!(
                "Can't remove \"{title}\" from Steam while Steam is running. Close Steam and try again (Steam overwrites shortcuts.vdf when it exits).",
                title
            )
        });
    }
    Ok(())
}

/// Adds `game` to every Steam user (`addNonSteamGame`, Steam closed).
pub fn add_game(steam: &Path, game: &GameId, spec: &ShortcutSpec, art: &ArtFiles) -> Result<Report> {
    ensure_not_running(steam, true, &game.title)?;
    let userdata = steam.join("userdata");
    let folders = user_folders(&userdata).map_err(|e| anyhow!(e))?;

    let mut report = Report::default();
    let mut all_already = true;
    for folder in folders {
        let config_dir = userdata.join(&folder).join("config");
        let file = config_dir.join("shortcuts.vdf");
        let step = (|| -> Result<bool> {
            fs::create_dir_all(&config_dir)?;
            if !file.exists() {
                write_doc(&file, &Document::empty_shortcuts())?;
            }
            let mut doc = read_doc(&file)?;
            let problems = validate(&doc);
            if !problems.is_empty() {
                bail!(tr!(
                    "Can't add \"{title}\" to Steam user \"{folder}\". \"{file}\" is corrupted!\n{problems}",
                    title = game.title,
                    folder = &folder,
                    file = file.display(),
                    problems = problems.join("\n")
                ));
            }
            let shortcuts = shortcuts_mut(&mut doc).expect("validated");
            if find_entry(shortcuts, game).is_some() {
                return Ok(true);
            }
            let fields = build_entry(&game.title, spec);
            let appid = shortcut_appid(&spec.exe, &game.title);
            let key = next_index(shortcuts);
            shortcuts.push(Entry { key: key.into_bytes(), value: Node::Obj(fields) });
            // Artwork first, like Heroic: a failed image is logged, not fatal.
            for e in install_art(&config_dir, &GridIds::from_appid(appid), art) {
                log::warn!("steam artwork for {}: {e}", game.title);
            }
            write_doc(&file, &doc)?;
            Ok(false)
        })();
        match step {
            Ok(already) => {
                all_already &= already;
                report.users_done.push(folder);
            }
            Err(e) => report.errors.push(format!("{e:#}")),
        }
    }
    if report.users_done.is_empty() {
        bail!("{}", report.errors.join("\n"));
    }
    report.already = all_already;
    Ok(report)
}

/// Removes `game` from every Steam user (`removeNonSteamGame`, Steam closed).
/// A missing Steam folder counts as "already removed", as in Heroic.
pub fn remove_game(steam: &Path, game: &GameId) -> Result<Report> {
    ensure_not_running(steam, false, &game.title)?;
    let userdata = steam.join("userdata");
    let Ok(folders) = user_folders(&userdata) else { return Ok(Report::default()) };

    let mut report = Report::default();
    for folder in folders {
        let config_dir = userdata.join(&folder).join("config");
        let file = config_dir.join("shortcuts.vdf");
        if !file.exists() {
            continue;
        }
        let step = (|| -> Result<bool> {
            let mut doc = read_doc(&file)?;
            let problems = validate(&doc);
            if !problems.is_empty() {
                bail!(tr!(
                    "Can't remove \"{title}\" from Steam user \"{folder}\". \"{file}\" is corrupted!\n{problems}",
                    title = game.title,
                    folder = &folder,
                    file = file.display(),
                    problems = problems.join("\n")
                ));
            }
            let shortcuts = shortcuts_mut(&mut doc).expect("validated");
            let Some(index) = find_entry(shortcuts, game) else { return Ok(false) };
            let removed = shortcuts.remove(index);
            renumber(shortcuts);
            write_doc(&file, &doc)?;
            let fields = removed.value.as_obj().cloned().unwrap_or_default();
            let ids = match fields.iter().find(|e| key_eq(e, "appid")).map(|e| &e.value) {
                Some(Node::Int(id)) => GridIds::from_appid(*id),
                _ => GridIds::generate(&field(&fields, "Exe").unwrap_or_default(), &field(&fields, "AppName").unwrap_or_default()),
            };
            for e in remove_art(&config_dir, &ids) {
                log::warn!("steam artwork for {}: {e}", game.title);
            }
            Ok(true)
        })();
        match step {
            Ok(true) => report.users_done.push(folder),
            Ok(false) => {}
            Err(e) => report.errors.push(format!("{e:#}")),
        }
    }
    Ok(report)
}

/// `enableSteamClientApiOnNextStart`: creates the flag file that makes the
/// next Steam start open its debugging interface.
pub fn enable_client_api_on_next_start(steam: &Path) {
    let flag = steam.join(crate::steam_client::FLAG_FILE);
    if !flag.exists() {
        if let Err(e) = fs::write(&flag, "") {
            log::warn!("Failed to create {} with: {e}", flag.display());
        }
    }
}

/// The `appid` of `game`'s shortcut in the first user file that has one.
pub fn find_appid(steam: &Path, game: &GameId) -> Option<u32> {
    let userdata = steam.join("userdata");
    user_folders(&userdata).ok()?.iter().find_map(|folder| {
        let doc = read_doc(&userdata.join(folder).join("config").join("shortcuts.vdf")).ok()?;
        let shortcuts = doc.get("shortcuts").and_then(Node::as_obj)?;
        let fields = shortcuts.get(find_entry(shortcuts, game)?)?.value.as_obj()?;
        match fields.iter().find(|e| key_eq(e, "appid")).map(|e| &e.value) {
            Some(Node::Int(id)) => Some(*id),
            _ => None,
        }
    })
}

/// `isAddedToSteam` for at least one user (reads the files; never writes).
pub fn is_added(steam: &Path, game: &GameId) -> bool {
    let userdata = steam.join("userdata");
    let Ok(folders) = user_folders(&userdata) else { return false };
    folders.iter().any(|folder| {
        let file = userdata.join(folder).join("config").join("shortcuts.vdf");
        let Ok(doc) = read_doc(&file) else { return false };
        if !validate(&doc).is_empty() {
            return false;
        }
        doc.get("shortcuts").and_then(Node::as_obj).is_some_and(|s| find_entry(s, game).is_some())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> GameId {
        GameId { runner: "legendary".into(), app_name: "Quail".into(), title: "Cool Game".into() }
    }

    fn spec() -> ShortcutSpec {
        ShortcutSpec {
            exe: "\"/opt/heronite-rs/heronite-rs\"".into(),
            start_dir: "\"/opt/heronite-rs\"".into(),
            launch_options: "--no-gui \"heronite://launch?appName=Quail&runner=legendary\"".into(),
            icon: Some("/icons/Quail.png".into()),
            last_play_time: Some(1_700_000_000),
        }
    }

    /// A Steam folder with one user (`1234`) holding `vdf` as shortcuts.vdf.
    fn steam_with(vdf: Option<&[u8]>) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let steam = dir.path().join("Steam");
        let config = steam.join("userdata/1234/config");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(steam.join("userdata/0/config")).unwrap();
        let file = config.join("shortcuts.vdf");
        if let Some(v) = vdf {
            fs::write(&file, v).unwrap();
        }
        (dir, steam, file)
    }

    #[test]
    fn every_write_backs_up_the_previous_file_and_leaves_no_temp() {
        let original = vdf::serialize(&Document::empty_shortcuts());
        let (_d, steam, file) = steam_with(Some(&original));
        let backup = PathBuf::from(format!("{}{BACKUP_SUFFIX}", file.display()));
        let art = ArtFiles::default();
        add_game(&steam, &game(), &spec(), &art).unwrap();
        // The backup is the file as it was before the write.
        assert_eq!(fs::read(&backup).unwrap(), original);
        let with_game = fs::read(&file).unwrap();
        assert_ne!(with_game, original);
        // The next write overwrites it with the then-current content.
        remove_game(&steam, &game()).unwrap();
        assert_eq!(fs::read(&backup).unwrap(), with_game);
        let names: Vec<String> =
            fs::read_dir(file.parent().unwrap()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
    }

    #[test]
    fn logo_png_is_heroics() {
        let hex = "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000b494441541895636000020000050001d69f16cd0000000049454e44ae426082";
        let bytes: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        assert_eq!(bytes, TRANSPARENT_LOGO_PNG);
    }

    #[test]
    fn crc32_matches_zlib() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn ids_follow_steam_rom_manager() {
        // Values computed independently with Python's zlib.crc32.
        let id = shortcut_appid("\"/usr/share/discord/Discord\"", "Discord");
        assert_eq!(id, 3_916_147_392);
        let ids = GridIds::from_appid(id);
        assert_eq!(ids.other, "3916147392");
        assert_eq!(ids.big_picture, "16819724974989246464");
        assert_eq!(GridIds::generate("\"/usr/share/discord/Discord\"", "Discord"), ids);
        assert!(id & 0x8000_0000 != 0);
    }

    #[test]
    fn launch_option_matching() {
        let g = game();
        assert!(matches_game("--no-gui \"heronite://launch?appName=Quail&runner=legendary\"", &g));
        assert!(matches_game("\"heronite://launch/Quail\"", &g));
        assert!(matches_game("\"heronite://launch/legendary/Quail\"", &g));
        assert!(matches_game("--no-gui \"heronite-rs://launch?appName=Quail&runner=legendary\"", &g), "made before the rename");
        assert!(!matches_game("--no-gui --no-sandbox \"heroic://launch?appName=Quail&runner=legendary\"", &g), "Heroic's shortcut");
        assert!(!matches_game("--no-gui \"heronite://launch?appName=QuailX&runner=legendary\"", &g));
        assert!(!matches_game("--no-gui \"heronite://launch?appName=Quail&runner=gog\"", &g));
        assert!(!matches_game("something else Quail legendary", &g));
    }

    #[test]
    fn wrapper_shortcuts_match_by_path() {
        let g = game();
        let ours = "/home/u/.local/share/applications/heronite-rs-steam-shortcuts/legendary-Quail.desktop";
        let heroics = "/home/u/.local/share/applications/heroic-steam-shortcuts/legendary-Quail.desktop";
        assert!(matches_shortcut("", ours, "", &g));
        assert!(!matches_shortcut(&format!("\"{heroics}\""), "", "", &g), "Heroic's wrapper");
        assert!(!matches_shortcut("", "/x/heronite-rs-steam-shortcuts/legendary-QuailX.desktop", "", &g));
        assert!(!matches_shortcut("", "/x/other/legendary-Quail.desktop", "", &g));
        assert!(matches_shortcut("", "", "\"heronite://launch/Quail\"", &g));
    }

    #[test]
    fn add_remove_round_trip_on_a_copy_of_a_real_file() {
        let (_d, steam, file) = steam_with(Some(crate::vdf::tests_support::REAL));
        let before = fs::read(&file).unwrap();
        let art_dir = tempfile::tempdir().unwrap();
        let cover = art_dir.path().join("c.jpg");
        fs::write(&cover, b"cover").unwrap();
        let art = ArtFiles { cover: Some(cover), ..Default::default() };

        assert!(!is_added(&steam, &game()));
        let report = add_game(&steam, &game(), &spec(), &art).unwrap();
        assert_eq!(report.users_done, vec!["1234".to_string()]);
        assert!(!report.already);
        assert!(is_added(&steam, &game()));

        // Parses, keeps Steam's own entry first, appends ours at index 1.
        let doc = read_doc(&file).unwrap();
        let shortcuts = doc.get("shortcuts").unwrap().as_obj().unwrap();
        assert_eq!(shortcuts.len(), 2);
        assert_eq!(shortcuts[1].key, b"1");
        let ours = shortcuts[1].value.as_obj().unwrap();
        assert_eq!(field(ours, "AppName").as_deref(), Some("Cool Game"));
        assert_eq!(field(ours, "Exe").as_deref(), Some("\"/opt/heronite-rs/heronite-rs\""));
        assert_eq!(field(ours, "icon").as_deref(), Some("/icons/Quail.png"));
        let appid = match &ours.iter().find(|e| e.key == b"appid").unwrap().value {
            Node::Int(v) => *v,
            other => panic!("{other:?}"),
        };
        assert_eq!(appid, shortcut_appid("\"/opt/heronite-rs/heronite-rs\"", "Cool Game"));
        assert!(appid & 0x8000_0000 != 0);

        // Grid artwork.
        let grid = steam.join("userdata/1234/config/grid");
        let ids = GridIds::from_appid(appid);
        assert_eq!(fs::read(grid.join(format!("{}p.jpg", ids.other))).unwrap(), b"cover");
        assert_eq!(fs::read(grid.join(format!("{}_logo.jpg", ids.other))).unwrap(), TRANSPARENT_LOGO_PNG);
        assert!(!grid.join(format!("{}.jpg", ids.other)).exists(), "no header given");

        // Adding again is a no-op.
        let again = add_game(&steam, &game(), &spec(), &art).unwrap();
        assert!(again.already);
        assert_eq!(read_doc(&file).unwrap().get("shortcuts").unwrap().as_obj().unwrap().len(), 2);

        // Remove: the file goes back to the original bytes, artwork is gone.
        let removed = remove_game(&steam, &game()).unwrap();
        assert_eq!(removed.users_done, vec!["1234".to_string()]);
        assert_eq!(fs::read(&file).unwrap(), before);
        assert!(!is_added(&steam, &game()));
        assert!(!grid.join(format!("{}p.jpg", ids.other)).exists());
        assert!(!grid.join(format!("{}_logo.jpg", ids.other)).exists());
        // Removing again changes nothing.
        assert!(remove_game(&steam, &game()).unwrap().users_done.is_empty());
    }

    #[test]
    fn removal_renumbers_the_remaining_entries() {
        let (_d, steam, file) = steam_with(Some(crate::vdf::tests_support::REAL));
        let other = GameId { runner: "gog".into(), app_name: "g1".into(), title: "Other".into() };
        let mut spec2 = spec();
        spec2.launch_options = "--no-gui \"heronite://launch?appName=g1&runner=gog\"".into();
        add_game(&steam, &game(), &spec(), &ArtFiles::default()).unwrap();
        add_game(&steam, &other, &spec2, &ArtFiles::default()).unwrap();
        remove_game(&steam, &game()).unwrap();
        let doc = read_doc(&file).unwrap();
        let keys: Vec<_> = doc.get("shortcuts").unwrap().as_obj().unwrap().iter().map(|e| e.key_str()).collect();
        assert_eq!(keys, vec!["0", "1"]);
        assert!(is_added(&steam, &other));
        assert!(!is_added(&steam, &game()));
    }

    #[test]
    fn new_user_without_shortcuts_file_gets_one() {
        let (_d, steam, file) = steam_with(None);
        assert!(!file.exists());
        add_game(&steam, &game(), &spec(), &ArtFiles::default()).unwrap();
        assert!(is_added(&steam, &game()));
    }

    #[test]
    fn corrupted_file_is_reported_and_left_alone() {
        let bad = b"\x00shortcuts\x00\x000\x00\x01AppName\x00x\x00\x08\x08\x08";
        let (_d, steam, file) = steam_with(Some(bad));
        let err = add_game(&steam, &game(), &spec(), &ArtFiles::default()).unwrap_err();
        assert!(format!("{err:#}").contains("is corrupted"), "{err:#}");
        assert!(format!("{err:#}").contains("missing the Exe parameter"));
        assert_eq!(fs::read(&file).unwrap(), bad);
    }

    #[test]
    fn steam_running_blocks_changes() {
        let (_d, steam, file) = steam_with(Some(crate::vdf::tests_support::REAL));
        // A pid file with a live process (this test process).
        fs::write(steam.join("steam.pid"), std::process::id().to_string()).unwrap();
        assert!(is_steam_running(&steam));
        let err = add_game(&steam, &game(), &spec(), &ArtFiles::default()).unwrap_err();
        assert!(err.to_string().contains("Steam is running"));
        assert!(remove_game(&steam, &game()).is_err());
        assert_eq!(fs::read(&file).unwrap(), crate::vdf::tests_support::REAL);
        // A stale pid does not count.
        fs::write(steam.join("steam.pid"), "999999999").unwrap();
        assert!(!is_steam_running(&steam));
    }

    #[test]
    fn missing_userdata_errors_like_heroic() {
        let dir = tempfile::tempdir().unwrap();
        let err = add_game(dir.path(), &game(), &spec(), &ArtFiles::default()).unwrap_err();
        assert!(err.to_string().contains("does not exist. Can't add/remove game to/from Steam!"));
        // Removal treats it as already removed.
        assert!(remove_game(dir.path(), &game()).unwrap().users_done.is_empty());
        fs::create_dir_all(dir.path().join("userdata/0")).unwrap();
        assert!(add_game(dir.path(), &game(), &spec(), &ArtFiles::default())
            .unwrap_err()
            .to_string()
            .contains("does not contain a valid user directory"));
    }

    #[test]
    fn existing_artwork_is_not_overwritten() {
        let (_d, steam, _file) = steam_with(None);
        let config = steam.join("userdata/1234/config");
        let appid = shortcut_appid(&spec().exe, "Cool Game");
        let ids = GridIds::from_appid(appid);
        fs::create_dir_all(config.join("grid")).unwrap();
        fs::write(config.join("grid").join(format!("{}p.png", ids.other)), b"mine").unwrap();
        let src = steam.join("new.jpg");
        fs::write(&src, b"new").unwrap();
        let errors = install_art(&config, &ids, &ArtFiles { cover: Some(src), ..Default::default() });
        assert!(errors.is_empty());
        assert!(!config.join("grid").join(format!("{}p.jpg", ids.other)).exists());
    }
}
