//! Builds the Collection view models from `heronite-data`.

use std::sync::Arc;

use gpui_kit::{rgb, Rgba, SharedString};
use heronite_data::collection::{self, RelatedKind};
use heronite_data::focus::focus_data;
use heronite_data::hltb::WikiCache;
use heronite_data::status::display_status;
use heronite_data::ui_state::UiState;
use heronite_data::{CollectionGame, Library, ViewQuery};

use super::vm::*;
use crate::theme;
use heronite_i18n::tr;

fn parse_hex(hex: &str) -> Rgba {
    let h = hex.trim_start_matches('#');
    u32::from_str_radix(h, 16).map(rgb).unwrap_or(rgb(0xe5e7eb))
}

fn card_vm(g: &CollectionGame, playing: bool) -> Arc<CardVm> {
    Arc::new(CardVm {
        key: g.key.clone().into(),
        title: g.title.clone().into(),
        playtime: collection::playtime_label(g.playtime_minutes).into(),
        installed: g.installed,
        playing,
        hidden: g.hidden,
        cover_key: g.key.clone().into(),
    })
}

/// The list's query: the search and the View menu's filters and sort (the
/// app's own conditions are [`CollectionView::keeps_game`](super::CollectionView::keeps_game)).
pub(super) fn view_query(ui: &UiState, search: &str) -> ViewQuery {
    ViewQuery {
        search: search.to_string(),
        install: ui.install_filter,
        facets: ui.facet_filters.clone(),
        sort: ui.sort,
        descending: ui.sort_descending,
        favourites_only: ui.only.favourites,
        offline_only: ui.only.offline,
    }
}

/// `keep`: the app's own conditions (categories, statuses, games with an update).
pub(super) fn build_groups(
    lib: &Library,
    ui: &UiState,
    search: &str,
    keep: &dyn Fn(&CollectionGame) -> bool,
    is_playing: &dyn Fn(&str) -> bool,
) -> Vec<GroupVm> {
    let filtered = collection::filter_games_where(&lib.games, &view_query(ui, search), keep);
    if !ui.group_by_status {
        return vec![GroupVm {
            id: "all".into(),
            name: "".into(),
            color: theme::status_color("custom"),
            cards: filtered.iter().map(|g| card_vm(g, is_playing(&g.key))).collect(),
            collapsed: false,
            show_header: false,
        }];
    }
    collection::group_by_status(&filtered, &lib.statuses)
        .into_iter()
        .map(|sg| GroupVm {
            collapsed: ui.collapsed_groups.contains(&sg.id),
            id: sg.id.clone().into(),
            name: sg.name.clone().into(),
            color: sg.color.map(parse_hex).unwrap_or(theme::status_color("custom")),
            cards: sg.games.iter().map(|g| card_vm(g, is_playing(&g.key))).collect(),
            show_header: true,
        })
        .collect()
}

pub(super) fn focus_vm(lib: &Library, wiki: &WikiCache, g: &CollectionGame, playing: bool) -> FocusVm {
    let fd = focus_data(lib, g, wiki);
    let d = &fd.details;
    let mut rows = Vec::new();
    let text = |s: &str| DetailValue::Text(s.to_string().into());
    let chips = |v: &[String], facet: Option<&'static str>, clamp: bool| DetailValue::Chips {
        values: v.iter().map(|s| SharedString::from(s.clone())).collect(),
        facet,
        clamp,
    };

    if !d.developers.is_empty() {
        rows.push(DetailRow { label: tr!("Developer").into(), value: chips(&d.developers, Some("developer"), false), split: false });
    }
    if !d.publishers.is_empty() {
        rows.push(DetailRow { label: tr!("Publisher").into(), value: chips(&d.publishers, Some("publisher"), false), split: false });
    }
    if let Some(series) = &d.series {
        rows.push(DetailRow {
            label: tr!("Series").into(),
            value: chips(std::slice::from_ref(series), Some("series"), false),
            split: false,
        });
    }
    rows.push(DetailRow { label: tr!("Last Played").into(), value: text(fd.last_played.as_deref().unwrap_or(tr!("Never"))), split: false });
    rows.push(DetailRow { label: tr!("Playtime").into(), value: text(&fd.playtime), split: false });
    if !d.features.is_empty() {
        rows.push(DetailRow {
            label: tr!("Features").into(),
            value: chips(&d.features[..d.features.len().min(8)], None, false),
            split: true,
        });
    }
    if !d.themes.is_empty() {
        rows.push(DetailRow { label: tr!("Themes").into(), value: chips(&d.themes[..d.themes.len().min(8)], None, false), split: true });
    }
    if d.platforms.len() > 1 {
        rows.push(DetailRow {
            label: tr!("Platforms").into(),
            value: chips(&d.platforms[..d.platforms.len().min(8)], None, false),
            split: true,
        });
    }
    if !d.genres.is_empty() {
        rows.push(DetailRow { label: tr!("Tags").into(), value: chips(&d.genres, Some("genre"), true), split: true });
    }
    if let Some(notes) = d.notes.as_ref().filter(|n| !n.is_empty()) {
        rows.push(DetailRow { label: tr!("Notes").into(), value: text(notes), split: true });
    }

    let related = collection::related_games_for(g, &lib.games)
        .into_iter()
        .filter(|r| !r.games.is_empty())
        .map(|r| RelatedGroupVm {
            title: match r.kind {
                RelatedKind::Series => tr!("Same series").into(),
                RelatedKind::Developer => tr!("Same developer").into(),
            },
            games: r.games.iter().map(|x| card_vm(x, false)).collect(),
        })
        .collect();

    let overview = if !d.description.trim().is_empty() { Some(d.description.clone().into()) } else { None };
    let overview_html = d.description_html.as_deref().map(super::html::parse).filter(|b| !b.is_empty()).map(Arc::new);

    FocusVm {
        key: g.key.clone().into(),
        title: g.title.clone().into(),
        meta_line: fd.meta_line.clone().into(),
        cover_key: g.key.clone().into(),
        overview,
        overview_html,
        related,
        details: rows,
        hltb: fd.hltb.as_ref().filter(|h| h.main_story > 0. || h.main_extra > 0.).map(|h| HltbVm {
            main: Some(h.main_story as f32),
            extra: Some(h.main_extra as f32),
            complete: Some(h.completionist as f32),
            played_hours: g.playtime_minutes as f32 / 60.,
        }),
        status: display_status(g.status_raw.as_deref(), &lib.statuses).map(|st| StatusVm {
            id: st.id.clone().into(),
            name: heronite_data::status::status_label(&st.name).to_string().into(),
            color: theme::status_color(&st.slug),
        }),
        score: d.critic_score.map(|s| fmt_number(s).into()),
        library: g.source_label.into(),
        install_size: d.install_size.clone().filter(|s| s != "0").map(Into::into),
        installed: g.installed,
        playing,
        favourite: g.favourite,
        playtime: fd.playtime.clone().into(),
        last_played: fd.last_played.clone().map(Into::into),
    }
}

fn fmt_number(v: f64) -> String {
    if v.fract() == 0. {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}
