//! "Add game" / "Edit game" in console mode: the whole form as console rows
//! (text fields through the on-screen keyboard, files and folders through
//! the console browser, art through a sheet and the image grid), a preview
//! of the cover on the right and the Add / Save button at the end.

use std::path::{Path, PathBuf};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::sideload::SideloadPlatform;
use heronite_i18n::tr;

use super::logic::*;
use super::picker::{ImagePicker, PickerEvent, Slot};
use super::{text_of, AddGameDialog};
use crate::console::browser::{Browser, BrowserEvent, PickKind};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row, Value};
use crate::console::osk::{Osk, OskEvent};
use crate::console::sheet::{Sheet, SheetEvent};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::dialogs::art_images::{chain_for_url, picture};
use crate::dialogs::settings_panel::logic::{default_wine, display_wine_name, wine_of, wine_options};
use crate::dialogs::DialogEvent;
use crate::icons::Icon;
use crate::theme;

const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "bmp", "gif"];

/// A text field edited with the on-screen keyboard.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextField {
    Title,
    Url,
    UserAgent,
    Command,
    Art(Slot),
}

/// What a file or folder picked with the console browser is for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    Executable,
    Folder,
    Prefix,
    Installer,
    Art(Slot),
}

/// `/a/b/c/game.exe` -> `…/c/game.exe`.
fn short_path(p: &str) -> String {
    let parts: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
    match parts.len() {
        0 => String::new(),
        1 | 2 => p.to_string(),
        n => format!("\u{2026}/{}/{}", parts[n - 2], parts[n - 1]),
    }
}

/// Keeps the start and the end of a long address.
fn ellipsize(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 36 {
        return s.to_string();
    }
    format!("{}\u{2026}{}", chars[..16].iter().collect::<String>(), chars[chars.len() - 16..].iter().collect::<String>())
}

/// What a stored art value looks like in a row.
fn art_label(value: &str) -> String {
    let v = value.trim();
    if let Some(rest) = v.strip_prefix("file://") {
        let path = heronite_data::paths::percent_decode(rest);
        return Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(path);
    }
    ellipsize(v)
}

impl AddGameDialog {
    fn wine_choices(&self) -> Vec<heronite_wine::WineInstallation> {
        let current = self.wine.clone().unwrap_or_else(default_wine);
        wine_options(self.wine_list.as_deref().unwrap_or(&[]), &current)
    }

