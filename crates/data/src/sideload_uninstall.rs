//! Uninstalling a sideloaded ("Local") game the user installed by hand:
//! deletes its install folder and its own Wine/Proton prefix, then marks the
//! entry not installed. The game stays in the Collection (playtime, status
//! and art are kept); "Remove from Collection" is a separate action.
//!
//! Deleting folders picked from a path the user typed is risky, so both
//! targets go through checks first. The install folder must not be the
//! filesystem root, the home folder or a folder right under it, a mount
//! point, a Steam library, a default Heroic folder, or hold another game. The
//! prefix must belong to this game only: the shared prefix, the default
//! prefixes and a prefix another game also uses are always kept.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;
use heronite_i18n::{tr, trn};
use serde_json::Value;

use crate::game_config;
use crate::json_store;
use crate::library::Library;
use crate::paths::DataPaths;
use crate::sideload::check_prefix_removable;

/// A folder the uninstall would delete, or the reason it will not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub path: PathBuf,
    pub blocked: Option<String>,
}

impl Target {
    pub fn removable(&self) -> bool {
        self.blocked.is_none()
    }
}

/// What uninstalling a game would delete.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UninstallPlan {
    /// The install folder (`None`: no folder found on this computer).
    pub files: Option<Target>,
    /// The Wine prefix (`None`: not a Windows game, or no prefix on disk).
    pub prefix: Option<Target>,
}

/// What the uninstall deleted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UninstallOutcome {
    pub files_removed: Option<PathBuf>,
    pub prefix_removed: Option<PathBuf>,
}

/// Inputs of [`plan`] / [`uninstall`].
pub struct UninstallInput<'a> {
    pub app_name: &'a str,
    pub home: &'a str,
    /// The game's install folder (from its executable / `folder_name`).
    pub folder: Option<&'a Path>,
    /// The game runs through Wine/Proton (only then the prefix is offered).
    pub windows: bool,
    /// Paths of the other games in the library (install folders and
    /// executables): a folder holding one of them is never deleted.
    pub others: &'a [PathBuf],
}

pub fn plan(paths: &DataPaths, input: &UninstallInput) -> UninstallPlan {
    let home = Path::new(input.home);
    let global = game_config::load_global(paths, input.home);
    let text = |k: &str| global.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let defaults: Vec<PathBuf> = [
        text("defaultInstallPath"),
        game_config::shared_wine_prefix(input.home),
        game_config::default_wine_prefix_dir(input.home),
        text("defaultWinePrefix"),
        text("defaultWinePrefixDir"),
        text("winePrefix"),
    ]
    .into_iter()
    .filter(|p| !p.is_empty())
    .map(|p| PathBuf::from(game_config::expand_tilde(&p, input.home)))
    .collect();
    let other_prefixes: Vec<PathBuf> = other_game_prefixes(paths, input.app_name, input.home).into_iter().map(|(_, p)| p).collect();

    let prefix = if input.windows { own_prefix(paths, input.app_name, input.home) } else { None };
    let prefix = prefix.filter(|p| p.exists()).map(|path| {
        let blocked = prefix_blocked(&path, &defaults, &other_prefixes, input.others, home);
        Target { path, blocked }
    });

    let files = input.folder.map(|folder| {
        let mut taken: Vec<PathBuf> = defaults.clone();
        taken.extend(other_prefixes.iter().cloned());
        let blocked = folder_blocked(folder, &taken, input.others, home);
        Target { path: folder.to_path_buf(), blocked }
    });
    UninstallPlan { files, prefix }
}

