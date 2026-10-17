//! This app's own UI state, kept in `AppPaths::ui_state()`.
//!
//! It mirrors what Heronite kept in Chromium's localStorage (imported once by
//! `migrate`): `collection_install_filter`, `collection_facet_filter`,
//! `collection_sort`, `collection_sidebar_hidden`, `collection_collapsed_groups`,
//! `collection_focused_game`. Loading is as lenient as the `readX` helpers of
//! `index.tsx`: anything missing or invalid falls back to its default.

use std::collections::BTreeSet;

use anyhow::Result;
use serde_json::{json, Value};

use crate::collection::{FacetFilters, FacetKind, InstallFilter, SortMode, FACET_KINDS};
use crate::json_store;
use crate::paths::AppPaths;

#[derive(Debug, Clone, PartialEq)]
pub struct UiState {
    pub install_filter: InstallFilter,
    pub facet_filters: FacetFilters,
    pub sort: SortMode,
    /// The sort turned around (View menu's arrow).
    pub sort_descending: bool,
    /// The View menu's "Only" switches.
    pub only: OnlyFilters,
    /// Selected custom categories (`heronite_data::categories`;
    /// `categories::UNCATEGORIZED` for the games in none). Empty: all games.
    pub categories: BTreeSet<String>,
    /// Ids of the selected completion statuses. Empty: all games.
    pub statuses: BTreeSet<String>,
    /// The View menu's views, in menu order.
    pub saved_views: Vec<SavedView>,
    /// The built-in views were added to [`saved_views`](Self::saved_views)
    /// once (after that they are the user's to edit or delete).
    pub views_seeded: bool,
    pub sidebar_hidden: bool,
    /// Ids of the collapsed status groups (`ungrouped` for "Other").
    pub collapsed_groups: BTreeSet<String>,
    /// `runner_appName` of the focused game.
    pub focused_game: Option<String>,
    /// "Group by status" toggle. Heronite does not persist it (always on at
    /// start); this app does.
    pub group_by_status: bool,
    /// "Show hidden games" (Heroic keeps it in the renderer's localStorage
    /// `show_hidden`, so it cannot be shared): hidden games stay in the
    /// Collection, flagged.
    pub show_hidden: bool,
    /// The app was left in console mode (it reopens in it).
    pub console_mode: bool,
    /// Ids of the command palette items run lately, most recent first.
    pub palette_recent: Vec<String>,
    /// The Collection shows the full-window grid instead of the list next to
    /// the focus panel.
    pub grid_view: bool,
    /// Smallest cover width of the grid, in logical px ([`GRID_COVER_W`]).
    pub grid_cover_w: f32,
    /// The grid shows each game's title under its cover.
    pub grid_titles: bool,
}

/// Range and default of [`UiState::grid_cover_w`].
pub const GRID_COVER_W: (f32, f32, f32) = (110., 240., 150.);

impl Default for UiState {
    fn default() -> Self {
        Self {
            install_filter: InstallFilter::All,
            facet_filters: FacetFilters::default(),
            sort: SortMode::Title,
            sort_descending: false,
            only: OnlyFilters::default(),
            categories: BTreeSet::new(),
            statuses: BTreeSet::new(),
            saved_views: Vec::new(),
            views_seeded: false,
            sidebar_hidden: false,
            collapsed_groups: BTreeSet::new(),
            focused_game: None,
            group_by_status: true,
            show_hidden: false,
            console_mode: false,
            palette_recent: Vec::new(),
            grid_view: false,
            grid_cover_w: GRID_COVER_W.2,
            grid_titles: false,
        }
    }
}

