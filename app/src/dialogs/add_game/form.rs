//! Rendering of the form, the Wine section and the remove confirmation.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::sideload::SideloadPlatform;
use heronite_i18n::tr;

use super::logic::*;
use super::picker::Slot;
use super::{text_of, AddGameDialog, Select};
use crate::dialogs::settings_panel::logic::{default_wine, display_wine_name, wine_of, wine_options};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, checkbox, ButtonKind};
use crate::ui::fields::{choose_path, field_label, input_box, labeled, meta_text, panel, path_field, select_field};
use crate::ui::icon;

/// Cover/background preview box sizes (600x900 and 1600x650 ratios).
const SQUARE_W: f32 = 168.;
const SQUARE_H: f32 = 252.;
const HERO_W: f32 = 168.;
const HERO_H: f32 = 68.;

impl AddGameDialog {
    fn platform_select(&self, cx: &mut Context<Self>) -> Div {
        let options: Vec<(SharedString, bool)> =
            PLATFORMS.iter().map(|p| (SharedString::from(platform_label(*p)), *p == self.platform)).collect();
        select_field(
            "ag-platform",
            Some(tr!("Platform")),
            platform_label(self.platform),
            false,
            self.open_select == Some(Select::Platform),
            options,
            cx,
            |this, _, cx| {
                this.open_select = if this.open_select == Some(Select::Platform) { None } else { Some(Select::Platform) };
                cx.notify();
            },
            |this, i, _, cx| this.set_platform(PLATFORMS[i], cx),
        )
    }

    fn wine_section(&mut self, cx: &mut Context<Self>) -> Div {
        let open = self.show_wine;
        let header = div()
            .id("ag-wine-toggle")
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .text_size(px(14.))
            .text_color(crate::theme::text_primary())
            .on_click(cx.listener(|this, _, _, cx| {
                this.show_wine = !this.show_wine;
                cx.notify();
            }))
            .child(icon(if open { Icon::FaChevronUp } else { Icon::FaChevronDown }, px(12.), crate::theme::text_primary()))
            .child(tr!("Show Wine settings"));
        let mut col = crate::ui::controls::card_box().gap(px(12.)).py(px(12.)).child(header);
        if !open {
            return col;
        }
        let shared = self.shared_prefix;
        let mut share_label = tr!("Use shared Wine prefix").to_string();
        if let Some(w) = self.global.get("wineVersion").map(|_| wine_of(&self.global)) {
            share_label = tr!("Use shared Wine prefix ({name})", name = display_wine_name(&w.name));
        }
        col = col.child(checkbox("ag-shared", shared, share_label).on_click(cx.listener(|this, _, window, cx| {
            let on = !this.shared_prefix;
            this.set_shared_prefix(on, window, cx);
        })));
        if shared {
            col = col.child(
                panel().child(meta_text(tr!(
                    "Only use this option if you know what you are doing. Sharing the same prefix for multiple games can create problems if their dependencies are incompatible."
                ))),
            );
        }
        if self.wine.as_ref().is_some_and(|w| w.kind == "crossover") {
            col = col.child(meta_text(tr!("CrossOver bottles are chosen in Game settings after the game is added.")));
        } else {
            let has = !text_of(&self.prefix, cx).is_empty();
            col = col.child(path_field(
                "ag-prefix-pick",
                Some(tr!("WinePrefix")),
                &self.prefix,
                has,
                shared,
                cx.listener(move |this, _, window, cx| {
                    if has {
                        this.prefix.update(cx, |s, cx| s.set_value("", window, cx));
                    } else {
                        choose_path(window, cx, true, tr!("Select WinePrefix Folder"), |this: &mut Self, path, window, cx| {
                            this.prefix.update(cx, |s, cx| s.set_value(path, window, cx));
                        });
                    }
                }),
            ));
        }
        let list = self.wine_list.clone();
        let loading = list.is_none();
        let current = self.wine.clone().unwrap_or_else(default_wine);
        let options = wine_options(list.as_deref().unwrap_or(&[]), &current);
        let none_found = list.as_ref().is_some_and(|l| l.is_empty()) && self.wine.is_none();
        let shown = if loading {
            tr!("Loading\u{2026}").to_string()
        } else if self.wine.is_none() {
            tr!("No Wine version found").to_string()
        } else {
            display_wine_name(&current.name)
        };
        let items: Vec<(SharedString, bool)> = options
            .iter()
            .map(|w| (SharedString::from(display_wine_name(&w.name)), self.wine.as_ref().is_some_and(|c| c.name == w.name)))
            .collect();
        col = col.child(select_field(
            "ag-wine",
            Some(tr!("Wine Version")),
            shown,
            shared || loading || options.is_empty() || none_found,
            self.open_select == Some(Select::Wine),
            items,
            cx,
            |this, _, cx| {
                this.open_select = if this.open_select == Some(Select::Wine) { None } else { Some(Select::Wine) };
                cx.notify();
            },
            move |this, i, _, cx| {
                let list = this.wine_list.clone().unwrap_or_default();
                let cur = this.wine.clone().unwrap_or_else(default_wine);
                if let Some(w) = wine_options(&list, &cur).get(i) {
                    this.wine = Some(w.clone());
                }
                this.open_select = None;
                cx.notify();
            },
        ));
        if none_found {
            col = col.child(
                div()
                    .text_size(px(13.33))
                    .text_color(theme::danger_text())
                    .child(tr!("No Wine version was found, download one from the Wine Manager")),
            );
        }
        col
    }

