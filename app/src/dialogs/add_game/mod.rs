//! "Add game" / "Edit game": Heroic's sideload dialog (`SideloadDialog` +
//! the platform and Wine selectors of `InstallModal`) for the Collection. Adds
//! a native Linux program, a Windows program (Wine/Proton) or a browser game
//! to `sideload_apps/library.json` the way Heroic does, and edits or removes
//! such games later. The file formats live in `heronite_data::sideload`; the
//! pure rules of the form in [`logic`].

mod actions;
mod art;
mod console;
mod debug;
mod form;
mod logic;
pub mod picker;
mod reactions;

use std::path::Path;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;
use heronite_data::game_config::{self, Settings};
use heronite_data::models::GameInfo;
use heronite_data::sideload::{self, SideloadPlatform};
use heronite_data::sideload_uninstall;
use heronite_i18n::tr;
use heronite_launch::SystemProbe;
use heronite_wine::WineInstallation;
use serde_json::Value;

use super::art_images::DialogImages;
use super::settings_panel::logic::wine_of;
use super::{DialogCtx, DialogEvent};
use crate::ui::controls::{action_button, button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{choose_path, new_input};
use logic::*;
use picker::ImagePicker;
#[derive(Clone, Debug)]
pub enum Mode {
    Add,
    Edit(String),
    /// "Install manually" of a game from the library: the Add form (Wine,
    /// prefix, "Run Installer First") with the game's title and art filled in.
    Install(Prefill),
}

/// What [`Mode::Install`] starts with. `app_name`: the sideloaded entry to
/// install into (it keeps its playtime and status); `None` adds a new local
/// game (a game of another store).
#[derive(Clone, Debug, Default)]
pub struct Prefill {
    pub app_name: Option<String>,
    pub title: String,
    pub square: String,
    pub hero: String,
    /// An emulated game becoming a PC game (a native port or a PC version):
    /// same form, its own title and note.
    pub convert: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Select {
    Platform,
    Wine,
}

pub struct AddGameDialog {
    ctx: DialogCtx,
    mode: Mode,
    app_name: String,
    home: String,
    global: Settings,
    title: Entity<InputState>,
    square: Entity<InputState>,
    hero: Entity<InputState>,
    exe: Entity<InputState>,
    /// Linux: the game's folder, and the command run in it when the game is
    /// started by a command (`by_command`) instead of an executable.
    folder: Entity<InputState>,
    command: Entity<InputState>,
    by_command: bool,
    url: Entity<InputState>,
    user_agent: Entity<InputState>,
    prefix: Entity<InputState>,
    bottle: String,
    fullscreen: bool,
    platform: SideloadPlatform,
    /// The user picked the platform; executable detection stops changing it.
    platform_manual: bool,
    open_select: Option<Select>,
    shared_prefix: bool,
    show_wine: bool,
    /// Last prefix this dialog suggested (typing over it stops suggestions).
    last_suggested: String,
    wine_list: Option<Vec<WineInstallation>>,
    wine: Option<WineInstallation>,
    images: Entity<DialogImages>,
    picker: Option<(Entity<ImagePicker>, Subscription)>,
    /// `runAs` the game had when it was opened for editing.
    existing_run_as: Option<String>,
    /// Looking for art on the network.
    finding: bool,
    auto_art_tried: Option<String>,
    saving: bool,
    installer_running: bool,
    /// Edit mode: asking to remove the game.
    confirm_remove: bool,
    remove_prefix_too: bool,
    /// Wine prefix of the edited game when it may be deleted with it.
    removable_prefix: Option<String>,
    /// Why `removable_prefix` is kept anyway: other games use it.
    prefix_shared: Option<String>,
    error: String,
    notice: String,
    /// Console mode: focus of the rows, and the sheet / keyboard / picker on top.
    console_list: crate::console::list::ListNav,
    layer: crate::console::Layer,
    _subs: Vec<Subscription>,
}

impl EventEmitter<DialogEvent> for AddGameDialog {}

pub(super) fn text_of(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

fn extra_str(g: &GameInfo, key: &str) -> String {
    g.rest.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

impl AddGameDialog {
    pub fn new(ctx: DialogCtx, mode: Mode, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let global = game_config::load_global(&ctx.data_paths, &home);
        let edited: Option<Game> = match &mode {
            Mode::Edit(app) => ctx.lib.games.iter().find(|g| &g.app_name == app && g.runner == "sideload").cloned(),
            Mode::Add | Mode::Install(_) => None,
        };
        let app_name = match (&mode, &edited) {
            (Mode::Edit(app), _) => app.clone(),
            (Mode::Install(Prefill { app_name: Some(app), .. }), _) => app.clone(),
            _ => sideload::generate_app_name(),
        };

        // Initial values: empty for a new game, the stored ones when editing.
        let (mut title, mut square, mut hero, mut exe, mut url, mut ua, mut fullscreen) =
            (String::new(), String::new(), String::new(), String::new(), String::new(), String::new(), false);
        let mut platform = SideloadPlatform::Windows;
        let mut existing_run_as = None;
        let (mut folder, mut command) = (String::new(), String::new());
        if let Some(game) = &edited {
            let g = &game.game;
            title = g.title.clone();
            square = g.art_square.clone().unwrap_or_default();
            let cover = g.art_cover.clone().unwrap_or_default();
            hero = if cover != square { cover } else { String::new() };
            exe = g.install.executable.clone().unwrap_or_default();
            url = extra_str(g, "browserUrl");
            ua = extra_str(g, "customUserAgent");
            fullscreen = g.rest.get("launchFullScreen").and_then(Value::as_bool).unwrap_or(false);
            existing_run_as = game.meta.as_ref().and_then(|m| m.run_as.clone());
            folder = g.install.install_path.clone().unwrap_or_default();
            command = extra_str(g, sideload::COMMAND_KEY);
            platform = if !url.is_empty() || g.install.platform.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("browser")) {
                SideloadPlatform::Browser
            } else {
                match heronite_launch::native::effective_platform(g, game.meta.as_deref(), &exe, &SystemProbe) {
                    heronite_launch::native::Platform::Linux => SideloadPlatform::Linux,
                    heronite_launch::native::Platform::Windows => SideloadPlatform::Windows,
                }
            };
        }
        if let Mode::Install(p) = &mode {
            // A port usually runs natively; picking a Windows program switches it.
            if p.convert {
                platform = SideloadPlatform::Linux;
            }
            title = p.title.clone();
            square = p.square.clone();
            hero = if p.hero != p.square { p.hero.clone() } else { String::new() };
        }
        let removable_prefix = edited.as_ref().and_then(|_| {
            if platform != SideloadPlatform::Windows {
                return None;
            }
            let prefix = game_config::load_game(&ctx.data_paths, &app_name, &home).get("winePrefix")?.as_str()?.to_string();
            let shared = global.get("winePrefix").and_then(Value::as_str).filter(|p| !p.is_empty());
            let is_shared = prefix == game_config::shared_wine_prefix(&home) || Some(prefix.as_str()) == shared;
            (!prefix.is_empty() && Path::new(&prefix).exists() && !is_shared).then_some(prefix)
        });
        let prefix_shared = removable_prefix
            .as_deref()
            .and_then(|p| sideload_uninstall::prefix_shared_note(&ctx.data_paths, &ctx.lib, &app_name, Path::new(p), &home));

        let prefix_dir = global.get("defaultWinePrefixDir").and_then(Value::as_str).unwrap_or("").to_string();
        let suggested = suggested_prefix(&prefix_dir, &title, &app_name);

        let mut subs = Vec::new();
        let title_in = new_input(window, cx, &title, tr!("Add a title to your Game/App"));
        let square_in = new_input(window, cx, &square, tr!("Paste an URL of an Image or select one from your computer"));
        let hero_in = new_input(window, cx, &hero, tr!("Paste an URL of an Image or select one from your computer"));
        let exe_in = new_input(window, cx, &exe, tr!("Select Executable"));
        let by_command = !command.is_empty();
        let folder_in = new_input(window, cx, &folder, tr!("The folder the game is in"));
        let command_in = new_input(window, cx, &command, "./start.sh");
        let url_in = new_input(window, cx, &url, tr!("Paste the Game URL here"));
        let ua_in = new_input(window, cx, &ua, tr!("Write a custom user agent here to be used on this browser app/game"));
        let prefix_in = new_input(window, cx, &suggested, "");

        subs.push(cx.subscribe_in(&title_in, window, |this, input, ev: &InputEvent, window, cx| match ev {
            InputEvent::Change => {
                let raw = input.read(cx).value().to_string();
                let clean = clean_title(&raw);
                if clean != raw {
                    input.update(cx, |s, cx| s.set_value(clean, window, cx));
                    return;
                }
                this.title_changed(window, cx);
                cx.notify();
            }
            // Heroic searches an image when the title field loses focus.
            InputEvent::Blur => this.auto_find_art(window, cx),
            _ => {}
        }));
        for input in [&square_in, &hero_in, &url_in, &ua_in, &prefix_in, &folder_in, &command_in] {
            subs.push(cx.subscribe_in(input, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            }));
        }
        subs.push(cx.subscribe_in(&exe_in, window, |this, input, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change | InputEvent::Blur) {
                let path = input.read(cx).value().to_string();
                this.executable_changed(&path);
                cx.notify();
            }
        }));

        let images = cx.new(|cx| DialogImages::new(&ctx, cx));
        cx.observe(&images, |_, _, cx| cx.notify()).detach();

        let wine_global = {
            let w = wine_of(&global);
            (global.get("wineVersion").is_some()).then_some(w)
        };
        let mut this = Self {
            ctx,
            mode,
            app_name,
            home,
            global,
            title: title_in,
            square: square_in,
            hero: hero_in,
            exe: exe_in,
            folder: folder_in,
            command: command_in,
            by_command,
            url: url_in,
            user_agent: ua_in,
            prefix: prefix_in,
            bottle: String::new(),
            fullscreen,
            platform,
            platform_manual: edited.is_some(),
            open_select: None,
            shared_prefix: false,
            show_wine: true,
            last_suggested: suggested,
            wine_list: None,
            wine: wine_global,
            images,
            picker: None,
            existing_run_as,
            finding: false,
            auto_art_tried: None,
            saving: false,
            installer_running: false,
            confirm_remove: false,
            remove_prefix_too: false,
            removable_prefix,
            prefix_shared,
            error: String::new(),
            notice: String::new(),
            console_list: Default::default(),
            layer: Default::default(),
            _subs: subs,
        };
        if !this.is_edit() {
            this.load_wine_list(cx);
        }
        this
    }

    fn is_edit(&self) -> bool {
        matches!(self.mode, Mode::Edit(_))
    }

    /// Text above the form: the store warning when adding, what to do when
    /// installing.
    fn top_note(&self) -> Option<&'static str> {
        match self.mode {
            Mode::Add => Some(tr!(
                "Important! Are you adding a game from Epic, GOG or Amazon? Do NOT use this feature for that. Log into the store from the menu, look for the game in your library and use Install / Import instead."
            )),
            Mode::Edit(_) => None,
            Mode::Install(Prefill { convert: true, .. }) => Some(tr!(
                "Play this game as a PC game (a native port or a PC version) instead of through its emulator: choose how it starts. Its playtime, status and art stay."
            )),
            Mode::Install(_) => Some(tr!(
                "Install this game without its store: use Run Installer First to run its setup program in the prefix below, then select the installed game's executable. Or select the executable of a copy you already have."
            )),
        }
    }

    /// How the game starts, as the form has it now.
    fn start<'a>(&self, exe: &'a str, folder: &'a str, command: &'a str) -> Start<'a> {
        if self.platform == SideloadPlatform::Linux && self.by_command {
            Start::Command { folder, command }
        } else {
            Start::Executable(exe)
        }
    }

    fn dialog_title(&self) -> &'static str {
        match self.mode {
            Mode::Add => tr!("Add game"),
            Mode::Edit(_) => tr!("Edit game"),
            Mode::Install(Prefill { convert: true, .. }) => tr!("Play as a PC game"),
            Mode::Install(_) => tr!("Install game"),
        }
    }
}

