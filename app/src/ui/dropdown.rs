//! `CollectionDropdown`: a header tool button whose menu opens on hover and
//! closes 180ms after the pointer leaves (button + menu count as one area).

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme;
use crate::ui::glass;

/// Glass menu panel (radius 14, padding 6): the popover of every dropdown,
/// context menu and select list. Paints the blurred art behind a translucent
/// fill (see `ui::glass`).
pub fn menu_panel(min_w: f32) -> Div {
    glass::glass_menu_surface(theme::R_MENU).flex().flex_col().gap(px(2.)).min_w(px(min_w)).p(px(6.))
}

/// One menu row: 14px, padding 7/10, radius 10, hover `rgba(255,255,255,.07)`;
/// the active item gets the accent tint.
pub fn menu_item(id: impl Into<ElementId>, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.))
        .w_full()
        .py(px(7.))
        .px(px(10.))
        .rounded(px(theme::R_ITEM))
        .text_size(px(14.))
        .cursor_pointer()
        .text_color(if active { white() } else { theme::text_primary().into() })
        .when(active, |d| d.bg(theme::accent_tint()))
        .hover(|s| s.bg(if active { theme::accent_tint() } else { theme::menu_item_hover() }).text_color(white()))
}

pub fn menu_separator() -> Div {
    div().h(px(1.)).my(px(4.)).mx(px(4.)).bg(theme::white_a(0.08))
}

/// Which edge of the button the menu lines up with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
}

/// Hover callback of a dropdown: `(part, hovered)`. Parts: 1 = button,
/// 2 = menu panel. The menu rendered in a deferred layer is outside the
/// button's hitbox, so each part reports its own hover and the owner closes
/// only when none is hovered.
pub type HoverFn = std::rc::Rc<dyn Fn(u8, bool, &mut Window, &mut App)>;

pub const PART_BUTTON: u8 = 1;
pub const PART_PANEL: u8 = 2;

/// Wraps `button` and, when open, the `menu` anchored below it.
pub fn dropdown(
    id: impl Into<ElementId>,
    button: impl IntoElement,
    menu: Option<AnyElement>,
    align: Align,
    on_hover: HoverFn,
) -> Stateful<Div> {
    let id: ElementId = id.into();
    let h1 = on_hover.clone();
    let mut container = div().id(id.clone()).relative().flex_none().on_hover(move |h, w, cx| h1(PART_BUTTON, *h, w, cx)).child(button);
    if let Some(menu) = menu {
        let h2 = on_hover.clone();
        let positioned = div()
            .id(ElementId::Name(format!("{id}-panel").into()))
            .absolute()
            .top(px(32.))
            .pt(px(4.))
            .occlude()
            .on_hover(move |h, w, cx| h2(PART_PANEL, *h, w, cx));
        let positioned = match align {
            Align::Left => positioned.left_0(),
        };
        container = container.child(deferred(positioned.child(menu)).with_priority(10));
    }
    container
}
