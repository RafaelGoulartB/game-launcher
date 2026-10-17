//! Command palette (Ctrl K): one box to reach pages, actions and games.
//!
//! * `matching` (`heronite_data::fuzzy`): folded fuzzy scoring. `sources`: what can be found.
//!   `rank`: query -> grouped results (pure, unit tested). `view`: the overlay.
//! * This file is the glue with [`CollectionView`]: opening and closing, the
//!   keys (palette navigation while open, global shortcuts while closed) and
//!   running what was chosen through the existing code paths.
//! * What it offers comes from the app's own lists, so new features show up
//!   without touching the palette: a game's actions from `game_actions.rs`
//!   (the context menu's list), the pages' commands from
//!   `pages/commands.rs`, the View menu from the Collection's state.
//! * Items run are kept (ids, most recent first) in the UI state: they fill
//!   the "Recent" group and rank first inside their tier.
//! * Keys are taken with a keystroke interceptor, i.e. before any input
//!   (search box, palette box) sees them, so Ctrl K, Ctrl 1..6, Ctrl `,`,
//!   Ctrl N and F5 work wherever the focus is (not over a dialog, also a
//!   page's own), and the arrows, Tab, Enter and Esc drive the list while
//!   the palette is open.
//!
//! Debug: `HERONITE_DEBUG_PALETTE=<query>` opens it with that query (`-` for
//! none); `HERONITE_DEBUG_PALETTE=<query>@pages|actions|games` also picks the
//! filter, and `HERONITE_DEBUG_PALETTE_KEYS=down,right,...` feeds it keys.

mod rank;
mod sources;
mod view;

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::collect_facet_options;
// The matching is shared with the Collection's search.
use heronite_data::fuzzy as matching;
use heronite_i18n::tr;

use self::sources::{Act, GameRow, ViewCmd};
use self::view::{PaletteEvent, PaletteView};
use super::CollectionView;
use crate::pages::Page;

/// Items kept in the palette's history.
const HISTORY: usize = 20;

/// Palette state of the Collection.
#[derive(Default)]
pub(super) struct Host {
    view: Option<Entity<PaletteView>>,
    sub: Option<Subscription>,
    /// Keeps the keystroke interceptor alive.
    _keys: Option<Subscription>,
    debug_done: bool,
}

impl Host {
    pub fn new(window: &Window, cx: &mut Context<CollectionView>) -> Host {
        let weak = cx.entity().downgrade();
        let handle = window.window_handle();
        let keys = cx.intercept_keystrokes(move |ev, window, cx| {
            if window.window_handle() != handle {
                return;
            }
            let Some(view) = weak.upgrade() else { return };
            if view.update(cx, |this, cx| this.palette_key(&ev.keystroke, window, cx)) {
                cx.stop_propagation();
            }
        });
        Host { _keys: Some(keys), ..Host::default() }
    }
}

