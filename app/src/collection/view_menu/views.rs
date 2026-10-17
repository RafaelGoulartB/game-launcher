//! Presets (saved views): everything the menu sets (filters, sort, grouping, "Show hidden
//! games") saved under a name and applied in one click. Four come with the
//! app; they are added to the user's list once (by status slug, so they
//! follow renamed statuses) and can then be edited or deleted like any other.

use std::collections::BTreeSet;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use heronite_data::models::CompletionStatus;
use heronite_data::ui_state::{SavedView, ViewFilters};
use heronite_data::{InstallFilter, Library, SortMode};
use heronite_i18n::tr;

use super::super::CollectionView;
use crate::theme;

/// Longest view name (one pill of the menu).
const NAME_MAX: usize = 32;

pub(super) struct ViewCard {
    pub name: String,
    /// Dot before the name: the view's first status, else purple for
    /// favourites, else the accent.
    pub color: Rgba,
    pub filters: ViewFilters,
}

/// The views that come with the app, for the statuses of this library.
fn default_views(statuses: &[CompletionStatus]) -> Vec<SavedView> {
    let ids = |slugs: &[&str]| -> BTreeSet<String> {
        slugs.iter().filter_map(|slug| statuses.iter().find(|s| s.slug == *slug)).map(|s| s.id.clone()).collect()
    };
    let view = |name: &str, f: ViewFilters| SavedView { name: name.into(), filters: f };
    let base = |sort, group| ViewFilters { sort, group_by_status: Some(group), show_hidden: Some(false), ..Default::default() };
    let mut out = Vec::new();
    for (name, slugs, f) in [
        (tr!("Playing now"), &["playing", "endless"][..], base(SortMode::LastPlayed, false)),
        (
            tr!("Backlog"),
            &["plan-to-play", "not-played"][..],
            ViewFilters { install_filter: InstallFilter::Installed, ..base(SortMode::Title, true) },
        ),
        (tr!("Finished"), &["beaten", "completed"][..], base(SortMode::Playtime, false)),
    ] {
        let statuses = ids(slugs);
        if !statuses.is_empty() {
            out.push(view(name, ViewFilters { statuses, ..f }));
        }
    }
    let mut favourites = base(SortMode::Playtime, false);
    favourites.only.favourites = true;
    out.push(view(tr!("Favourites"), favourites));
    out
}

impl CollectionView {
    pub(super) fn view_cards(&self) -> Vec<ViewCard> {
        self.ui
            .saved_views
            .iter()
            .map(|v| ViewCard { name: v.name.clone(), color: self.view_color(&v.filters), filters: v.filters.clone() })
            .collect()
    }

    fn view_color(&self, f: &ViewFilters) -> Rgba {
        let statuses = self.lib.as_deref().map(|l| l.statuses.as_slice()).unwrap_or(&[]);
        match statuses.iter().find(|s| f.statuses.contains(&s.id)) {
            Some(s) => theme::status_color(&s.slug),
            None if f.only.favourites => theme::status_color("completed"),
            None => theme::accent(),
        }
    }

    /// Adds the built-in views the first time a library is loaded.
    pub(in crate::collection) fn seed_default_views(&mut self, lib: &Library) {
        if self.ui.views_seeded {
            return;
        }
        let mut views = default_views(&lib.statuses);
        views.retain(|d| !self.ui.saved_views.iter().any(|v| v.name.eq_ignore_ascii_case(&d.name)));
        views.append(&mut self.ui.saved_views);
        self.ui.saved_views = views;
        self.ui.views_seeded = true;
        self.save_ui();
    }

    /// The built-in views that are missing (deleted or renamed).
    pub(super) fn missing_default_views(&self) -> Vec<SavedView> {
        let statuses = self.lib.as_deref().map(|l| l.statuses.as_slice()).unwrap_or(&[]);
        default_views(statuses).into_iter().filter(|d| !self.ui.saved_views.iter().any(|v| v.name.eq_ignore_ascii_case(&d.name))).collect()
    }

    /// Puts the missing built-in views back at the end.
    pub(super) fn restore_default_views(&mut self, cx: &mut Context<Self>) {
        let mut missing = self.missing_default_views();
        self.ui.saved_views.append(&mut missing);
        self.save_ui();
        cx.notify();
    }