    fn console_rows(&self, cx: &App) -> Vec<Row> {
        let title = text_of(&self.title, cx);
        if self.confirm_remove {
            let mut rows = Vec::new();
            if let Some(note) = &self.prefix_shared {
                rows.push(Row::toggle("remove-prefix", tr!("Also delete the Wine prefix"), false).desc(note.clone()).enabled(false));
            } else if self.removable_prefix.is_some() {
                rows.push(
                    Row::toggle("remove-prefix", tr!("Also delete the Wine prefix"), self.remove_prefix_too)
                        .desc(tr!("Saves and settings inside the prefix will be lost.")),
                );
            }
            rows.push(Row::action("back", tr!("Back")).enabled(!self.saving));
            rows.push(
                Row::action("remove-yes", if self.saving { tr!("Removing\u{2026}") } else { tr!("Remove game") })
                    .icon(Icon::DeleteForever)
                    .danger()
                    .enabled(!self.saving),
            );
            return rows;
        }

        let edit = self.is_edit();
        let busy = self.saving || self.installer_running;
        let mut rows = vec![
            Row::header(tr!("Game")),
            Row::text("title", tr!("Title"), title.trim()).icon(Icon::Edit).desc(tr!("Letters and numbers only, up to 40 characters.")),
            Row::choice(
                "platform",
                tr!("Platform"),
                PLATFORMS.iter().map(|p| SharedString::from(platform_label(*p))).collect(),
                PLATFORMS.iter().position(|p| *p == self.platform).unwrap_or(0),
            )
            .desc(tr!("What the game needs to run. Picking an executable chooses it for you.")),
        ];
        if self.platform == SideloadPlatform::Browser {
            rows.push(Row::text("url", tr!("Game URL"), text_of(&self.url, cx).trim()).icon(Icon::OpenInNew));
            rows.push(
                Row::text("ua", tr!("Custom user agent"), text_of(&self.user_agent, cx).trim())
                    .desc(tr!("Optional. Used only by this browser game.")),
            );
            rows.push(Row::toggle("fullscreen", tr!("Launch fullscreen"), self.fullscreen).desc(tr!("Press F11 to exit.")));
        } else {
            let linux = self.platform == SideloadPlatform::Linux;
            if linux {
                rows.push(
                    Row::text("folder", tr!("Game folder"), short_path(text_of(&self.folder, cx).trim()))
                        .icon(Icon::FolderOpen)
                        .desc(tr!("The folder the game is in")),
                );
                rows.push(Row::choice(
                    "start",
                    tr!("Start with"),
                    vec![tr!("An executable").into(), tr!("A command").into()],
                    usize::from(self.by_command),
                ));
            }
            if linux && self.by_command {
                rows.push(
                    Row::text("command", tr!("Command"), text_of(&self.command, cx).trim())
                        .icon(Icon::Edit)
                        .desc(tr!("Runs inside the game folder, like a terminal opened there. Example: ./start.sh --fullscreen")),
                );
            } else {
                let exe = text_of(&self.exe, cx);
                let mut r = Row::text("exe", tr!("Executable"), short_path(exe.trim())).icon(Icon::FolderOpen);
                r = match install_folder(&exe) {
                    Some(f) => r.desc(tr!("{exe}\nInstall folder: {folder}", exe, folder = f)),
                    None => r.desc(tr!("Choose the program that starts the game.")),
                };
                rows.push(r);
            }
        }

        rows.push(Row::header(tr!("Art")));
        let finding = self.finding;
        rows.push(
            Row::action("find-art", if finding { tr!("Searching\u{2026}") } else { tr!("Find art for this title") })
                .icon(Icon::TravelExplore)
                .desc(tr!("Looks the title up on Steam, SteamGridDB and Bottles' mirror and fills the empty art."))
                .enabled(!finding && !title.trim().is_empty()),
        );
        rows.push(
            Row::text("art-square", tr!("Cover art"), art_label(&text_of(&self.square, cx)))
                .icon(Icon::GridView)
                .desc(tr!("Tall image of the game. Search the web, choose a file or type an address.")),
        );
        rows.push(
            Row::text("art-hero", tr!("Background art"), art_label(&text_of(&self.hero, cx)))
                .icon(Icon::GridView)
                .desc(tr!("Optional wide background. The cover is used when empty.")),
        );

        if self.platform == SideloadPlatform::Windows && !edit {
            rows.push(Row::header("Wine"));
            let mut share_label = tr!("Use shared Wine prefix").to_string();
            if self.global.get("wineVersion").is_some() {
                share_label = tr!("Use shared Wine prefix ({name})", name = display_wine_name(&wine_of(&self.global).name));
            }
            rows.push(
                Row::toggle("shared", share_label, self.shared_prefix)
                    .desc(tr!("Only if you know what you are doing: games sharing a prefix can break each other's dependencies.")),
            );
            if self.wine.as_ref().is_some_and(|w| w.kind == "crossover") {
                rows.push(Row::info("crossover", "CrossOver", tr!("Choose the bottle in Game settings")));
            } else {
                rows.push(
                    Row::text("prefix", tr!("Wine prefix"), short_path(text_of(&self.prefix, cx).trim()))
                        .icon(Icon::FolderOpen)
                        .desc(text_of(&self.prefix, cx))
                        .enabled(!self.shared_prefix),
                );
            }
            match &self.wine_list {
                None => rows.push(Row::info("wine-wait", tr!("Wine version"), tr!("Loading\u{2026}"))),
                Some(list) if list.is_empty() && self.wine.is_none() => rows
                    .push(Row::info("wine-none", tr!("Wine version"), tr!("None found")).desc(tr!("Download one from the Wine Manager."))),
                Some(_) => {
                    let options = self.wine_choices();
                    let index = options.iter().position(|w| self.wine.as_ref().is_some_and(|c| c.name == w.name)).unwrap_or(0);
                    rows.push(
                        Row::choice(
                            "wine",
                            tr!("Wine version"),
                            options.iter().map(|w| SharedString::from(display_wine_name(&w.name))).collect(),
                            index,
                        )
                        .enabled(!self.shared_prefix),
                    );
                }
            }
        }

        rows.push(Row::header(if edit { tr!("Save") } else { tr!("Add") }));
        if !edit && self.platform == SideloadPlatform::Windows {
            rows.push(
                Row::action("installer", if self.installer_running { tr!("Running setup\u{2026}") } else { tr!("Run installer first") })
                    .icon(Icon::PlayArrow)
                    .desc(tr!("Runs a setup program in the Wine prefix above before the game is added."))
                    .enabled(!busy && !title.trim().is_empty()),
            );
        }
        let (exe, url) = (text_of(&self.exe, cx), normalize_url(&text_of(&self.url, cx)));
        let (folder, command) = (text_of(&self.folder, cx), text_of(&self.command, cx));
        let valid = missing_field(&title, self.platform, self.start(&exe, &folder, &command), &url).is_none();
        let finish = match (edit, self.saving) {
            (false, false) => tr!("Add game"),
            (true, false) => tr!("Save"),
            (_, true) => tr!("Saving\u{2026}"),
        };
        rows.push(Row::action("finish", finish).icon(Icon::Check).enabled(valid && !busy));
        rows.push(Row::action("cancel", tr!("Cancel")).icon(Icon::Close).enabled(!self.saving));
        if edit {
            rows.push(Row::action("remove", tr!("Remove game")).icon(Icon::DeleteForever).danger().enabled(!busy));
        }
        rows
    }

