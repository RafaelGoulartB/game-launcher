//! `header.collection__header`: 42px. Left, over the list column: ☰, the
//! search pill and the View button (its menu is `view_menu/`); then one chip
//! per active filter; right: bulk metadata, refresh and the focus panel's
//! close.

use std::time::Duration;

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{FacetKind, InstallFilter};

use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::dropdown::{dropdown, Align, PART_BUTTON};
use crate::ui::{icon, tool_button, tool_button_base};
use heronite_i18n::tr;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Menu {
    /// Show / sort / group / filter.
    View,
    Nav,
}

const CLOSE_DELAY: Duration = Duration::from_millis(180);
/// Hover intent: passing over a button on the way elsewhere opens nothing.
const OPEN_DELAY: Duration = Duration::from_millis(140);

pub(super) fn facet_label(k: FacetKind) -> &'static str {
    match k {
        FacetKind::Series => tr!("Series"),
        FacetKind::Developer => tr!("Developer"),
        FacetKind::Publisher => tr!("Publisher"),
        FacetKind::Genre => tr!("Genre"),
        FacetKind::Platform => tr!("Platform"),
        FacetKind::Store => tr!("Store"),
    }
}

impl CollectionView {
    pub(super) fn new_search_input(window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        let state = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_placeholder(tr!("Search games"), window, cx);
            s
        });
        cx.subscribe_in(&state, window, |this, state, _ev: &gpui_kit::component::input::InputEvent, _window, cx| {
            let value = state.read(cx).value().to_string();
            if value != this.search {
                this.search = value;
                this.refresh_groups();
                cx.notify();
            }
        })
        .detach();
        state
    }

    fn open_header_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        if self.open_menu != Some(menu) {
            self.open_menu = Some(menu);
            self.menu_parts = PART_BUTTON;
        }
        self.menu_open = None;
        self.menu_close = None;
        cx.notify();
    }

    /// `part`: which piece of the dropdown changed hover (see
    /// `ui::dropdown::PART_*`). Opens after [`OPEN_DELAY`] on the button and
    /// closes 180ms after the last part is left.
    pub(super) fn menu_hover(&mut self, menu: Menu, part: u8, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            if self.open_menu != Some(menu) {
                self.menu_open = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(OPEN_DELAY).await;
                    this.update(cx, |this, cx| this.open_header_menu(menu, cx)).ok();
                }));
                return;
            }
            self.menu_parts |= part;
            self.menu_close = None;
            cx.notify();
        } else if self.open_menu == Some(menu) {
            self.menu_parts &= !part;
            if self.menu_parts != 0 {
                return;
            }
            self.menu_close = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(CLOSE_DELAY).await;
                this.update(cx, |this, cx| {
                    // While typing in the View menu it stays until Esc or a click outside.
                    let held = menu == Menu::View && this.view_menu.holds_open();
                    if this.open_menu == Some(menu) && this.menu_parts == 0 && !held {
                        this.open_menu = None;
                        cx.notify();
                    }
                })
                .ok();
            }));
        } else if part == PART_BUTTON {
            self.menu_open = None;
        }
    }

    pub(super) fn hover_dropdown(
        &self,
        menu: Menu,
        button: Stateful<Div>,
        align: Align,
        body: impl FnOnce(&Self, &mut Context<Self>) -> AnyElement,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let open = self.open_menu == Some(menu);
        let content = open.then(|| body(self, cx));
        dropdown(
            SharedString::from(format!("dd-{menu:?}")),
            button.on_click(cx.listener(move |this, _, _, cx| this.open_header_menu(menu, cx))),
            content,
            align,
            {
                let weak = cx.entity().downgrade();
                std::rc::Rc::new(move |part, hovered, _: &mut Window, cx: &mut App| {
                    weak.update(cx, |this, cx| this.menu_hover(menu, part, hovered, cx)).ok();
                })
            },
        )
    }

    /// "Show hidden games": persists the flag and reloads the library with or
    /// without the games hidden in Heroic.
    pub(super) fn set_show_hidden(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.ui.show_hidden == on {
            return;
        }
        self.ui.show_hidden = on;
        self.save_ui();
        self.refreshing = true;
        self.spawn_load(cx);
        cx.notify();
    }

    pub(super) fn set_install_filter(&mut self, f: InstallFilter, cx: &mut Context<Self>) {
        self.ui.install_filter = f;
        self.save_ui();
        self.refresh_groups();
        cx.notify();
    }

    /// Whether the View menu hides any game (its button turns accent).
    fn view_filtered(&self) -> bool {
        self.ui.active_filters() > 0 || self.ui.show_hidden
    }

    pub(super) fn render_header(&mut self, list_w: f32, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let open = self.open_menu == Some(Menu::View);
        let view_btn =
            tool_button_base("btn-View", open).on_click(cx.listener(|this, _, window, cx| this.focus_view_search(window, cx))).child(icon(
                Icon::Tune,
                theme::TOOL_ICON,
                if self.view_filtered() {
                    theme::accent().into()
                } else if open {
                    theme::white_a(1.)
                } else {
                    theme::tool_icon()
                },
            ));
        let view = self.hover_dropdown(Menu::View, view_btn, Align::Left, |this, cx| this.view_menu(cx), cx);
        let refresh = self.refresh_button(cx);
        let close = self.focus_close_button(cx);

        div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .h(theme::HEADER_H)
            .px(px(12.))
            .gap(px(10.))
            .child(
                // Spans the list column (up to its border) so the search
                // reads as the list's own.
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .w(px((list_w - 24.).max(300.)))
                    .child(self.nav_dropdown(cx))
                    .child(self.search_box(window, cx))
                    .child(view)
                    .children(self.onboarding_button(cx)),
            )
            .child(div().flex().flex_1().min_w_0().items_center().gap(px(6.)).overflow_hidden().children(self.filter_chips(cx)))
            .child(self.view_mode_controls(cx))
            .child(div().flex().flex_none().items_center().gap(px(2.)).children(self.bulk_chip(cx)).child(refresh).children(close))
    }

    /// One removable chip per filter the View menu has set.
    fn filter_chips(&self, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        self.view_tokens()
            .into_iter()
            .enumerate()
            .map(|(i, (kind, choice))| {
                let pick = choice.pick;
                filter_chip(("chip", i), kind, &choice.label).on_click(cx.listener(move |this, _, _, cx| this.apply_pick(&pick, cx)))
            })
            .collect()
    }

    fn refresh_button(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let refreshing = self.refreshing;
        let refresh_icon = icon(Icon::Refresh, theme::TOOL_ICON, theme::tool_icon());
        tool_button_base("hdr-refresh", false).on_click(cx.listener(|this, _, _, cx| this.refresh_all(cx))).child(if refreshing {
            refresh_icon
                .with_animation("refresh-spin", Animation::new(Duration::from_secs(1)).repeat(), |svg, t| {
                    svg.with_transformation(Transformation::rotate(percentage(t)))
                })
                .into_any_element()
        } else {
            refresh_icon.into_any_element()
        })
    }

    /// Closes the focus panel (deselects the game); only shown while it is
    /// open (the grid's drawer has its own).
    fn focus_close_button(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        (self.focus.is_some() && !self.ui.grid_view).then(|| {
            tool_button("hdr-focus-close", Icon::Close, false).on_click(cx.listener(|this, _, _, cx| {
                this.focus = None;
                this.selected = None;
                cx.notify();
            }))
        })
    }

    /// 32px pill: faint fill, brighter with an accent ring while focused;
    /// a clear button once something is typed.
    fn search_box(&self, window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let focused = self.search_input.read(cx).focus_handle(cx).is_focused(window);
        let has_text = !self.search.is_empty();
        div()
            .id("hdr-search")
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(px(8.))
            .h(theme::TOOL_BTN)
            .pl(px(12.))
            .pr(px(if has_text { 4. } else { 12. }))
            .rounded_full()
            .border_1()
            .border_color(if focused { theme::input_border_focus() } else { theme::white_a(0.) })
            .bg(if focused || has_text { theme::search_fill_hover() } else { theme::search_fill() })
            .hover(|s| s.bg(theme::search_fill_hover()))
            .child(icon(Icon::FaSearch, px(12.), theme::white_a(if focused || has_text { 0.75 } else { 0.42 })))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(14.))
                    .text_color(theme::white_a(0.92))
                    .child(Input::new(&self.search_input).appearance(false)),
            )
            .when(has_text, |d| {
                d.child(
                    div()
                        .id("hdr-search-clear")
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(24.))
                        .rounded_full()
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::white_a(0.1)))
                        .child(icon(Icon::Close, px(14.), theme::white_a(0.7)))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.search_input.update(cx, |s, cx| s.set_value("", window, cx));
                            this.search.clear();
                            this.refresh_groups();
                            cx.notify();
                        })),
                )
            })
    }
}

