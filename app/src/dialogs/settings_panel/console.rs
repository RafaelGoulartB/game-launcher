//! The Heroic settings panel in console mode: every setting of the current
//! section as a console row (switches, ◀ ▶ choices, text typed on the
//! on-screen keyboard, folders and files picked with the console browser,
//! the Wine tools). Values go through the same setters as the desktop rows,
//! so they are saved the same way. Hosted by the console game settings and
//! the console Settings page.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_launch::wine_tools::WineTool;
use serde_json::{json, Value};

use super::advanced::{PairTable, ENV, WRAPPERS};
use super::gamescope::{UPSCALE_METHODS, WINDOW_TYPES};
use super::logic::{self, launcher_args_error, pairs};
use super::saves::{GOG_MODES, MODES};
use super::{GlobalView, Section, SettingsPanel};
use crate::console::browser::{Browser, BrowserEvent, PickKind};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row, Value as RowValue};
use crate::console::osk::{Osk, OskEvent};
use crate::console::sheet::{Sheet, SheetEvent};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use heronite_i18n::tr;

mod fps;
mod gpu;
mod lsfg;
mod steam;
mod tweaks;
mod winetricks;

type Pick = Rc<dyn Fn(&mut SettingsPanel, usize, &mut Context<SettingsPanel>)>;
type Run = Rc<dyn Fn(&mut SettingsPanel, &mut Window, &mut Context<SettingsPanel>)>;

