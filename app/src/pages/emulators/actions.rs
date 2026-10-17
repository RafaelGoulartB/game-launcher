//! Background actions of the Emulators page: open or duplicate an emulator
//! and scan ROM folders. They run off the UI thread and finish with a notice
//! or a toast.

use gpui_kit::prelude::*;
use heronite_emulation::{duplicate_user_emulator, run_rom_scanners, AutoScanReport};
use heronite_i18n::{tr, trn};
use heronite_launch::plan::{open_emulator_command, PlanContext};
use heronite_launch::probe::{Probe, SystemProbe};
use heronite_launch::settings::GameSettings;

use super::{scan_notice, EmulatorsPage, Notice};
use crate::pages::PageEvent;

impl EmulatorsPage {
    /// Starts the emulator on its own, without a game (to change its
    /// settings, install firmware...). Windows emulators run in the default
    /// Wine prefix with the default Wine.
    pub(super) fn open_emulator(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(e) = self.summaries.iter().find(|s| s.emulator.id == id).map(|s| s.emulator.clone()) else { return };
        let paths = self.ctx.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let name = e.name.clone();
            let res = cx
                .background_spawn(async move {
                    let probe = SystemProbe;
                    let ctx = PlanContext::load(&paths);
                    let home = probe.home();
                    let settings = GameSettings::from_map(&heronite_data::game_config::load_global(&paths, &home), &home);
                    let c = open_emulator_command(&ctx, &probe, &settings, &e)?;
                    log::info!("opening emulator {}: {}", e.name, c.display());
                    crate::runtime::spawn_detached(&c)
                })
                .await;
            this.update(cx, |this, cx| match res {
                Ok(()) => this.toast(tr!("Opening {name}\u{2026}", name), false, cx),
                Err(e) => this.toast(e, true, cx),
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn duplicate(&mut self, id: &str, cx: &mut Context<Self>) {
        let (paths, id) = (self.ctx.data_paths.clone(), id.to_string());
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { duplicate_user_emulator(&paths, &id) }).await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(e) => this.notice = Some(Notice { lines: vec![tr!("Created {name}.", name = e.name)], error: false }),
                    Err(e) => this.notice = Some(Notice { lines: vec![format!("{e:#}")], error: true }),
                }
                this.reload(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Scans the given saved folders now (even those with "scan on refresh"
    /// off) and imports the games not imported yet.
    pub(super) fn scan(&mut self, ids: Vec<String>, label: &'static str, cx: &mut Context<Self>) {
        if self.busy.is_some() || ids.is_empty() {
            return;
        }
        self.busy = Some(label.into());
        self.notice = None;
        let paths = self.ctx.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let report = cx
                .background_spawn(async move {
                    let mut total = AutoScanReport::default();
                    for id in ids {
                        match run_rom_scanners(&paths, Some(&id)) {
                            Ok(r) => {
                                total.scanned += r.scanned;
                                total.imported += r.imported;
                                total.errors.extend(r.errors);
                            }
                            Err(e) => total.errors.push(format!("{e:#}")),
                        }
                    }
                    total
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = None;
                let notice = scan_notice(&report);
                if report.imported > 0 {
                    this.toast(trn!(report.imported, "Added {n} new game", "Added {n} new games"), false, cx);
                    cx.emit(PageEvent::LibraryChanged);
                }
                this.notice = Some(notice);
                this.reload(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn scan_all(&mut self, cx: &mut Context<Self>) {
        let ids = self.summaries.iter().flat_map(|s| s.scanners.iter().map(|x| x.id.clone())).collect();
        self.scan(ids, tr!("Scanning all ROM folders\u{2026}"), cx);
    }
}