    pub(super) fn form(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let _ = window;
        let (square_url, hero_url) = (text_of(&self.square, cx), text_of(&self.hero, cx));
        let title = text_of(&self.title, cx);
        let preview_hero = if hero_url.trim().is_empty() { square_url.clone() } else { hero_url.clone() };

        let left = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(12.))
            .w(px(SQUARE_W))
            .child(self.art_box("ag-art-square", Slot::Square, &square_url, SQUARE_W, SQUARE_H, cx))
            .child(self.art_box("ag-art-hero", Slot::Cover, &preview_hero, HERO_W, HERO_H, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(theme::W_BOLD)
                            .text_color(crate::theme::text_primary())
                            .child(if title.trim().is_empty() { tr!("Title").to_string() } else { title.clone() }),
                    )
                    .child(meta_text(platform_label(self.platform))),
            );

        let mut right = div().flex().flex_col().flex_1().min_w_0().gap(px(16.));
        if let Some(note) = self.top_note() {
            right = right.child(panel().child(div().text_size(px(14.)).line_height(px(20.)).text_color(white()).child(note)));
        }
        right = right.child(div().flex().flex_col().child(field_label(tr!("Game/App Title"))).child(input_box(&self.title, false)));

        let finding = self.finding;
        right = right
            .child(self.art_field(
                "ag-square",
                tr!("Square Art (paste an URL, pick a file or click the image to search the web)"),
                &self.square,
                Slot::Square,
                cx,
            ))
            .child(self.art_field("ag-hero", tr!("Cover/Hero Art (optional, wide background)"), &self.hero, Slot::Cover, cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .child(action_button(
                        "ag-find-art",
                        ButtonKind::Glass,
                        if finding { tr!("Searching\u{2026}") } else { tr!("Find art for this title") },
                        !finding && !title.trim().is_empty(),
                        cx.listener(|this, _, window, cx| this.find_art(window, cx)),
                    ))
                    .child(action_button(
                        "ag-search-square",
                        ButtonKind::Glass,
                        tr!("Search the web: cover"),
                        true,
                        cx.listener(|this, _, window, cx| this.open_picker(Slot::Square, window, cx)),
                    ))
                    .child(action_button(
                        "ag-search-hero",
                        ButtonKind::Glass,
                        tr!("Search the web: background"),
                        true,
                        cx.listener(|this, _, window, cx| this.open_picker(Slot::Cover, window, cx)),
                    )),
            );

        right = right.child(self.platform_select(cx));

        if self.platform == SideloadPlatform::Windows && !self.is_edit() {
            right = right.child(self.wine_section(cx));
        }

        if self.platform == SideloadPlatform::Browser {
            right = right
                .child(labeled(tr!("BrowserURL"), input_box(&self.url, false)))
                .child(labeled(tr!("Custom User Agent"), input_box(&self.user_agent, false)))
                .child(crate::ui::controls::card([checkbox("ag-fullscreen", self.fullscreen, tr!("Launch Fullscreen (F11 to exit)"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.fullscreen = !this.fullscreen;
                        cx.notify();
                    }))
                    .into_any_element()]));
        } else if self.platform == SideloadPlatform::Linux {
            right = right.child(self.linux_start(cx));
        } else {
            let exe = text_of(&self.exe, cx);
            let has = !exe.is_empty();
            right = right.child(path_field(
                "ag-exe-pick",
                Some(tr!("Select Executable")),
                &self.exe,
                has,
                false,
                cx.listener(move |this, _, window, cx| {
                    if has {
                        this.exe.update(cx, |s, cx| s.set_value("", window, cx));
                    } else {
                        choose_path(window, cx, false, tr!("Select Executable"), |this: &mut Self, path, window, cx| {
                            this.exe.update(cx, |s, cx| s.set_value(path, window, cx));
                        });
                    }
                }),
            ));
            if let Some(folder) = install_folder(&exe) {
                right = right.child(meta_text(tr!("Install folder: {folder}", folder)));
            }
        }

