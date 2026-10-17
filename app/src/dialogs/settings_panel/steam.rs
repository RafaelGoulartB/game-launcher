//! Steam games. Steam keeps launching them with its own Proton and prefix;
//! these tabs read and change Steam's settings for the game (its launch
//! options and compatibility tool, `heronite_steam`) instead of this app's
//! game config. The panel's settings map holds the launch options as fields
//! (`heronite_steam::options`), and every change rebuilds the line and hands
//! it to Steam.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::services::steam_library::steam;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::settings::{GameSettings, WineVersion};
use heronite_launch::wine_tools::WineTool;
use heronite_steam::{options, wrap, Access, Change, Snapshot, ToolGroup};
use serde_json::Value;

use super::advanced::{ENV, WRAPPERS};
use super::logic::launcher_args_error;
use super::parts::{column, danger_text, heading, help, hint, rows};
use super::protondb::{best_ge, runs_well, ProtonDb, Recommendation};
use super::{Notice, SettingsPanel};
use crate::ui::fields::{choose_path, SelectOption};

/// How long Steam may run without its client answering before the panel
/// reads it anyway (it then shows why nothing can change).
const CLIENT_WAIT: Duration = Duration::from_secs(20);

/// The "Restart Steam" button: idle, asking to confirm, or restarting.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Restart {
    #[default]
    Idle,
    Confirm,
    Running(Instant),
}

/// One entry of the compatibility tool menu.
pub(super) struct ToolOption {
    pub(super) label: String,
    /// What Steam stores (`""`: its default).
    pub(super) name: String,
    pub(super) group: Option<&'static str>,
    /// "last used".
    pub(super) note: Option<&'static str>,
}

/// What the panel knows of the game in Steam.
pub(super) struct SteamState {
    pub(super) app_id: String,
    /// `None` while Steam is read.
    pub(super) snapshot: Option<Result<Snapshot, String>>,
    /// The launch options as Steam has them (or as last sent).
    pub(super) line: String,
    /// `false`: the line uses shell syntax; only the text field edits it.
    pub(super) editable: bool,
    /// The inputs still show values from before the last read or change.
    inputs_stale: bool,
    pending: Change,
    writing: bool,
    pub(super) restart: Restart,
    /// Last `(running, client answers)` of Steam, to read again when it changes.
    seen: Option<(bool, bool)>,
    /// Since when Steam runs without its client answering yet.
    starting_since: Option<Instant>,
    /// Steam opened or closed: read it again once no change is on its way.
    reload_wanted: bool,
    /// Steam can be restarted from here (not in a gamescope session).
    pub(super) can_restart: bool,
}

impl SteamState {
    pub(super) fn new(app_id: String) -> Self {
        Self {
            app_id,
            snapshot: None,
            line: String::new(),
            editable: true,
            inputs_stale: false,
            pending: Change::default(),
            writing: false,
            restart: Restart::Idle,
            seen: None,
            starting_since: None,
            reload_wanted: false,
            can_restart: heronite_steam::can_restart(),
        }
    }

    fn ready(&self) -> Option<&Snapshot> {
        self.snapshot.as_ref().and_then(|s| s.as_ref().ok())
    }
}

impl SettingsPanel {
    pub(super) fn is_steam(&self) -> bool {
        self.steam.is_some()
    }

