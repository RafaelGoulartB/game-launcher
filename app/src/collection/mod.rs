//! The Collection screen: stage background, header, grouped list, focus panel.

mod achievements;
mod actions;
mod add_game;
mod backdrop;
mod card;
mod console;
mod context_menu;
mod data;
mod debug;
mod emulation;
mod epic;
mod extras;
mod focus;
mod focus_left;
mod game_actions;
mod game_facts;
mod grid;
mod grid_drawer;
mod header;
mod html;
mod install_options;
mod launch_options;
mod list;
mod list_view;
mod load;
mod metadata_ui;
mod nav;
mod onboarding;
mod open_links;
mod palette;
mod pumps;
mod render_focus;
mod screenshot_viewer;
mod screenshots;
mod scroll;
mod sideload_uninstall;
mod steam_copy;
mod system_glue;
mod timeline;
mod update_banner;
mod user_actions;
mod view_filters;
mod view_menu;
mod welcome;
pub use screenshots::screenshots_global;
pub mod stage;
mod vm;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::hltb::WikiCache;
use heronite_data::ui_state::UiState;
use heronite_data::{AppPaths, DataPaths, Library};
use heronite_images::ImageRef;

use crate::images::ImageStore;
use crate::runtime::GameRuntime;
use crate::theme;
use header::Menu;
use scroll::WheelScroll;
use vm::{FocusVm, GroupVm, ListItem};

pub struct CollectionView {
    started: Instant,
    first_frame_logged: bool,
    data_paths: DataPaths,
    app_paths: AppPaths,
    lib: Option<Arc<Library>>,
    wiki: Arc<WikiCache>,
    ui: UiState,
    /// Custom categories (`games.customCategories`), loaded with the library.
    categories: heronite_data::categories::Categories,
    /// The actions users added to their games, by game key, loaded with the library.
    user_actions: std::collections::HashMap<String, Vec<heronite_data::game_actions::Action>>,
    /// Games with an update waiting (app names), for "Only with updates".
    update_apps: std::collections::HashSet<String>,
    search: String,
    search_input: Entity<InputState>,
    open_menu: Option<Menu>,
    /// Pending hover-intent open of a header menu.
    menu_open: Option<Task<()>>,
    menu_close: Option<Task<()>>,
    /// Hovered parts of the open header menu (`ui::dropdown::PART_*`).
    menu_parts: u8,
    /// Smooth mouse-wheel scrolling of the game list.
    wheel: WheelScroll,
    /// The View menu's search box, typed text and saved-view name box.
    view_menu: view_menu::ViewMenu,
    refreshing: bool,
    runtime: Arc<GameRuntime>,
    /// app_name -> started tracking (Launching/Running).
    playing: HashMap<String, bool>,
    toast: Option<(SharedString, bool)>,
    context_menu: Option<context_menu::ContextMenuState>,
    focus_handle: FocusHandle,
    dialog: Option<AnyView>,
    /// Controller hook of the open dialog (console mode).
    dialog_hook: Option<crate::console::PadHook>,
    /// `HERONITE_DEBUG_DIALOG` (opened on the first render with a window).
    pending_debug_dialog: Option<String>,
    dialog_sub: Option<Subscription>,
    epic: epic::EpicState,
    extras: extras::Extras,
    welcome: welcome::Welcome,
    onboarding: onboarding::OnboardingGlue,
    /// Rating, save backup, Linux facts and timeline of the focused game.
    facts: game_facts::Facts,
    focus_scroll: ScrollHandle,
    shots: screenshots::Shots,
    toast_task: Option<Task<()>>,
    images: ImageStore,
    cover_chains: HashMap<SharedString, Vec<ImageRef>>,
    stage_chains: HashMap<SharedString, Vec<ImageRef>>,
    /// Last stage texture shown, kept while the next one loads.
    shown_stage: Option<(SharedString, Arc<RenderImage>)>,
    /// Last glass backdrop (blurred art behind menus and dialogs), kept while
    /// the next one loads.
    shown_glass: Option<(SharedString, Arc<RenderImage>)>,
    groups: Vec<GroupVm>,
    items: Vec<ListItem>,
    columns: usize,
    /// Card width of the list or the grid, in logical px.
    card_w: f32,
    /// Grid view state (`grid.rs`).
    grid: grid::GridUi,
    list_state: ListState,
    selected: Option<SharedString>,
    hovered: Option<SharedString>,
    leaving: Option<SharedString>,
    focus: Option<FocusVm>,
    scrim: stage::Scrim,
    blur_px: f32,
    saturation: f32,
    stage_enabled: bool,
    grey_uninstalled: bool,
    cover_aspect: f32,
    /// Page shown (☰ menu); the Collection itself is `Page::Collection`.
    page: crate::pages::Page,
    pages: nav::Pages,
    /// Command palette (Ctrl K), see `palette/mod.rs`.
    palette: palette::Host,
    /// Console mode state (`console/`).
    console: console::ConsoleUi,
    /// Language the precomputed view models and the search placeholder were
    /// built in.
    lang: heronite_i18n::Lang,
}

