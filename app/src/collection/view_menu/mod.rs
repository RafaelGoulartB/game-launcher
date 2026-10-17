//! The View menu of the header's tune button: a search box that finds any
//! filter value (the active ones sit in it as tokens), then Presets, Sort,
//! Group by, Show, the "Only" switches and a shortcut to list each kind.
//! The panel is drawn in `render.rs`, what the box finds is `choices.rs`
//! and the presets (saved views) are `views.rs`.

mod choices;
mod render;
mod views;

use std::collections::HashMap;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use heronite_data::categories::UNCATEGORIZED;
use heronite_data::collection::{collect_facet_options, runs_offline, FACET_KINDS};
use heronite_data::status::{resolve_status_id, status_label};
use heronite_data::{InstallFilter, Library};
use heronite_i18n::tr;

pub(super) use choices::Pick;
use choices::{Choice, Kind, Row, Section};

use super::header::{facet_label, Menu};
use super::view_filters::{category_label, Only};
use super::CollectionView;
use crate::theme;

pub(super) struct ViewMenu {
    input: Entity<InputState>,
    /// What the search box holds.
    query: String,
    /// The search box has the keyboard (the menu then stays open when the
    /// pointer leaves it).
    focused: bool,
    /// Results row picked by Enter (moved with ↑ ↓).
    highlight: usize,
    /// The placeholder is the one for a box with tokens.
    with_tokens: bool,
    /// Name box of "Save view", while it is open.
    naming: Option<(Entity<InputState>, Subscription)>,
    /// The Views select is open.
    select_open: bool,
    /// The Views section lists the views to rename, update, move or delete.
    editing: bool,
    /// Name box of the view being renamed (its index).
    renaming: Option<(usize, Entity<InputState>, Subscription)>,
    /// Height the menu may take below the header, in logical px.
    room: f32,
    /// `HERONITE_DEBUG_DROPDOWN=view:<text>`: typed on the next frame.
    pub debug_query: Option<String>,
    _sub: Subscription,
}

impl ViewMenu {
    pub(super) fn new(window: &mut Window, cx: &mut Context<CollectionView>) -> Self {
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_placeholder(tr!("Filter by genre, studio, store, status…"), window, cx);
            s
        });
        let sub = cx.subscribe_in(&input, window, |this, state, ev: &InputEvent, _, cx| {
            let menu = &mut this.view_menu;
            match ev {
                InputEvent::Change => {
                    let value = state.read(cx).value().to_string();
                    if value != menu.query {
                        menu.query = value;
                        menu.highlight = 0;
                    }
                }
                InputEvent::Focus => menu.focused = true,
                InputEvent::Blur => menu.focused = false,
                InputEvent::PressEnter { .. } => {}
            }
            cx.notify();
        });
        Self {
            input,
            query: String::new(),
            focused: false,
            highlight: 0,
            with_tokens: false,
            naming: None,
            select_open: false,
            editing: false,
            renaming: None,
            room: 600.,
            debug_query: None,
            _sub: sub,
        }
    }

    pub(super) fn set_editing(&mut self, on: bool) {
        self.editing = on;
    }

    pub(super) fn set_select_open(&mut self, on: bool) {
        self.select_open = on;
    }

    /// Typing (or naming a view) keeps the menu open when the pointer leaves.
    pub(super) fn holds_open(&self) -> bool {
        self.focused || !self.query.is_empty() || self.naming.is_some() || self.editing || self.select_open
    }
}