    /// The line under the list: what is wrong, what happened, what runs.
    fn console_status(&self, cx: &App) -> Option<(SharedString, Hsla)> {
        if !self.error.is_empty() {
            return Some((self.error.clone().into(), theme::danger_text().into()));
        }
        if self.finding {
            return Some((tr!("Looking for art\u{2026}").into(), theme::text_muted().into()));
        }
        if self.installer_running {
            return Some((tr!("The setup program is running. This waits until it closes.").into(), theme::text_muted().into()));
        }
        if !self.notice.is_empty() {
            return Some((self.notice.clone().into(), theme::success().into()));
        }
        if self.confirm_remove {
            return None;
        }
        let url = normalize_url(&text_of(&self.url, cx));
        let (exe, folder, command) = (text_of(&self.exe, cx), text_of(&self.folder, cx), text_of(&self.command, cx));
        missing_hint(&text_of(&self.title, cx), self.platform, self.start(&exe, &folder, &command), &url)
            .map(|m| (SharedString::from(m), theme::console_btn_y().into()))
    }

    fn console_art(&self, url: &str, w: f32, h: f32, empty: &'static str, cx: &mut Context<Self>) -> Div {
        let chain = chain_for_url(&self.ctx.data_paths, None, url.trim());
        let src = if chain.is_empty() {
            None
        } else {
            self.images.update(cx, |im, _| im.get(&format!("cag:{url}"), chain, f32::from(u(w)), f32::from(u(h))))
        };
        let none = src.is_none();
        div()
            .relative()
            .flex_none()
            .w(u(w))
            .h(u(h))
            .rounded(u(14.))
            .overflow_hidden()
            .border_1()
            .border_color(theme::white_a(0.1))
            .child(picture(src).size_full())
            .when(none, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(u(14.))
                        .text_color(theme::text_muted())
                        .child(empty),
                )
            })
    }

    pub(super) fn render_console(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows(cx);
        let (square, hero) = (text_of(&self.square, cx), text_of(&self.hero, cx));
        let preview_hero = if hero.trim().is_empty() { square.clone() } else { hero };
        let title = text_of(&self.title, cx);
        let heading = if self.confirm_remove {
            tr!("REMOVE GAME")
        } else {
            match self.mode {
                super::Mode::Add => tr!("ADD GAME"),
                super::Mode::Edit(_) => tr!("EDIT GAME"),
                super::Mode::Install(_) => tr!("INSTALL GAME"),
            }
        };

        let mut left = div().w(u(820.)).max_w_full().flex_none().flex().flex_col().min_h_0().gap(u(8.));
        if self.confirm_remove {
            left = left.child(kit::body_text(tr!(
                "Remove {title} from your library? The game's files are not deleted. Its playtime, status, notes, images and Collection data are removed, and so are its shortcuts.",
                title = if title.trim().is_empty() { tr!("this game") } else { title.trim() }
            )));
        } else if let Some(note) = self.top_note() {
            left = left.child(kit::body_text(note));
        }
        left = left.child(self.console_list.render("console-add-game", &rows, !self.layer.is_open()).flex_1().min_h_0());
        left = left.child(
            div()
                .min_h(u(24.))
                .text_size(u(15.))
                .line_height(u(22.))
                .children(self.console_status(cx).map(|(t, c)| div().text_color(c).child(t))),
        );

        let right = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .items_start()
            .gap(u(14.))
            .child(self.console_art(&square, 200., 300., tr!("No cover"), cx))
            .child(self.console_art(&preview_hero, 300., 122., tr!("No background"), cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(u(2.))
                    .child(kit::title(if title.trim().is_empty() { tr!("New game").to_string() } else { title.trim().to_string() }, 22.))
                    .child(kit::body_text(platform_label(self.platform))),
            );

        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .pb(u(60.))
            .bg(theme::console_bg())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(u(36.))
                    .h(u(76.))
                    .px(u(46.))
                    .child(kit::wordmark(heading))
                    .child(div().flex_1())
                    .child(kit::clock()),
            )
            .child(div().flex().flex_1().min_h_0().gap(u(40.)).px(u(46.)).pb(u(16.)).child(left).child(right))
            .children(self.layer.view())
            .into_any_element()
    }

    fn open_text(&mut self, field: TextField, window: &mut Window, cx: &mut Context<Self>) {
        let (title, initial) = match field {
            TextField::Title => (tr!("Title"), text_of(&self.title, cx)),
            TextField::Url => (tr!("Game URL"), text_of(&self.url, cx)),
            TextField::UserAgent => (tr!("Custom user agent"), text_of(&self.user_agent, cx)),
            TextField::Command => (tr!("Command"), text_of(&self.command, cx)),
            TextField::Art(Slot::Square) => (tr!("Cover art address"), text_of(&self.square, cx)),
            TextField::Art(Slot::Cover) => (tr!("Background art address"), text_of(&self.hero, cx)),
        };
        let osk = cx.new(|_| Osk::new(title, initial.trim()));
        let sub = cx.subscribe_in(&osk, window, move |this, _, ev: &OskEvent, window, cx| {
            if let OskEvent::Done(text) = ev {
                this.apply_text(field, text.trim().to_string(), window, cx);
            }
            this.layer.close();
            cx.notify();
        });
        self.layer.open(&osk, sub);
    }

    fn apply_text(&mut self, field: TextField, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let set = |input: &Entity<gpui_kit::component::input::InputState>, v: String, window: &mut Window, cx: &mut Context<Self>| {
            input.update(cx, |s, cx| s.set_value(v, window, cx));
        };
        match field {
            TextField::Title => {
                let title = self.title.clone();
                set(&title, clean_title(&text), window, cx);
                self.title_changed(window, cx);
                // Heroic looks for an image when the title field loses focus.
                self.auto_find_art(window, cx);
            }
            TextField::Url => set(&self.url.clone(), text, window, cx),
            TextField::UserAgent => set(&self.user_agent.clone(), text, window, cx),
            TextField::Command => set(&self.command.clone(), text, window, cx),
            TextField::Art(slot) => set(&self.art_input(slot), text, window, cx),
        }
    }

    fn art_input(&self, slot: Slot) -> Entity<gpui_kit::component::input::InputState> {
        if slot == Slot::Square {
            self.square.clone()
        } else {
            self.hero.clone()
        }
    }

    fn open_browser(&mut self, what: Pick, window: &mut Window, cx: &mut Context<Self>) {
        let (title, kind, start) = match what {
            Pick::Executable => (tr!("Select the executable"), PickKind::File(Vec::new()), {
                let exe = text_of(&self.exe, cx);
                if exe.trim().is_empty() {
                    text_of(&self.folder, cx)
                } else {
                    exe
                }
            }),
            Pick::Folder => (tr!("Select the game folder"), PickKind::Folder, text_of(&self.folder, cx)),
            Pick::Prefix => (tr!("Select the Wine prefix folder"), PickKind::Folder, text_of(&self.prefix, cx)),
            Pick::Installer => (tr!("Select the setup program to run"), PickKind::File(Vec::new()), String::new()),
            Pick::Art(_) => {
                (tr!("Select an image"), PickKind::File(IMAGE_EXTENSIONS.iter().map(|e| e.to_string()).collect()), String::new())
            }
        };
        let start = (!start.trim().is_empty()).then(|| PathBuf::from(start.trim()));
        let browser = cx.new(|cx| Browser::new(title, kind, start.as_deref(), cx));
        let sub = cx.subscribe_in(&browser, window, move |this, _, ev: &BrowserEvent, window, cx| {
            this.layer.close();
            if let BrowserEvent::Picked(p) = ev {
                let path = p.display().to_string();
                match what {
                    Pick::Executable => {
                        this.exe.update(cx, |s, cx| s.set_value(path.clone(), window, cx));
                        this.executable_changed(&path);
                        this.fill_folder_from(&path, window, cx);
                    }
                    Pick::Folder => this.folder.update(cx, |s, cx| s.set_value(path, window, cx)),
                    Pick::Prefix => this.prefix.update(cx, |s, cx| s.set_value(path, window, cx)),
                    Pick::Installer => this.run_installer(path, cx),
                    Pick::Art(slot) => this.art_input(slot).update(cx, |s, cx| s.set_value(file_url(&path), window, cx)),
                }
            }
            cx.notify();
        });
        self.layer.open(&browser, sub);
    }

    fn open_picker_console(&mut self, slot: Slot, window: &mut Window, cx: &mut Context<Self>) {
        let title = text_of(&self.title, cx);
        let ctx = self.ctx.clone();
        let view = cx.new(|cx| ImagePicker::new(&ctx, &title, slot, window, cx));
        let sub = cx.subscribe_in(&view, window, |this, _, ev: &PickerEvent, window, cx| match ev {
            PickerEvent::Picked(slot, url) => this.art_input(*slot).update(cx, |s, cx| s.set_value(url.clone(), window, cx)),
            PickerEvent::Close => {
                this.layer.close();
                cx.notify();
            }
        });
        self.layer.open(&view, sub);
    }

    /// A on an art row: where the image comes from.
    fn open_art_sheet(&mut self, slot: Slot, window: &mut Window, cx: &mut Context<Self>) {
        let has = !text_of(&self.art_input(slot), cx).trim().is_empty();
        let mut rows = vec![
            Row::action("web", tr!("Search the web")).icon(Icon::FaSearch),
            Row::action("file", tr!("Choose an image file")).icon(Icon::FolderOpen),
            Row::action("url", tr!("Type an address")).icon(Icon::OpenInNew),
        ];
        if has {
            rows.push(Row::action("clear", tr!("Remove the image")).icon(Icon::Close).danger());
        }
        let name = if slot == Slot::Square { tr!("Cover art") } else { tr!("Background art") };
        let sheet = cx.new(|_| Sheet::new(name, None, rows));
        let sub = cx.subscribe_in(&sheet, window, move |this, _, ev: &SheetEvent, window, cx| {
            let SheetEvent::Activate(id) = ev;
            this.layer.close();
            match id.as_ref() {
                "web" => this.open_picker_console(slot, window, cx),
                "file" => this.open_browser(Pick::Art(slot), window, cx),
                "url" => this.open_text(TextField::Art(slot), window, cx),
                "clear" => this.art_input(slot).update(cx, |s, cx| s.set_value("", window, cx)),
                _ => {}
            }
            cx.notify();
        });
        self.layer.open(&sheet, sub);
    }
}

