//! `HERONITE_DEBUG_*` hooks of the Collection: open a menu or dialog on start
//! so screenshots and tests can reach it without clicking.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::header::Menu;
use super::{context_menu, CollectionView};

impl CollectionView {
    /// `HERONITE_DEBUG_MENU=<card key>` opens its context menu (with the
    /// status flyout; `HERONITE_DEBUG_MENU_CATEGORIES=1` for the categories
    /// one), `HERONITE_DEBUG_DROPDOWN=view|nav` opens a header menu
    /// (`facets`, `categories`, `stores` or `view:<text>`: the View menu with
    /// that search typed; `views`: its views in edit mode; `views-select`:
    /// their select open), `HERONITE_DEBUG_DIALOG=<name>` opens a dialog on
    /// the first render that has a window (`open_pending_debug_dialog`).
    pub(super) fn apply_debug_env(&mut self) {
        if let Some(key) = crate::env::value("HERONITE_DEBUG_MENU") {
            self.context_menu = Some(context_menu::ContextMenuState {
                key: key.into(),
                position: point(px(200.), px(160.)),
                status_open: crate::env::value("HERONITE_DEBUG_MENU_INSTALL").is_none()
                    && crate::env::value("HERONITE_DEBUG_MENU_CATEGORIES").is_none(),
                install_open: crate::env::value("HERONITE_DEBUG_MENU_INSTALL").is_some(),
                categories_open: crate::env::value("HERONITE_DEBUG_MENU_CATEGORIES").is_some(),
            });
        }
        if let Some(which) = crate::env::value("HERONITE_DEBUG_DIALOG") {
            self.pending_debug_dialog = Some(which);
        }
        if let Some(menu) = crate::env::value("HERONITE_DEBUG_DROPDOWN") {
            let query = match menu.as_str() {
                "facets" => Some("developer:"),
                "categories" => Some("category:"),
                "stores" => Some("store:"),
                m => m.strip_prefix("view:"),
            };
            self.open_menu = match menu.as_str() {
                "view" | "install" | "sort" | "status" => Some(Menu::View),
                "views" => {
                    self.view_menu.set_editing(true);
                    Some(Menu::View)
                }
                "views-select" => {
                    self.view_menu.set_select_open(true);
                    Some(Menu::View)
                }
                "nav" => Some(Menu::Nav),
                _ if query.is_some() => Some(Menu::View),
                _ => None,
            };
            self.view_menu.debug_query = query.map(str::to_owned);
        }
    }

    pub(super) fn open_pending_debug_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(which) = self.pending_debug_dialog.take() else { return };
        match which.as_str() {
            w if w == "settings" || w.starts_with("settings:") => {
                let section = w.strip_prefix("settings:").and_then(crate::pages::settings::Section::parse);
                self.open_settings_section(section.unwrap_or(crate::pages::settings::Section::Appearance), window, cx);
            }
            "statuses" => self.open_manage_statuses(window, cx),
            "categories" => self.open_manage_categories(window, cx),
            // What Play shows for a store game whose folder is gone.
            w if w.starts_with("folder-missing:") => {
                self.epic.pending = Some(super::epic::Request::FolderMissing(w["folder-missing:".len()..].to_string()));
            }
            w if w == "onboarding" || w.starts_with("onboarding:") => {
                let step = w.strip_prefix("onboarding:").and_then(crate::dialogs::onboarding::Step::parse);
                self.open_onboarding_at(step, cx);
            }
            "wine-missing" => self.report_launch_error(heronite_launch::wine::no_wine_message(""), cx),
            "gallery" | "viewer" | "viewer-zoom" | "viewer-fit" | "confirm" | "delete" => self.shots.debug = Some(which),
            // Game settings tabs: gamesettings:wine|other|advanced|gamescope|gpu|saves|proton|backups
            w @ ("gamesettings:wine"
            | "gamesettings:gpu"
            | "gamesettings:other"
            | "gamesettings:advanced"
            | "gamesettings:gamescope"
            | "gamesettings:saves"
            | "gamesettings:proton"
            | "gamesettings:backups") => {
                use crate::dialogs::game_settings::Tab;
                let tab = match w {
                    "gamesettings:wine" | "gamesettings:proton" => Tab::Compatibility,
                    "gamesettings:advanced" => Tab::Launch,
                    "gamesettings:saves" => Tab::CloudSaves,
                    "gamesettings:backups" => Tab::Saves,
                    "gamesettings:gpu" => Tab::Gpu,
                    _ => Tab::Performance,
                };
                self.open_game_settings_tab(tab, window, cx);
            }
            "metadata" | "gamesettings" | "gamesettings:details" | "gamesettings:launch" | "webimage" | "wizard" => {
                self.open_metadata_debug(which.as_str(), window, cx)
            }
            "viewlog" => {
                if let Some(key) = self.selected.as_ref().map(|k| k.to_string()) {
                    self.open_game_log(&key, cx);
                }
            }
            w if w.starts_with("manual-install:") => self.open_manual_install(&w["manual-install:".len()..], window, cx),
            w if w.starts_with("play-as-pc:") => self.open_play_as_pc(&w["play-as-pc:".len()..], window, cx),
            w if w.starts_with("local-uninstall:") => self.open_sideload_uninstall(&w["local-uninstall:".len()..], window, cx),
            w if matches!(w, "playtime" | "remove" | "ludusavi" | "steam-missing") || w.starts_with("ludusavi:") => {
                self.open_extras_debug(which.as_str(), window, cx)
            }
            w if w.starts_with("add-game") || w.starts_with("edit-game") => {
                self.add_game_debug_dialog(w, window, cx);
            }
            other => {
                if !self.epic_debug_dialog(other, window, cx) {
                    self.open_emu_debug_dialog(other, window, cx);
                }
            }
        }
    }

    /// `HERONITE_DEBUG_BROWSE=<count>@<ms>`: selects the next `count` games of
    /// the list, one every `ms`, like a user browsing (memory checks).
    pub(super) fn spawn_debug_browse(&self, cx: &mut Context<Self>) {
        let Some(spec) = crate::env::value("HERONITE_DEBUG_BROWSE") else { return };
        let (count, ms) = spec.split_once('@').unwrap_or((spec.as_str(), "300"));
        let (Ok(count), Ok(ms)) = (count.parse::<usize>(), ms.parse::<u64>()) else { return };
        cx.spawn(async move |this, cx| {
            for i in 0..count {
                cx.background_executor().timer(std::time::Duration::from_millis(ms)).await;
                let alive = this.update(cx, |this, cx| {
                    let keys: Vec<SharedString> = this.groups.iter().flat_map(|g| g.cards.iter().map(|c| c.key.clone())).collect();
                    if let Some(key) = keys.get(i % keys.len().max(1)).cloned() {
                        this.select(key, false);
                        this.reveal_selected();
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
            log::info!("[debug] browse done");
        })
        .detach();
    }
}