impl CollectionView {
    /// Every value of each kind, with counts over the whole library.
    fn view_sections(&self) -> Vec<Section> {
        let Some(lib) = self.lib.as_deref() else { return Vec::new() };
        let ui = &self.ui;
        let count = |f: &dyn Fn(&heronite_data::CollectionGame) -> bool| lib.games.iter().filter(|g| f(g)).count();
        let only =
            |o: Only, n: Option<usize>| Choice { pick: Pick::Only(o), label: o.label().into(), count: n, on: self.only_on(o), color: None };
        let mut out = vec![Section {
            kind: Kind::Only,
            choices: vec![
                only(Only::Favourites, Some(count(&|g| g.favourite))),
                only(Only::Updates, None),
                only(Only::Offline, Some(count(&|g| runs_offline(g)))),
                Choice { pick: Pick::Hidden, label: tr!("Show hidden games").into(), count: None, on: ui.show_hidden, color: None },
            ],
        }];
        let mut by_status: HashMap<String, usize> = HashMap::new();
        for g in &lib.games {
            *by_status.entry(resolve_status_id(g.status_raw.as_deref(), &lib.statuses)).or_default() += 1;
        }
        out.push(Section {
            kind: Kind::Status,
            choices: lib
                .statuses
                .iter()
                .map(|s| Choice {
                    pick: Pick::Status(s.id.clone()),
                    label: status_label(&s.name).to_string(),
                    count: Some(by_status.get(&s.id).copied().unwrap_or(0)),
                    on: ui.statuses.contains(&s.id),
                    color: Some(theme::status_color(&s.slug)),
                })
                .collect(),
        });
        if !self.categories.is_empty() {
            let names = self.categories.names().map(str::to_owned).chain([UNCATEGORIZED.to_owned()]);
            out.push(Section {
                kind: Kind::Category,
                choices: names
                    .map(|name| {
                        let one = std::iter::once(name.clone()).collect();
                        Choice {
                            label: category_label(&name),
                            count: Some(count(&|g| self.categories.matches(&one, &g.runner, &g.app_name))),
                            on: ui.categories.contains(&name),
                            pick: Pick::Category(name),
                            color: None,
                        }
                    })
                    .collect(),
            });
        }
        let options = collect_facet_options(&lib.games);
        for kind in Kind::ALL {
            let Kind::Facet(facet) = kind else { continue };
            let mut choices: Vec<Choice> = options
                .get(facet)
                .iter()
                .map(|o| Choice {
                    pick: Pick::Facet(facet, o.value.clone()),
                    label: o.value.clone(),
                    count: Some(o.count),
                    on: ui.facet_filters.contains(facet, &o.value),
                    color: None,
                })
                .collect();
            choices.sort_by_key(|c| std::cmp::Reverse(c.count));
            out.push(Section { kind, choices });
        }
        out
    }

    /// The search box's results, kind by kind (empty while nothing is typed).
    fn view_results(&self) -> Vec<choices::Found> {
        if self.view_menu.query.trim().is_empty() {
            return Vec::new();
        }
        choices::find(self.view_sections(), &self.view_menu.query)
    }

    fn view_rows(&self) -> Vec<Row> {
        self.view_results().into_iter().flat_map(|f| f.rows).collect()
    }

    /// The active filters, each with the name of its kind: the tokens of the
    /// search box and the chips of the header.
    pub(super) fn view_tokens(&self) -> Vec<(Option<&'static str>, Choice)> {
        let ui = &self.ui;
        let token = |pick: Pick, label: String, color: Option<Rgba>| Choice { pick, label, count: None, on: true, color };
        let mut out = Vec::new();
        match ui.install_filter {
            InstallFilter::All => {}
            f => out.push((
                None,
                token(Pick::Show(f), if f == InstallFilter::Installed { tr!("Installed") } else { tr!("Not installed") }.into(), None),
            )),
        }
        for o in Only::ALL.into_iter().filter(|o| self.only_on(*o)) {
            out.push((Some(tr!("Only")), token(Pick::Only(o), o.label().into(), None)));
        }
        if ui.show_hidden {
            out.push((None, token(Pick::Hidden, tr!("Hidden shown").into(), None)));
        }
        let statuses = self.lib.as_deref().map(|l| l.statuses.as_slice()).unwrap_or(&[]);
        for id in &ui.statuses {
            let st = statuses.iter().find(|s| &s.id == id);
            let label = st.map_or_else(|| id.clone(), |s| status_label(&s.name).to_string());
            out.push((Some(tr!("Status")), token(Pick::Status(id.clone()), label, st.map(|s| theme::status_color(&s.slug)))));
        }
        for name in &ui.categories {
            out.push((Some(tr!("Category")), token(Pick::Category(name.clone()), category_label(name), None)));
        }
        for kind in FACET_KINDS {
            for value in ui.facet_filters.get(kind) {
                out.push((Some(facet_label(kind)), token(Pick::Facet(kind, value.clone()), value.clone(), None)));
            }
        }
        out
    }

    /// Switches one filter on or off.
    pub(super) fn apply_pick(&mut self, pick: &Pick, cx: &mut Context<Self>) {
        match pick {
            Pick::Show(f) => {
                let next = if self.ui.install_filter == *f { InstallFilter::All } else { *f };
                self.set_install_filter(next, cx);
            }
            Pick::Only(o) => self.set_only(*o, !self.only_on(*o), cx),
            Pick::Hidden => self.set_show_hidden(!self.ui.show_hidden, cx),
            Pick::Status(id) => {
                self.ui.toggle_status(id);
                self.save_ui();
                self.refresh_groups();
            }
            Pick::Category(name) => self.toggle_category_filter(name, cx),
            Pick::Facet(kind, value) => self.toggle_facet(kind.as_str(), value),
        }
        cx.notify();
    }

    /// Enter or a click on a result: switches the value (and empties the
    /// box for the next one), or narrows the search to a kind.
    fn activate_row(&mut self, row: Row, window: &mut Window, cx: &mut Context<Self>) {
        match row {
            Row::Choice(c) => {
                self.apply_pick(&c.pick, cx);
                self.set_view_query("", window, cx);
            }
            Row::All { kind, .. } => {
                let text = choices::scoped_query(kind, &self.view_menu.query);
                self.set_view_query(&text, window, cx);
            }
        }
    }

