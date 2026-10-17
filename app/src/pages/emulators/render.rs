//! Rendering of the Emulators page: toolbar, notices and one card per
//! emulator with its ROM folders.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::manager::{EmulatorSummary, Health};
use heronite_emulation::RomScanner;
use heronite_i18n::{tr, trn};

use super::EmulatorsPage;
use crate::dialogs::emulator_detect::DetectEmulatorsDialog;
use crate::dialogs::scan_roms::{profile_label, ScanInitial};
use crate::icons::Icon;
use crate::pages::{status_pill, tag_pill, PageEvent};
use crate::theme;
use crate::ui::controls::{action_button, button, button_sm, card_box, icon_button_sm, ButtonKind};
use crate::ui::fields::{meta_text, paragraph};
use crate::ui::icon;

fn health_badge(health: Health) -> Div {
    div().flex().child(status_pill(health.label(), if health.is_ok() { theme::success() } else { theme::danger_text() }))
}

pub(super) fn launch_label(kind: &str) -> &'static str {
    match kind {
        "flatpak" => "Flatpak",
        "wine" => "Wine",
        _ => tr!("Native"),
    }
}

pub(super) fn folder_flags(s: &RomScanner) -> String {
    let mut flags = vec![];
    flags.push(if s.scan_subfolders { tr!("Subfolders") } else { tr!("Top folder only") });
    if s.merge_related_files {
        flags.push(tr!("Discs merged"));
    }
    flags.push(if s.auto_scan_on_refresh { tr!("Scanned on refresh") } else { tr!("Manual scan") });
    flags.join(" \u{b7} ")
}

fn separator() -> Div {
    div().h(px(1.)).w_full().bg(theme::row_separator())
}

/// Card shown when no emulator is configured.
pub(super) fn render_empty() -> Div {
    card_box().py(px(16.)).child(paragraph(tr!(
        "No emulators configured yet. Detect the ones installed on this PC, or add one from the catalog or as a custom program."
    )))
}

impl EmulatorsPage {
    pub(super) fn render_toolbar(&self, folders: usize, cx: &mut Context<Self>) -> Div {
        let busy = self.busy.is_some();
        div()
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .child(
                button("emus-detect", ButtonKind::Glass, tr!("Detect emulators\u{2026}"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_detect(window, cx))),
            )
            .child(button("emus-scan-folder", ButtonKind::Glass, tr!("Scan a folder for emulators\u{2026}")).on_click(cx.listener(
                |this, _, window, cx| {
                    this.open_detect(window, cx);
                    if let Some(d) = this.open_dialog_as::<DetectEmulatorsDialog>() {
                        d.update(cx, |d, cx| d.pick_folder(window, cx));
                    }
                },
            )))
            .child(
                button("emus-add-folder", ButtonKind::Glass, tr!("Add ROM folder"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_folder_dialog(None, None, window, cx))),
            )
            .child(action_button(
                "emus-scan-all",
                ButtonKind::Glass,
                tr!("Scan all folders now"),
                !busy && folders > 0,
                cx.listener(|this, _, _, cx| this.scan_all(cx)),
            ))
    }