        if !self.error.is_empty() {
            right = right.child(div().text_size(px(14.)).line_height(px(20.)).text_color(theme::danger_text()).child(self.error.clone()));
        } else if !self.notice.is_empty() {
            right = right.child(div().text_size(px(14.)).line_height(px(20.)).text_color(theme::success()).child(self.notice.clone()));
        }

        div().flex().gap(px(24.)).child(left).child(right)
    }

    /// Linux: the game's folder, then how it starts (an executable, or a
    /// command run inside the folder).
    fn linux_start(&mut self, cx: &mut Context<Self>) -> Div {
        let folder = text_of(&self.folder, cx);
        let has_folder = !folder.trim().is_empty();
        let mut col = div().flex().flex_col().gap(px(16.)).child(path_field(
            "ag-folder-pick",
            Some(tr!("Game folder")),
            &self.folder,
            has_folder,
            false,
            cx.listener(move |this, _, window, cx| {
                if has_folder {
                    this.folder.update(cx, |s, cx| s.set_value("", window, cx));
                } else {
                    choose_path(window, cx, true, tr!("Select the game folder"), |this: &mut Self, path, window, cx| {
                        this.folder.update(cx, |s, cx| s.set_value(path, window, cx));
                    });
                }
            }),
        ));
        let weak = cx.entity().downgrade();
        col = col.child(div().flex().flex_col().child(field_label(tr!("Start with"))).child(crate::ui::controls::tabs(
            "ag-start",
            &[(false, tr!("An executable")), (true, tr!("A command"))],
            self.by_command,
            move |by_command, _, cx| {
                weak.update(cx, |this, cx| {
                    this.by_command = by_command;
                    this.error.clear();
                    cx.notify();
                })
                .ok();
            },
        )));
        if self.by_command {
            col = col
                .child(labeled(tr!("Command"), input_box(&self.command, false)))
                .child(meta_text(tr!("Runs inside the game folder, like a terminal opened there. Example: ./start.sh --fullscreen")));
        } else {
            let exe = text_of(&self.exe, cx);
            let has = !exe.is_empty();
            col = col.child(path_field(
                "ag-exe-pick",
                Some(tr!("Select Executable")),
                &self.exe,
                has,
                false,
                cx.listener(move |this, _, window, cx| {
                    if has {
                        this.exe.update(cx, |s, cx| s.set_value("", window, cx));
                    } else {
                        choose_path(window, cx, false, tr!("Select Executable"), |this: &mut Self, path, window, cx| {
                            this.exe.update(cx, |s, cx| s.set_value(path.clone(), window, cx));
                            this.fill_folder_from(&path, window, cx);
                        });
                    }
                }),
            ));
        }
        col
    }

    pub(super) fn remove_confirmation(&self, cx: &mut Context<Self>) -> Div {
        let title = text_of(&self.title, cx);
        let mut col = div().flex().flex_col().gap(px(16.)).child(div().text_size(px(14.)).line_height(px(20.)).text_color(crate::theme::text_primary()).child(tr!(
            "Remove {title} from your library? The game's files are not deleted. Its playtime, status, notes, images and Collection data are removed, and so are its shortcuts.",
            title = if title.trim().is_empty() { tr!("this game") } else { title.trim() }
        )));
        if let (Some(prefix), Some(note)) = (&self.removable_prefix, &self.prefix_shared) {
            col = col.child(meta_text(tr!("Wine prefix {prefix}: {note}", prefix, note)));
        } else if let Some(prefix) = &self.removable_prefix {
            col = col
                .child(checkbox("ag-remove-prefix", self.remove_prefix_too, tr!("Also delete the Wine prefix")).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.remove_prefix_too = !this.remove_prefix_too;
                        cx.notify();
                    },
                )))
                .child(meta_text(tr!("{prefix}  (saves and settings inside it will be lost)", prefix)));
        }
        if !self.error.is_empty() {
            col = col.child(div().text_size(px(14.)).text_color(theme::danger_text()).child(self.error.clone()));
        }
        col
    }
}