impl PadTarget for AddGameDialog {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        if self.saving {
            return true;
        }
        if self.confirm_remove && pad == Pad::B {
            self.confirm_remove = false;
            self.error.clear();
            cx.notify();
            return true;
        }
        if pad == Pad::X && !self.confirm_remove {
            if !self.finding && !text_of(&self.title, cx).trim().is_empty() {
                self.find_art(window, cx);
            }
            return true;
        }
        let rows = self.console_rows(cx);
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Changed(id, value) => match (id.as_ref(), value) {
                ("platform", Value::Choice { index, .. }) => self.set_platform(PLATFORMS[index.min(PLATFORMS.len() - 1)], cx),
                ("wine", Value::Choice { index, .. }) => self.wine = self.wine_choices().get(index).cloned().or_else(|| self.wine.clone()),
                ("fullscreen", Value::Toggle(on)) => self.fullscreen = on,
                ("start", Value::Choice { index, .. }) => self.by_command = index == 1,
                ("shared", Value::Toggle(on)) => self.set_shared_prefix(on, window, cx),
                ("remove-prefix", Value::Toggle(on)) => self.remove_prefix_too = on,
                _ => {}
            },
            Outcome::Activate(id) => match id.as_ref() {
                "title" => self.open_text(TextField::Title, window, cx),
                "url" => self.open_text(TextField::Url, window, cx),
                "ua" => self.open_text(TextField::UserAgent, window, cx),
                "exe" => self.open_browser(Pick::Executable, window, cx),
                "folder" => self.open_browser(Pick::Folder, window, cx),
                "command" => self.open_text(TextField::Command, window, cx),
                "prefix" => self.open_browser(Pick::Prefix, window, cx),
                "installer" => self.open_browser(Pick::Installer, window, cx),
                "find-art" => self.find_art(window, cx),
                "art-square" => self.open_art_sheet(Slot::Square, window, cx),
                "art-hero" => self.open_art_sheet(Slot::Cover, window, cx),
                "finish" => self.save(cx),
                "cancel" => cx.emit(DialogEvent::Close),
                "remove" => {
                    self.confirm_remove = true;
                    self.error.clear();
                    self.notice.clear();
                    self.console_list.reset();
                }
                "back" => {
                    self.confirm_remove = false;
                    self.error.clear();
                }
                "remove-yes" => self.remove(cx),
                _ => {}
            },
            Outcome::Ignored => return false,
            _ => {}
        }
        cx.notify();
        true
    }

    fn hints(&self, cx: &App) -> Vec<Hint> {
        if let Some(h) = self.layer.hints(cx) {
            return h;
        }
        let rows = self.console_rows(cx);
        let focused = self.console_list.focused(&rows).or_else(|| rows.iter().find(|r| r.value != Value::Header));
        let changes = focused.is_some_and(|r| matches!(r.value, Value::Toggle(_) | Value::Choice { .. }));
        let mut h =
            vec![hint(Btn::A, if focused.is_some_and(|r| matches!(r.value, Value::Text(_))) { tr!("Edit") } else { tr!("Select") })];
        if changes {
            h.push(hint(Btn::LeftRight, tr!("Change")));
        }
        if !self.confirm_remove {
            h.push(hint(Btn::X, tr!("Find art")));
        }
        h.push(hint(Btn::B, if self.confirm_remove { tr!("Back") } else { tr!("Cancel") }));
        h
    }
}