    /// Reads the game's settings from Steam (again).
    pub(super) fn load_steam(&mut self, cx: &mut Context<Self>) {
        let Some(st) = self.steam.as_mut() else { return };
        st.snapshot = None;
        let (paths, app_id) = (self.paths.clone(), st.app_id.clone());
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { steam(&paths).read(&app_id) }).await;
            this.update(cx, |this, cx| {
                if let Ok(snap) = &res {
                    let parsed = options::parse(&snap.launch_options);
                    this.settings = parsed.settings;
                    if let Some(st) = this.steam.as_mut() {
                        st.line = snap.launch_options.clone();
                        st.editable = parsed.editable;
                    }
                }
                let read = res.is_ok();
                if let Some(st) = this.steam.as_mut() {
                    st.snapshot = Some(res);
                    st.inputs_stale = true;
                }
                if read {
                    this.apply_debug_settings(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Why nothing can be changed now, if so.
    pub(super) fn steam_locked(&self) -> Option<String> {
        let st = self.steam.as_ref()?;
        match &st.snapshot {
            None => Some(tr!("Reading Steam's settings\u{2026}").into()),
            Some(Err(e)) => Some(e.clone()),
            Some(Ok(s)) if s.access == Access::ReadOnly => Some(heronite_steam::read_only_message().into()),
            Some(Ok(_)) => None,
        }
    }

    /// Whether the fields (not the text of the line) can change.
    pub(super) fn steam_fields_editable(&self) -> bool {
        self.steam_locked().is_none() && self.steam.as_ref().is_some_and(|s| s.editable)
    }

    /// `set` for Steam games: rebuilds the launch options from the fields.
    pub(super) fn steam_set(&mut self, key: &str, value: Value, cx: &mut Context<Self>) {
        if let Some(why) = self.steam_locked() {
            self.notice = Some(Notice { text: why.into(), error: true });
            self.steam_mut().inputs_stale = true;
            cx.notify();
            return;
        }
        if !self.steam.as_ref().is_some_and(|s| s.editable) {
            return;
        }
        self.settings.insert(key.to_string(), value);
        self.steam_lsfg_dll();
        let line = options::build(&self.settings, &wrap::path(&self.paths).to_string_lossy());
        self.steam_send_line(line, cx);
    }

    /// The text of the launch options, typed by the user.
    pub(super) fn steam_commit_line(&mut self, text: String, cx: &mut Context<Self>) {
        let text = text.replace(['\n', '\r'], " ").trim().to_string();
        if self.steam.as_ref().is_none_or(|s| s.line == text) {
            return;
        }
        if let Some(why) = self.steam_locked() {
            self.notice = Some(Notice { text: why.into(), error: true });
            self.steam_mut().inputs_stale = true;
            cx.notify();
            return;
        }
        let parsed = options::parse(&text);
        self.settings = parsed.settings;
        self.steam_mut().editable = parsed.editable;
        self.steam_send_line(text, cx);
    }

    fn steam_mut(&mut self) -> &mut SteamState {
        self.steam.as_mut().expect("a Steam game")
    }

    fn steam_send_line(&mut self, line: String, cx: &mut Context<Self>) {
        let st = self.steam_mut();
        st.line = line.clone();
        st.inputs_stale = true;
        st.pending.launch_options = Some(line);
        self.steam_flush(cx);
        cx.notify();
    }

    fn steam_set_tool(&mut self, name: String, cx: &mut Context<Self>) {
        if let Some(why) = self.steam_locked() {
            self.notice = Some(Notice { text: why.into(), error: true });
            cx.notify();
            return;
        }
        let st = self.steam_mut();
        if let Some(Ok(snap)) = st.snapshot.as_mut() {
            snap.compat_tool = name.clone();
        }
        st.pending.compat_tool = Some(name);
        self.steam_flush(cx);
        cx.notify();
    }

    /// Sends the pending change to Steam off the UI thread, one at a time.
    fn steam_flush(&mut self, cx: &mut Context<Self>) {
        let paths = self.paths.clone();
        let st = self.steam_mut();
        if st.writing || st.pending == Change::default() {
            return;
        }
        st.writing = true;
        let change = std::mem::take(&mut st.pending);
        let app_id = st.app_id.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    if change.launch_options.as_deref().is_some_and(|l| l.contains(options::WRAP_NAME)) {
                        wrap::install(&paths)
                            .map_err(|e| tr!("Could not write {path}: {error}", path = wrap::path(&paths).display(), error = e))?;
                    }
                    steam(&paths).apply(&app_id, &change)
                })
                .await;
            this.update(cx, |this, cx| {
                this.steam_mut().writing = false;
                match res {
                    Ok(access) => {
                        if let Some(Ok(snap)) = this.steam_mut().snapshot.as_mut() {
                            snap.access = access;
                        }
                        this.steam_flush(cx);
                    }
                    Err(e) => {
                        this.notice =
                            Some(Notice { text: tr!("Could not change Steam's settings: {error}", error = e).into(), error: true });
                        // Show what Steam really has.
                        this.steam_mut().pending = Change::default();
                        this.load_steam(cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Puts the values of a read (or a typed line) into the inputs; needs the
    /// window, so it runs from `render`.
    pub(super) fn sync_steam_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(st) = self.steam.as_mut().filter(|s| s.inputs_stale) else { return };
        st.inputs_stale = false;
        let line = st.line.clone();
        self.reload_inputs(window, cx);
        self.set_input("steamLine", &line, window, cx);
    }

    /// Winecfg & co. in Steam's prefix, with the Proton build that last ran
    /// the game (the same one, so the prefix is not upgraded or downgraded).
    pub(super) fn steam_tool_settings(&self) -> Option<GameSettings> {
        let prefix = self.steam.as_ref()?.ready()?.prefix.clone()?;
        let proton = prefix.proton?;
        let mut s = GameSettings::from_map(&Default::default(), &self.home);
        s.wine_version = WineVersion {
            bin: proton.join("proton").to_string_lossy().into_owned(),
            name: prefix.proton_version,
            kind: "proton".into(),
            ..Default::default()
        };
        s.wine_prefix = prefix.compatdata.to_string_lossy().into_owned();
        Some(s)
    }

    /// `pfx/drive_c` of Steam's prefix, if it exists.
    pub(super) fn steam_drive_c(&self) -> Option<PathBuf> {
        let prefix = self.steam.as_ref()?.ready()?.prefix.as_ref()?;
        let drive_c = prefix.compatdata.join("pfx").join("drive_c");
        Some(if drive_c.is_dir() { drive_c } else { prefix.compatdata.join("pfx") })
    }

    pub(super) fn open_steam_prefix(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self.steam_drive_c() else { return };
        if let Err(e) = crate::pages::settings::open_folder(&dir) {
            self.notice = Some(Notice { text: format!("{}: {e}", dir.display()).into(), error: true });
            cx.notify();
        }
    }

    /// The tool menu: "Steam's default" (with what it runs, when known),
    /// then the tools by group; the one that last ran the game is marked.
    pub(super) fn steam_tool_options(&self) -> Vec<ToolOption> {
        let snap = self.steam.as_ref().and_then(SteamState::ready);
        let default = match snap.and_then(|s| s.default_tool.as_deref()) {
            Some(tool) => tr!("Steam's default ({tool})", tool),
            None => tr!("Steam's default").to_string(),
        };
        let mut out = vec![ToolOption { label: default, name: String::new(), group: None, note: None }];
        if let Some(snap) = snap {
            out.extend(snap.tools.iter().map(|t| ToolOption {
                label: t.display.clone(),
                name: t.name.clone(),
                group: Some(match t.group() {
                    ToolGroup::Custom => tr!("Your versions"),
                    ToolGroup::Valve => "Valve",
                    ToolGroup::Runtime => tr!("Runtimes"),
                }),
                note: (snap.last_ran.as_deref() == Some(t.name.as_str())).then(|| tr!("last used")),
            }));
        }
        out
    }

    /// What ProtonDB's tier suggests among `options`: Steam's default for
    /// games that run well, else the newest GE-Proton.
    pub(super) fn steam_recommendation(&self, options: &[ToolOption]) -> Option<Recommendation> {
        let ProtonDb::Tier(tier) = &self.protondb else { return None };
        if tier == "pending" || options.is_empty() {
            return None;
        }
        let current = self.steam_current_tool();
        if runs_well(tier) {
            return Some(Recommendation { label: options[0].label.clone(), index: Some(0), in_use: current.is_empty() });
        }
        Some(match best_ge(options.iter().skip(1).map(|o| o.label.as_str())).map(|i| i + 1) {
            Some(i) => Recommendation { label: options[i].label.clone(), index: Some(i), in_use: options[i].name == current },
            None => Recommendation { label: "GE-Proton".into(), index: None, in_use: false },
        })
    }

    /// Reads Steam again when it opens or closes (once its client answers).
    /// Runs while the panel lives.
    pub(super) fn watch_steam(&mut self, cx: &mut Context<Self>) {
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let p = paths.clone();
            let status = cx.background_spawn(async move { steam(&p).status() }).await;
            if this.update(cx, |this, cx| this.on_steam_status(status, cx)).is_err() {
                break;
            }
        })
        .detach();
    }

    fn on_steam_status(&mut self, status: (bool, bool), cx: &mut Context<Self>) {
        let typing = !self.debounce.is_empty();
        let st = self.steam_mut();
        let Some(before) = st.seen.replace(status) else { return };
        let restarting = matches!(st.restart, Restart::Running(_));
        let read = if before == status {
            // Steam up without its client for a while: read it as it is.
            let stuck = st.starting_since.is_some_and(|t| t.elapsed() > CLIENT_WAIT);
            let gave_up = matches!(st.restart, Restart::Running(t) if t.elapsed() > CLIENT_WAIT * 4);
            stuck || gave_up
        } else if status == (true, false) {
            st.starting_since = Some(Instant::now());
            false
        } else {
            // Closed, or its client answers. Mid-restart, only the latter.
            !restarting || status.1
        };
        if read {
            st.starting_since = None;
            st.restart = Restart::Idle;
            st.reload_wanted = true;
        }
        // A read now would put Steam's old values over a change still being
        // typed or sent (and the next change would build on them).
        let busy = st.writing || st.pending != Change::default() || typing;
        if !st.reload_wanted || busy {
            return;
        }
        st.reload_wanted = false;
        self.notice = None;
        self.load_steam(cx);
    }

    /// Closes and starts Steam; the watcher reads it again once it is back.
    pub(super) fn restart_steam(&mut self, cx: &mut Context<Self>) {
        self.steam_mut().restart = Restart::Running(Instant::now());
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let bin = crate::env::value("HERONITE_STEAM_BIN")
                .or_else(|| heronite_launch::steam::find_steam_binary(&heronite_launch::SystemProbe))
                .unwrap_or_else(|| "steam".into());
            let res = cx.background_spawn(async move { steam(&paths).restart(&bin) }).await;
            this.update(cx, |this, cx| {
                if let Err(e) = res {
                    this.steam_mut().restart = Restart::Idle;
                    this.notice = Some(Notice { text: e.into(), error: true });
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn steam_restart_state(&mut self, state: Restart, cx: &mut Context<Self>) {
        self.steam_mut().restart = state;
        cx.notify();
    }

    /// "Restart Steam", which asks once before closing Steam's games.
    fn restart_buttons(&self, cx: &mut Context<Self>) -> Div {
        let state = self.steam.as_ref().map(|s| s.restart).unwrap_or_default();
        let row = div().flex().flex_wrap().items_center().gap(px(8.));
        if !self.steam.as_ref().is_some_and(|s| s.can_restart) {
            return row;
        }
        match state {
            Restart::Idle => row.child(self.outline_button("steam-restart", tr!("Restart Steam"), true, cx, |this, _, cx| {
                this.steam_mut().restart = Restart::Confirm;
                cx.notify();
            })),
            Restart::Confirm => row
                .child(self.outline_button("steam-restart-yes", tr!("Restart now"), true, cx, |this, _, cx| this.restart_steam(cx)))
                .child(self.outline_button("steam-restart-no", tr!("Cancel"), true, cx, |this, _, cx| {
                    this.steam_mut().restart = Restart::Idle;
                    cx.notify();
                }))
                .child(help(tr!("Games running from Steam will close."))),
            Restart::Running(_) => {
                row.child(self.outline_button("steam-restart", tr!("Restarting Steam\u{2026}"), false, cx, |_, _, _| {}))
            }
        }
    }

    /// "2 new Proton builds: restart Steam to use them", when there are.
    pub(super) fn steam_unseen_text(&self) -> Option<String> {
        let snap = self.steam.as_ref().and_then(SteamState::ready)?;
        let names: Vec<&str> = snap.unseen.iter().map(|t| t.display.as_str()).collect();
        Some(match names.len() {
            0 => return None,
            1 => tr!("A new Proton build, {names}: restart Steam to use it.", names = names[0]),
            n => tr!("{count} new Proton builds, {names}: restart Steam to use them.", count = n, names = names.join(", ")),
        })
    }

    pub(super) fn steam_current_tool(&self) -> String {
        self.steam.as_ref().and_then(SteamState::ready).map(|s| s.compat_tool.clone()).unwrap_or_default()
    }

    pub(super) fn pick_steam_tool(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(name) = self.steam_tool_options().into_iter().nth(index).map(|o| o.name) {
            if name != self.steam_current_tool() {
                self.steam_set_tool(name, cx);
            }
        }
    }

    /// One line on where the settings go (or why they cannot change).
    pub(super) fn steam_status(&self) -> (String, bool) {
        if let Some(why) = self.steam_locked() {
            let error = self.steam.as_ref().is_some_and(|s| matches!(s.snapshot, Some(Err(_))));
            return (why, error);
        }
        let snap = self.steam.as_ref().and_then(SteamState::ready).expect("unlocked means read");
        let text = match snap.access {
            Access::Client => {
                tr!("Saved in Steam right away; used however the game starts.")
            }
            _ => tr!("Steam is closed: the change is saved in its files and Steam uses it from its next start."),
        };
        (text.to_string(), false)
    }

    /// The status box at the top of every Steam tab, with "Read again".
    fn steam_banner(&self, cx: &mut Context<Self>) -> Div {
        let (text, error) = self.steam_status();
        let loading = self.steam.as_ref().is_some_and(|s| s.snapshot.is_none());
        let mut col = column().child(if error { danger_text(&text).px(px(6.)) } else { hint(&text) });
        // Steam runs without the access Heronite needs: a restart opens it.
        let read_only = self.steam.as_ref().and_then(SteamState::ready).is_some_and(|s| s.access == Access::ReadOnly);
        if read_only {
            col = col.child(self.restart_buttons(cx));
        }
        if !self.steam.as_ref().is_some_and(|s| s.editable) && !loading {
            col = col.child(hint(tr!(
                "These launch options use shell syntax (like ; $VAR or redirections) that the fields here cannot rebuild. Edit them as text in Advanced, or clear them there to use the fields."
            )));
        }
        if self.steam.as_ref().and_then(SteamState::ready).is_some_and(|s| s.flatpak) {
            col = col.child(
                help(tr!(
                    "Steam is a Flatpak: programs it runs (gamescope, MangoHud, GameMode, scripts) must be visible inside its sandbox."
                ))
                .px(px(6.)),
            );
        }
        if !loading {
            col = col.child(div().flex().child(self.outline_button(
                "steam-reload",
                tr!("Read from Steam again"),
                true,
                cx,
                |this, _, cx| {
                    this.notice = None;
                    this.load_steam(cx)
                },
            )));
        }
        col
    }

    pub(super) fn proton_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let mut col = column().child(self.steam_banner(cx));
        let locked = self.steam_locked().is_some();
        if let Some(text) = self.steam_unseen_text() {
            let restarting = self.steam.as_ref().is_some_and(|s| s.restart != Restart::Idle);
            let read_only = self.steam.as_ref().and_then(SteamState::ready).is_some_and(|s| s.access == Access::ReadOnly);
            // The banner already offers the restart when Steam is read-only.
            let mut box_ = column().child(hint(&text));
            if !read_only || restarting {
                box_ = box_.child(self.restart_buttons(cx));
            }
            col = col.child(box_);
        }
        let options = self.steam_tool_options();
        let current = self.steam_current_tool();
        let shown = options.iter().find(|o| o.name == current).map(|o| o.label.clone()).unwrap_or(current.clone());
        let items = options
            .iter()
            .map(|o| SelectOption {
                label: o.label.clone().into(),
                selected: o.name == current,
                dot: None,
                group: o.group.map(SharedString::from),
                note: o.note.map(SharedString::from),
            })
            .collect();
        let rec = self.steam_recommendation(&options);
        let tool = self.select_menu_row("steam-tool", Some(tr!("Compatibility tool")), shown, items, locked, cx, |this, i, _, cx| {
            this.pick_steam_tool(i, cx)
        });
        let mut card =
            rows().field_help(tool, vec![help(tr!("The build Steam runs the game with. Steam sees a new build after it restarts."))]);
        if let Some(row) = self.protondb_row(rec.filter(|_| !locked), cx, |this, i, cx| this.pick_steam_tool(i, cx)) {
            card = card.child(row);
        }
        col = col.child(heading("Proton")).child(card.card());
        if self.steam.as_ref().is_some_and(|s| s.editable) {
            let off = !self.steam_fields_editable();
            let sync = rows().field_help(
                self.sync_select(off, cx),
                vec![help(tr!("Less CPU overhead in busy games. Fsync is the fastest where the kernel has it."))],
            );
            col = col
                .child(heading(tr!("Graphics and libraries")))
                .child(self.proton_graphics(off, cx).card())
                .child(heading(tr!("Sync")))
                .child(sync.card())
                .child(heading(tr!("Experimental")))
                .child(self.proton_experimental(off, cx).card());
        }
        col.child(heading(tr!("Prefix"))).child(self.steam_prefix_card(cx))
    }

    fn proton_graphics(&self, off: bool, cx: &mut Context<Self>) -> super::parts::Rows {
        rows()
            .child(self.toggle_row("p-nvapi", options::KEY_NVAPI, false, "NVAPI", Some(tr!("DLSS and other NVIDIA features")), off, cx))
            .child(self.toggle_row(
                "p-wined3d",
                options::KEY_WINED3D,
                false,
                "WineD3D",
                Some(tr!("OpenGL instead of DXVK and VKD3D: slower, but can help games that misbehave on Vulkan")),
                off,
                cx,
            ))
    }

    /// Wayland, HDR and WoW64 (Proton 10 or newer, or GE-Proton).
    fn proton_experimental(&self, off: bool, cx: &mut Context<Self>) -> super::parts::Rows {
        let mut r = rows().child(self.toggle_row(
            "p-wayland",
            "enableWineWayland",
            false,
            tr!("Wayland driver"),
            Some(tr!("Proton 10 or newer, or GE-Proton")),
            off,
            cx,
        ));
        if self.flag("enableWineWayland", false) || self.flag("enableHDR", false) {
            r = r.child(self.toggle_row("p-hdr", "enableHDR", false, "HDR", Some(tr!("The screen and the game must support it")), off, cx));
        }
        r.child(self.toggle_row(
            "p-wow64",
            "enableWoW64",
            false,
            "WoW64",
            Some(tr!("32-bit games on 64-bit Wine; can break older games")),
            off,
            cx,
        ))
    }

    fn steam_prefix_card(&mut self, cx: &mut Context<Self>) -> Div {
        let prefix = self.steam.as_ref().and_then(SteamState::ready).and_then(|s| s.prefix.clone());
        let Some(prefix) = prefix else {
            return rows().child(help(tr!("Steam creates the prefix the first time it runs the game through Proton.")).py(px(12.))).card();
        };
        let idle = self.tool_busy.is_none();
        let can_run = idle && prefix.proton.is_some();
        let busy = self.tool_busy;
        let label =
            |name: &'static str, text: &'static str| if busy == Some(name) { tr!("{text} (running)", text) } else { text.to_string() };
        let mut notes = vec![help(&prefix.compatdata.join("pfx").display().to_string())];
        notes.push(help(&match &prefix.proton {
            Some(dir) => tr!(
                "Tools run with {proton} ({path}), the build that last ran the game.",
                proton = prefix.proton_version,
                path = dir.display()
            ),
            None => tr!("The Proton build that last ran the game was not found; run the game once from Steam.").to_string(),
        }));
        let buttons = div()
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .child(self.outline_button("steam-winecfg", &label("winecfg", "Winecfg"), can_run, cx, |this, _, cx| {
                this.run_tool("winecfg", WineTool::Winecfg, cx)
            }))
            .child(self.outline_button("steam-winetricks", &label("winetricks", "Winetricks"), can_run, cx, |this, window, cx| {
                this.open_winetricks(window, cx)
            }))
            .child(self.outline_button("steam-runexe", &label("runexe", tr!("Run .exe in prefix")), can_run, cx, |_, window, cx| {
                choose_path(window, cx, false, tr!("Select EXE to Run"), |this: &mut Self, path, _, cx| {
                    this.run_tool("runexe", WineTool::RunExe(path), cx)
                });
            }))
            .child(self.outline_button("steam-openpfx", tr!("Open prefix folder"), true, cx, |this, _, cx| this.open_steam_prefix(cx)));
        let mut col = div().flex().flex_col().gap(px(6.)).py(px(12.)).child(buttons);
        for n in notes {
            col = col.child(n);
        }
        rows().child(col).card()
    }

    pub(super) fn steam_perf_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let mut col = column().child(self.steam_banner(cx));
        if !self.steam.as_ref().is_some_and(|s| s.editable) {
            return col;
        }
        let off = !self.steam_fields_editable();
        col = col.child(heading(tr!("On screen"))).child(self.overlay_rows(off, cx).card());
        col = col.child(self.fps_section(off, cx)).child(self.lsfg_section(off, cx));
        col = col.child(self.gamescope_section(rows(), off, cx));
        col.child(heading(tr!("System"))).child(self.game_system_rows(off, cx).card())
    }

    pub(super) fn steam_gpu_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let col = column().child(self.steam_banner(cx));
        if !self.steam.as_ref().is_some_and(|s| s.editable) {
            return col;
        }
        let off = !self.steam_fields_editable();
        col.child(self.gpu_tab(off, cx))
    }

    pub(super) fn steam_launch_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let mut col = column().child(self.steam_banner(cx));
        if self.steam.as_ref().is_some_and(|s| s.editable) {
            let off = !self.steam_fields_editable();
            let args = self.input_text("launcherArgs", cx);
            let err = launcher_args_error(&args);
            let mut notes = Vec::new();
            if !err.is_empty() {
                notes.push(danger_text(err));
            }
            notes.push(help(tr!("Added after the game's command, like -nolauncher")));
            let launch = rows().field_help(self.text_row("launcherArgs", Some(tr!("Game arguments")), None, cx), notes);
            col = col.child(heading(tr!("Starting"))).child(launch.card());
            col = col.child(self.scripts_section(true, cx));
            let logs = rows().child(self.toggle_row(
                "p-log",
                "verboseLogs",
                false,
                tr!("Proton logs"),
                Some(tr!("Writes steam-<app id>.log in your home folder")),
                off,
                cx,
            ));
            col = col
                .child(heading(tr!("Advanced")))
                .child(logs.card())
                .child(self.presets_section(off, cx))
                .child(self.pair_table(WRAPPERS, cx))
                .child(self.pair_table(ENV, cx));
        }
        let line = rows().field_help(
            self.text_row("steamLine", Some(tr!("Steam launch options")), None, cx),
            vec![help(tr!("What these tabs write in Steam (Properties → General). You can edit it here too."))],
        );
        col.child(heading("Steam")).child(line.card())
    }
}
