//! Settings → Metadata: IGDB credentials, SteamGridDB key, background fill,
//! image copy, per-field source priority, bulk download.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::game_config;
use heronite_data::models::MetadataSettings;
use heronite_data::settings::{self, default_field_priority, EffectiveSettings, METADATA_FIELDS};
use heronite_i18n::{n_, tr};

use super::SettingsPage;
use crate::dialogs::metadata::field_label as metadata_field_label;
use crate::dialogs::metadata_wizard::hub;
use crate::icons::Icon;
use crate::pages::PageCtx;
use crate::theme;
use crate::ui::controls::{action_button, caption, card, card_box, toggle_row, ButtonKind};
use crate::ui::fields::{input_box, labeled, meta_text, new_input, on_commit, select_field};
use crate::ui::icon;

pub struct State {
    client_id: Entity<InputState>,
    secret: Entity<InputState>,
    sgdb_key: Entity<InputState>,
    testing: bool,
    ok_message: Option<SharedString>,
    advanced_open: bool,
    open_select: Option<&'static str>,
}

// "playnite" stays in the saved priorities (kept from Heronite's data) but is
// not offered: nothing feeds that source here.
const SOURCES: [(&str, &str); 5] =
    [("auto", n_!("Auto (IGDB first)")), ("igdb", "IGDB"), ("steam", "Steam"), ("store", n_!("Store")), ("lutris", "Lutris")];

impl State {
    pub fn new(ctx: &PageCtx, s: &EffectiveSettings, window: &mut Window, cx: &mut Context<SettingsPage>) -> Self {
        let client_id = new_input(window, cx, &s.metadata.igdb_client_id, "");
        on_commit(&client_id, window, cx, |d: &mut SettingsPage, value, _, cx| {
            if value != d.settings.metadata.igdb_client_id {
                persist(d, cx, |m| m.igdb_client_id = value);
            }
        });
        let secret = cx.new(|cx| {
            let mut st = InputState::new(window, cx).masked(true);
            st.set_value(s.metadata.igdb_client_secret.clone(), window, cx);
            st
        });
        on_commit(&secret, window, cx, |d: &mut SettingsPage, value, _, cx| {
            if value != d.settings.metadata.igdb_client_secret {
                persist(d, cx, |m| m.igdb_client_secret = value);
            }
        });
        let saved_key = heronite_metadata::covers::steamgrid_api_key(&ctx.data_paths).unwrap_or_default();
        let sgdb_key = cx.new(|cx| {
            let mut st = InputState::new(window, cx).masked(true);
            st.set_value(saved_key, window, cx);
            st
        });
        on_commit(&sgdb_key, window, cx, |d: &mut SettingsPage, value, _, cx| save_sgdb_key(d, value, cx));
        Self { client_id, secret, sgdb_key, testing: false, ok_message: None, advanced_open: false, open_select: None }
    }
}

/// `persistMetadata`: replaces the metadata section with the edited copy.
fn persist(d: &mut SettingsPage, cx: &mut Context<SettingsPage>, edit: impl FnOnce(&mut MetadataSettings)) {
    let mut m = d.settings.metadata.clone();
    edit(&mut m);
    d.metadata.ok_message = None;
    let res = settings::save_metadata(&d.ctx.data_paths, &m);
    d.saved(res, cx);
}

/// `steamGridDbApiKey` of the global settings: adds SteamGridDB to the cover
/// and background search.
fn save_sgdb_key(d: &mut SettingsPage, value: String, cx: &mut Context<SettingsPage>) {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let value = value.trim().to_string();
    d.metadata.ok_message = None;
    d.error = game_config::set_global_setting(&d.ctx.data_paths, &home, "steamGridDbApiKey", value.into())
        .err()
        .map(|err| format!("{err:#}").into());
    cx.notify();
}

fn preferred_source(d: &SettingsPage, field: &str) -> String {
    let default = default_field_priority(field);
    match d.settings.metadata.field_priority.get(field) {
        Some(cur) if *cur != default => cur.first().cloned().unwrap_or_else(|| "auto".into()),
        _ => "auto".into(),
    }
}