impl CollectionView {
    pub fn new(started: Instant, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let list_state = ListState::new(0, ListAlignment::Top, px(500.));
        let weak = cx.entity().downgrade();
        list_state.set_scroll_handler(move |_, _, cx| {
            // The sticky header overlay depends on the scroll position.
            if let Some(view) = weak.upgrade() {
                view.update(cx, |_, cx| cx.notify());
            }
        });
        let data_paths = DataPaths::detect();
        let app_paths = AppPaths::for_data(&data_paths);
        let ui = UiState::load(&app_paths);
        // Interface / Collection-art appearance (this app's own file, tiny).
        crate::theme::set_appearance(heronite_data::appearance::Appearance::load(&app_paths));
        let (images, image_rx) = ImageStore::new(data_paths.images_cache_dir(), &app_paths);
        Self::spawn_image_pump(image_rx, cx);

        let (runtime, events) = crate::system::attach_runtime(&data_paths, cx);
        Self::spawn_launch_events(events, cx);

        // Results of the post-game work (toasts, saved captures).
        if let Some(session) = crate::services::session::get(cx) {
            cx.subscribe(&session, |this, _, ev: &crate::services::session::SessionEvent, cx| this.on_session_event(ev, cx)).detach();
        }
        // Captures converted to WebP in the background (renamed files).
        if let Some(compression) = crate::services::compression::get(cx) {
            cx.subscribe(&compression, |this, _, ev, cx| this.on_compression_event(ev, cx)).detach();
        }
        // The update banner and the ☰ Settings row.
        if let Some(updater) = crate::services::update::get(cx) {
            cx.observe(&updater, |_, _, cx| cx.notify()).detach();
        }

        let search_input = Self::new_search_input(window, cx);
        let view_menu = view_menu::ViewMenu::new(window, cx);
        let onboarding = onboarding::OnboardingGlue::load(&app_paths);
        let mut this = Self {
            started,
            first_frame_logged: false,
            data_paths,
            app_paths,
            lib: None,
            wiki: Arc::new(WikiCache::default()),
            ui,
            categories: Default::default(),
            user_actions: Default::default(),
            update_apps: Default::default(),
            search: String::new(),
            search_input,
            open_menu: None,
            menu_open: None,
            menu_close: None,
            menu_parts: 0,
            wheel: WheelScroll::default(),
            view_menu,
            refreshing: false,
            runtime,
            playing: HashMap::new(),
            toast: None,
            toast_task: None,
            context_menu: None,
            focus_handle: cx.focus_handle(),
            dialog: None,
            dialog_hook: None,
            pending_debug_dialog: None,
            dialog_sub: None,
            epic: Default::default(),
            extras: Default::default(),
            welcome: Default::default(),
            onboarding,
            facts: Default::default(),
            focus_scroll: ScrollHandle::new(),
            shots: Default::default(),
            images,
            cover_chains: HashMap::new(),
            stage_chains: HashMap::new(),
            shown_stage: None,
            shown_glass: None,
            groups: Vec::new(),
            items: Vec::new(),
            columns: 3,
            card_w: theme::CARD_W,
            grid: Default::default(),
            list_state,
            selected: None,
            hovered: None,
            leaving: None,
            focus: None,
            scrim: stage::Scrim { dim: 0.14, scrim: 1. },
            blur_px: 6.,
            saturation: 0.94,
            stage_enabled: true,
            grey_uninstalled: true,
            cover_aspect: 1.5,
            page: crate::pages::Page::Collection,
            pages: nav::Pages::default(),
            palette: palette::Host::new(window, cx),
            console: Default::default(),
            lang: heronite_i18n::current(),
        };
        window.focus(&this.focus_handle, cx);
        this.console_init(window, cx);
        if let Some(page) = crate::env::value("HERONITE_DEBUG_PAGE").and_then(|p| crate::pages::Page::from_debug(&p)) {
            this.navigate(page, window, cx);
        }
        this.spawn_load(cx);
        this.start_idle_jobs(cx);
        this.init_metadata(window, cx);
        this.spawn_disk_watch(cx);
        this.spawn_debug_browse(cx);
        this
    }
}

impl CollectionView {
    fn render_toast(&self) -> Option<Div> {
        self.toast.clone().map(|(text, error)| {
            div().absolute().bottom(px(84.)).left_0().right_0().flex().justify_center().child(
                div()
                    .max_w(px(560.))
                    .px(px(16.))
                    .py(px(10.))
                    .rounded(px(10.))
                    .bg(theme::rgb_a(18, 22, 28, 0.94))
                    .border_1()
                    .border_color(if error { theme::danger().into() } else { theme::white_a(0.12) })
                    .text_size(px(13.33))
                    .text_color(theme::white_a(0.92))
                    .child(text),
            )
        })
    }