    /// The line shown while a background scan runs.
    pub(super) fn render_busy(&self) -> Option<Div> {
        let label = self.busy.clone()?;
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .mx(px(6.))
                .text_size(px(13.))
                .text_color(theme::accent())
                .child(icon(Icon::Refresh, px(16.), theme::accent()))
                .child(label),
        )
    }

    /// The result of the last action with a "Dismiss" button.
    pub(super) fn render_notice(&self, cx: &mut Context<Self>) -> Option<Div> {
        let n = self.notice.clone()?;
        let color = if n.error { theme::danger_text() } else { theme::success() };
        let mut lines = div().flex().flex_col().gap(px(2.)).flex_1().min_w_0().py(px(12.));
        for (i, l) in n.lines.iter().enumerate() {
            lines = lines.child(div().text_size(px(13.)).text_color(if i == 0 { color } else { theme::text_muted() }).child(l.clone()));
        }
        Some(card_box().flex_row().items_center().gap(px(12.)).child(lines).child(
            button_sm("emus-notice-x", ButtonKind::Ghost, tr!("Dismiss")).on_click(cx.listener(|this, _, _, cx| {
                this.notice = None;
                cx.notify();
            })),
        ))
    }

    /// An emulator: its header, then "ROM folders" and one row per folder.
    pub(super) fn render_card(&self, s: &EmulatorSummary, cx: &mut Context<Self>) -> Div {
        let mut card = card_box().child(self.render_emulator_header(s, cx)).child(separator()).child(self.render_folders_header(s, cx));
        if s.scanners.is_empty() {
            card = card
                .child(div().pb(px(14.)).child(meta_text(tr!("No ROM folders yet. Add one and its games are added to the Collection."))));
        }
        for sc in &s.scanners {
            card = card.child(separator()).child(self.render_folder_row(s, sc, cx));
        }
        card
    }

    /// Name, launch kind, health, location, games using it, platforms and the
    /// Open / Edit / Duplicate / Remove buttons.
    fn render_emulator_header(&self, s: &EmulatorSummary, cx: &mut Context<Self>) -> Div {
        let e = &s.emulator;
        let id = e.id.clone();
        let location = if e.launch_kind == "flatpak" {
            format!("Flatpak \u{b7} {}", e.flatpak_id.clone().unwrap_or_default())
        } else if e.executable.is_empty() {
            tr!("No executable set").to_string()
        } else {
            e.executable.clone()
        };
        let mut platforms: Vec<String> = e
            .profiles
            .iter()
            .map(|p| {
                p.platform_name.clone().filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case("other")).unwrap_or_else(|| p.name.clone())
            })
            .collect();
        platforms.dedup();
        let extra = platforms.len().saturating_sub(8);
        platforms.truncate(8);

        let actions = div()
            .flex()
            .flex_none()
            .flex_wrap()
            .justify_end()
            .gap(px(8.))
            .child({
                let id = id.clone();
                icon_button_sm(
                    SharedString::from(format!("emu-open-{id}")),
                    ButtonKind::Primary,
                    Icon::PlayArrow,
                    tr!("Open"),
                    s.health.is_ok(),
                    cx.listener(move |this, _, _, cx| this.open_emulator(&id, cx)),
                )
            })
            .child({
                let id = id.clone();
                button_sm(SharedString::from(format!("emu-edit-{id}")), ButtonKind::Glass, tr!("Edit"))
                    .on_click(cx.listener(move |this, _, window, cx| this.open_edit_emulator(&id, window, cx)))
            })
            .child({
                let id = id.clone();
                button_sm(SharedString::from(format!("emu-dup-{id}")), ButtonKind::Glass, tr!("Duplicate"))
                    .on_click(cx.listener(move |this, _, _, cx| this.duplicate(&id, cx)))
            })
            .child({
                let id = id.clone();
                button_sm(SharedString::from(format!("emu-rm-{id}")), ButtonKind::Danger, tr!("Remove"))
                    .on_click(cx.listener(move |this, _, window, cx| this.confirm_remove_emulator(&id, window, cx)))
            });

        div()
            .flex()
            .justify_between()
            .items_start()
            .gap(px(12.))
            .py(px(14.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(10.))
                            .child(
                                div()
                                    .font_family(theme::FONT_HEADING)
                                    .font_weight(theme::W_BOLD)
                                    .text_size(px(18.))
                                    .text_color(theme::text_primary())
                                    .child(e.name.clone()),
                            )
                            .child(tag_pill(launch_label(&e.launch_kind)))
                            .child(health_badge(s.health)),
                    )
                    .child(div().text_size(px(13.)).text_color(theme::text_primary()).truncate().child(location))
                    .child(meta_text(trn!(s.games, "{n} game using it", "{n} games using it")))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(6.))
                            .mt(px(4.))
                            .children(platforms.into_iter().map(tag_pill))
                            .children((extra > 0).then(|| tag_pill(tr!("+{n} more", n = extra)))),
                    ),
            )
            .child(actions)
    }

    /// "ROM folders" label with the "Add ROM folder" button.
    fn render_folders_header(&self, s: &EmulatorSummary, cx: &mut Context<Self>) -> Div {
        let id = s.emulator.id.clone();
        div()
            .flex()
            .items_center()
            .justify_between()
            .py(px(10.))
            .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(tr!("ROM folders")))
            .child(
                button_sm(SharedString::from(format!("emu-addfolder-{id}")), ButtonKind::Glass, tr!("Add ROM folder"))
                    .on_click(cx.listener(move |this, _, window, cx| this.open_folder_dialog(Some(id.clone()), None, window, cx))),
            )
    }

    /// One ROM folder: path, profile and flags, and the Scan now / Review /
    /// Edit / Remove buttons.
    fn render_folder_row(&self, s: &EmulatorSummary, sc: &RomScanner, cx: &mut Context<Self>) -> Div {
        let busy = self.busy.is_some();
        let sid = sc.id.clone();
        let missing = s.missing_folders.contains(&sc.directory);
        let profile = s.emulator.profile(&sc.profile_id).map(profile_label).unwrap_or_else(|| tr!("Unknown profile").into());
        let init = ScanInitial {
            emulator_id: Some(sc.emulator_id.clone()),
            profile_id: Some(sc.profile_id.clone()),
            directory: Some(sc.directory.clone()),
        };
        let scanner = sc.clone();
        div()
            .flex()
            .justify_between()
            .items_center()
            .gap(px(12.))
            .py(px(11.))
            .child(icon(Icon::Folder, px(18.), theme::text_muted()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().text_size(px(14.)).text_color(theme::text_primary()).truncate().child(sc.directory.clone()))
                            .children(missing.then(|| status_pill(tr!("Folder not found"), theme::danger_text()))),
                    )
                    .child(meta_text(format!("{profile}  \u{b7}  {}", folder_flags(sc)))),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap(px(6.))
                    .child({
                        let sid = sid.clone();
                        let b = button_sm(SharedString::from(format!("scan-now-{sid}")), ButtonKind::Glass, tr!("Scan now"));
                        if !busy && !missing {
                            b.on_click(cx.listener(move |this, _, _, cx| this.scan(vec![sid.clone()], tr!("Scanning\u{2026}"), cx)))
                        } else {
                            b.opacity(0.45).cursor_not_allowed()
                        }
                    })
                    .child(
                        button_sm(SharedString::from(format!("scan-review-{sid}")), ButtonKind::Glass, tr!("Review\u{2026}"))
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(PageEvent::ScanRoms(init.clone())))),
                    )
                    .child({
                        let scanner = scanner.clone();
                        button_sm(SharedString::from(format!("scan-edit-{sid}")), ButtonKind::Ghost, tr!("Edit")).on_click(
                            cx.listener(move |this, _, window, cx| this.open_folder_dialog(None, Some(scanner.clone()), window, cx)),
                        )
                    })
                    .child({
                        let scanner = scanner.clone();
                        button_sm(SharedString::from(format!("scan-rm-{sid}")), ButtonKind::Ghost, tr!("Remove"))
                            .on_click(cx.listener(move |this, _, window, cx| this.confirm_remove_folder(&scanner, window, cx)))
                    }),
            )
    }
}
