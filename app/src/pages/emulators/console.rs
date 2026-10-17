//! The Emulators page in console mode: the global actions first, then one
//! group per emulator with the emulator itself and its ROM folders. A on an
//! emulator or a folder opens a sheet with what the desktop buttons do; the
//! dialogs those flows open take the controller through the registry.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::manager::EmulatorSummary;
use heronite_i18n::{tr, trn};

use super::render::{folder_flags, launch_label};
use super::EmulatorsPage;
use crate::console::kit::{self, u};
use crate::console::layers::{open_sheet, HasLayer};
use crate::console::list::{Outcome, Row};
use crate::console::{hint, Btn, Hint, Layer, Pad, PadTarget};
use crate::dialogs::emulator_detect::DetectEmulatorsDialog;
use crate::dialogs::scan_roms::{profile_label, ScanInitial};
use crate::icons::Icon;
use crate::pages::PageEvent;
use crate::theme;

impl HasLayer for EmulatorsPage {
    fn layer_mut(&mut self) -> &mut Layer {
        &mut self.layer
    }
}

fn location(s: &EmulatorSummary) -> String {
    let e = &s.emulator;
    if e.launch_kind == "flatpak" {
        format!("Flatpak \u{b7} {}", e.flatpak_id.clone().unwrap_or_default())
    } else if e.executable.is_empty() {
        tr!("No executable set").to_string()
    } else {
        e.executable.clone()
    }
}

fn platforms(s: &EmulatorSummary) -> String {
    let mut names: Vec<String> = s
        .emulator
        .profiles
        .iter()
        .map(|p| p.platform_name.clone().filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case("other")).unwrap_or_else(|| p.name.clone()))
        .collect();
    names.dedup();
    let extra = names.len().saturating_sub(6);
    names.truncate(6);
    let mut text = names.join(" \u{b7} ");
    if extra > 0 {
        text.push_str(&format!(" \u{b7} {}", tr!("+{n} more", n = extra)));
    }
    text
}

impl EmulatorsPage {
    fn console_rows(&self) -> Vec<Row> {
        let busy = self.busy.is_some();
        let folders = self.total_folders();
        let mut rows = vec![
            Row::action("add", tr!("Add emulator\u{2026}")).desc(tr!("Pick one from the catalog or describe any program")).icon(Icon::Add),
            Row::action("detect", tr!("Detect emulators on this PC"))
                .desc(tr!("Looks in PATH, Flatpak and the usual folders"))
                .icon(Icon::TravelExplore),
            Row::action("scan-folder", tr!("Scan a folder for emulators\u{2026}"))
                .desc(tr!("Searches a folder you pick, for example your Emulation folder"))
                .icon(Icon::FolderOpen),
            Row::action("add-folder", tr!("Add ROM folder")).desc(tr!("Games found in it are added to the Collection")).icon(Icon::Folder),
            Row::action("scan-all", if busy { tr!("Scanning\u{2026}") } else { tr!("Scan all ROM folders") })
                .desc(tr!("Adds the games not imported yet from every saved folder"))
                .icon(Icon::Refresh)
                .enabled(!busy && folders > 0),
        ];
        for s in &self.summaries {
            let e = &s.emulator;
            rows.push(Row::header(format!("{} \u{b7} {} \u{b7} {}", e.name, launch_label(&e.launch_kind), s.health.label())));
            let mut desc = format!("{}  \u{b7}  {}", location(s), trn!(s.games, "{n} game using it", "{n} games using it"));
            let p = platforms(s);
            if !p.is_empty() {
                desc.push_str(&format!("  \u{b7}  {p}"));
            }
            rows.push(
                Row::info(format!("e:{}", e.id), tr!("Emulator"), location(s).rsplit('/').next().unwrap_or_default().to_string())
                    .desc(desc)
                    .icon(Icon::Settings),
            );
            for sc in &s.scanners {
                let missing = s.missing_folders.contains(&sc.directory);
                let profile = e.profile(&sc.profile_id).map(profile_label).unwrap_or_else(|| tr!("Unknown profile").into());
                let mut r = Row::action(format!("f:{}", sc.id), sc.directory.clone())
                    .desc(format!("{profile}  \u{b7}  {}", folder_flags(sc)))
                    .icon(Icon::Folder);
                if missing {
                    r = r.danger().desc(format!("{}  \u{b7}  {profile}", tr!("Folder not found")));
                }
                rows.push(r);
            }
            rows.push(Row::action(format!("af:{}", e.id), tr!("Add ROM folder")).icon(Icon::Add));
        }
        rows
    }

    fn console_summary(&self, id: &str) -> Option<&EmulatorSummary> {
        self.summaries.iter().find(|s| s.emulator.id == id)
    }