fn test_igdb(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    if d.metadata.testing {
        return;
    }
    let id = d.metadata.client_id.read(cx).value().trim().to_string();
    let secret = d.metadata.secret.read(cx).value().to_string();
    if id != d.settings.metadata.igdb_client_id || secret != d.settings.metadata.igdb_client_secret {
        persist(d, cx, |m| {
            m.igdb_client_id = id;
            m.igdb_client_secret = secret;
        });
    }
    d.metadata.testing = true;
    d.metadata.ok_message = None;
    d.error = None;
    let service = hub(cx).read(cx).service.clone();
    cx.spawn(async move |this, cx| {
        let res = cx.background_spawn(async move { service.test_igdb_credentials() }).await;
        this.update(cx, |d, cx| {
            d.metadata.testing = false;
            if res.ok {
                d.metadata.ok_message = Some(tr!("IGDB credentials work.").into());
            } else {
                d.error = Some(res.error.unwrap_or_else(|| tr!("IGDB test failed.").into()).into());
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
    cx.notify();
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let m = &d.settings.metadata;
    let (auto_fill, download) = (m.auto_fill_missing, m.download_images);
    let testing = d.metadata.testing;

    let creds = card_box()
        .py(px(12.))
        .gap(px(12.))
        .child(labeled(tr!("Twitch / IGDB Client ID"), input_box(&d.metadata.client_id, false)))
        .child(labeled(tr!("Twitch / IGDB Client Secret"), input_box(&d.metadata.secret, false)));
    let actions = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .mt(px(12.))
        .child(action_button("md-test", ButtonKind::Glass, tr!("Test IGDB"), !testing, cx.listener(|d, _, _, cx| test_igdb(d, cx))))
        .child(action_button(
            "md-wizard",
            ButtonKind::Primary,
            tr!("Download metadata\u{2026}"),
            true,
            cx.listener(|_, _, _, cx| hub(cx).update(cx, |h, cx| h.request_wizard(cx))),
        ))
        .children(d.metadata.ok_message.clone().map(|t| div().ml(px(6.)).text_size(px(13.)).text_color(theme::success()).child(t)));
    let images = card([
        toggle_row("md-autofill", tr!("Fill missing metadata in the background"), None, auto_fill)
            .on_click(cx.listener(move |d, _, _, cx| persist(d, cx, |m| m.auto_fill_missing = !auto_fill)))
            .into_any_element(),
        toggle_row("md-download", tr!("Copy covers to disk"), Some(tr!("Off keeps the CDN URLs instead of storing the images.")), download)
            .on_click(cx.listener(move |d, _, _, cx| persist(d, cx, |m| m.download_images = !download)))
            .into_any_element(),
    ]);

    let mut col = div()
        .flex()
        .flex_col()
        .child(caption(tr!("Twitch / IGDB API")))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(tr!("Create a Twitch app to get IGDB keys: https://dev.twitch.tv/console"))))
        .child(creds)
        .child(actions)
        .child(caption("SteamGridDB"))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(tr!(
            "Optional. Adds SteamGridDB covers and backgrounds to the image search. Get a key at https://www.steamgriddb.com/profile/preferences/api"
        ))))
        .child(card_box().py(px(12.)).child(labeled(tr!("SteamGridDB API key"), input_box(&d.metadata.sgdb_key, false))))
        .child(caption(tr!("Images & fields")))
        .child(images);

    // Field priority (advanced), collapsible.
    let open = d.metadata.advanced_open;
    col = col.child(
        div()
            .id("md-advanced")
            .flex()
            .items_center()
            .gap(px(6.))
            .mt(px(14.))
            .mx(px(6.))
            .mb(px(6.))
            .cursor_pointer()
            .on_click(cx.listener(|d, _, _, cx| {
                d.metadata.advanced_open = !d.metadata.advanced_open;
                d.metadata.open_select = None;
                cx.notify();
            }))
            .child(icon(if open { Icon::ExpandMore } else { Icon::ChevronRight }, px(16.), theme::text_muted()))
            .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(tr!("Field priority (advanced)"))),
    );
    if open {
        col = col
            .child(div().px(px(6.)).pb(px(8.)).child(meta_text(tr!("Preferred source per field. Other sources still fill empty values."))));
        let mut grid = card_box().py(px(12.)).flex_row().flex_wrap().gap_x(px(12.)).gap_y(px(10.));
        for field in METADATA_FIELDS {
            let current = preferred_source(d, field);
            let label = SOURCES.iter().find(|(v, _)| *v == current).map(|(_, l)| tr(l)).unwrap_or(tr!("Auto (IGDB first)"));
            let is_open = d.metadata.open_select == Some(field);
            let options: Vec<(SharedString, bool)> = SOURCES.iter().map(|(v, l)| (SharedString::from(tr(l)), *v == current)).collect();
            grid = grid.child(div().w(px(200.)).child(select_field(
                &format!("md-prio-{field}"),
                Some(metadata_field_label(field)),
                label,
                false,
                is_open,
                options,
                cx,
                move |d: &mut SettingsPage, _, cx| {
                    d.metadata.open_select = if d.metadata.open_select == Some(field) { None } else { Some(field) };
                    cx.notify();
                },
                move |d: &mut SettingsPage, i, _, cx| {
                    let value = SOURCES[i].0;
                    d.metadata.open_select = None;
                    let default = default_field_priority(field);
                    let next: Vec<String> = if value == "auto" {
                        default
                    } else {
                        std::iter::once(value.to_string()).chain(default.into_iter().filter(|s| s != value)).collect()
                    };
                    persist(d, cx, |m| {
                        m.field_priority.insert(field.to_string(), next);
                    });
                },
            )));
        }
        col = col.child(grid);
    }
    col.into_any_element()
}