/// Active filter in the header: faint pill, `kind` muted before the value;
/// clicking it removes the filter.
fn filter_chip(id: impl Into<ElementId>, kind: Option<&'static str>, value: &str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .h(px(26.))
        .pl(px(10.))
        .pr(px(6.))
        .max_w(px(220.))
        .rounded_full()
        .bg(theme::chip_fill())
        .text_size(px(12.5))
        .text_color(theme::text_primary())
        .cursor_pointer()
        .hover(|s| s.bg(theme::white_a(0.12)))
        .children(kind.map(|k| div().flex_none().text_color(theme::text_muted()).child(k)))
        .child(div().min_w_0().truncate().child(value.to_owned()))
        .child(icon(Icon::Close, px(13.), theme::white_a(0.6)))
}

impl CollectionView {
    /// Opens the Settings page on `section`.
    pub(super) fn open_settings_section(&mut self, section: crate::pages::settings::Section, window: &mut Window, cx: &mut Context<Self>) {
        crate::pages::settings::request_section(section, cx);
        self.navigate(crate::pages::Page::Settings, window, cx);
    }

    /// Hands the Settings page what it needs from the Collection (the
    /// settings already loaded and the focused game's stage artwork).
    pub(super) fn offer_settings_data(&self, cx: &mut Context<Self>) {
        let Some(lib) = self.lib.as_ref() else { return };
        let chain = self.selected.as_ref().and_then(|k| self.stage_chains.get(k).cloned()).unwrap_or_default();
        crate::pages::settings::offer(lib.settings.clone(), chain, cx);
    }
}