impl UiState {
    /// Parses the stored document leniently.
    pub fn from_value(value: &Value) -> Self {
        let mut state = Self::default();
        state.apply_view(&ViewFilters::from_value(value));
        if let Some(list) = value.get("savedViews").and_then(Value::as_array) {
            state.saved_views = list
                .iter()
                .filter_map(|v| {
                    let name = v.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())?;
                    Some(SavedView { name: name.to_owned(), filters: ViewFilters::from_value(v) })
                })
                .collect();
        }
        let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
        state.views_seeded = flag("viewsSeeded");
        state.sidebar_hidden = value.get("sidebarHidden").and_then(Value::as_bool).unwrap_or(false);
        if let Some(list) = value.get("collapsedGroups").and_then(Value::as_array) {
            state.collapsed_groups = list.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        }
        state.focused_game = value.get("focusedGame").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
        state.group_by_status = value.get("groupByStatus").and_then(Value::as_bool).unwrap_or(true);
        state.show_hidden = value.get("showHidden").and_then(Value::as_bool).unwrap_or(false);
        state.console_mode = value.get("consoleMode").and_then(Value::as_bool).unwrap_or(false);
        if let Some(list) = value.get("paletteRecent").and_then(Value::as_array) {
            state.palette_recent = list.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        }
        state.grid_view = flag("gridView");
        if let Some(w) = value.get("gridCoverWidth").and_then(Value::as_f64).filter(|w| w.is_finite()) {
            state.grid_cover_w = (w as f32).clamp(GRID_COVER_W.0, GRID_COVER_W.1);
        }
        state.grid_titles = flag("gridTitles");
        state
    }

    pub fn to_value(&self) -> Value {
        let mut doc = json!({
            "savedViews": self.saved_views.iter().map(SavedView::to_value).collect::<Vec<_>>(),
            "viewsSeeded": self.views_seeded,
            "sidebarHidden": self.sidebar_hidden,
            "collapsedGroups": self.collapsed_groups,
            "focusedGame": self.focused_game,
            "groupByStatus": self.group_by_status,
            "showHidden": self.show_hidden,
            "consoleMode": self.console_mode,
            "paletteRecent": self.palette_recent,
            "gridView": self.grid_view,
            "gridCoverWidth": self.grid_cover_w,
            "gridTitles": self.grid_titles,
        });
        if let (Value::Object(doc), Value::Object(view)) = (&mut doc, self.view_filters().to_value()) {
            doc.extend(view);
        }
        doc
    }

    /// Loads the state; a missing or unreadable file gives the defaults.
    pub fn load(paths: &AppPaths) -> Self {
        match json_store::read_value(&paths.ui_state()) {
            Ok(value) if value.is_object() => Self::from_value(&value),
            _ => Self::default(),
        }
    }

    /// Writes the state atomically (creating the folder if needed). Keys of
    /// the file this struct does not model are kept.
    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        let Value::Object(known) = self.to_value() else { unreachable!("to_value builds an object") };
        json_store::update_value(&paths.ui_state(), |doc| {
            if !doc.is_object() {
                *doc = Value::Object(Default::default());
            }
            let obj = doc.as_object_mut().expect("object");
            for (k, v) in known {
                obj.insert(k, v);
            }
            Ok(())
        })
    }

    /// `toggleGroup`; returns whether the group is now collapsed.
    pub fn toggle_group(&mut self, id: &str) -> bool {
        if !self.collapsed_groups.remove(id) {
            self.collapsed_groups.insert(id.to_owned());
            true
        } else {
            false
        }
    }

    /// `handleFacetFilter`.
    pub fn toggle_facet(&mut self, kind: FacetKind, value: &str) {
        self.facet_filters = self.facet_filters.toggled(kind, value);
    }

    /// `handleClearFacets`.
    pub fn clear_facets(&mut self) {
        self.facet_filters = FacetFilters::default();
    }

    /// Selects or clears one category of the filter.
    pub fn toggle_category(&mut self, name: &str) {
        if !self.categories.remove(name) {
            self.categories.insert(name.to_owned());
        }
    }

    /// Selects or clears one completion status of the filter.
    pub fn toggle_status(&mut self, id: &str) {
        if !self.statuses.remove(id) {
            self.statuses.insert(id.to_owned());
        }
    }

    /// Everything a view stores: the filters, the sort, the grouping and
    /// "Show hidden games".
    pub fn view_filters(&self) -> ViewFilters {
        ViewFilters {
            install_filter: self.install_filter,
            sort: self.sort,
            sort_descending: self.sort_descending,
            only: self.only,
            facet_filters: self.facet_filters.clone(),
            categories: self.categories.clone(),
            statuses: self.statuses.clone(),
            group_by_status: Some(self.group_by_status),
            show_hidden: Some(self.show_hidden),
        }
    }

    /// Sets what `view` stores (a view saved without the grouping or
    /// "Show hidden games" leaves them as they are).
    pub fn apply_view(&mut self, view: &ViewFilters) {
        self.install_filter = view.install_filter;
        self.sort = view.sort;
        self.sort_descending = view.sort_descending;
        self.only = view.only;
        self.facet_filters = view.facet_filters.clone();
        self.categories = view.categories.clone();
        self.statuses = view.statuses.clone();
        if let Some(on) = view.group_by_status {
            self.group_by_status = on;
        }
        if let Some(on) = view.show_hidden {
            self.show_hidden = on;
        }
    }

    /// Whether the Collection shows exactly `view` now.
    pub fn shows_view(&self, view: &ViewFilters) -> bool {
        self.install_filter == view.install_filter
            && self.sort == view.sort
            && self.sort_descending == view.sort_descending
            && self.only == view.only
            && self.categories == view.categories
            && self.statuses == view.statuses
            && self.facet_filters.same_as(&view.facet_filters)
            && view.group_by_status.is_none_or(|on| on == self.group_by_status)
            && view.show_hidden.is_none_or(|on| on == self.show_hidden)
    }

    /// Filters that hide games (install, "Only" switches, categories,
    /// statuses and facet values), counted one per value.
    pub fn active_filters(&self) -> usize {
        usize::from(self.install_filter != InstallFilter::All)
            + usize::from(self.only.favourites)
            + usize::from(self.only.updates)
            + usize::from(self.only.offline)
            + self.categories.len()
            + self.statuses.len()
            + self.facet_filters.len()
    }

    /// Clears every filter of [`active_filters`](Self::active_filters); the sort stays.
    pub fn clear_filters(&mut self) {
        self.install_filter = InstallFilter::All;
        self.only = OnlyFilters::default();
        self.facet_filters = FacetFilters::default();
        self.categories.clear();
        self.statuses.clear();
    }
}