    fn console_activate(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "add" => self.open_add_emulator(window, cx),
            "detect" => self.open_detect(window, cx),
            "scan-folder" => {
                self.open_detect(window, cx);
                if let Some(d) = self.open_dialog_as::<DetectEmulatorsDialog>() {
                    d.update(cx, |d, cx| d.console_pick_folder(window, cx));
                }
            }
            "add-folder" => self.open_folder_dialog(None, None, window, cx),
            "scan-all" => self.scan_all(cx),
            _ => {
                if let Some(eid) = id.strip_prefix("af:") {
                    self.open_folder_dialog(Some(eid.to_string()), None, window, cx);
                } else if let Some(eid) = id.strip_prefix("e:") {
                    self.emulator_sheet(eid, window, cx);
                } else if let Some(sid) = id.strip_prefix("f:") {
                    self.folder_sheet(sid, window, cx);
                }
            }
        }
    }

    fn emulator_sheet(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.console_summary(id) else { return };
        let title = s.emulator.name.clone();
        let rows = vec![
            Row::action("open", tr!("Open emulator"))
                .desc(tr!("Starts it without a game, to change its settings"))
                .icon(Icon::PlayArrow)
                .enabled(s.health.is_ok()),
            Row::action("edit", tr!("Edit\u{2026}")).icon(Icon::Edit),
            Row::action("dup", tr!("Duplicate")).icon(Icon::Add),
            Row::action("rm", tr!("Remove\u{2026}")).icon(Icon::FaTrash).danger(),
        ];
        let id = id.to_string();
        open_sheet(&mut self.layer, title, None, rows, window, cx, move |this: &mut Self, pick, window, cx| match pick {
            "open" => this.open_emulator(&id, cx),
            "edit" => this.open_edit_emulator(&id, window, cx),
            "dup" => this.duplicate(&id, cx),
            "rm" => this.confirm_remove_emulator(&id, window, cx),
            _ => {}
        });
    }

    fn folder_sheet(&mut self, sid: &str, window: &mut Window, cx: &mut Context<Self>) {
        let found = self.summaries.iter().find_map(|s| s.scanners.iter().find(|x| x.id == sid).map(|x| (s, x.clone())));
        let Some((s, sc)) = found else { return };
        let missing = s.missing_folders.contains(&sc.directory);
        let rows = vec![
            Row::action("scan", if missing { tr!("Scan now (folder not found)") } else { tr!("Scan now") })
                .desc(tr!("Adds the games not imported yet"))
                .icon(Icon::Refresh)
                .enabled(self.busy.is_none() && !missing),
            Row::action("review", tr!("Preview and import\u{2026}")).desc(tr!("Choose which games to add")).icon(Icon::Visibility),
            Row::action("edit", tr!("Edit\u{2026}")).icon(Icon::Edit),
            Row::action("rm", tr!("Remove\u{2026}")).icon(Icon::FaTrash).danger(),
        ];
        let init = ScanInitial {
            emulator_id: Some(sc.emulator_id.clone()),
            profile_id: Some(sc.profile_id.clone()),
            directory: Some(sc.directory.clone()),
        };
        let title = sc.directory.clone();
        open_sheet(&mut self.layer, title, None, rows, window, cx, move |this: &mut Self, pick, window, cx| match pick {
            "scan" => this.scan(vec![sc.id.clone()], tr!("Scanning\u{2026}"), cx),
            "review" => cx.emit(PageEvent::ScanRoms(init.clone())),
            "edit" => this.open_folder_dialog(None, Some(sc.clone()), window, cx),
            "rm" => this.confirm_remove_folder(&sc, window, cx),
            _ => {}
        });
    }

    pub(super) fn render_console(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows();
        let overlay = self.layer.is_open() || self.dialog.is_some();
        let list = self.console_list.render("console-emulators", &rows, !overlay);

        let mut status = div().flex().flex_col().gap(u(4.));
        if let Some(label) = self.busy.clone() {
            status = status.child(kit::body_text(label).text_color(theme::accent()));
        }
        if let Some(n) = &self.notice {
            let color = if n.error { theme::danger_text() } else { theme::success() };
            for (i, l) in n.lines.iter().enumerate() {
                status = status.child(kit::body_text(l.clone()).text_color(if i == 0 { color } else { theme::text_muted() }));
            }
        }
        if !self.loaded {
            status = status.child(kit::body_text(tr!("Loading\u{2026}")));
        } else if self.summaries.is_empty() {
            status = status.child(kit::body_text(tr!("No emulators configured yet. Detect the ones installed on this PC, or add one.")));
        }
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(u(76.))
                    .px(u(46.))
                    .child(kit::wordmark(tr!("EMULATORS")))
                    .child(div().flex_1())
                    .child(kit::clock()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap(u(10.))
                    .px(u(46.))
                    .pb(u(16.))
                    .child(div().w(u(900.)).max_w_full().flex().flex_col().gap(u(8.)).child(status))
                    .child(div().w(u(900.)).max_w_full().flex().flex_col().flex_1().min_h_0().child(list.flex_1().min_h_0())),
            )
            .children(self.layer.view())
            .children(self.dialog.clone())
            .into_any_element()
    }
}

impl PadTarget for EmulatorsPage {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        if let Some(view) = self.dialog.clone() {
            let used = crate::console::registry::hook_for(&view).is_some_and(|h| h.pad(pad, window, cx));
            if !used && pad == Pad::B {
                self.dialog = None;
                self.dialog_sub = None;
            }
            cx.notify();
            return true;
        }
        if pad == Pad::X && self.notice.is_some() {
            self.notice = None;
            cx.notify();
            return true;
        }
        let rows = self.console_rows();
        let used = match self.console_list.on_pad(pad, &rows) {
            Outcome::Activate(id) => {
                self.console_activate(&id, window, cx);
                true
            }
            Outcome::Ignored => false,
            _ => true,
        };
        cx.notify();
        used
    }

    fn hints(&self, cx: &App) -> Vec<Hint> {
        if let Some(h) = self.layer.hints(cx) {
            return h;
        }
        if let Some(view) = &self.dialog {
            return crate::console::registry::hook_for(view).map(|h| h.hints(cx)).unwrap_or_else(|| vec![hint(Btn::B, tr!("Close"))]);
        }
        let mut h = vec![hint(Btn::A, tr!("Select"))];
        if self.notice.is_some() {
            h.push(hint(Btn::X, tr!("Dismiss")));
        }
        h
    }
}
