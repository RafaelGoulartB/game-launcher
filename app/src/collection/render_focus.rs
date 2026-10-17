//! The focus panel as the Collection view builds it: image resolution and the
//! callbacks into the view.

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_images::{ImageRef, Variant};

use super::{focus, html, CollectionView};
use crate::theme;
use heronite_i18n::tr;

impl CollectionView {
    pub(super) fn render_focus_panel(&mut self, vw: f32, vh: f32, list_w: f32, scale: f32, cx: &mut Context<Self>) -> AnyElement {
        let active_facets = self.active_facets();
        let Some(vm) = self.focus.clone() else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(15.2))
                .text_color(theme::white_a(0.52))
                .child(tr!("No game selected"))
                .into_any_element();
        };
        let vm = self.with_folder_rows(vm);
        let covers = self.focus_covers(&vm, scale);
        let html_images = self.focus_html_images(&vm, focus::main_column_width(vw - list_w), scale);
        let lay = focus::layout(vw - list_w);
        let epic_focus = self.epic_focus(&vm.key, vm.installed, cx);
        let left = self.focus_left(&vm, &lay, epic_focus, scale, vh, cx);
        let screenshots = self.screenshots_section(cx);
        let timeline = self.timeline_section(&vm.key, lay.main_w, cx);
        let focus_extras = self.focus_extras(&vm.key, scale);
        focus::focus_panel(
            focus::FocusProps {
                vm,
                cover_for: Rc::new(move |k| covers.get(k).cloned()),
                html_images: Rc::new(move |u| html_images.get(u).cloned()),
                scroll: self.focus_scroll.clone(),
                viewport_w: vw,
                panel_w: vw - list_w,
                active_facets,
                left,
                screenshots,
                timeline,
                extras: focus_extras,
            },
            self.focus_actions(cx),
        )
    }

    /// The filters set by facet, for the Details chips.
    pub(super) fn active_facets(&self) -> Vec<(&'static str, SharedString)> {
        heronite_data::collection::FACET_KINDS
            .iter()
            .flat_map(|k| self.ui.facet_filters.get(*k).iter().map(|v| (k.as_str(), SharedString::from(v.clone()))))
            .collect()
    }

    /// Callbacks of the details into the view (focus panel and grid drawer).
    pub(super) fn focus_actions(&self, cx: &mut Context<Self>) -> focus::FocusActions {
        let weak = cx.entity().downgrade();
        let (w1, w2, w3) = (weak.clone(), weak.clone(), weak);
        focus::FocusActions {
            on_scroll: Box::new(cx.listener(|_, _: &(), _, cx| cx.notify()).into_fn()),
            on_facet: Rc::new(move |f, v, _, cx| {
                w1.update(cx, |this, cx| {
                    this.toggle_facet(f, &v);
                    cx.notify();
                })
                .ok();
            }),
            on_select: Rc::new(move |k, _, cx| {
                w2.update(cx, |this, cx| {
                    this.select(k, true);
                    this.reveal_selected();
                    cx.notify();
                })
                .ok();
            }),
            on_open_folder: Rc::new(move |path, _, cx| {
                w3.update(cx, |this, cx| this.open_folder(path.to_string().into(), cx)).ok();
            }),
        }
    }

    /// Details with the folders found on disk (install, or emulator + ROM)
    /// after "Playtime".
    pub(super) fn with_folder_rows(&self, mut vm: super::vm::FocusVm) -> super::vm::FocusVm {
        let rows = self.facts.folders.iter().enumerate().map(|(i, (kind, path))| super::vm::DetailRow {
            label: kind.label().into(),
            value: super::vm::DetailValue::Folder {
                path: path.to_string_lossy().into_owned().into(),
                size: (*kind == super::open_links::FolderKind::Install).then(|| vm.install_size.clone()).flatten(),
            },
            split: i == 0,
        });
        let at = vm.details.iter().position(|r| r.label.as_ref() == tr!("Playtime")).map_or(vm.details.len(), |i| i + 1);
        vm.details.splice(at..at, rows.collect::<Vec<_>>());
        vm
    }

    /// Covers of the related games (the game's own is in the left column).
    pub(super) fn focus_covers(&mut self, vm: &super::vm::FocusVm, scale: f32) -> HashMap<SharedString, ImageSource> {
        let mut covers: HashMap<SharedString, ImageSource> = HashMap::new();
        for g in &vm.related {
            for c in &g.games {
                if let Some(src) = self.cover(&c.cover_key, 78., 117., scale, 50) {
                    covers.insert(c.cover_key.clone(), src);
                }
            }
        }
        covers
    }

    /// Images inside the overview HTML, by URL, sized for a `column` wide text.
    pub(super) fn focus_html_images(&mut self, vm: &super::vm::FocusVm, column: f32, scale: f32) -> HashMap<String, ImageSource> {
        let mut html_images: HashMap<String, ImageSource> = HashMap::new();
        let Some(blocks) = vm.overview_html.as_ref() else { return html_images };
        for b in blocks.iter() {
            if let html::Block::Image { url, width, height } = b {
                let (w, h) = focus::html_image_size(*width, *height, column);
                let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
                let id: SharedString = format!("h:{url}:{pw}x{ph}").into();
                let chain = vec![ImageRef::Url(url.clone())];
                if let Some(img) = self.images.get_or_request(&id, || chain, pw, ph, Variant::Cover, 20) {
                    html_images.insert(url.clone(), ImageSource::Render(img));
                }
            }
        }
        html_images
    }
}

trait IntoFn {
    fn into_fn(self) -> Box<dyn Fn(&mut Window, &mut App)>;
}

impl<F: Fn(&(), &mut Window, &mut App) + 'static> IntoFn for F {
    fn into_fn(self) -> Box<dyn Fn(&mut Window, &mut App)> {
        Box::new(move |w, cx| self(&(), w, cx))
    }
}
