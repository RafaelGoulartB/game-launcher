//! "FPS limit" in the Other tab: one limit and the limiter that applies it
//! (`heronite_launch::fps_limit`). Only the limit shows until one is set;
//! then the limiter (automatic, or picked) with what it means for the game,
//! and the option only that limiter has.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::fps_limit::{self, FpsLimit, Method, Tools};
use serde_json::{json, Value};

use super::parts::{help, rows};
use super::SettingsPanel;
use crate::theme;
use crate::ui::controls::{row, tabs};
use crate::ui::fields::{input_box, SelectOption};

/// A line under the limiter; `.1`: a warning.
pub(super) type Note = (String, bool);

/// Width of the number fields.
const NUMBER_W: f32 = 120.;

impl SettingsPanel {
    pub(super) fn fps(&self) -> FpsLimit {
        FpsLimit::from_map(&self.settings)
    }

    /// What can limit this game on this computer.
    pub(super) fn fps_tools(&self) -> Tools {
        let gamescope = self.gamescope_installed;
        let wine = self.wine_version();
        let wined3d = self.settings.get("enviromentOptions").and_then(Value::as_array).is_some_and(|a| {
            a.iter().any(|e| {
                e.get("key").and_then(Value::as_str) == Some("PROTON_USE_WINED3D") && e.get("value").and_then(Value::as_str) == Some("1")
            })
        });
        let dxvk = match self.facts() {
            _ if self.is_steam() => !self.flag(heronite_steam::options::KEY_WINED3D, false),
            None => true,
            Some(_) if self.native() => false,
            Some(_) => fps_limit::uses_dxvk(&wine.kind, &wine.name, &wine.bin, self.flag("autoInstallDxvk", false), wined3d),
        };
        Tools { gamescope_runs: gamescope && self.gs_flag("enableUpscaling"), gamescope, mangohud: self.mangohud_installed, dxvk }
    }

    /// The limiters the menu offers. A Steam game's launch options name one
    /// limiter, so they have no "Automatic"; a Linux game has no DXVK.
    pub(super) fn fps_methods(&self) -> Vec<Method> {
        Method::ALL
            .into_iter()
            .filter(|m| match m {
                Method::Auto => !self.is_steam(),
                Method::Dxvk => !self.native(),
                _ => true,
            })
            .collect()
    }

    /// The limiter the game gets. Steam games keep the pick (the line is
    /// written as is) except MangoHud inside gamescope.
    pub(super) fn fps_effective(&self, f: &FpsLimit) -> Option<Method> {
        let tools = self.fps_tools();
        if self.is_steam() {
            return Some(match f.method {
                Method::MangoHud if tools.gamescope_runs => Method::Gamescope,
                Method::Auto => tools.auto().unwrap_or(Method::Dxvk),
                m => m,
            });
        }
        tools.resolve(f.method)
    }