impl Render for AddGameDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.images.update(cx, |im, _| im.frame(window));
        if crate::console::active() {
            return self.render_console(window, cx);
        }
        let edit = self.is_edit();
        let entity = cx.entity().downgrade();
        let vw = f32::from(window.viewport_size().width);

        let (title, body, footer) = if self.confirm_remove {
            let footer = div()
                .flex()
                .gap(px(10.))
                .child(action_button(
                    "ag-remove-back",
                    ButtonKind::Ghost,
                    tr!("Back"),
                    !self.saving,
                    cx.listener(|this, _, _, cx| {
                        this.confirm_remove = false;
                        this.error.clear();
                        cx.notify();
                    }),
                ))
                .child(action_button(
                    "ag-remove-yes",
                    ButtonKind::Danger,
                    if self.saving { tr!("Removing\u{2026}") } else { tr!("Remove game") },
                    !self.saving,
                    cx.listener(|this, _, _, cx| this.remove(cx)),
                ));
            (tr!("Remove game"), self.remove_confirmation(cx), footer)
        } else {
            let can_save = !self.saving && !self.installer_running;
            let finish_label = match (edit, self.saving) {
                (false, false) => tr!("Finish"),
                (true, false) => tr!("Save"),
                (_, true) => tr!("Saving\u{2026}"),
            };
            let mut footer = div().flex().items_center().w_full().gap(px(16.));
            if edit {
                footer = footer.child(action_button(
                    "ag-remove",
                    ButtonKind::Glass,
                    tr!("Remove game"),
                    can_save,
                    cx.listener(|this, _, _, cx| {
                        this.confirm_remove = true;
                        this.error.clear();
                        this.notice.clear();
                        cx.notify();
                    }),
                ));
            }
            footer = footer.child(div().flex_1());
            footer = footer.child(action_button(
                "ag-cancel",
                ButtonKind::Ghost,
                tr!("Cancel"),
                !self.saving,
                cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)),
            ));
            if !edit && self.platform == SideloadPlatform::Windows {
                let can_run = can_save && !text_of(&self.title, cx).trim().is_empty();
                footer = footer.child(action_button(
                    "ag-run-installer",
                    ButtonKind::Glass,
                    if self.installer_running { tr!("Running Setup") } else { tr!("Run Installer First") },
                    can_run,
                    cx.listener(|_, _, window, cx| {
                        choose_path(window, cx, false, tr!("Select EXE to Run"), |this: &mut Self, path, _, cx| {
                            this.run_installer(path, cx)
                        });
                    }),
                ));
            }
            footer = footer.child(
                button("ag-finish", ButtonKind::Primary, finish_label)
                    .when(!can_save, |b| b.opacity(0.5))
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            );
            (self.dialog_title(), self.form(window, cx), footer)
        };

        let main = dialog(
            DialogProps {
                id: "add-game".into(),
                title: title.into(),
                width: 860f32.min(vw * 0.96),
                height: super::settings_panel::debug_dialog_height(),
            },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        );
        let nested = self.picker.as_ref().map(|(p, _)| AnyView::from(p.clone()));
        div().absolute().inset_0().child(main).children(nested).into_any_element()
    }
}