/// Deletes what was asked for, then marks the game not installed. Every
/// target is checked again first; when one asked for is refused, nothing is
/// deleted and the reason is the error.
pub fn uninstall(paths: &DataPaths, input: &UninstallInput, delete_files: bool, delete_prefix: bool) -> Result<UninstallOutcome> {
    let plan = plan(paths, input);
    let files = if delete_files {
        let t = plan.files.ok_or_else(|| anyhow::anyhow!(tr!("The install folder was not found on this computer")))?;
        if let Some(why) = t.blocked {
            anyhow::bail!(why);
        }
        Some(t.path)
    } else {
        None
    };
    let prefix = match plan.prefix.filter(|_| delete_prefix) {
        Some(Target { blocked: Some(why), .. }) => anyhow::bail!(why),
        Some(t) => Some(t.path),
        None => None,
    };
    let remove =
        |p: &Path| std::fs::remove_dir_all(p).map_err(|e| anyhow::anyhow!(tr!("Could not delete {path}: {e}", path = p.display(), e)));
    if let Some(p) = &files {
        remove(p)?;
    }
    // The prefix may have held the game (`drive_c/...`) and be gone already.
    if let Some(p) = prefix.as_deref().filter(|p| p.exists()) {
        remove(p)?;
    }
    mark_uninstalled(paths, input.app_name)?;
    Ok(UninstallOutcome { files_removed: files, prefix_removed: prefix })
}

/// The folder to offer for deletion from the game's executable: its folder,
/// minus trailing binary subfolders (`Game/Binaries/Win64/game.exe` gives
/// `Game`, `Game/bin/x64/game.exe` gives `Game`).
pub fn guess_game_folder(executable: &Path) -> Option<PathBuf> {
    const BIN_DIRS: &[&str] =
        &["bin", "bin32", "bin64", "binaries", "x64", "x86", "x86_64", "win32", "win64", "windows", "linux", "release", "shipping"];
    let mut dir = executable.parent()?.to_path_buf();
    while dir.file_name().is_some_and(|n| BIN_DIRS.contains(&n.to_string_lossy().to_lowercase().as_str())) {
        match dir.parent() {
            Some(p) if p.components().count() > 2 => dir = p.to_path_buf(),
            _ => break,
        }
    }
    Some(dir)
}

/// Sets `is_installed: false` on the sideload entry (the rest is kept, so
/// "Edit game" can point it at a new executable later).
pub fn mark_uninstalled(paths: &DataPaths, app_name: &str) -> Result<()> {
    json_store::update_value(&paths.sideload_library(), |doc| {
        let list = doc.get_mut("games").and_then(Value::as_array_mut);
        let entry = list.and_then(|l| l.iter_mut().find(|g| g.get("app_name").and_then(Value::as_str) == Some(app_name)));
        match entry.and_then(Value::as_object_mut) {
            Some(g) => {
                g.insert("is_installed".into(), Value::Bool(false));
                Ok(())
            }
            None => anyhow::bail!(tr!("{app_name} is not in the local games library")),
        }
    })
}

/// The prefix the game's own settings point at (the default one when it has
/// none, which [`plan`] then refuses).
fn own_prefix(paths: &DataPaths, app_name: &str, home: &str) -> Option<PathBuf> {
    let settings = game_config::load_game(paths, app_name, home);
    settings.get("winePrefix").and_then(Value::as_str).filter(|p| !p.trim().is_empty()).map(PathBuf::from)
}

/// `(app name, winePrefix)` set in the config file of every other game.
fn other_game_prefixes(paths: &DataPaths, app_name: &str, home: &str) -> Vec<(String, PathBuf)> {
    let Ok(dir) = std::fs::read_dir(paths.games_config_dir()) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in dir.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(app) = name.strip_suffix(".json") else { continue };
        if app == app_name {
            continue;
        }
        let Ok(doc) = json_store::read_value(&entry.path()) else { continue };
        if let Some(p) = doc.get(app).and_then(|s| s.get("winePrefix")).and_then(Value::as_str) {
            if !p.trim().is_empty() {
                out.push((app.to_string(), PathBuf::from(game_config::expand_tilde(p, home))));
            }
        }
    }
    out
}