/// What a view of the View menu sets.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewFilters {
    pub install_filter: InstallFilter,
    pub sort: SortMode,
    pub sort_descending: bool,
    pub only: OnlyFilters,
    pub facet_filters: FacetFilters,
    pub categories: BTreeSet<String>,
    pub statuses: BTreeSet<String>,
    /// `None`: the view leaves it as it is.
    pub group_by_status: Option<bool>,
    /// `None`: the view leaves it as it is.
    pub show_hidden: Option<bool>,
}

impl ViewFilters {
    /// Reads the keys [`UiState`] stores them under, leniently. A facet kind
    /// may hold one value (as Heronite stored it) or a list.
    pub fn from_value(value: &Value) -> Self {
        let mut v = Self::default();
        if let Some(f) = value.get("installFilter").and_then(Value::as_str).and_then(InstallFilter::parse) {
            v.install_filter = f;
        }
        if let Some(sort) = value.get("sort").and_then(Value::as_str).and_then(SortMode::parse) {
            v.sort = sort;
        }
        if let Some(obj) = value.get("facetFilters").and_then(Value::as_object) {
            for kind in FACET_KINDS {
                let values: Vec<&str> = match obj.get(kind.as_str()) {
                    Some(Value::String(s)) => vec![s.as_str()],
                    Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
                    _ => Vec::new(),
                };
                for value in values.into_iter().filter(|s| !s.trim().is_empty()) {
                    if !v.facet_filters.contains(kind, value) {
                        v.facet_filters = v.facet_filters.toggled(kind, value);
                    }
                }
            }
        }
        let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
        v.sort_descending = flag("sortDescending");
        v.only = OnlyFilters { favourites: flag("onlyFavourites"), updates: flag("onlyUpdates"), offline: flag("onlyOffline") };
        let names = |key: &str| -> BTreeSet<String> {
            value
                .get(key)
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned)).collect())
                .unwrap_or_default()
        };
        v.categories = names("categories");
        v.statuses = names("statuses");
        v.group_by_status = value.get("groupByStatus").and_then(Value::as_bool);
        v.show_hidden = value.get("showHidden").and_then(Value::as_bool);
        v
    }

    pub fn to_value(&self) -> Value {
        let facets = &self.facet_filters;
        json!({
            "installFilter": self.install_filter,
            "facetFilters": {
                "series": facets.series,
                "developer": facets.developer,
                "publisher": facets.publisher,
                "genre": facets.genre,
                "platform": facets.platform,
                "store": facets.store,
            },
            "sort": self.sort,
            "sortDescending": self.sort_descending,
            "onlyFavourites": self.only.favourites,
            "onlyUpdates": self.only.updates,
            "onlyOffline": self.only.offline,
            "categories": self.categories,
            "statuses": self.statuses,
            "groupByStatus": self.group_by_status,
            "showHidden": self.show_hidden,
        })
    }
}