/// What a row does.
#[derive(Clone)]
pub(super) enum Act {
    None,
    Toggle(&'static str, bool),
    GsToggle(&'static str),
    Choice(Pick),
    /// Input id, keyboard title.
    Text(&'static str, SharedString),
    /// Input id, folder?, picker title (X clears it).
    Path(&'static str, bool, SharedString),
    Do(Run),
}

pub(super) struct Item {
    row: Row,
    act: Act,
}

pub(super) fn item(row: Row, act: Act) -> Item {
    Item { row, act }
}

impl Item {
    fn desc_item(mut self, d: &str) -> Self {
        self.row = self.row.desc(d.to_string());
        self
    }
}

impl SettingsPanel {
    fn c_toggle(&self, key: &'static str, default: bool, label: &str, desc: Option<&str>, disabled: bool) -> Item {
        let mut r = Row::toggle(SharedString::from(format!("t:{key}")), label.to_string(), self.flag(key, default)).enabled(!disabled);
        if let Some(d) = desc {
            r = r.desc(d.to_string());
        }
        item(r, Act::Toggle(key, default))
    }

    fn c_text(&self, id: &'static str, label: &str, cx: &App) -> Item {
        item(
            Row::text(SharedString::from(format!("x:{id}")), label.to_string(), self.input_text(id, cx)).icon(Icon::Edit),
            Act::Text(id, SharedString::from(label.to_string())),
        )
    }

    fn c_path(&self, id: &'static str, label: &str, dir: bool, cx: &App) -> Item {
        item(
            Row::text(SharedString::from(format!("p:{id}")), label.to_string(), self.input_text(id, cx)).icon(if dir {
                Icon::FolderOpen
            } else {
                Icon::Description
            }),
            Act::Path(id, dir, SharedString::from(label.to_string())),
        )
    }

    pub(super) fn c_header(text: &str) -> Item {
        item(Row::header(text.to_string()), Act::None)
    }

    pub(super) fn c_info(id: &str, label: &str, value: impl Into<SharedString>) -> Item {
        item(Row::info(SharedString::from(format!("i:{id}")), label.to_string(), value), Act::None)
    }

    pub(super) fn c_do(
        id: &str,
        label: &str,
        ic: Icon,
        enabled: bool,
        run: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Item {
        item(Row::action(SharedString::from(format!("d:{id}")), label.to_string()).icon(ic).enabled(enabled), Act::Do(Rc::new(run)))
    }

    fn console_items(&self, cx: &App) -> Vec<Item> {
        if self.winetricks_open() {
            return self.c_winetricks(cx);
        }
        let mut v = match self.section {
            Section::Compatibility if self.is_steam() => self.c_proton(),
            Section::Performance if self.is_steam() => self.c_steam_other(cx),
            Section::Launch if self.is_steam() => self.c_steam_advanced(cx),
            Section::Gpu if self.is_steam() => self.c_steam_gpu(),
            Section::Compatibility => self.c_wine(cx),
            Section::Performance => self.c_other(cx),
            Section::Gpu => self.c_gpu(false),
            Section::Launch => self.c_advanced(cx),
            Section::Saves => self.c_saves(cx),
            Section::General => match self.global_view {
                GlobalView::General => self.c_general(cx),
                GlobalView::Downloads => self.c_downloads(cx),
                GlobalView::Advanced => vec![self.c_toggle(
                    "disableLogs",
                    false,
                    tr!("Disable logs"),
                    Some(tr!("Most log writes stop (critical information is always logged)")),
                    false,
                )],
            },
        };
        if let Some(n) = &self.notice {
            v.push(Self::c_info("notice", if n.error { tr!("Problem") } else { tr!("Done") }, n.text.clone()));
        }
        for i in &mut v {
            i.row.changed = self.console_changed(&i.row.id).is_some();
        }
        if !self.console_tab_changes().is_empty() {
            v.insert(
                0,
                Self::c_do("reset-tab", tr!("Reset this tab"), Icon::Replay, true, |this, _, cx| {
                    let groups = this.console_tab_changes();
                    this.reset(&groups, cx);
                })
                .desc_item(tr!("The marked settings go back to the game defaults")),
            );
        }
        v
    }

    fn c_wine(&self, cx: &App) -> Vec<Item> {
        let wine = self.wine_version();
        let mut v = Vec::new();
        let list = self.wine_list.clone().unwrap_or_default();
        let options = logic::wine_options(&list, &wine);
        let names: Vec<SharedString> = options
            .iter()
            .map(|w| {
                let name = logic::display_wine_name(&w.name);
                SharedString::from(match self.global_wine.as_deref() == Some(w.name.as_str()) {
                    true => format!("{name} \u{b7} {}", tr!("default")),
                    false => name,
                })
            })
            .collect();
        let selected = options.iter().position(|w| w.name == wine.name).unwrap_or(0);
        let desc = if self.wine_list.is_some() && list.is_empty() {
            tr!("No Wine version was found, download one from the Wine Manager").to_string()
        } else if !std::path::Path::new(&wine.bin).exists() && !list.is_empty() {
            tr!("Wine path is invalid, please select another one").to_string()
        } else {
            wine.bin.clone()
        };
        let pick: Pick = Rc::new(|this, i, cx| {
            let list = this.wine_list.clone().unwrap_or_default();
            let options = logic::wine_options(&list, &this.wine_version());
            if let Some(w) = options.get(i) {
                this.set("wineVersion", logic::wine_to_value(w), cx);
            }
        });
        if names.is_empty() {
            v.push(Self::c_info(
                "wine",
                tr!("Wine / Proton version"),
                if self.wine_list.is_none() { tr!("Loading\u{2026}").to_string() } else { tr!("None found").into() },
            ));
        } else {
            v.push(item(
                Row::choice("c:wine", tr!("Wine / Proton version"), names, selected).icon(Icon::WineBar).desc(desc),
                Act::Choice(pick),
            ));
        }
        v.extend(self.c_protondb(self.wine_recommendation(&options, &wine), Self::pick_wine));
        if wine.kind == "crossover" {
            v.push(self.c_text("crossoverBottle", tr!("CrossOver bottle"), cx));
            return v;
        }
        if self.is_game() {
            let current = std::path::PathBuf::from(self.text("winePrefix").trim().trim_end_matches('/'));
            let label = self.prefix_choices().iter().find(|p| p.path == current).map(|p| p.name.clone());
            let mut it =
                Self::c_do("prefix-pick", tr!("Choose a prefix\u{2026}"), Icon::Folder, self.prefix_list.is_some(), |this, window, cx| {
                    this.console_prefix_sheet(window, cx)
                });
            it.row = it.row.desc(label.unwrap_or_else(|| tr!("Another folder (below)").to_string()));
            v.push(it);
        }
        v.push(self.c_path("winePrefix", tr!("Prefix"), true, cx));
        let busy = self.tool_toggle.is_some();
        let dxvk_on = self.flag("autoInstallDxvk", false);
        v.push(Self::c_header(tr!("Graphics and libraries")));
        if logic::offers_dxvk(&wine) {
            v.push(self.c_toggle(
                "autoInstallDxvk",
                false,
                "DXVK",
                Some(tr!("DirectX 9 to 11 through Vulkan, installed and kept up to date in the prefix")),
                self.flag("autoInstallDxvkNvapi", false) || self.flag("autoInstallVkd3d", false) || busy,
            ));
        }
        if wine.kind != "proton" {
            v.push(self.c_toggle(
                "autoInstallVkd3d",
                false,
                "VKD3D",
                Some(tr!("DirectX 12 through Vulkan (needs DXVK)")),
                !dxvk_on || busy,
            ));
        }
        v.push(self.c_toggle(
            "autoInstallDxvkNvapi",
            false,
            "DXVK-NVAPI",
            Some(tr!("DLSS and other NVIDIA features (needs DXVK)")),
            !dxvk_on || busy,
        ));
        if self.is_game() && logic::offers_system_libs(&wine) {
            v.push(self.c_toggle(
                "preferSystemLibs",
                false,
                tr!("Prefer system libraries"),
                Some(tr!("Uses this computer's libraries instead of the ones shipped with this Wine build")),
                false,
            ));
        }
        v.extend(self.c_fixes(cx));
        v.push(Self::c_header(tr!("Sync")));
        v.push(self.c_sync(false));
        v.push(Self::c_header(tr!("Experimental")));
        v.push(self.c_toggle(
            "enableWineWayland",
            false,
            tr!("Wayland driver"),
            Some(tr!("Runs the game on Wayland instead of X11; needs a Wine build that has it")),
            false,
        ));
        if self.flag("enableWineWayland", false) {
            v.push(self.c_toggle("enableHDR", false, "HDR", Some(tr!("The screen and the game must support it")), false));
        }
        v.push(self.c_toggle("enableWoW64", false, "WoW64", Some(tr!("32-bit games on 64-bit Wine; can break older games")), false));
        if self.is_game() {
            v.push(Self::c_header(tr!("Tools")));
            let idle = self.tool_busy.is_none();
            v.push(Self::c_do("winecfg", "Winecfg", Icon::Tune, idle, |this, _, cx| this.run_tool("winecfg", WineTool::Winecfg, cx)));
            v.push(Self::c_do("winetricks", "Winetricks", Icon::Build, idle, |this, window, cx| this.open_winetricks(window, cx)));
            v.push(Self::c_do("runexe", tr!("Run a program in the prefix\u{2026}"), Icon::PlayArrow, idle, |this, window, cx| {
                this.console_pick_file(
                    tr!("Run a program in the prefix"),
                    vec!["exe".into(), "msi".into(), "bat".into()],
                    window,
                    cx,
                    |this, path, cx| this.run_tool("runexe", WineTool::RunExe(path), cx),
                )
            }));
            v.push(Self::c_do("openpfx", tr!("Open prefix folder"), Icon::FolderOpen, true, |this, _, cx| this.open_wine_prefix(cx)));
        }
        if self.is_game() {
            let note = |on: bool, installed: bool| (on && !installed).then(|| tr!("Downloaded before the next launch"));
            v.push(Self::c_header(tr!("Anti-cheat runtimes")));
            let be = note(self.flag("battlEyeRuntime", false), self.runtimes.0);
            let eac = note(self.flag("eacRuntime", false), self.runtimes.1);
            v.push(self.c_toggle("battlEyeRuntime", false, "BattlEye", be, false));
            v.push(self.c_toggle("eacRuntime", false, "Easy Anti-Cheat", eac, false));
        }
        v
    }

    /// Esync and Fsync as one choice (see `sync_select`).
    fn c_sync(&self, steam: bool) -> Item {
        let (esync, fsync) = (self.flag("enableEsync", steam), self.flag("enableFsync", steam));
        let index = if fsync {
            0
        } else if esync {
            1
        } else {
            2
        };
        let opts = vec!["Fsync".into(), "Esync".into(), tr!("Off").into()];
        let enabled = !(steam && !self.steam_fields_editable());
        item(
            Row::choice("c:sync", tr!("Sync"), opts, index)
                .desc(tr!("Less CPU overhead in busy games. Fsync is the fastest where the kernel has it."))
                .enabled(enabled),
            Act::Choice(Rc::new(|this, i, cx| {
                let (e, f) = [(true, true), (true, false), (false, false)][i];
                this.set_all(vec![("enableEsync", Value::Bool(e)), ("enableFsync", Value::Bool(f))], cx);
            })),
        )
    }

    /// Wine's FSR in fullscreen and its sharpness.
    fn c_fsr(&self) -> Vec<Item> {
        let mut v = vec![self.c_toggle(
            "enableFSR",
            false,
            tr!("Wine FSR"),
            Some(tr!("Upscales fullscreen games running below the screen's resolution (needs a Wine build with the FSR hack)")),
            false,
        )];
        if self.flag("enableFSR", false) {
            let sharp = self.number("maxSharpness").unwrap_or(5).clamp(1, 5);
            let opts = (1..=5).map(|n| SharedString::from(n.to_string())).collect();
            v.push(item(
                Row::choice("c:sharp", tr!("FSR sharpness"), opts, (sharp - 1) as usize),
                Act::Choice(Rc::new(|this, i, cx| this.set("maxSharpness", json!(i as i64 + 1), cx))),
            ));
        }
        v
    }

    fn c_other(&self, cx: &App) -> Vec<Item> {
        let native = self.native();
        let mut v = vec![Self::c_header(tr!("On screen"))];
        if !native {
            v.push(self.c_toggle("showFps", false, tr!("FPS counter"), Some(tr!("A small counter for DirectX 9 to 11 games")), false));
        }
        v.push(self.c_toggle(
            "showMangohud",
            false,
            "MangoHud",
            Some(tr!("FPS, frame times, temperatures and load (needs MangoHud)")),
            false,
        ));
        v.extend(self.c_fps(false));
        v.extend(self.c_lsfg(false));
        v.push(Self::c_header(tr!("Image")));
        if !native {
            v.extend(self.c_fsr());
        }
        v.extend(self.c_gamescope(cx));
        v.extend(self.c_system(false));
        v
    }

    fn c_pairs(&self, t: PairTable) -> Vec<Item> {
        let mut v = vec![Self::c_header(heronite_i18n::tr(t.title).trim_end_matches(':'))];
        for (i, (a, b)) in pairs(self.settings.get(t.setting), t.k1, t.k2).into_iter().enumerate() {
            v.push(item(
                Row::info(SharedString::from(format!("pair:{}:{i}", t.id)), a.clone(), b.clone()),
                Act::Do(Rc::new(move |this, window, cx| this.console_pair_sheet(t, i, window, cx))),
            ));
        }
        let label = if t.env { tr!("Add an environment variable") } else { tr!("Add a wrapper") };
        v.push(Self::c_do(&format!("add-{}", t.id), label, Icon::Add, true, move |this, window, cx| {
            this.console_pair_edit(t, None, window, cx)
        }));
        if t.env {
            let mut paste = Self::c_do("env-text", tr!("Type several at once\u{2026}"), Icon::Edit, true, |this, window, cx| {
                // One line: values with spaces are quoted so they read back whole.
                let current = pairs(this.settings.get(ENV.setting), ENV.k1, ENV.k2)
                    .iter()
                    .map(|(k, v)| format!("{k}={}", shlex::try_quote(v).map(|q| q.into_owned()).unwrap_or_else(|_| v.clone())))
                    .collect::<Vec<_>>()
                    .join(" ");
                this.console_osk(tr!("Environment Variables"), &current, window, cx, |this, text, _, cx| {
                    this.apply_env_text(&text, cx);
                });
            });
            paste.row = paste.row.desc(tr!("NAME=value pairs separated by spaces, like PROTON_NO_ESYNC=1 DXVK_ASYNC=1; replaces the list"));
            v.push(paste);
        }
        if t.env && !self.env_error.is_empty() {
            v.push(Self::c_info("env-error", tr!("Problem"), self.env_error.clone()));
        }
        v
    }

    /// One row per program (left / right change its delay), then add and remove.
    fn c_companions(&self) -> Vec<Item> {
        use super::companions::{delay_index, delay_label, file_name, DELAYS};
        let list = self.companions();
        let mut v = vec![Self::c_header(tr!("Programs alongside the game"))];
        for (i, c) in list.iter().enumerate() {
            let options = DELAYS.iter().map(|d| SharedString::from(delay_label(*d))).collect();
            let row = Row::choice(SharedString::from(format!("companion:{i}")), file_name(&c.path), options, delay_index(c.delay_secs));
            v.push(item(
                row.desc(c.path.clone()),
                Act::Choice(Rc::new(move |this, index, cx| this.set_companion_delay(i, DELAYS[index], cx))),
            ));
        }
        v.push(Self::c_do("companion-add", tr!("Add a program\u{2026}"), Icon::Add, true, |this, window, cx| {
            this.console_pick_file(tr!("Program to run alongside the game"), Vec::new(), window, cx, |this, path, cx| {
                this.add_companion(path, cx)
            })
        }));
        if !list.is_empty() {
            v.push(Self::c_do("companion-remove", tr!("Remove a program\u{2026}"), Icon::DeleteOutline, true, |this, window, cx| {
                this.console_companion_remove(window, cx)
            }));
        }
        v
    }

    fn console_prefix_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let choices = self.prefix_choices();
        let mut rows: Vec<Row> = choices
            .iter()
            .enumerate()
            .map(|(i, p)| Row::action(SharedString::from(format!("pfx:{i}")), Self::prefix_label(p)).icon(Icon::Folder))
            .collect();
        rows.push(Row::action("pfx-new", tr!("New prefix for this game")).icon(Icon::Add));
        let paths: Vec<std::path::PathBuf> = choices.into_iter().map(|p| p.path).collect();
        let sheet = cx.new(|_| Sheet::new(tr!("Prefix"), None, rows));
        let sub = cx.subscribe_in(&sheet, window, move |this, _, ev: &SheetEvent, window, cx| {
            let SheetEvent::Activate(id) = ev;
            this.console_layer.close();
            if id.as_ref() == "pfx-new" {
                this.new_prefix_for_game(window, cx);
            } else if let Some(path) = id.strip_prefix("pfx:").and_then(|n| n.parse::<usize>().ok()).and_then(|i| paths.get(i)) {
                this.set_prefix(path, window, cx);
            }
            cx.notify();
        });
        self.console_layer.open(&sheet, sub);
    }

    fn console_companion_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self
            .companions()
            .iter()
            .enumerate()
            .map(|(i, c)| {
                Row::action(SharedString::from(format!("rm:{i}")), super::companions::file_name(&c.path)).icon(Icon::DeleteOutline).danger()
            })
            .collect();
        let sheet = cx.new(|_| Sheet::new(tr!("Remove a program"), None, rows));
        let sub = cx.subscribe_in(&sheet, window, move |this, _, ev: &SheetEvent, _, cx| {
            let SheetEvent::Activate(id) = ev;
            this.console_layer.close();
            if let Some(i) = id.strip_prefix("rm:").and_then(|n| n.parse::<usize>().ok()) {
                this.remove_companion(i, cx);
            }
            cx.notify();
        });
        self.console_layer.open(&sheet, sub);
    }

    fn c_advanced(&self, cx: &App) -> Vec<Item> {
        let sideload = self.facts().is_some_and(|f| f.is_sideload());
        let legendary = self.facts().is_some_and(|f| f.runner == "legendary");
        let game = self.is_game();
        let mut v = vec![Self::c_header(tr!("General"))];
        if game && !sideload {
            v.push(self.c_toggle("ignoreGameUpdates", false, tr!("Ignore game updates"), None, false));
        }
        if game && legendary {
            v.push(self.c_toggle("offlineMode", false, tr!("Run the game offline"), None, false));
        }
        if game && !self.launch_options.is_empty() {
            let (names, index) = self.launch_option_choices();
            v.push(item(
                Row::choice("c:launch-option", tr!("Start with"), names, index).icon(Icon::PlayArrow),
                Act::Choice(Rc::new(|this, i, cx| this.pick_launch_option(i, cx))),
            ));
        }
        v.push(self.c_toggle("verboseLogs", true, tr!("Verbose logs"), None, false));
        if game && !sideload {
            v.push(self.c_path("targetExe", tr!("Alternative executable"), false, cx));
        }
        if !sideload {
            let mut it = self.c_text("language", tr!("Preferred language code"), cx);
            it.row = it.row.desc(tr!("Like \u{201c}fr\u{201d} or \u{201c}pt-BR\u{201d}; empty uses the app's language"));
            v.push(it);
        }
        v.push(Self::c_header(tr!("Launch")));
        if game {
            let err = launcher_args_error(&self.input_text("launcherArgs", cx));
            let mut it = self.c_text("launcherArgs", tr!("Game arguments"), cx);
            it.row = it.row.desc(if err.is_empty() { tr!("Appended to the launch command, like -nolauncher") } else { err });
            v.push(it);
        }
        v.push(self.c_path("beforeScript", tr!("Script before launch"), false, cx));
        v.push(self.c_path("afterScript", tr!("Script after exit"), false, cx));
        if game {
            v.extend(self.c_companions());
        }
        v.extend(self.c_presets(false));
        v.extend(self.c_pairs(WRAPPERS));
        v.extend(self.c_pairs(ENV));
        v.extend(self.c_preview());
        if game && !self.native() && self.wine_version().kind == "proton" {
            v.push(Self::c_header(tr!("Legacy (mostly deprecated)")));
            v.push(self.c_toggle("disableUMU", false, tr!("Disable umu"), None, false));
        }
        if self.native() {
            let current = match self.settings.get("steamRuntime") {
                Some(Value::String(s)) if !s.is_empty() => s.clone(),
                _ => "none".to_string(),
            };
            let mut opts: Vec<SharedString> = vec![tr!("None").into()];
            opts.extend(logic::STEAM_RUNTIMES.iter().map(|(_, n)| SharedString::from(*n)));
            let index = logic::STEAM_RUNTIMES.iter().position(|(id, _)| *id == current).map_or(0, |i| i + 1);
            v.push(item(
                Row::choice("c:runtime", tr!("Steam Runtime"), opts, index),
                Act::Choice(Rc::new(|this, i, cx| {
                    let val = if i == 0 { Value::Bool(false) } else { Value::String(logic::STEAM_RUNTIMES[i - 1].0.into()) };
                    this.set("steamRuntime", val, cx);
                })),
            ));
        }
        v
    }

    fn c_gamescope(&self, cx: &App) -> Vec<Item> {
        if !self.gamescope_installed {
            return vec![Self::c_info("gs-missing", "gamescope", tr!("Not installed"))];
        }
        let g = self.gamescope();
        let gs_choice = |id: &'static str, field: &'static str, label: &str, table: &'static [(&'static str, &'static str)]| {
            let current = g.get(field).and_then(Value::as_str).unwrap_or("").to_string();
            let opts = table.iter().map(|(_, l)| SharedString::from(heronite_i18n::tr(l))).collect();
            let index = table.iter().position(|(k, _)| *k == current).unwrap_or(0);
            item(
                Row::choice(id, label.to_string(), opts, index),
                Act::Choice(Rc::new(move |this, i, cx| this.set_gamescope(field, Value::String(table[i].0.into()), cx))),
            )
        };
        let gs_toggle = |field: &'static str, label: &str| {
            item(Row::toggle(SharedString::from(format!("g:{field}")), label.to_string(), self.gs_flag(field)), Act::GsToggle(field))
        };
        let mut v = vec![gs_toggle("enableUpscaling", tr!("gamescope upscaling"))];
        if self.gs_flag("enableUpscaling") {
            v.push(gs_choice("c:gs-method", "upscaleMethod", tr!("Upscale method"), &UPSCALE_METHODS));
            v.push(self.c_text("gs.gameWidth", tr!("Game width"), cx));
            v.push(self.c_text("gs.gameHeight", tr!("Game height"), cx));
            v.push(self.c_text("gs.upscaleWidth", tr!("Screen width"), cx));
            v.push(self.c_text("gs.upscaleHeight", tr!("Screen height"), cx));
            v.push(gs_choice("c:gs-window", "windowType", tr!("Window"), &WINDOW_TYPES));
            v.push(gs_toggle("enableForceGrabCursor", tr!("Keep the mouse in the game")));
            v.push(self.c_text("gs.additionalOptions", tr!("Extra gamescope options"), cx));
        }
        v
    }

    fn c_gog_saves(&self, cx: &App) -> Vec<Item> {
        let mut v = vec![Self::c_header(tr!("Location (beta: back up your saves first)"))];
        match &self.saves.gog.locations {
            None => v.push(Self::c_info("gog-loading", tr!("Save folders"), tr!("Loading\u{2026}"))),
            Some(Err(err)) => v.push(Self::c_info("gog-error", tr!("Save folders"), err.clone())),
            Some(Ok(list)) if list.is_empty() => {
                v.push(Self::c_info("gog-none", tr!("Cloud saves"), tr!("This game does not support them")))
            }
            Some(Ok(list)) => {
                for (i, l) in list.iter().enumerate() {
                    let name = if l.name == "__default" { tr!("Default folder").to_string() } else { l.name.clone() };
                    v.push(Self::c_info(&format!("gog-loc{i}"), &name, l.location.clone()));
                }
            }
        }
        let loading = self.saves.gog.loading;
        v.push(Self::c_do(
            "gog-refresh",
            if loading { tr!("Loading\u{2026}") } else { tr!("Refresh locations") },
            Icon::FaSearch,
            !loading,
            |this, _, cx| this.load_gog_locations(true, cx),
        ));
        let has_places = !self.gog_locations().is_empty();
        let syncing = self.gog_syncing(cx);
        v.push(Self::c_header(tr!("Sync")));
        let opts = GOG_MODES.iter().map(|(l, _)| SharedString::from(heronite_i18n::tr(l))).collect();
        v.push(item(
            Row::choice("c:saves-mode", tr!("Manual sync"), opts, self.saves.mode).enabled(has_places),
            Act::Choice(Rc::new(|this, i, cx| {
                this.saves.mode = i;
                cx.notify();
            })),
        ));
        v.push(Self::c_do(
            "sync",
            if syncing { tr!("Syncing\u{2026}") } else { tr!("Sync now") },
            Icon::CloudSync,
            has_places && !syncing,
            |this, _, cx| this.gog_sync(cx),
        ));
        v.push(Self::c_header(tr!("Automatic")));
        let no_cloud = matches!(&self.saves.gog.locations, Some(Ok(list)) if list.is_empty());
        v.push(self.c_toggle(
            "autoSyncSaves",
            false,
            tr!("Autosync saves"),
            Some(tr!("Before the game starts and after it ends")),
            no_cloud,
        ));
        v
    }

    fn c_saves(&self, cx: &App) -> Vec<Item> {
        if self.is_gog() {
            return self.c_gog_saves(cx);
        }
        if !self.facts().is_some_and(|f| f.cloud_saves) {
            return vec![Self::c_info("no-saves", tr!("Cloud saves"), tr!("This game does not support them"))];
        }
        if self.saves.detecting {
            return vec![Self::c_info("detecting", tr!("Saves folder"), tr!("Detecting\u{2026}"))];
        }
        let has_path = !self.text("savesPath").is_empty();
        let mut v = vec![
            Self::c_header(tr!("Location (beta: back up your saves first)")),
            self.c_path("savesPath", tr!("Saves folder"), true, cx),
            Self::c_do("detect", tr!("Detect the saves folder again"), Icon::FaSearch, true, |this, window, cx| {
                this.detect_saves(window, cx)
            }),
            Self::c_header(tr!("Sync")),
        ];
        let opts = MODES.iter().map(|(l, _)| SharedString::from(heronite_i18n::tr(l))).collect();
        v.push(item(
            Row::choice("c:saves-mode", tr!("Manual sync"), opts, self.saves.mode).enabled(has_path),
            Act::Choice(Rc::new(|this, i, cx| {
                this.saves.mode = i;
                cx.notify();
            })),
        ));
        v.push(Self::c_do(
            "sync",
            if self.saves.syncing { tr!("Syncing\u{2026}") } else { tr!("Sync now") },
            Icon::CloudSync,
            has_path && !self.saves.syncing,
            |this, _, cx| this.sync_saves(cx),
        ));
        if let Some(lines) = &self.saves.output {
            for (i, l) in lines.iter().rev().take(4).enumerate() {
                v.push(Self::c_info(&format!("out{i}"), "", l.clone()));
            }
        }
        v.push(Self::c_header(tr!("Automatic")));
        v.push(self.c_toggle(
            "autoSyncSaves",
            false,
            tr!("Autosync saves"),
            Some(tr!("Before the game starts and after it ends")),
            !has_path,
        ));
        if has_path {
            v.push(self.c_toggle("enableQuickSavesMenu", false, tr!("Quick save-sync menu on the game page"), None, false));
        }
        v
    }

    fn c_general(&self, cx: &App) -> Vec<Item> {
        let no_tray = self.flag("noTrayIcon", false);
        let with_system = self.flag("startWithSystem", false);
        let app_paths = heronite_data::AppPaths::for_data(&self.paths);
        let (choices, names, selected) = super::general::language_options(&app_paths);
        let mut v = vec![
            Self::c_header(tr!("Language")),
            item(
                Row::choice("c:language", tr!("Interface language"), names, selected).icon(Icon::Language),
                Act::Choice(Rc::new(move |_, i, cx| {
                    if let Some(&p) = choices.get(i) {
                        crate::language::choose(&app_paths, p, cx);
                    }
                })),
            ),
            Self::c_header(tr!("Folders")),
            self.c_path("defaultInstallPath", tr!("Default install folder"), true, cx),
            self.c_path("defaultWinePrefixDir", tr!("Folder for new Wine prefixes"), true, cx),
            self.c_path("defaultSteamPath", tr!("Steam folder"), true, cx),
            Self::c_header(tr!("Startup")),
            self.c_toggle("startWithSystem", false, tr!("Start with the system"), None, false),
            self.c_toggle("startInTray", false, tr!("Start minimized"), None, !with_system || no_tray),
            Self::c_header(tr!("System Tray")),
            self.c_toggle("exitToTray", false, tr!("Exit to tray"), None, no_tray),
            self.c_toggle("darkTrayIcon", false, tr!("Dark tray icon (needs restart)"), None, no_tray),
            self.c_toggle("noTrayIcon", false, tr!("Hide the tray icon (needs restart)"), None, false),
            Self::c_header(tr!("When a game starts")),
            self.c_toggle("minimizeOnLaunch", false, tr!("Minimize after game launch"), None, false),
            self.c_toggle("hideWindowOnProtocolLaunch", false, tr!("Hide the window for heronite:// launches"), None, false),
            Self::c_header(tr!("Game Shortcuts")),
            self.c_toggle("addDesktopShortcuts", false, tr!("Add desktop shortcuts automatically"), None, false),
            self.c_toggle("addStartMenuShortcuts", false, tr!("Add games to the start menu automatically"), None, false),
            self.c_toggle("addSteamShortcuts", false, tr!("Add games to Steam automatically"), None, false),
            Self::c_header(tr!("Wine and Proton")),
            self.c_toggle(
                "showValveProton",
                false,
                tr!("Allow Valve Proton builds"),
                Some(tr!("Custom forks (GE-Proton, Proton-CachyOS) are recommended")),
                false,
            ),
            self.c_toggle("downloadProtonToSteam", false, tr!("Download GE-Proton to Steam"), None, false),
            Self::c_header(tr!("Custom Wine / Proton paths")),
        ];
        for (i, p) in self.custom_paths().into_iter().enumerate() {
            v.push(item(
                Row::info(SharedString::from(format!("wp:{i}")), p.clone(), tr!("A removes")),
                Act::Do(Rc::new(move |this, _, cx| {
                    let mut now = this.custom_paths();
                    if i < now.len() {
                        now.remove(i);
                        this.set("customWinePaths", json!(now), cx);
                    }
                })),
            ));
        }
        v.push(Self::c_do("add-wine-path", tr!("Add a Wine or Proton binary\u{2026}"), Icon::Add, true, |this, window, cx| {
            this.console_pick_file(tr!("Wine or Proton binary"), Vec::new(), window, cx, |this, path, cx| {
                let mut now = this.custom_paths();
                if !now.contains(&path) {
                    now.push(path);
                    this.set("customWinePaths", json!(now), cx);
                }
            })
        }));
        v
    }

    fn c_downloads(&self, cx: &App) -> Vec<Item> {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let current = self.number("maxWorkers").unwrap_or(0).max(0) as usize;
        let mut opts: Vec<SharedString> = vec![tr!("Max").into()];
        opts.extend((1..=cores).map(|n| SharedString::from(n.to_string())));
        vec![
            Self::c_header(tr!("Downloads")),
            item(
                Row::choice("c:workers", tr!("Download workers"), opts, current.min(cores))
                    .desc(tr!("More workers download faster but use more CPU, memory and disk")),
                Act::Choice(Rc::new(|this, i, cx| this.set("maxWorkers", json!(i as i64), cx))),
            ),
            self.c_path("altLegendaryBin", tr!("Alternative legendary binary"), false, cx),
            self.c_path("altGogdlBin", tr!("Alternative gogdl binary"), false, cx),
            self.c_path("altNileBin", tr!("Alternative nile binary"), false, cx),
            self.c_toggle("downloadNoHttps", false, tr!("Download without HTTPS (LanCache)"), None, false),
            Self::c_header(tr!("Updates")),
            self.c_toggle("checkForUpdatesOnStartup", true, tr!("Check for game updates at startup"), None, false),
            self.c_toggle("autoUpdateGames", false, tr!("Update games automatically"), None, false),
        ]
    }

    // ------------------------------------------------------------- editors

    fn console_osk(
        &mut self,
        title: &str,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl Fn(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let osk = cx.new(|_| Osk::new(title.to_string(), initial));
        let sub = cx.subscribe_in(&osk, window, move |this, _, ev: &OskEvent, window, cx| {
            this.console_layer.close();
            if let OskEvent::Done(text) = ev {
                done(this, text.clone(), window, cx);
            }
            cx.notify();
        });
        self.console_layer.open(&osk, sub);
    }

    fn console_pick_file(
        &mut self,
        title: &str,
        exts: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl Fn(&mut Self, String, &mut Context<Self>) + 'static,
    ) {
        let browser = cx.new(|cx| Browser::new(title.to_string(), PickKind::File(exts), None, cx));
        let sub = cx.subscribe_in(&browser, window, move |this, _, ev: &BrowserEvent, _, cx| {
            this.console_layer.close();
            if let BrowserEvent::Picked(p) = ev {
                done(this, p.display().to_string(), cx);
            }
            cx.notify();
        });
        self.console_layer.open(&browser, sub);
    }

    fn console_pick_path(&mut self, id: &'static str, dir: bool, title: &str, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.input_text(id, cx);
        let start = (!current.is_empty()).then(|| std::path::PathBuf::from(&current));
        let kind = if dir { PickKind::Folder } else { PickKind::File(Vec::new()) };
        let browser = cx.new(|cx| Browser::new(title.to_string(), kind, start.as_deref(), cx));
        let sub = cx.subscribe_in(&browser, window, move |this, _, ev: &BrowserEvent, window, cx| {
            this.console_layer.close();
            if let BrowserEvent::Picked(p) = ev {
                this.set_input(id, &p.display().to_string(), window, cx);
                this.commit(id, cx);
            }
            cx.notify();
        });
        self.console_layer.open(&browser, sub);
    }

    /// Name then value on the keyboard, then saved like the desktop's
    /// Add / Update.
    fn console_pair_edit(&mut self, t: PairTable, existing: Option<(String, String)>, window: &mut Window, cx: &mut Context<Self>) {
        let (a, b) = existing.unwrap_or_default();
        let first = if t.env { tr!("Variable name") } else { tr!("Wrapper command") };
        let second = if t.env { tr!("Value") } else { tr!("Wrapper arguments") };
        self.console_osk(first, &a, window, cx, move |this, name, window, cx| {
            this.set_input(t.in1, &name, window, cx);
            let b = b.clone();
            this.console_osk(second, &b, window, cx, move |this, value, window, cx| {
                this.set_input(t.in2, &value, window, cx);
                this.pair_add(t, window, cx);
            });
        });
    }

    fn console_pair_sheet(&mut self, t: PairTable, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((a, b)) = pairs(self.settings.get(t.setting), t.k1, t.k2).get(i).cloned() else { return };
        let rows = vec![
            Row::action("edit", tr!("Edit")).icon(Icon::Edit),
            Row::action("remove", tr!("Remove")).icon(Icon::DeleteOutline).danger(),
        ];
        let sheet = cx.new(|_| Sheet::new(a.clone(), Some(b.clone()), rows));
        let sub = cx.subscribe_in(&sheet, window, move |this, _, ev: &SheetEvent, window, cx| {
            let SheetEvent::Activate(id) = ev;
            this.console_layer.close();
            match id.as_ref() {
                "edit" => this.console_pair_edit(t, Some((a.clone(), b.clone())), window, cx),
                "remove" => this.pair_remove(t, i, cx),
                _ => {}
            }
            cx.notify();
        });
        self.console_layer.open(&sheet, sub);
    }

    /// The rows in a scroll area, plus the open keyboard / picker over the
    /// whole window.
    pub fn render_console(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let rows: Vec<Row> = self.console_items(cx).into_iter().map(|i| i.row).collect();
        let active = !self.console_layer.is_open() && self.console_active;
        let list = self.console_list.render(SharedString::from(format!("console-hs-{:?}", self.section)), &rows, active);
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(list.flex_1().min_h_0().pb(u(10.)))
            .children(self.console_layer.view().map(|v| kit::window_layer(window, v)))
            .into_any_element()
    }

    /// Whether the panel's rows have the focus (the host may focus its own
    /// rows, like the Settings section list).
    pub fn set_console_active(&mut self, on: bool) {
        self.console_active = on;
    }

    pub fn console_reset(&mut self) {
        self.console_list.reset();
    }
}

impl PadTarget for SettingsPanel {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.console_layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        // B leaves the Winetricks page (not while it installs).
        if pad == Pad::B && self.winetricks_open() {
            self.close_winetricks(cx);
            return true;
        }
        let items = self.console_items(cx);
        let rows: Vec<Row> = items.iter().map(|i| i.row.clone()).collect();
        let act = |id: &SharedString| items.iter().find(|i| &i.row.id == id).map(|i| i.act.clone()).unwrap_or(Act::None);
        if pad == Pad::X {
            // Clears a path / text row.
            if let Some(Act::Path(input, ..) | Act::Text(input, _)) = self.console_list.focused(&rows).map(|r| act(&r.id)) {
                self.set_input(input, "", window, cx);
                self.commit(input, cx);
                cx.notify();
                return true;
            }
            return false;
        }
        if pad == Pad::Y {
            // Resets the focused setting when it differs from the defaults.
            let group = self.console_list.focused(&rows).and_then(|r| self.console_changed(&r.id));
            if let Some(g) = group {
                self.reset(&[g], cx);
                return true;
            }
            return false;
        }
        let outcome = self.console_list.on_pad(pad, &rows);
        match outcome {
            Outcome::Changed(id, value) => match (act(&id), value) {
                (Act::Toggle(key, default), _) => self.toggle(key, default, cx),
                (Act::GsToggle(field), _) => {
                    let now = self.gs_flag(field);
                    self.set_gamescope(field, Value::Bool(!now), cx);
                }
                (Act::Choice(pick), RowValue::Choice { index, .. }) => pick(self, index, cx),
                _ => {}
            },
            Outcome::Activate(id) => match act(&id) {
                Act::Text(input, title) => {
                    let current = self.input_text(input, cx);
                    self.console_osk(&title, &current, window, cx, move |this, text, window, cx| {
                        this.set_input(input, &text, window, cx);
                        this.commit(input, cx);
                    });
                }
                Act::Path(input, dir, title) => self.console_pick_path(input, dir, &title, window, cx),
                Act::Do(run) => run(self, window, cx),
                _ => {}
            },
            Outcome::Ignored => return false,
            Outcome::Edge(_) => return false,
            Outcome::Moved => {}
        }
        cx.notify();
        true
    }

    fn hints(&self, cx: &App) -> Vec<Hint> {
        if let Some(h) = self.console_layer.hints(cx) {
            return h;
        }
        let mut v = vec![hint(Btn::A, tr!("Select")), hint(Btn::LeftRight, tr!("Change")), hint(Btn::X, tr!("Clear"))];
        let items = self.console_items(cx);
        let rows: Vec<Row> = items.into_iter().map(|i| i.row).collect();
        if self.console_list.focused(&rows).is_some_and(|r| r.changed) {
            v.push(hint(Btn::Y, tr!("Reset")));
        }
        v
    }
}