/// Console mode rows.
pub(super) fn console_rows(d: &SettingsPage, cx: &App) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    let m = &d.settings.metadata;
    let secret = d.metadata.secret.read(cx).value().to_string();
    let sgdb_key = d.metadata.sgdb_key.read(cx).value().to_string();
    let mut rows = vec![
        Row::header(tr!("Twitch / IGDB API")),
        Row::text("client-id", tr!("Client ID"), d.metadata.client_id.read(cx).value().to_string())
            .desc(tr!("Create a Twitch app to get IGDB keys: dev.twitch.tv/console")),
        Row::text("secret", tr!("Client secret"), "\u{2022}".repeat(secret.chars().count().min(24))),
        Row::action("test", if d.metadata.testing { tr!("Testing\u{2026}") } else { tr!("Test IGDB") })
            .enabled(!d.metadata.testing)
            .desc(d.metadata.ok_message.clone().unwrap_or_default()),
        Row::header("SteamGridDB"),
        Row::text("sgdb-key", tr!("SteamGridDB API key"), "\u{2022}".repeat(sgdb_key.chars().count().min(24)))
            .desc(tr!("Optional. Get a key at steamgriddb.com/profile/preferences/api")),
        Row::header(tr!("Images and fields")),
        Row::toggle("autofill", tr!("Fill missing metadata in the background"), m.auto_fill_missing),
        Row::toggle("download", tr!("Copy covers to disk"), m.download_images).desc(tr!("Off keeps the CDN addresses instead")),
        Row::header(tr!("Preferred source per field")),
    ];
    for field in METADATA_FIELDS {
        let current = preferred_source(d, field);
        let i = SOURCES.iter().position(|(v, _)| *v == current).unwrap_or(0);
        rows.push(Row::choice(
            SharedString::from(format!("prio:{field}")),
            metadata_field_label(field),
            SOURCES.iter().map(|(_, l)| SharedString::from(tr(l))).collect(),
            i,
        ));
    }
    rows
}

pub(super) fn console_apply(
    d: &mut SettingsPage,
    outcome: crate::console::list::Outcome,
    window: &mut Window,
    cx: &mut Context<SettingsPage>,
) {
    use crate::console::list::{Outcome, Value};
    match outcome {
        Outcome::Activate(id) => match id.as_ref() {
            "client-id" => {
                let cur = d.metadata.client_id.read(cx).value().to_string();
                d.console_osk(tr!("IGDB client ID"), &cur, window, cx, |d, text, window, cx| {
                    let text = text.trim().to_string();
                    d.metadata.client_id.update(cx, |s, cx| s.set_value(text.clone(), window, cx));
                    persist(d, cx, |m| m.igdb_client_id = text);
                });
            }
            "secret" => d.console_osk(tr!("IGDB client secret"), "", window, cx, |d, text, window, cx| {
                let text = text.trim().to_string();
                d.metadata.secret.update(cx, |s, cx| s.set_value(text.clone(), window, cx));
                persist(d, cx, |m| m.igdb_client_secret = text);
            }),
            "sgdb-key" => d.console_osk(tr!("SteamGridDB API key"), "", window, cx, |d, text, window, cx| {
                let text = text.trim().to_string();
                d.metadata.sgdb_key.update(cx, |s, cx| s.set_value(text.clone(), window, cx));
                save_sgdb_key(d, text, cx);
            }),
            "test" => test_igdb(d, cx),
            _ => {}
        },
        Outcome::Changed(id, Value::Toggle(on)) => match id.as_ref() {
            "autofill" => persist(d, cx, |m| m.auto_fill_missing = on),
            "download" => persist(d, cx, |m| m.download_images = on),
            _ => {}
        },
        Outcome::Changed(id, Value::Choice { index, .. }) => {
            if let Some(field) = id.strip_prefix("prio:").and_then(|f| METADATA_FIELDS.iter().find(|x| **x == f)).copied() {
                let value = SOURCES[index].0;
                let default = default_field_priority(field);
                let next: Vec<String> = if value == "auto" {
                    default
                } else {
                    std::iter::once(value.to_string()).chain(default.into_iter().filter(|s| s != value)).collect()
                };
                persist(d, cx, |m| {
                    m.field_priority.insert(field.to_string(), next);
                });
            }
        }
        _ => {}
    }
}
