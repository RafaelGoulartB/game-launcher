//! Glass form controls and typography: toggle switches, pill buttons,
//! segmented tabs, cards of rows, captions, kicker, kbd chips, close button
//! and navigation pills.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// Critically damped spring: the knob settles in about 150ms.
const TOGGLE_SPRING: SpringConfig = SpringConfig::new(900., 60., 1.);

fn knob_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow { color: theme::black_a(0.4), offset: point(px(0.), px(1.)), blur_radius: px(3.), spread_radius: px(0.), inset: false }]
}

/// Accent fill (fading in) and white knob (sliding) of a switch `w` wide
/// with a `knob` circle inset by `inset`.
fn switch_fx(key: String, on: bool, fill: Hsla, w: f32, knob: f32, inset: f32) -> AnyElement {
    let travel = w - knob - 2. * inset;
    let build = move |fx: Div, t: f32| {
        let t = t.clamp(0., 1.);
        fx.child(div().absolute().inset_0().rounded_full().bg(fill).opacity(t)).child(
            div().absolute().top(px(inset)).left(px(inset + travel * t)).size(px(knob)).rounded_full().bg(white()).shadow(knob_shadow()),
        )
    };
    let fx = div().absolute().inset_0();
    if theme::animations() {
        fx.with_spring(
            SharedString::from(format!("{key}-fx")),
            SpringAnimation::new(TOGGLE_SPRING).to(on).with_epsilon(0.002),
            move |d, phase| build(d, phase.0),
        )
        .into_any_element()
    } else {
        build(fx, if on { 1. } else { 0. }).into_any_element()
    }
}

fn switch_box(id: impl Into<ElementId>, on: bool, disabled: bool, fill: Hsla, w: f32, h: f32, knob: f32, inset: f32) -> Stateful<Div> {
    let id: ElementId = id.into();
    let key = format!("{id}");
    div()
        .id(id)
        .relative()
        .flex_none()
        .w(px(w))
        .h(px(h))
        .rounded_full()
        .bg(theme::toggle_off())
        .when(disabled, |d| d.opacity(0.4).cursor_not_allowed())
        .when(!disabled, |d| d.cursor_pointer())
        .child(switch_fx(key, on, fill, w, knob, inset))
}

/// Toggle switch (40x24 pill, 18px knob): off `rgba(255,255,255,.16)`, on =
/// accent. The caller adds `.on_click(..)`. Disabled is drawn at 40% opacity.
pub fn toggle(id: impl Into<ElementId>, on: bool, disabled: bool) -> Stateful<Div> {
    switch_box(id, on, disabled, theme::accent().into(), theme::TOGGLE_W, theme::TOGGLE_H, theme::TOGGLE_KNOB, 3.)
}

/// A [`toggle`] that is on but has no effect yet (something it needs is
/// missing): grey instead of the accent, still clickable.
pub fn toggle_inactive(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    switch_box(id, on, false, theme::white_a(0.3), theme::TOGGLE_W, theme::TOGGLE_H, theme::TOGGLE_KNOB, 3.)
}

/// Compact switch (30x18) for dense lists (ROM / emulator pickers): the caller
/// adds the click handler and the label next to it.
pub fn mini_checkbox(id: impl Into<ElementId>, checked: bool, disabled: bool) -> Stateful<Div> {
    switch_box(id, checked, disabled, theme::accent().into(), 30., 18., 12., 3.).mt(px(2.))
}

/// A settings row (see [`row`]) whose control is a toggle; the whole row is
/// clickable (the caller adds `.on_click(..)`).
pub fn toggle_row(id: impl Into<ElementId>, label: impl Into<SharedString>, description: Option<&str>, on: bool) -> Stateful<Div> {
    let id: ElementId = id.into();
    let sw = toggle(SharedString::from(format!("{id}-sw")), on, false);
    row(label, description, sw).id(id).cursor_pointer()
}

/// A [`toggle_row`] without description and with less padding.
pub fn checkbox(id: impl Into<ElementId>, checked: bool, label: impl Into<SharedString>) -> Stateful<Div> {
    toggle_row(id, label, None, checked).py(px(7.))
}

/// Label (14px) with an optional description (12px) on the left, `control`
/// on the right. Padding 11px 0; put rows inside a [`card`].
pub fn row(label: impl Into<SharedString>, description: Option<&str>, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .py(px(11.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(1.))
                .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(label.into()))
                .children(description.map(|d| {
                    div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(SharedString::from(d.to_string()))
                })),
        )
        .child(div().flex_none().child(control))
}

