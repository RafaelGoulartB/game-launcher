//! Shortcuts for games, mirroring Heroic's `backend/shortcuts/**`:
//!
//! * [`desktop`]: `.desktop` files on the Desktop folder and in the
//!   application menu (`~/.local/share/applications`).
//! * [`vdf`] + [`steam`]: the binary `shortcuts.vdf` of every Steam user
//!   ("Add to Steam" as a non-Steam game) and its `grid/` artwork.
//! * [`steam_client`] + [`steam_running`]: the same while Steam runs, through
//!   its CEF debugging interface or the `steam://addnonsteamgame` handler.
//!
//! Nothing here knows about GPUI or the library: callers pass plain data, and
//! every root (Desktop, applications, Steam) is a parameter so tests (and the
//! `HERONITE_*_DIR` overrides of the app) never touch the real ones.

pub mod desktop;
pub mod steam;
pub mod steam_client;
pub mod steam_running;
pub mod sweep;
pub mod vdf;

pub use sweep::sweep_stale_temps_at_startup;

/// Identity of a game for shortcuts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameId {
    pub runner: String,
    pub app_name: String,
    pub title: String,
}

/// Command line pieces of a launch request for this app.
///
/// `--launch <runner> <appName>` is what `.desktop` files use; Steam gets the
/// Heroic-compatible protocol URL (`heronite://launch?appName=..&runner=..`).
pub fn launch_url(scheme: &str, runner: &str, app_name: &str) -> String {
    format!("{scheme}://launch?appName={app_name}&runner={runner}")
}