    /// Rebuilds what was precomputed in another language after a switch.
    fn sync_language(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lang = heronite_i18n::current();
        if self.lang == lang {
            return;
        }
        self.lang = lang;
        self.refresh_groups();
        self.pages.language_changed();
        self.search_input.update(cx, |s, cx| s.set_placeholder(heronite_i18n::tr!("Search games"), window, cx));
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.shots_key(ev, cx) || self.grid_key(ev, window, cx) {
            cx.notify();
            return;
        }
        if ev.keystroke.key == "escape" {
            if self.dialog.is_some() {
                self.dialog = None;
                self.dialog_sub = None;
            } else if self.context_menu.is_some() {
                self.context_menu = None;
            } else if self.open_menu.is_some() {
                self.open_menu = None;
            } else {
                // Heronite clears the selected game on Esc.
                self.focus = None;
                self.selected = None;
            }
            cx.notify();
        }
    }
}

impl Render for CollectionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_desktop(window, cx)
    }
}

impl CollectionView {
    fn render_desktop(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let _timer = RenderTimer(Instant::now());
        if !self.first_frame_logged {
            self.first_frame_logged = true;
            log::info!("first render after {:?}", self.started.elapsed());
        }
        self.images.drop_evicted(window);
        self.open_pending_debug_dialog(window, cx);
        self.drive_epic(window, cx);
        self.sync_extras(window, cx);
        self.sync_view_menu(window, cx);
        self.sync_onboarding(window, cx);
        self.sync_language(window, cx);
        self.sync_facts(cx);
        self.sync_shots(window, cx);
        self.drive_palette(window, cx);
        crate::ui::glass::set_backdrop(self.glass_image(window));
        self.sync_nav_meta(cx);
        if crate::console::active() {
            return self.render_console(window, cx);
        }
        let vw = f32::from(window.viewport_size().width);
        let scale = window.scale_factor();
        let (list_w, columns) = self.list_layout(vw);
        if columns != self.columns {
            self.columns = columns;
            self.rebuild_items();
        }
        if std::mem::take(&mut self.grid.reveal) {
            self.reveal_selected();
        }

        // The grid keeps the search as wide as next to the list.
        let header_w = if self.ui.grid_view { theme::LIST_COL_W.min(vw * 0.35) } else { list_w };
        let header = self.render_header(header_w, window, cx);
        let page_el = self.render_page(cx);
        let on_page = page_el.is_some();
        let stage_el = match self.stage_image(window) {
            Some((key, src)) => stage::stage(Some(src), key, self.scrim, crate::theme::appearance().collection_opacity / 100.),
            None => stage::stage(None, SharedString::default(), self.scrim, 1.),
        };
        let vh = f32::from(window.viewport_size().height);
        // A new setup without games gets the welcome across the whole width.
        let body = if self.lib.as_ref().is_some_and(|lib| lib.games.is_empty()) {
            self.render_welcome(vw, vh - 42., cx)
        } else if let Some(err) = self.lib.is_none().then(|| self.welcome.load_error.clone()).flatten() {
            self.render_load_error(err, cx)
        } else if self.ui.grid_view {
            self.render_grid_body(list_w, vw, vh, scale, cx)
        } else {
            let list_column = self.render_list_column(list_w, cx);
            let focus_panel = self.render_focus_panel(vw, vh, list_w, scale, cx);
            div().size_full().flex().child(list_column).child(div().flex_1().min_w_0().h_full().child(focus_panel)).into_any_element()
        };
        let toast = self.render_toast();
        let update_banner = self.render_update_banner(cx);

        div()
            .id("collection")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| this.on_key_down(ev, window, cx)))
            // Capture phase: runs before children, so a clicked input (search,
            // dialog fields) takes focus right after; clicks elsewhere blur it.
            .capture_any_mouse_down(cx.listener(|this, _, window, cx| {
                window.focus(&this.focus_handle, cx);
            }))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme::page_bg())
            .font_family(theme::FONT_BODY)
            .text_color(white())
            .child(stage_el)
            .children(page_el)
            .when(!on_page, |root| {
                root.child(div().absolute().inset_0().flex().flex_col().child(header).child(div().flex().flex_1().min_h_0().child(body)))
            })
            .children(toast)
            .children(update_banner)
            .children(self.render_context_menu(window, cx))
            .children(self.dialog.clone())
            .children(self.render_gallery(window, cx))
            .children(self.render_viewer(window, cx))
            .children(self.render_palette())
            .into_any_element()
    }
}

/// Logs how long `CollectionView::render` took (RUST_LOG=debug).
struct RenderTimer(Instant);

impl Drop for RenderTimer {
    fn drop(&mut self) {
        log::debug!("render {:?}", self.0.elapsed());
    }
}
