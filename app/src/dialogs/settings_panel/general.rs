//! "General" tab of the global settings: the subset of Heroic's general and
//! advanced settings that matter to this app.

use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::Value;

use super::parts::{column, heading, help, rows, Rows};
use super::{Section, SettingsPanel, Target};
use crate::icons::Icon;
use crate::theme;
use crate::ui::fields::choose_path;
use crate::ui::icon;
use heronite_data::{AppPaths, DataPaths};
use heronite_i18n::{tr, Lang, Preference};

/// What the global ("General") panel shows. The Settings page hosts one
/// panel and switches between these; the Game defaults use the other tabs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GlobalView {
    #[default]
    General,
    Downloads,
    Advanced,
}

impl SettingsPanel {
    /// A panel bound to the global settings showing `view`.
    pub fn new_global(paths: DataPaths, view: GlobalView, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::new(paths, Target::Defaults, Section::General, window, cx);
        this.global_view = view;
        this
    }

    /// Switches between the global views (closing open menus).
    pub fn set_global_view(&mut self, view: GlobalView, cx: &mut Context<Self>) {
        if self.global_view != view {
            self.global_view = view;
            self.open_select = None;
            self.notice = None;
            cx.notify();
        }
    }

    pub(super) fn general_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        match self.global_view {
            GlobalView::General => self.general_view(window, cx),
            GlobalView::Downloads => self.downloads_view(cx),
            GlobalView::Advanced => self.advanced_view(cx),
        }
    }

    /// Store downloads (Epic, GOG, Amazon Games) and game updates.
    fn downloads_view(&mut self, cx: &mut Context<Self>) -> Div {
        let fields = rows()
            .field_help(
                self.max_workers_field(cx),
                vec![help(tr!("More workers download faster but use more CPU, memory and disk. \"Max\" uses every core."))],
            )
            .field_help(
                self.path_row(
                    "altLegendaryBin",
                    Some(tr!("Choose an Alternative Legendary Binary")),
                    false,
                    tr!("Select Legendary Binary"),
                    true,
                    cx,
                ),
                vec![help(tr!("Leave it empty to use the one in the data folder (tools/bin/legendary)."))],
            )
            .field_help(
                self.path_row("altGogdlBin", Some(tr!("Choose an Alternative gogdl Binary")), false, tr!("Select gogdl Binary"), true, cx),
                vec![help(tr!("Leave it empty to use the one in the data folder (tools/bin/gogdl)."))],
            )
            .field_help(
                self.path_row("altNileBin", Some(tr!("Choose an Alternative nile Binary")), false, tr!("Select nile Binary"), true, cx),
                vec![help(tr!("Leave it empty to use the one in the data folder (tools/bin/nile)."))],
            );
        let toggles = rows()
            .child(self.toggle_row(
                "download-no-https",
                "downloadNoHttps",
                false,
                tr!("Download games without HTTPS (useful for CDNs e.g. LanCache)"),
                None,
                false,
                cx,
            ))
            .child(self.toggle_row(
                "check-updates",
                "checkForUpdatesOnStartup",
                true,
                tr!("Check for game updates at startup"),
                Some(tr!("Off, updates are only looked for when you refresh a store's library. Automatic updates still check on their own.")),
                false,
                cx,
            ))
            .child(self.toggle_row(
                "auto-update",
                "autoUpdateGames",
                false,
                tr!("Automatically update games"),
                Some(tr!("Updates are queued in the background when they are found, except offline, on a metered connection or while the game is running.")),
                false,
                cx,
            ));
        column().child(heading(tr!("Downloads"))).child(fields.card()).child(heading(tr!("Updates"))).child(toggles.card())
    }

    /// Logging and advanced switches (folders and cache live in the page).
    fn advanced_view(&mut self, cx: &mut Context<Self>) -> Div {
        let r = rows()
            .child(self.toggle_row(
                "disable-logs",
                "disableLogs",
                false,
                tr!("Disable Logs"),
                Some(tr!("Toggle this switch ON to disable most of the writes to log files (critical information is always logged). Make sure to turn OFF this setting before reporting any issue.")),
                false,
                cx,
            ));
        column().child(heading(tr!("Logging"))).child(r.card())
    }

    fn general_view(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let folders = rows()
            .field(self.path_row(
                "defaultInstallPath",
                Some(tr!("Default Installation Path")),
                true,
                tr!("Choose the default installation folder"),
                false,
                cx,
            ))
            .field(self.path_row(
                "defaultWinePrefixDir",
                Some(tr!("Set Folder for new Wine Prefixes")),
                true,
                tr!("Select a Folder for new Wine Prefixes"),
                false,
                cx,
            ))
            .field_help(
                self.path_row("defaultSteamPath", Some(tr!("Default Steam path")), true, tr!("Steam path."), false, cx),
                vec![help(tr!(
                    "This path lets Heronite determine what version of Proton Steam uses, for adding non-Steam games to Steam."
                ))],
            );
        let wine = rows()
            .child(self.toggle_row(
                "valve-proton",
                "showValveProton",
                false,
                tr!("Allow using Valve Proton builds to run games"),
                Some(tr!("We recommend custom Proton forks (GE-Proton, Proton-CachyOS, etc.) to be used because Valve's Proton builds lack winetricks and protofixes. You can still use them if you want.")),
                false,
                cx,
            ))
            .child(self.toggle_row(
                "proton-to-steam",
                "downloadProtonToSteam",
                false,
                tr!("Download GE-Proton to Steam directory"),
                Some(tr!("When enabled, GE-Proton will be downloaded directly to the Steam compatibility tools directory instead of the tools folder. It will use the Steam path set in the 'Default Steam path' setting above.")),
                false,
                cx,
            ));
        column()
            .child(heading(tr!("Language")))
            .child(rows().field(self.language_field(cx)).card())
            .child(heading(tr!("Folders")))
            .child(folders.card())
            .child(heading(tr!("Startup")))
            .child(self.startup_rows(cx).card())
            .child(heading(tr!("System Tray")))
            .child(self.tray_rows(cx).card())
            .child(heading(tr!("When a game starts")))
            .child(self.game_start_rows(cx).card())
            .child(heading(tr!("Game Shortcuts")))
            .child(self.shortcut_rows(cx).card())
            .child(heading(tr!("Wine and Proton")))
            .child(wine.card())
            .child(heading(tr!("Custom Wine/Proton Paths")))
            .child(self.custom_wine_paths(cx))
    }

    /// Starting with the session and without the window (`system/`).
    fn startup_rows(&self, cx: &mut Context<Self>) -> Rows {
        let no_tray = self.flag("noTrayIcon", false);
        let with_system = self.flag("startWithSystem", false);
        rows()
            .child(self.toggle_row(
                "start-with-system",
                "startWithSystem",
                false,
                tr!("Start with the system"),
                Some(tr!("Opens Heronite when you log in.")),
                false,
                cx,
            ))
            .child(self.toggle_row(
                "start-in-tray",
                "startInTray",
                false,
                tr!("Start Minimized"),
                Some(tr!("When Heronite opens with the system, it stays in the tray without the window.")),
                !with_system || no_tray,
                cx,
            ))
    }

    fn tray_rows(&self, cx: &mut Context<Self>) -> Rows {
        let no_tray = self.flag("noTrayIcon", false);
        rows()
            .child(self.toggle_row("exit-to-tray", "exitToTray", false, tr!("Exit to System Tray"), None, no_tray, cx))
            .child(self.toggle_row("dark-tray", "darkTrayIcon", false, tr!("Use Dark Tray Icon (needs restart)"), None, no_tray, cx))
            .child(self.toggle_row("no-tray", "noTrayIcon", false, tr!("Hide System Tray Icon (requires restart)"), None, false, cx))
    }

    fn game_start_rows(&self, cx: &mut Context<Self>) -> Rows {
        rows().child(self.toggle_row("minimize", "minimizeOnLaunch", false, tr!("Minimize after game launch"), None, false, cx)).child(
            self.toggle_row(
                "hide-on-protocol",
                "hideWindowOnProtocolLaunch",
                false,
                tr!("Hide Heronite window when launching games from heronite:// links"),
                Some(tr!("Keeps the window hidden when a game is launched from an external shortcut, like the 'Add to Steam' feature.")),
                false,
                cx,
            ),
        )
    }

    /// Shortcuts made when a game is added (`system/shortcuts.rs`).
    fn shortcut_rows(&self, cx: &mut Context<Self>) -> Rows {
        rows()
            .child(self.toggle_row(
                "shortcuts-desktop",
                "addDesktopShortcuts",
                false,
                tr!("Add desktop shortcuts automatically"),
                None,
                false,
                cx,
            ))
            .child(self.toggle_row(
                "shortcuts-menu",
                "addStartMenuShortcuts",
                false,
                tr!("Add games to start menu automatically"),
                None,
                false,
                cx,
            ))
            .child(self.toggle_row("shortcuts-steam", "addSteamShortcuts", false, tr!("Add games to Steam automatically"), None, false, cx))
    }

    /// Interface language: the system's (English when it is not one we
    /// have) or a fixed one. Applies at once.
    fn language_field(&self, cx: &mut Context<Self>) -> Div {
        let app_paths = AppPaths::for_data(&self.paths);
        let (choices, names, selected) = language_options(&app_paths);
        let items = names.iter().enumerate().map(|(i, n)| (n.clone(), i == selected)).collect();
        let shown = names[selected].to_string();
        div().child(self.select_row("language", Some(tr!("Interface language")), shown, items, false, cx, move |_, i, _, cx| {
            if let Some(&p) = choices.get(i) {
                crate::language::choose(&app_paths, p, cx);
            }
        }))
    }

    fn max_workers_field(&self, cx: &mut Context<Self>) -> Div {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let current = self.number("maxWorkers").unwrap_or(0).max(0) as usize;
        let mut items: Vec<(SharedString, bool)> = vec![(tr!("Max").into(), current == 0)];
        items.extend((1..=cores).map(|n| (SharedString::from(n.to_string()), n == current)));
        let shown = if current == 0 { tr!("Max").to_string() } else { current.to_string() };
        div().child(self.select_row(
            "max-workers",
            Some(tr!("Maximum Number of Workers when downloading")),
            shown,
            items,
            false,
            cx,
            |this, i, _, cx| this.set("maxWorkers", serde_json::json!(i as i64), cx),
        ))
    }

    fn custom_wine_paths(&mut self, cx: &mut Context<Self>) -> Div {
        let paths: Vec<String> = self.custom_paths();
        let mut card_rows = rows();
        for (i, p) in paths.iter().enumerate() {
            card_rows = card_rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .py(px(8.))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(theme::text_primary()).child(p.clone()))
                    .child(
                        div()
                            .id(SharedString::from(format!("wine-path-del-{i}")))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.))
                            .rounded_full()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::white_a(0.10)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let mut now = this.custom_paths();
                                if i < now.len() {
                                    now.remove(i);
                                    this.set("customWinePaths", serde_json::json!(now), cx);
                                }
                            }))
                            .child(icon(Icon::DeleteOutline, px(16.), theme::text_muted())),
                    ),
            );
        }
        let add = self.outline_button("wine-path-add", tr!("Add path"), true, cx, |_, window, cx| {
            choose_path(window, cx, false, tr!("Select the Wine or Proton Binary"), |this: &mut Self, path, _, cx| {
                let mut now = this.custom_paths();
                if !now.contains(&path) {
                    now.push(path);
                    this.set("customWinePaths", serde_json::json!(now), cx);
                }
            });
        });
        card_rows = card_rows.child(div().flex().py(px(10.)).child(add));
        card_rows.card()
    }

    pub(super) fn custom_paths(&self) -> Vec<String> {
        self.settings
            .get("customWinePaths")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }
}

/// The language choices (system first), their names and the saved one.
pub(super) fn language_options(app_paths: &AppPaths) -> (Vec<Preference>, Vec<SharedString>, usize) {
    let pref = heronite_data::language::load(app_paths);
    let mut choices = vec![Preference::System];
    choices.extend(Lang::ALL.map(Preference::Fixed));
    let names = choices
        .iter()
        .map(|&p| match p {
            Preference::System => tr!("System default ({language})", language = heronite_i18n::system_lang().native_name()).into(),
            Preference::Fixed(l) => l.native_name().into(),
        })
        .collect();
    let selected = choices.iter().position(|&p| p == pref).unwrap_or(0);
    (choices, names, selected)
}