impl CollectionView {
    /// Opens the palette (the ☰ menu offers it too). Does nothing while a
    /// dialog is open.
    pub(super) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mark_onboarding(crate::dialogs::onboarding::Step::Palette);
        self.open_palette_with("", window, cx);
    }

    /// A dialog (the Collection's or the shown page's own) or the screenshot
    /// viewer is up.
    fn modal_open(&self, cx: &App) -> bool {
        self.dialog.is_some() || self.shots.is_open() || self.page_modal(cx)
    }

    fn open_palette_with(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<PaletteView>> {
        if self.modal_open(cx) {
            return None;
        }
        if let Some(v) = self.palette.view.clone() {
            return Some(v);
        }
        self.open_menu = None;
        self.context_menu = None;
        let entries = Rc::new(self.palette_entries(cx));
        let recent = Rc::new(self.ui.palette_recent.clone());
        let view = cx.new(|cx| PaletteView::new(entries, recent, query, window, cx));
        self.palette.sub = Some(cx.subscribe_in(&view, window, Self::on_palette_event));
        self.palette.view = Some(view.clone());
        cx.notify();
        Some(view)
    }

    pub(super) fn palette_open(&self) -> bool {
        self.palette.view.is_some()
    }

    pub(super) fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.view.take().is_some() {
            self.palette.sub = None;
            window.focus(&self.focus_handle, cx);
            cx.notify();
        }
    }

    fn on_palette_event(&mut self, view: &Entity<PaletteView>, ev: &PaletteEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            PaletteEvent::Close => self.close_palette(window, cx),
            PaletteEvent::Run { act, id } => {
                self.remember_palette_item(id);
                self.close_palette(window, cx);
                self.run_palette_act(act.clone(), window, cx);
            }
            PaletteEvent::OpenGame(key) => {
                let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
                let items = self.game_actions(&game, cx);
                let entries = sources::action_entries(key, &game.title, &items, false);
                view.update(cx, |p, cx| p.show_game(game.title.clone().into(), entries, window, cx));
            }
        }
    }

    /// Puts `id` first in the history (saved with the UI state).
    fn remember_palette_item(&mut self, id: &str) {
        let recent = &mut self.ui.palette_recent;
        recent.retain(|r| r != id);
        recent.insert(0, id.to_string());
        recent.truncate(HISTORY);
        self.save_ui();
    }

    /// Everything the palette offers now: pages and global actions, the
    /// selected game's actions, the View menu, the pages' commands and the
    /// games of the library.
    fn palette_entries(&mut self, cx: &mut Context<Self>) -> Vec<sources::Entry> {
        let mut v = sources::static_entries();
        let signed_in = crate::services::downloads_started(cx).map(|s| s.read(cx).account().logged_in);
        if let Some(e) = v.iter_mut().find(|e| e.act == Act::EpicUpdates) {
            if signed_in == Some(false) {
                e.disable(tr!("Sign in to Epic Games first"));
            }
        }
        // The game selected in the Collection: its actions, by name.
        let selected = self.selected.clone().filter(|_| self.page == Page::Collection);
        if let Some(game) = selected.and_then(|k| self.lib.as_ref().and_then(|l| l.get(&k)).cloned()) {
            let items = self.game_actions(&game, cx);
            v.extend(sources::action_entries(&game.key, &game.title, &items, true));
        }
        v.extend(self.page_commands(cx).iter().map(|(page, c)| sources::page_entry(*page, c)));
        let Some(lib) = self.lib.clone() else { return v };
        v.extend(sources::view_entries(&self.ui, &collect_facet_options(&lib.games), &self.categories));
        let mut games: Vec<_> = lib.games.iter().collect();
        games.sort_by(|a, b| a.title_lower.cmp(&b.title_lower).then(a.key.cmp(&b.key)));
        v.reserve(games.len());
        for g in games {
            let playtime = heronite_data::collection::playtime_label(g.playtime_minutes);
            v.push(sources::game_entry(&GameRow {
                key: &g.key,
                title: &g.title,
                store_title: &g.game.title,
                source: g.source_label,
                playtime: &playtime,
                installed: g.installed,
                has_folder: g.details.install_path.is_some(),
                playing: self.playing.contains_key(&g.app_name),
                favourite: g.favourite,
                hidden: g.hidden,
                last_played: g.last_played.as_deref(),
            }));
        }
        v
    }

    /// Every keystroke of the window, before anything else sees it. Returns
    /// whether it was taken.
    fn palette_key(&mut self, ks: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // F11: fullscreen on / off, in both modes (`--fullscreen` starts there).
        let plain = !ks.modifiers.control && !ks.modifiers.alt && !ks.modifiers.platform && !ks.modifiers.shift;
        if ks.key == "f11" && plain {
            window.toggle_fullscreen();
            return true;
        }
        if crate::console::active() {
            return self.console_key(ks, window, cx);
        }
        let m = &ks.modifiers;
        let key = ks.key.as_str();
        let ctrl = m.control && !m.alt && !m.platform && !m.shift;
        if let Some(view) = self.palette.view.clone() {
            if ctrl && key == "k" {
                self.close_palette(window, cx);
                return true;
            }
            if ctrl && key == "enter" {
                view.update(cx, |p, cx| p.open_selected_game(cx));
                return true;
            }
            let plain = !m.control && !m.alt && !m.platform;
            if !plain {
                return false;
            }
            match key {
                "escape" => {
                    if !view.update(cx, |p, cx| p.back(window, cx)) {
                        self.close_palette(window, cx);
                    }
                }
                // In the empty box of a game's actions: back to the main list.
                "backspace" if view.read(cx).in_game() && view.read(cx).query_is_empty() => {
                    view.update(cx, |p, cx| p.back(window, cx));
                }
                // On a game: its actions (else the input moves its caret).
                "right" => return view.update(cx, |p, cx| p.open_selected_game(cx)),
                "up" => view.update(cx, |p, cx| p.move_selection(-1, cx)),
                "down" => view.update(cx, |p, cx| p.move_selection(1, cx)),
                "pageup" => view.update(cx, |p, cx| p.move_selection(-8, cx)),
                "pagedown" => view.update(cx, |p, cx| p.move_selection(8, cx)),
                "tab" => {
                    let back = m.shift;
                    view.update(cx, |p, cx| p.cycle_filter(back, cx));
                }
                "enter" => view.update(cx, |p, cx| p.activate(cx)),
                _ => return false,
            }
            return true;
        }
        // Closed: global shortcuts, unless something modal is up.
        if self.modal_open(cx) {
            return false;
        }
        if self.view_menu_key(ks, window, cx) {
            return true;
        }
        if ctrl {
            match key {
                "k" => {
                    self.open_palette(window, cx);
                }
                "1" => self.navigate(Page::Collection, window, cx),
                "2" => self.navigate(Page::Downloads, window, cx),
                "3" => self.navigate(Page::WineManager, window, cx),
                "4" => self.navigate(Page::Epic, window, cx),
                "5" => self.navigate(Page::Emulators, window, cx),
                "6" => self.navigate(Page::Statistics, window, cx),
                "," => self.navigate(Page::Settings, window, cx),
                "n" => {
                    self.open_add_game(window, cx);
                }
                _ => return false,
            }
            return true;
        }
        if key == "f5" && !m.control && !m.alt && !m.platform && !m.shift {
            self.refresh_all(cx);
            return true;
        }
        false
    }

    /// Runs what the palette item asks for, through the same paths as the
    /// menus, buttons and context menu.
    fn run_palette_act(&mut self, act: Act, window: &mut Window, cx: &mut Context<Self>) {
        match act {
            Act::Go(page) => self.navigate(page, window, cx),
            Act::Section(section) => self.open_settings_section(section, window, cx),
            Act::AddGame => {
                self.open_add_game(window, cx);
            }
            Act::Refresh => self.refresh_all(cx),
            Act::ScanRoms => self.open_scan_roms(Default::default(), window, cx),
            Act::EpicUpdates => {
                let svc = crate::services::downloads(cx);
                if svc.read(cx).account().logged_in {
                    svc.update(cx, |s, cx| s.refresh_library(cx));
                } else {
                    self.show_toast(tr!("Sign in to Epic Games first (\u{2630} \u{203a} Epic Games)"), true, cx);
                }
            }
            Act::OpenLogs => {
                let dir = heronite_launch::log_writer::LogPaths::for_data(&self.data_paths).base;
                if dir.is_dir() {
                    self.open_in_file_manager(&dir, cx);
                } else {
                    self.show_toast(tr!("There is no logs folder yet: it appears when a game is launched"), true, cx);
                }
            }
            Act::Console => self.set_console(true, window, cx),
            Act::YearReview => self.open_year_review(window, cx),
            Act::Onboarding => self.open_onboarding(cx),
            Act::ManageStatuses => self.open_manage_statuses(window, cx),
            // The quit question needs the window: ask once this update ended.
            Act::Quit => cx.defer(crate::system::request_quit),
            Act::View(cmd) => self.run_view_cmd(cmd, window, cx),
            Act::PageCmd { page, id, navigate } => self.run_page_command(page, &id, navigate, window, cx),
            Act::SelectGame(key) => self.palette_select(&key, window, cx),
            Act::Game(key, action) => {
                self.palette_select(&key, window, cx);
                self.run_game_action(&key, &action, window, cx);
            }
            // The view handles it (second level); nothing to run here.
            Act::GameActions(_) => {}
        }
    }

    /// The View menu's controls, shown on the Collection.
    fn run_view_cmd(&mut self, cmd: ViewCmd, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Collection {
            self.navigate(Page::Collection, window, cx);
        }
        match cmd {
            ViewCmd::Sort(mode) => {
                self.ui.sort = mode;
                self.save_ui();
                self.refresh_groups();
            }
            ViewCmd::Show(filter) => self.set_install_filter(filter, cx),
            ViewCmd::GroupByStatus => {
                self.ui.group_by_status = !self.ui.group_by_status;
                self.save_ui();
                self.refresh_groups();
            }
            ViewCmd::GridView => self.set_grid_view(!self.ui.grid_view, cx),
            ViewCmd::ShowHidden => self.set_show_hidden(!self.ui.show_hidden, cx),
            ViewCmd::ClearFilters => {
                self.ui.clear_facets();
                self.ui.categories.clear();
                self.ui.statuses.clear();
                self.save_ui();
                self.refresh_groups();
            }
            ViewCmd::Facet(kind, value) => self.toggle_facet(kind.as_str(), &value),
            ViewCmd::SortDirection => self.toggle_sort_direction(cx),
            ViewCmd::Only(only, on) => self.set_only(only, on, cx),
            ViewCmd::Category(name) => self.toggle_category_filter(&name, cx),
            ViewCmd::ManageCategories => self.open_manage_categories(window, cx),
        }
        self.reveal_selected();
        cx.notify();
    }

    fn open_in_file_manager(&mut self, dir: &std::path::Path, cx: &mut Context<Self>) {
        if let Err(err) = std::process::Command::new("xdg-open").arg(dir).spawn() {
            self.show_toast(format!("{err}"), true, cx);
        }
    }

    /// Focuses `key` in the Collection (leaving the page that is shown).
    fn palette_select(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Collection {
            self.navigate(Page::Collection, window, cx);
        }
        if self.lib.as_ref().is_some_and(|l| l.get(key).is_some()) {
            self.select(key.to_string().into(), true);
            self.reveal_selected();
        }
        cx.notify();
    }

    /// Called from `render`: opens the palette the debug hook asks for and
    /// drops it when a dialog came up (e.g. the quit question).
    pub(super) fn drive_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.view.is_some() && self.modal_open(cx) {
            self.close_palette(window, cx);
        }
        if self.palette.debug_done || self.lib.is_none() {
            return;
        }
        self.palette.debug_done = true;
        let Some(spec) = crate::env::value("HERONITE_DEBUG_PALETTE") else { return };
        let (query, filter) = spec.split_once('@').unwrap_or((spec.as_str(), ""));
        let query = if query == "-" { "" } else { query };
        if let Some(view) = self.open_palette_with(query, window, cx) {
            let n = match filter {
                "pages" => 1,
                "actions" => 2,
                "games" => 3,
                _ => 0,
            };
            for _ in 0..n {
                view.update(cx, |p, cx| p.cycle_filter(false, cx));
            }
            // `HERONITE_DEBUG_PALETTE_KEYS=down,down,tab`: keys fed to the
            // palette, once its events are subscribed (the next cycle).
            if let Some(keys) = crate::env::value("HERONITE_DEBUG_PALETTE_KEYS") {
                cx.defer_in(window, move |this, window, cx| {
                    for k in keys.split(',').filter(|k| !k.is_empty()) {
                        if let Ok(ks) = Keystroke::parse(k) {
                            this.palette_key(&ks, window, cx);
                        }
                    }
                });
            }
        }
    }

    pub(super) fn render_palette(&self) -> Option<AnyView> {
        self.palette.view.clone().map(Into::into)
    }
}