    /// Puts `text` in the search box and gives it the keyboard.
    fn set_view_query(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.view_menu.input.update(cx, |s, cx| {
            s.set_value(text.to_string(), window, cx);
            s.focus(window, cx);
        });
        self.view_menu.query = text.to_string();
        self.view_menu.highlight = 0;
        cx.notify();
    }

    /// The tune button was clicked: typing goes to the search box.
    pub(super) fn focus_view_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.view_menu.input.update(cx, |s, cx| s.focus(window, cx));
    }

    /// "Reset all": every filter off ("Show hidden games" too); the sort and
    /// the grouping stay.
    fn reset_view_filters(&mut self, cx: &mut Context<Self>) {
        self.ui.clear_filters();
        self.save_ui();
        self.refresh_groups();
        self.set_show_hidden(false, cx);
        cx.notify();
    }

    /// Games shown with every filter but the install one: (all, installed).
    fn show_counts(&self) -> (usize, usize) {
        let Some(lib) = self.lib.as_deref() else { return (0, 0) };
        let query = heronite_data::ViewQuery { install: InstallFilter::All, ..super::data::view_query(&self.ui, &self.search) };
        let games = heronite_data::collection::filter_games_where(&lib.games, &query, &|g| self.keeps_game(g, &lib.statuses));
        (games.len(), games.iter().filter(|g| g.installed).count())
    }

    /// A removed status no longer filters.
    pub(super) fn prune_status_filter(&mut self, lib: &Library) {
        let before = self.ui.statuses.len();
        self.ui.statuses.retain(|id| lib.statuses.iter().any(|s| &s.id == id));
        if self.ui.statuses.len() != before {
            self.save_ui();
        }
    }

    /// Per frame: a closed menu forgets what was typed; the placeholder
    /// follows the tokens.
    pub(super) fn sync_view_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.view_menu.room = (f32::from(window.viewport_size().height) - f32::from(theme::HEADER_H) - 24.).max(240.);
        if let Some(text) = self.view_menu.debug_query.take() {
            self.set_view_query(&text, window, cx);
        }
        if self.open_menu != Some(Menu::View) {
            if !self.view_menu.query.is_empty() {
                self.view_menu.input.update(cx, |s, cx| s.set_value("", window, cx));
                self.view_menu.query.clear();
            }
            self.view_menu.naming = None;
            self.view_menu.editing = false;
            self.view_menu.renaming = None;
            self.view_menu.select_open = false;
        }
        let with_tokens = self.ui.active_filters() > 0 || self.ui.show_hidden;
        if with_tokens != self.view_menu.with_tokens {
            self.view_menu.with_tokens = with_tokens;
            let text = if with_tokens { tr!("Add a filter…") } else { tr!("Filter by genre, studio, store, status…") };
            self.view_menu.input.update(cx, |s, cx| s.set_placeholder(text, window, cx));
        }
    }

    /// Keys of the open View menu, before the focused input sees them.
    /// Returns whether the key was taken.
    pub(super) fn view_menu_key(&mut self, ks: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.open_menu != Some(Menu::View) || ks.modifiers.control || ks.modifiers.alt || ks.modifiers.platform {
            return false;
        }
        let menu = &self.view_menu;
        match ks.key.as_str() {
            "escape" => {
                if menu.select_open {
                    self.view_menu.select_open = false;
                } else if menu.naming.is_some() || menu.renaming.is_some() {
                    self.cancel_view_naming(cx);
                } else if menu.editing {
                    self.view_menu.editing = false;
                } else if !menu.query.is_empty() {
                    self.set_view_query("", window, cx);
                } else {
                    self.open_menu = None;
                    window.focus(&self.focus_handle, cx);
                }
                cx.notify();
                true
            }
            "up" | "down" if menu.focused && !menu.query.is_empty() => {
                let n = self.view_rows().len();
                if n > 0 {
                    let h = self.view_menu.highlight.min(n - 1);
                    self.view_menu.highlight = if ks.key == "up" { (h + n - 1) % n } else { (h + 1) % n };
                    cx.notify();
                }
                true
            }
            "enter" if menu.focused && !menu.query.is_empty() => {
                let rows = self.view_rows();
                if let Some(row) = rows.get(self.view_menu.highlight.min(rows.len().saturating_sub(1))).cloned() {
                    self.activate_row(row, window, cx);
                }
                true
            }
            // The empty box: Backspace takes the last token off.
            "backspace" if menu.focused && menu.query.is_empty() => {
                if let Some((_, last)) = self.view_tokens().pop() {
                    self.apply_pick(&last.pick, cx);
                }
                true
            }
            _ => false,
        }
    }
}
