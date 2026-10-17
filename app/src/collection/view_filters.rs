//! The View menu's sort direction, "Only" switches and custom categories,
//! and the category edits of a game. The menu itself is drawn in `header.rs`; the categories are
//! `heronite_data::categories`.

use gpui_kit::*;
use heronite_data::categories::{self, Categories, UNCATEGORIZED};
use heronite_i18n::tr;

use super::CollectionView;
use crate::services::downloads::Downloads;

/// One of the View menu's "Only" switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Only {
    Favourites,
    Updates,
    Offline,
}

impl Only {
    pub const ALL: [Only; 3] = [Only::Favourites, Only::Updates, Only::Offline];

    pub fn label(self) -> &'static str {
        match self {
            Only::Favourites => tr!("Favourites"),
            Only::Updates => tr!("With an update"),
            Only::Offline => tr!("Run offline"),
        }
    }
}

/// How a selected category reads in the menu and its chip.
pub(super) fn category_label(name: &str) -> String {
    if name == UNCATEGORIZED {
        tr!("Uncategorized").to_string()
    } else {
        name.to_string()
    }
}

impl CollectionView {
    pub(super) fn only_on(&self, only: Only) -> bool {
        match only {
            Only::Favourites => self.ui.only.favourites,
            Only::Updates => self.ui.only.updates,
            Only::Offline => self.ui.only.offline,
        }
    }

    pub(super) fn set_only(&mut self, only: Only, on: bool, cx: &mut Context<Self>) {
        match only {
            Only::Favourites => self.ui.only.favourites = on,
            Only::Updates => self.ui.only.updates = on,
            Only::Offline => self.ui.only.offline = on,
        }
        if only == Only::Updates && on {
            self.update_apps = crate::services::downloads(cx).read(cx).update_apps();
        }
        self.save_ui();
        self.refresh_groups();
        cx.notify();
    }

    pub(super) fn toggle_sort_direction(&mut self, cx: &mut Context<Self>) {
        self.ui.sort_descending = !self.ui.sort_descending;
        self.save_ui();
        self.refresh_groups();
        cx.notify();
    }

    /// The store service changed: "Only with updates" follows its list.
    pub(super) fn sync_update_apps(&mut self, svc: &Entity<Downloads>, cx: &mut Context<Self>) {
        if !self.ui.only.updates {
            return;
        }
        let apps = svc.read(cx).update_apps();
        if apps != self.update_apps {
            self.update_apps = apps;
            self.refresh_groups();
        }
    }

    /// Selects or clears one category of the filter (several add up).
    pub(super) fn toggle_category_filter(&mut self, name: &str, cx: &mut Context<Self>) {
        self.ui.toggle_category(name);
        self.save_ui();
        self.refresh_groups();
        cx.notify();
    }

    /// Re-reads the categories after an edit (the library itself is unchanged).
    pub(super) fn reload_categories(&mut self, cx: &mut Context<Self>) {
        self.categories = Categories::load(&self.data_paths);
        self.prune_category_filter();
        self.refresh_groups();
        cx.notify();
    }

    /// A removed or renamed category no longer filters.
    pub(super) fn prune_category_filter(&mut self) {
        let cats = &self.categories;
        let before = self.ui.categories.len();
        self.ui.categories.retain(|n| n == UNCATEGORIZED || cats.exists(n));
        if self.ui.categories.len() != before {
            self.save_ui();
        }
    }

    /// Puts the game under `key` in `name`, or takes it out.
    pub(super) fn set_game_category(&mut self, key: &str, name: &str, on: bool, cx: &mut Context<Self>) {
        let Some(g) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        if let Err(err) = categories::set_in_category(&self.data_paths, name, &g.runner, &g.app_name, on) {
            self.show_toast(tr!("Could not change the categories: {err}", err = format!("{err:#}")), true, cx);
        }
        self.reload_categories(cx);
    }

    pub(super) fn open_manage_categories(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ctx) = self.dialog_ctx() else { return };
        self.open_menu = None;
        self.context_menu = None;
        let view = cx.new(|cx| crate::dialogs::manage_categories::ManageCategoriesDialog::new(ctx, window, cx));
        self.open_dialog(view, cx);
    }
}