/// Why `prefix` must be kept when `app_name` goes: other games have their
/// prefix there (or inside it), so deleting it deletes theirs. `None` when
/// no other game uses it. Games are named by their title in `lib`; settings
/// of games `lib` leaves out (hidden ones) still count, unnamed.
pub fn prefix_shared_note(paths: &DataPaths, lib: &Library, app_name: &str, prefix: &Path, home: &str) -> Option<String> {
    let users: Vec<String> = other_game_prefixes(paths, app_name, home)
        .into_iter()
        .filter(|(_, p)| inside(p, prefix) || same_path(p, prefix))
        .map(|(app, _)| app)
        .collect();
    if users.is_empty() {
        return None;
    }
    let mut titles: Vec<&str> =
        users.iter().filter_map(|app| lib.games.iter().find(|g| &g.app_name == app)).map(|g| g.title.as_str()).collect();
    titles.sort_by_key(|t| t.to_lowercase());
    titles.dedup();
    let shown = titles.len().min(3);
    let rest = users.len() - shown;
    if shown == 0 {
        return Some(trn!(rest, "Also used by {n} other game, so it is kept.", "Also used by {n} other games, so it is kept."));
    }
    let mut games = titles[..shown].join(", ");
    if rest > 0 {
        games = trn!(rest, "{games} and {n} other game", "{games} and {n} other games", games);
    }
    Some(tr!("Also used by {games}, so it is kept.", games))
}

fn prefix_blocked(prefix: &Path, defaults: &[PathBuf], other_prefixes: &[PathBuf], others: &[PathBuf], home: &Path) -> Option<String> {
    if !is_clean_absolute(prefix) {
        return Some(tr!("Can't delete {path}: not a full path", path = prefix.display()));
    }
    if defaults.iter().any(|d| same_path(d, prefix)) {
        return Some(tr!("{path} is the shared or default prefix, so it is kept", path = prefix.display()));
    }
    if other_prefixes.iter().any(|p| same_path(p, prefix)) {
        return Some(tr!("{path} is also used by another game, so it is kept", path = prefix.display()));
    }
    if other_prefixes.iter().chain(others).chain(defaults).any(|p| inside(p, prefix)) {
        return Some(tr!("{path} holds another game or a default folder, so it is kept", path = prefix.display()));
    }
    check_prefix_removable(prefix, &[], home).err()
}

fn folder_blocked(folder: &Path, taken: &[PathBuf], others: &[PathBuf], home: &Path) -> Option<String> {
    let shown = folder.display();
    if !is_clean_absolute(folder) {
        return Some(tr!("Can't delete {path}: not a full path", path = shown));
    }
    let Ok(meta) = std::fs::symlink_metadata(folder) else {
        return Some(tr!("{path} doesn't exist", path = shown));
    };
    if meta.file_type().is_symlink() {
        return Some(tr!("Can't delete {path}: it is a link. Delete it by hand", path = shown));
    }
    if !meta.is_dir() {
        return Some(tr!("Can't delete {path}: not a folder", path = shown));
    }
    if folder.parent().is_none() || inside(home, folder) {
        return Some(tr!("Won't delete {path}: it contains your home folder", path = shown));
    }
    if folder.parent() == Some(home) || folder.components().count() < 3 {
        return Some(tr!("Won't delete {path}: it is a top-level folder. Delete the game by hand", path = shown));
    }
    if is_mount_point(folder) {
        return Some(tr!("Won't delete {path}: it is a whole drive", path = shown));
    }
    if folder.join("steamapps").exists() || folder.file_name().is_some_and(|n| n == "steamapps" || n == "common") {
        return Some(tr!("Won't delete {path}: it is a Steam library", path = shown));
    }
    if taken.iter().any(|p| inside(p, folder)) {
        return Some(tr!("Won't delete {path}: it is or holds a default folder or another game's prefix", path = shown));
    }
    if others.iter().any(|p| inside(p, folder)) {
        return Some(tr!("Won't delete {path}: another game is installed in it", path = shown));
    }
    None
}

/// `path` is `dir` or below it.
fn inside(path: &Path, dir: &Path) -> bool {
    !path.as_os_str().is_empty() && path.starts_with(dir)
}

