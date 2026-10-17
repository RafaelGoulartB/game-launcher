//! `SelectField`: label, 40px select and its menu.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::input::{input_surface, search_box};
use super::text::field_label;
use crate::icons::Icon;
use crate::theme;
use crate::ui::dropdown::{menu_item, menu_panel};
use crate::ui::icon;

/// Height of the menu's list before it scrolls.
const MENU_MAX_H: f32 = 280.;

/// One entry of a select menu.
#[derive(Clone, Default)]
pub struct SelectOption {
    pub label: SharedString,
    pub selected: bool,
    /// Heading drawn above the first entry of each group.
    pub group: Option<SharedString>,
    /// Muted text after the label ("last used"...).
    pub note: Option<SharedString>,
    /// Colour dot before the label (the closed select shows the selected
    /// entry's).
    pub dot: Option<Rgba>,
}

impl From<(SharedString, bool)> for SelectOption {
    fn from((label, selected): (SharedString, bool)) -> Self {
        Self { label, selected, ..Default::default() }
    }
}

thread_local! {
    /// Scroll position of each menu (the element is rebuilt every frame).
    static SCROLLS: RefCell<HashMap<String, ScrollHandle>> = Default::default();
    /// The menu open in the last frame, to bring its selection into view
    /// once when it opens.
    static OPEN: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// `SelectField`: label, 40px select and, when `open`, its menu of `options`
/// `(label, selected)`. `on_toggle` opens/closes it, `on_pick(index)` picks.
pub fn select_field<V: 'static>(
    id: &str,
    label: Option<&str>,
    value: impl Into<SharedString>,
    disabled: bool,
    open: bool,
    options: Vec<(SharedString, bool)>,
    cx: &mut Context<V>,
    on_toggle: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    on_pick: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + 'static,
) -> Div {
    let options = options.into_iter().map(SelectOption::from).collect();
    select_menu_field(id, label, value, disabled, open, options, None, cx, on_toggle, on_pick)
}

/// [`select_field`] with group headings, notes and, given `filter`, a search
/// box over the menu that narrows it to the matching entries. `on_pick`
/// still gets the index in `options`.
#[allow(clippy::too_many_arguments)]
pub fn select_menu_field<V: 'static>(
    id: &str,
    label: Option<&str>,
    value: impl Into<SharedString>,
    disabled: bool,
    open: bool,
    options: Vec<SelectOption>,
    filter: Option<&Entity<InputState>>,
    cx: &mut Context<V>,
    on_toggle: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    on_pick: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + 'static,
) -> Div {
    let value_dot = options.iter().find(|o| o.selected).and_then(|o| o.dot);
    let select = input_surface()
        .id(SharedString::from(format!("{id}-select")))
        .relative()
        .flex()
        .items_center()
        .h(px(theme::INPUT_H))
        .w_full()
        .pl(px(14.))
        .pr(px(40.))
        .when(open && !disabled, |d| d.border_color(theme::input_border_focus()))
        .when(disabled, |d| d.opacity(0.5).cursor_not_allowed())
        .when(!disabled, |d| d.cursor_pointer().on_click(cx.listener(move |this, _, w, cx| on_toggle(this, w, cx))))
        .gap(px(9.))
        .children(value_dot.map(dot))
        .child(div().min_w_0().truncate().child(value.into()))
        .child(div().absolute().right(px(14.)).top_0().bottom_0().flex().items_center().child(icon(
            Icon::ExpandMore,
            px(18.),
            theme::text_muted(),
        )));
    let mut wrap = div().relative().flex().flex_col().children(label.map(|l| field_label(l.to_string()))).child(select);
    if !(open && !disabled) {
        OPEN.with(|o| {
            if o.borrow().as_deref() == Some(id) {
                *o.borrow_mut() = None;
            }
        });
        return wrap;
    }
    let query = filter.map(|f| f.read(cx).value().trim().to_lowercase()).unwrap_or_default();
    let on_pick = Rc::new(on_pick);
    let scroll = SCROLLS.with(|s| s.borrow_mut().entry(id.to_string()).or_insert_with(ScrollHandle::new).clone());
    let mut list = div()
        .id(SharedString::from(format!("{id}-list")))
        .flex()
        .flex_col()
        .gap(px(2.))
        .max_h(px(MENU_MAX_H))
        .overflow_y_scroll()
        .track_scroll(&scroll);
    let (mut group, mut children, mut selected_child) = (None, 0, None);
    for (i, opt) in options.into_iter().enumerate() {
        let matches = query.is_empty()
            || opt.label.to_lowercase().contains(&query)
            || opt.group.as_ref().is_some_and(|g| g.to_lowercase().contains(&query));
        if !matches {
            continue;
        }
        if opt.group.is_some() && opt.group != group {
            group = opt.group.clone();
            list = list.child(group_heading(group.clone().unwrap_or_default(), children == 0));
            children += 1;
        }
        if opt.selected {
            selected_child = Some(children);
        }
        let on_pick = on_pick.clone();
        list = list.child(
            menu_item(SharedString::from(format!("{id}-opt-{i}")), opt.selected)
                .children(opt.dot.map(dot))
                .child(div().flex_1().min_w_0().truncate().child(opt.label))
                .children(opt.note.map(|n| div().flex_none().text_size(px(12.)).text_color(theme::text_muted()).child(n)))
                .on_click(cx.listener(move |this, _, w, cx| on_pick(this, i, w, cx))),
        );
        children += 1;
    }
    if children == 0 {
        list = list.child(div().py(px(7.)).px(px(10.)).text_size(px(14.)).text_color(theme::text_muted()).child(tr!("No matches")));
    }
    // Opening: start at the selected entry.
    let opening = OPEN.with(|o| o.borrow_mut().replace(id.to_string()).as_deref() != Some(id));
    if opening {
        scroll.set_offset(point(px(0.), px(0.)));
        if let Some(child) = selected_child {
            scroll.scroll_to_item(child);
        }
    }
    // The list's size is known only after it is laid out: draw once more
    // so the scrollbar shows from the start.
    if scroll.bounds().size.height <= px(0.) {
        let view = cx.entity_id();
        cx.defer(move |cx| cx.notify(view));
    }
    let scrollbar = crate::ui::scrollbar_draggable(
        "select-menu-scrollbar",
        scroll.offset().y,
        scroll.max_offset().y,
        scroll.bounds().size.height,
        {
            let scroll = scroll.clone();
            move |distance, window, _| {
                scroll.set_offset(point(px(0.), px(-distance)));
                window.refresh();
            }
        },
        None,
    );
    let mut menu = menu_panel(100.).w_full();
    if let Some(f) = filter {
        menu = menu.child(search_box(f).w_full().mb(px(4.)));
    }
    let menu = menu.child(div().relative().pr(px(6.)).child(list).children(scrollbar));
    let top = if label.is_some() { 22. + 44. } else { 44. };
    wrap = wrap.child(deferred(div().absolute().top(px(top)).left_0().right_0().occlude().child(menu)).with_priority(40));
    wrap
}

fn dot(color: Rgba) -> Div {
    div().flex_none().size(px(8.)).rounded_full().bg(color)
}

fn group_heading(text: SharedString, first: bool) -> Div {
    div()
        .px(px(10.))
        .pt(px(if first { 4. } else { 10. }))
        .pb(px(2.))
        .text_size(px(11.))
        .font_weight(theme::W_BOLD)
        .text_color(theme::text_muted())
        .child(text)
}