/// A view the user saved from the View menu.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedView {
    pub name: String,
    pub filters: ViewFilters,
}

impl SavedView {
    fn to_value(&self) -> Value {
        let mut v = self.filters.to_value();
        if let Value::Object(obj) = &mut v {
            obj.insert("name".into(), Value::String(self.name.clone()));
        }
        v
    }
}

/// The View menu's "Only" switches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OnlyFilters {
    pub favourites: bool,
    /// Installed games with an update waiting.
    pub updates: bool,
    /// Games that run without a connection.
    pub offline: bool,
}

impl OnlyFilters {
    pub fn is_active(&self) -> bool {
        self.favourites || self.updates || self.offline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_paths(dir: &std::path::Path) -> AppPaths {
        AppPaths { config: dir.join("cfg"), cache: dir.join("cache") }
    }

    #[test]
    fn defaults_when_missing_or_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        assert_eq!(UiState::load(&paths), UiState::default());
        std::fs::create_dir_all(&paths.config).unwrap();
        std::fs::write(paths.ui_state(), "{not json").unwrap();
        assert_eq!(UiState::load(&paths), UiState::default());
        std::fs::write(paths.ui_state(), r#"{"sort":"bogus","installFilter":7,"collapsedGroups":"x","facetFilters":{"genre":""}}"#)
            .unwrap();
        assert_eq!(UiState::load(&paths), UiState::default());
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        let mut state = UiState {
            install_filter: InstallFilter::Installed,
            sort: SortMode::LastPlayed,
            sidebar_hidden: true,
            focused_game: Some("sideload_steam_400".into()),
            group_by_status: false,
            show_hidden: true,
            console_mode: true,
            palette_recent: vec!["go:downloads".into(), "game:sideload_steam_400:primary".into()],
            sort_descending: true,
            only: OnlyFilters { favourites: true, updates: false, offline: true },
            grid_view: true,
            grid_cover_w: 180.,
            grid_titles: true,
            ..Default::default()
        };
        state.toggle_category("RPG");
        state.toggle_category(crate::categories::UNCATEGORIZED);
        state.toggle_category("Gone");
        state.toggle_category("Gone");
        state.toggle_facet(FacetKind::Genre, "RPG");
        assert!(state.toggle_group("beaten"));
        assert!(state.toggle_group("ungrouped"));
        assert!(!state.toggle_group("beaten"));
        state.save(&paths).unwrap();
        let loaded = UiState::load(&paths);
        assert_eq!(loaded, state);
        assert_eq!(loaded.facet_filters.genre, ["RPG"]);
        assert_eq!(loaded.collapsed_groups.iter().collect::<Vec<_>>(), ["ungrouped"]);
        let on_disk = json_store::read_value(&paths.ui_state()).unwrap();
        assert_eq!(on_disk["sort"], "lastPlayed");
        assert_eq!(on_disk["installFilter"], "installed");
        assert_eq!(on_disk["showHidden"], true);
        assert_eq!(on_disk["consoleMode"], true);
        assert_eq!(on_disk["onlyOffline"], true);
        assert_eq!(on_disk["gridView"], true);
        assert_eq!(on_disk["categories"], json!([crate::categories::UNCATEGORIZED, "RPG"]));
        state.clear_facets();
        assert!(!state.facet_filters.is_active());
    }

    #[test]
    fn facet_filters_read_one_value_or_a_list() {
        let state = UiState::from_value(&json!({ "facetFilters": { "genre": "RPG", "developer": ["Valve", "valve ", "", "Remedy"] } }));
        assert_eq!(state.facet_filters.genre, ["RPG"]);
        assert_eq!(state.facet_filters.developer, ["Valve", "Remedy"]);
        assert_eq!(state.to_value()["facetFilters"]["genre"], json!(["RPG"]));
    }

    #[test]
    fn views_save_apply_and_match() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        let mut state = UiState { group_by_status: false, show_hidden: true, ..Default::default() };
        state.toggle_status("beaten");
        state.toggle_facet(FacetKind::Genre, "RPG");
        state.toggle_facet(FacetKind::Genre, "Indie");
        state.only.favourites = true;
        state.sort = SortMode::Playtime;
        assert_eq!(state.active_filters(), 4);
        state.saved_views.push(SavedView { name: "Done RPGs".into(), filters: state.view_filters() });
        state.save(&paths).unwrap();
        let mut loaded = UiState::load(&paths);
        assert_eq!(loaded, state);
        let view = loaded.saved_views[0].filters.clone();
        loaded.clear_filters();
        assert_eq!(loaded.active_filters(), 0);
        assert_eq!(loaded.sort, SortMode::Playtime);
        loaded.group_by_status = true;
        loaded.show_hidden = false;
        assert!(!loaded.shows_view(&view));
        loaded.apply_view(&view);
        assert!(loaded.shows_view(&view));
        assert!(!loaded.group_by_status && loaded.show_hidden, "a view sets grouping and hidden games");
        // the same values in another order still match
        loaded.facet_filters = loaded.facet_filters.toggled(FacetKind::Genre, "rpg").toggled(FacetKind::Genre, "RPG");
        assert!(loaded.shows_view(&view));
        let on_disk = json_store::read_value(&paths.ui_state()).unwrap();
        assert_eq!(on_disk["savedViews"][0]["name"], "Done RPGs");
        assert_eq!(on_disk["savedViews"][0]["statuses"], json!(["beaten"]));
        assert_eq!(on_disk["statuses"], json!(["beaten"]));
        // a view saved without them leaves both alone
        let old = ViewFilters::from_value(&json!({ "sort": "playtime" }));
        loaded.apply_view(&old);
        assert!(!loaded.group_by_status && loaded.show_hidden);
        assert!(loaded.shows_view(&old), "matches whatever the grouping");
    }

    #[test]
    fn grid_cover_width_is_clamped() {
        let state = UiState::from_value(&json!({ "gridCoverWidth": 9000 }));
        assert_eq!(state.grid_cover_w, GRID_COVER_W.1);
        let state = UiState::from_value(&json!({ "gridCoverWidth": "big" }));
        assert_eq!(state.grid_cover_w, GRID_COVER_W.2);
    }

    #[test]
    fn save_keeps_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        std::fs::create_dir_all(&paths.config).unwrap();
        std::fs::write(paths.ui_state(), "{\n\t\"futureKey\": [1, 2],\n\t\"sort\": \"title\"\n}").unwrap();
        let mut state = UiState::load(&paths);
        assert!(!state.show_hidden);
        state.show_hidden = true;
        state.save(&paths).unwrap();
        let on_disk = json_store::read_value(&paths.ui_state()).unwrap();
        assert_eq!(on_disk["futureKey"], json!([1, 2]));
        assert_eq!(on_disk["showHidden"], true);
        assert!(UiState::load(&paths).show_hidden);
    }
}