    /// Applies everything the view stores ("Show hidden games" reloads the
    /// library when it changes).
    pub(super) fn apply_view_filters(&mut self, filters: &ViewFilters, cx: &mut Context<Self>) {
        let hidden = self.ui.show_hidden;
        self.ui.apply_view(filters);
        let want_hidden = std::mem::replace(&mut self.ui.show_hidden, hidden);
        if self.ui.only.updates {
            self.update_apps = crate::services::downloads(cx).read(cx).update_apps();
        }
        self.save_ui();
        self.refresh_groups();
        self.set_show_hidden(want_hidden, cx);
        cx.notify();
    }

    /// A focused name box whose Enter runs `on_enter`.
    fn name_input(
        &mut self,
        value: &str,
        on_enter: fn(&mut Self, &mut Window, &mut Context<Self>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<InputState>, Subscription) {
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_placeholder(tr!("Name of the preset"), window, cx);
            s.set_value(value.to_string(), window, cx);
            s
        });
        let sub = cx.subscribe_in(&input, window, move |this, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                on_enter(this, window, cx);
            }
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        (input, sub)
    }

    /// "Save view": opens the name box.
    pub(super) fn start_naming_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.view_menu.renaming = None;
        self.view_menu.naming = Some(self.name_input("", Self::save_named_view, window, cx));
        cx.notify();
    }

    pub(super) fn cancel_view_naming(&mut self, cx: &mut Context<Self>) {
        self.view_menu.naming = None;
        self.view_menu.renaming = None;
        cx.notify();
    }

    /// Saves the current state under the typed name (a view with the same
    /// name is replaced).
    pub(super) fn save_named_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((input, _)) = self.view_menu.naming.as_ref() else { return };
        let name: String = input.read(cx).value().trim().chars().take(NAME_MAX).collect();
        if name.is_empty() {
            return;
        }
        let view = SavedView { name, filters: self.ui.view_filters() };
        match self.ui.saved_views.iter_mut().find(|v| v.name.eq_ignore_ascii_case(&view.name)) {
            Some(old) => *old = view,
            None => self.ui.saved_views.push(view),
        }
        self.save_ui();
        self.view_menu.naming = None;
        self.focus_view_search(window, cx);
        cx.notify();
    }

    pub(super) fn start_renaming_view(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.ui.saved_views.get(index).map(|v| v.name.clone()) else { return };
        self.view_menu.naming = None;
        let (input, sub) = self.name_input(&name, Self::commit_view_rename, window, cx);
        self.view_menu.renaming = Some((index, input, sub));
        cx.notify();
    }

    /// Keeps the typed name (unless it is empty or another view has it).
    pub(super) fn commit_view_rename(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some((index, input, _)) = self.view_menu.renaming.take() else { return };
        let name: String = input.read(cx).value().trim().chars().take(NAME_MAX).collect();
        let taken = self.ui.saved_views.iter().enumerate().any(|(i, v)| i != index && v.name.eq_ignore_ascii_case(&name));
        if let Some(view) = self.ui.saved_views.get_mut(index).filter(|_| !name.is_empty() && !taken) {
            view.name = name;
            self.save_ui();
        }
        cx.notify();
    }

    /// Stores the current state in the view, keeping its name.
    pub(super) fn update_view(&mut self, index: usize, cx: &mut Context<Self>) {
        let filters = self.ui.view_filters();
        let Some(view) = self.ui.saved_views.get_mut(index) else { return };
        view.filters = filters;
        let name = view.name.clone();
        self.save_ui();
        self.show_toast(tr!("“{name}” now holds the current filters", name), false, cx);
        cx.notify();
    }

    pub(super) fn move_view(&mut self, index: usize, up: bool, cx: &mut Context<Self>) {
        let len = self.ui.saved_views.len();
        let to = if up { index.checked_sub(1) } else { Some(index + 1).filter(|i| *i < len) };
        if let Some(to) = to.filter(|_| index < len) {
            self.ui.saved_views.swap(index, to);
            self.view_menu.renaming = None;
            self.save_ui();
            cx.notify();
        }
    }

    pub(super) fn delete_view(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.ui.saved_views.len() {
            self.ui.saved_views.remove(index);
            self.view_menu.renaming = None;
            self.save_ui();
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{default_views, InstallFilter};

    #[test]
    fn defaults_follow_the_library_statuses() {
        let statuses = heronite_data::status::default_statuses();
        let views = default_views(&statuses);
        let names: Vec<_> = views.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["Playing now", "Backlog", "Finished", "Favourites"]);
        assert!(views.iter().all(|v| v.filters.group_by_status.is_some() && v.filters.show_hidden == Some(false)));
        assert_eq!(views[1].filters.install_filter, InstallFilter::Installed);
        // no matching status: only Favourites is left
        assert_eq!(default_views(&[]).len(), 1);
    }
}