/// The styled container of a [`card`] without rows (fill, border, radius 16,
/// padding 4px 16px) for custom content.
pub fn card_box() -> Div {
    div()
        .flex()
        .flex_col()
        .w_full()
        .px(px(16.))
        .py(px(4.))
        .rounded(px(theme::R_CARD))
        .bg(theme::card_fill())
        .border_1()
        .border_color(theme::card_border())
}

/// Group of rows with 1px separators between them.
pub fn card(rows: impl IntoIterator<Item = AnyElement>) -> Div {
    let mut c = card_box();
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            c = c.child(div().h(px(1.)).w_full().bg(theme::row_separator()));
        }
        c = c.child(r);
    }
    c
}

/// Line height of a [`caption`].
pub const CAPTION_H: f32 = 16.;

/// Section caption above a card: 12px muted, sentence case.
pub fn caption(text: impl Into<SharedString>) -> Div {
    div().mt(px(14.)).mx(px(6.)).mb(px(6.)).text_size(px(12.)).line_height(px(CAPTION_H)).text_color(theme::text_muted()).child(text.into())
}

/// Page / sheet title: Rubik 600, 26px.
pub fn page_heading(text: impl Into<SharedString>) -> Div {
    div()
        .font_family(theme::FONT_HEADING)
        .text_size(px(26.))
        .line_height(px(32.))
        .font_weight(theme::W_BOLD)
        .text_color(theme::text_primary())
        .child(text.into())
}

/// Drawer / dialog title: Rubik 600, 22px.
pub fn dialog_title(text: impl Into<SharedString>) -> Div {
    div()
        .font_family(theme::FONT_HEADING)
        .text_size(px(22.))
        .line_height(px(27.))
        .font_weight(theme::W_BOLD)
        .text_color(theme::text_primary())
        .child(text.into())
}

/// Lead text under a title: 13px muted.
pub fn help_text(text: impl Into<SharedString>) -> Div {
    div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(text.into())
}

/// Small accent label above a title, the only uppercase text (11px).
pub fn kicker(text: impl Into<SharedString>) -> Div {
    let text: SharedString = text.into();
    div().text_size(px(11.)).text_color(theme::accent()).child(SharedString::from(text.to_uppercase()))
}

/// Keyboard hint chip (`Ctrl K`): 11px mono, radius 5.
pub fn kbd(text: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(5.))
        .bg(theme::white_a(0.08))
        .border_1()
        .border_color(theme::white_a(0.12))
        .font_family(theme::FONT_MONO)
        .text_size(px(11.))
        .text_color(theme::text_muted())
        .child(text.into())
}

/// Round 32px close button (X icon); the caller adds `.on_click(..)`.
pub fn close_button(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(32.))
        .rounded_full()
        .bg(theme::white_a(0.08))
        .cursor_pointer()
        .hover(|s| s.bg(theme::white_a(0.14)))
        .child(icon(Icon::Close, px(16.), theme::text_primary()))
}