fn same_path(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// Absolute and without `.` / `..` parts, so `starts_with` checks hold.
fn is_clean_absolute(p: &Path) -> bool {
    p.is_absolute() && p.components().all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}

#[cfg(unix)]
fn is_mount_point(p: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(p), p.parent().map(std::fs::metadata)) {
        (Ok(a), Some(Ok(b))) => a.dev() != b.dev(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn is_mount_point(_: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    struct Env {
        _dir: tempfile::TempDir,
        paths: DataPaths,
        home: PathBuf,
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heroic"));
        let home = dir.path().join("home");
        fs::create_dir_all(home.join("Games")).unwrap();
        let lib = json!({"games": [
            {"runner": "sideload", "app_name": "a", "title": "A", "is_installed": true, "install": {"executable": "x"}},
            {"runner": "sideload", "app_name": "b", "title": "B", "is_installed": true, "install": {}}
        ]});
        json_store::write(&paths.sideload_library(), &lib).unwrap();
        Env { _dir: dir, paths, home }
    }

    impl Env {
        fn home(&self) -> &str {
            self.home.to_str().unwrap()
        }
        fn mkdir(&self, rel: &str) -> PathBuf {
            let p = self.home.join(rel);
            fs::create_dir_all(&p).unwrap();
            p
        }
        fn set_prefix(&self, app: &str, prefix: &Path) {
            game_config::set_game_setting(&self.paths, app, self.home(), "winePrefix", json!(prefix.to_str().unwrap())).unwrap();
        }
        fn plan(&self, folder: Option<&Path>, others: &[PathBuf]) -> UninstallPlan {
            plan(&self.paths, &UninstallInput { app_name: "a", home: self.home(), folder, windows: true, others })
        }
    }

    #[test]
    fn deletes_folder_and_own_prefix_then_marks_uninstalled() {
        let e = env();
        let game = e.mkdir("Games/A");
        fs::write(game.join("a.exe"), "x").unwrap();
        let prefix = e.mkdir("Prefixes/A/pfx");
        let prefix = prefix.parent().unwrap().to_path_buf();
        e.set_prefix("a", &prefix);
        let input = UninstallInput { app_name: "a", home: e.home(), folder: Some(&game), windows: true, others: &[] };
        let out = uninstall(&e.paths, &input, true, true).unwrap();
        assert_eq!(out.files_removed.as_deref(), Some(game.as_path()));
        assert_eq!(out.prefix_removed.as_deref(), Some(prefix.as_path()));
        assert!(!game.exists() && !prefix.exists());
        let doc = json_store::read_value(&e.paths.sideload_library()).unwrap();
        assert_eq!(doc["games"][0]["is_installed"], false);
        assert_eq!(doc["games"][1]["is_installed"], true);
    }

    #[test]
    fn shared_and_default_prefixes_are_kept() {
        let e = env();
        let game = e.mkdir("Games/A");
        // No prefix of its own: the shared one.
        let shared = PathBuf::from(game_config::shared_wine_prefix(e.home()));
        fs::create_dir_all(shared.join("pfx")).unwrap();
        let plan = e.plan(Some(&game), &[]);
        let prefix = plan.prefix.unwrap();
        assert_eq!(prefix.path, shared);
        assert!(!prefix.removable());
        assert!(plan.files.unwrap().removable());

        let input = UninstallInput { app_name: "a", home: e.home(), folder: Some(&game), windows: true, others: &[] };
        let err = uninstall(&e.paths, &input, true, true).unwrap_err();
        assert!(err.to_string().contains("shared or default"));
        assert!(shared.exists() && game.exists(), "nothing deleted when a target is refused");
        let doc = json_store::read_value(&e.paths.sideload_library()).unwrap();
        assert_eq!(doc["games"][0]["is_installed"], true);
    }

    #[test]
    fn prefix_used_by_another_game_is_kept() {
        let e = env();
        let prefix = e.mkdir("Prefixes/Both");
        fs::create_dir_all(prefix.join("drive_c")).unwrap();
        e.set_prefix("a", &prefix);
        e.set_prefix("b", &prefix);
        let p = e.plan(None, &[]).prefix.unwrap();
        assert!(p.blocked.unwrap().contains("another game"));
    }

    #[test]
    fn linux_games_have_no_prefix_target() {
        let e = env();
        let prefix = e.mkdir("Prefixes/A");
        fs::create_dir_all(prefix.join("pfx")).unwrap();
        e.set_prefix("a", &prefix);
        let p = plan(&e.paths, &UninstallInput { app_name: "a", home: e.home(), folder: None, windows: false, others: &[] });
        assert_eq!(p.prefix, None);
    }

    #[test]
    fn dangerous_folders_are_refused() {
        let e = env();
        let blocked = |p: &Path, others: &[PathBuf]| e.plan(Some(p), others).files.unwrap().blocked;
        assert!(blocked(Path::new("/"), &[]).is_some());
        assert!(blocked(&e.home, &[]).is_some());
        assert!(blocked(e.home.parent().unwrap(), &[]).is_some());
        assert!(blocked(&e.home.join("Games"), &[]).is_some(), "folder right under home");
        assert!(blocked(&e.home.join("Games/../Games/A"), &[]).is_some(), "parent components");
        assert!(blocked(&e.home.join("Games/Missing"), &[]).is_some());

        let lib = e.mkdir("Games/SteamLib");
        fs::create_dir_all(lib.join("steamapps")).unwrap();
        assert!(blocked(&lib, &[]).is_some());

        let default_install = e.mkdir("Games/Heronite");
        assert!(blocked(&default_install, &[]).is_some(), "default install path");
        assert!(e.plan(Some(&e.mkdir("Games/Heronite/A")), &[]).files.unwrap().removable());

        let pack = e.mkdir("Games/Pack");
        let other = e.mkdir("Games/Pack/Other").join("other.exe");
        assert!(blocked(&pack, &[other]).unwrap().contains("another game"));
    }

    #[test]
    fn game_folder_skips_binary_subfolders() {
        let g = |p: &str| guess_game_folder(Path::new(p)).unwrap();
        assert_eq!(g("/home/u/Games/A/a.exe"), PathBuf::from("/home/u/Games/A"));
        assert_eq!(g("/home/u/Games/A/Binaries/Win64/a.exe"), PathBuf::from("/home/u/Games/A"));
        assert_eq!(g("/home/u/Games/A/bin/x64/a.exe"), PathBuf::from("/home/u/Games/A"));
        assert_eq!(g("/bin/a"), PathBuf::from("/bin"));
    }

    #[test]
    fn a_link_is_not_followed() {
        let e = env();
        let real = e.mkdir("Games/Real");
        let link = e.home.join("Games/Link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(e.plan(Some(&link), &[]).files.unwrap().blocked.unwrap().contains("link"));
    }

    #[test]
    fn shared_prefix_note_names_the_other_games() {
        let e = env();
        let lib = |e: &Env| Library::load(&e.paths).unwrap();
        let prefix = e.mkdir("Prefixes/A");
        e.set_prefix("a", &prefix);
        assert_eq!(prefix_shared_note(&e.paths, &lib(&e), "a", &prefix, e.home()), None);

        e.set_prefix("b", &prefix.join("pfx"));
        assert_eq!(prefix_shared_note(&e.paths, &lib(&e), "a", &prefix, e.home()).unwrap(), "Also used by B, so it is kept.");

        // A game the library leaves out (hidden) still keeps the prefix.
        e.set_prefix("gone", &prefix);
        assert_eq!(
            prefix_shared_note(&e.paths, &lib(&e), "a", &prefix, e.home()).unwrap(),
            "Also used by B and 1 other game, so it is kept."
        );
        e.set_prefix("b", &e.mkdir("Prefixes/B"));
        assert_eq!(prefix_shared_note(&e.paths, &lib(&e), "a", &prefix, e.home()).unwrap(), "Also used by 1 other game, so it is kept.");
    }
}