    /// Changes the limit and writes all of it, over the old limiters'
    /// settings (cleared, so they never limit on top of it).
    pub(super) fn update_fps(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut FpsLimit)) {
        let old = self.fps();
        let mut f = old.clone();
        change(&mut f);
        if f == old {
            return;
        }
        if self.is_steam() && f.is_on() && f.method == Method::Auto {
            f.method = self.fps_tools().auto().unwrap_or(Method::Dxvk);
        }
        let number = |n: u32| Value::String(if n == 0 { String::new() } else { n.to_string() });
        let mut values: Vec<(&str, Value)> = vec![
            (fps_limit::KEY_LIMIT, number(f.limit)),
            (fps_limit::KEY_METHOD, Value::String(f.method.id().into())),
            (fps_limit::KEY_SMOOTH, Value::Bool(f.smooth)),
            (fps_limit::KEY_NO_FOCUS, number(f.no_focus)),
        ];
        if !self.is_steam() {
            if self.flag(fps_limit::LEGACY_DXVK.0, false) {
                values.push((fps_limit::LEGACY_DXVK.0, Value::Bool(false)));
            }
            if !self.text(fps_limit::LEGACY_MANGOHUD).is_empty() {
                values.push((fps_limit::LEGACY_MANGOHUD, Value::String(String::new())));
            }
            let mut g = self.gamescope();
            let [on, fps, no_focus] = fps_limit::LEGACY_GAMESCOPE;
            if self.gs_flag(on) || !super::value_text(g.get(fps)).is_empty() || !super::value_text(g.get(no_focus)).is_empty() {
                g.insert(on.into(), Value::Bool(false));
                g.insert(fps.into(), json!(""));
                g.insert(no_focus.into(), json!(""));
                values.push(("gamescope", Value::Object(g)));
            }
        }
        self.set_all(values, cx);
    }

    /// The caption and card. `disabled`: a Steam game whose launch options
    /// cannot change now.
    pub(super) fn fps_section(&mut self, disabled: bool, cx: &mut Context<Self>) -> Div {
        let f = self.fps();
        let limit = div().w(px(NUMBER_W)).child(input_box(&self.input("fps.limit"), disabled));
        let mut card =
            rows().child(row(tr!("Maximum FPS"), Some(tr!("The most frames per second the game draws. Empty: no limit.")), limit));
        if f.is_on() {
            let effective = self.fps_effective(&f);
            let notes =
                self.fps_notes(&f, effective).into_iter().map(|(text, warn)| if warn { warning(text) } else { help(&text) }).collect();
            card = card.field_help(self.fps_method_select(&f, effective, disabled, cx), notes);
            match effective {
                Some(Method::MangoHud) => card = card.child(self.fps_pacing_row(&f, disabled, cx)),
                Some(Method::Gamescope) => {
                    let input = div().w(px(NUMBER_W)).child(input_box(&self.input("fps.noFocus"), disabled));
                    card = card.child(row(
                        tr!("In the background"),
                        Some(tr!("A lower limit while the game is not focused, to save power. Empty: the same limit.")),
                        input,
                    ));
                }
                _ => {}
            }
        }
        div().flex().flex_col().gap(px(6.)).child(self.marked_heading(tr!("FPS limit"), "fps", cx)).child(card.card())
    }

    fn fps_method_select(&self, f: &FpsLimit, effective: Option<Method>, disabled: bool, cx: &mut Context<Self>) -> Div {
        let methods = self.fps_methods();
        let tools = self.fps_tools();
        let shown = match f.method {
            Method::Auto => match effective {
                Some(m) => format!("{} \u{b7} {}", tr!("Automatic"), method_name(m)),
                None => tr!("Automatic").to_string(),
            },
            m => method_name(m).to_string(),
        };
        let options = methods
            .iter()
            .map(|m| SelectOption {
                label: match m {
                    Method::Auto => tr!("Automatic").into(),
                    m => method_name(*m).into(),
                },
                selected: *m == f.method,
                dot: None,
                group: None,
                note: match m {
                    Method::Auto => Some(match tools.auto() {
                        Some(a) => method_name(a).into(),
                        None => tr!("nothing available").into(),
                    }),
                    m => self.fps_unavailable(*m, &tools).map(Into::into),
                },
            })
            .collect();
        self.select_menu_row("fps-method", Some(tr!("Limited by")), shown, options, disabled, cx, move |this, i, _, cx| {
            let method = methods[i];
            this.update_fps(cx, |f| f.method = method);
        })
    }

    /// Why `method` cannot limit this game (`None`: it can).
    pub(super) fn fps_unavailable(&self, method: Method, tools: &Tools) -> Option<&'static str> {
        match method {
            Method::MangoHud if tools.gamescope_runs => Some(tr!("not inside gamescope")),
            Method::MangoHud if !tools.mangohud => Some(tr!("not installed")),
            Method::Dxvk if !tools.dxvk => Some(tr!("not with this Wine version")),
            Method::Gamescope if !tools.gamescope => Some(tr!("not installed")),
            _ => None,
        }
    }

    /// What the limiter means for the game, or why the pick is not used.
    /// `.1`: a warning.
    pub(super) fn fps_notes(&self, f: &FpsLimit, effective: Option<Method>) -> Vec<Note> {
        let mut notes = Vec::new();
        let tools = self.fps_tools();
        let picked = method_name(f.method);
        match (f.method, effective) {
            (Method::Auto, _) => {}
            (m, Some(used)) if used != m => {
                let why = self.fps_unavailable(m, &tools).unwrap_or_default();
                notes.push((
                    tr!("{picked} cannot limit this game ({why}), so {used} does.", picked, why, used = method_name(used)).to_string(),
                    true,
                ));
            }
            (m, None) => {
                let why = self.fps_unavailable(m, &tools).unwrap_or_default();
                notes.push((tr!("{picked} cannot limit this game ({why}) and nothing else here can.", picked, why).to_string(), true));
            }
            // A Steam game's pick is written as it is.
            (m, Some(_)) => {
                if let Some(why) = self.fps_unavailable(m, &tools) {
                    notes.push((tr!("{picked}: {why} on this computer. Steam may not start the game.", picked, why).to_string(), true));
                }
            }
        }
        match effective {
            Some(Method::MangoHud) => {
                notes.push((tr!("Works with Vulkan, OpenGL and DirectX games.").to_string(), false));
                if !self.flag("showMangohud", false) {
                    notes.push((tr!("The MangoHud overlay stays hidden (its toggle key still shows it).").to_string(), false));
                }
            }
            Some(Method::Dxvk) => {
                notes.push((tr!("Built into Proton, nothing to install. DirectX games only (9 to 12).").to_string(), false))
            }
            Some(Method::Gamescope) => {
                if f.method == Method::Auto {
                    notes.push((tr!("gamescope already runs this game, so it limits it too.").to_string(), false));
                }
                notes.push((tr!("The most even frame pacing, with input lag close to VSync.").to_string(), false));
                if !tools.gamescope_runs {
                    notes.push((tr!("The game runs inside gamescope.").to_string(), false));
                }
            }
            Some(Method::Auto) => {}
            None if f.method == Method::Auto => {
                notes.push((tr!("Nothing on this computer can limit this game: install MangoHud.").to_string(), true))
            }
            None => {}
        }
        notes
    }

    fn fps_pacing_row(&self, f: &FpsLimit, disabled: bool, cx: &mut Context<Self>) -> Div {
        let this = cx.entity().downgrade();
        let control = tabs("fps-pacing", &[(false, tr!("Lower latency")), (true, tr!("Smoother"))], f.smooth, move |smooth, _, cx| {
            if disabled {
                return;
            }
            this.update(cx, |this, cx| this.update_fps(cx, |f| f.smooth = smooth)).ok();
        });
        row(
            tr!("Frame pacing"),
            Some(tr!("Lower latency: the game reacts sooner. Smoother: more even frame times, for a little more latency.")),
            control,
        )
        .when(disabled, |d| d.opacity(0.4))
    }
}

pub(super) fn method_name(m: Method) -> &'static str {
    match m {
        Method::Auto => tr!("Automatic"),
        Method::MangoHud => "MangoHud",
        Method::Dxvk => "DXVK / VKD3D-Proton",
        Method::Gamescope => "gamescope",
    }
}

fn warning(text: impl Into<SharedString>) -> Div {
    div().text_size(px(12.)).line_height(px(16.)).text_color(theme::accent()).child(text.into())
}