/// Side navigation item (settings sections): pill, icon + label; the active
/// one gets the accent tint. The caller adds `.on_click(..)`.
pub fn nav_pill(id: impl Into<ElementId>, ic: Icon, label: impl Into<SharedString>, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .py(px(9.))
        .mb(px(2.))
        .rounded_full()
        .text_size(px(14.))
        .cursor_pointer()
        .text_color(if active { white() } else { theme::nav_text().into() })
        .when(active, |d| d.bg(theme::accent_tint()))
        .when(!active, |d| d.hover(|s| s.bg(theme::white_a(0.05))))
        .child(icon(ic, px(16.), if active { theme::accent() } else { theme::nav_text() }))
        .child(label.into())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonKind {
    /// Accent fill, dark text.
    Primary,
    /// Translucent pill: the default secondary button.
    Glass,
    /// No fill, muted text.
    Ghost,
    /// Destructive actions: red tint.
    Danger,
}

fn button_base(
    id: impl Into<ElementId>,
    kind: ButtonKind,
    ic: Option<Icon>,
    label: SharedString,
    small: bool,
    enabled: bool,
) -> Stateful<Div> {
    let b = div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .h(px(if small { theme::BUTTON_H_SM } else { theme::BUTTON_H }))
        .px(px(if small { 12. } else { 16. }))
        .rounded_full()
        .whitespace_nowrap()
        .text_size(px(13.))
        .when_some(ic, |b, ic| {
            let color = match kind {
                ButtonKind::Primary => theme::on_accent(),
                ButtonKind::Glass => theme::button_text(),
                ButtonKind::Ghost => theme::text_muted(),
                ButtonKind::Danger => theme::danger_text(),
            };
            b.child(icon(ic, px(if small { 15. } else { 16. }), color))
        })
        .child(label);
    let b = match kind {
        ButtonKind::Glass => b.bg(theme::button_fill()).border_1().border_color(theme::button_border()).text_color(theme::button_text()),
        ButtonKind::Primary => b.bg(theme::accent()).text_color(theme::on_accent()).font_weight(theme::W_BOLD),
        ButtonKind::Ghost => b.text_color(theme::text_muted()),
        ButtonKind::Danger => b.bg(theme::danger_fill()).text_color(theme::danger_text()),
    };
    if !enabled {
        return b.opacity(0.45).cursor_not_allowed();
    }
    let b = b.cursor_pointer();
    match kind {
        ButtonKind::Glass => b.hover(|s| s.bg(theme::button_fill_hover())),
        ButtonKind::Primary => b.hover(|s| s.bg(theme::accent_overlay())),
        ButtonKind::Ghost => b.hover(|s| s.bg(theme::white_a(0.06)).text_color(theme::text_primary())),
        ButtonKind::Danger => b.hover(|s| s.bg(theme::danger_fill_hover())),
    }
}

/// Pill button (36px, 13px, sentence case). See [`ButtonKind`] for the look.
pub fn button(id: impl Into<ElementId>, kind: ButtonKind, label: impl Into<SharedString>) -> Stateful<Div> {
    button_base(id, kind, None, label.into(), false, true)
}

/// Small pill button (30px).
pub fn button_sm(id: impl Into<ElementId>, kind: ButtonKind, label: impl Into<SharedString>) -> Stateful<Div> {
    button_base(id, kind, None, label.into(), true, true)
}

/// A [`button_sm`] with an icon before the label that only reacts when
/// `enabled`; disabled ones are drawn at 45% opacity.
pub fn icon_button_sm(
    id: impl Into<ElementId>,
    kind: ButtonKind,
    ic: Icon,
    label: impl Into<SharedString>,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let b = button_base(id, kind, Some(ic), label.into(), true, enabled);
    if enabled {
        b.on_click(on_click)
    } else {
        b
    }
}

/// A [`button`] that only reacts when `enabled`; disabled ones are drawn at
/// 45% opacity.
pub fn action_button(
    id: impl Into<ElementId>,
    kind: ButtonKind,
    label: impl Into<SharedString>,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let b = button_base(id, kind, None, label.into(), false, enabled);
    if enabled {
        b.on_click(on_click)
    } else {
        b
    }
}

/// Segmented pill tabs: a dark track with the active item lifted
/// (`rgba(255,255,255,.14)`, white text). Tabs that do not fit the width
/// (it depends on the language, not on how many there are) wrap to another
/// row; one row keeps the pill look.
pub fn tabs<T: Copy + PartialEq + 'static>(
    id: &str,
    items: &[(T, &'static str)],
    active: T,
    on_select: impl Fn(T, &mut Window, &mut App) + 'static,
) -> Div {
    let on_select = std::rc::Rc::new(on_select);
    let mut track = div()
        .id(SharedString::from(format!("{id}-seg")))
        .flex()
        .items_center()
        .gap(px(2.))
        .p(px(4.))
        .flex_wrap()
        .max_w_full()
        .rounded(px(18.))
        .bg(theme::seg_track());
    for (i, (value, label)) in items.iter().enumerate() {
        let is_active = *value == active;
        let v = *value;
        let cb = on_select.clone();
        track = track.child(
            div()
                .id(SharedString::from(format!("{id}-tab-{i}")))
                .flex_none()
                .px(px(12.))
                .py(px(6.))
                .rounded_full()
                .text_size(px(13.))
                .whitespace_nowrap()
                .cursor_pointer()
                .text_color(if is_active { white() } else { theme::seg_text().into() })
                .when(is_active, |d| d.bg(theme::seg_active()))
                .when(!is_active, |d| d.hover(|s| s.text_color(white())))
                .on_click(move |_, w, cx| cb(v, w, cx))
                .child(*label),
        );
    }
    div().flex().min_w_0().child(track)
}
