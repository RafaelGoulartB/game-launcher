//! What the View menu's search box finds: every filter value of the
//! Collection with its count, matched against the typed text. `kind: text`
//! (the kind's name, in English or translated) narrows it to one kind.

use gpui_kit::Rgba;
use heronite_data::fuzzy::fold;
use heronite_data::{FacetKind, InstallFilter};
use heronite_i18n::tr;

use super::super::header::facet_label;
use super::super::view_filters::Only;
use crate::icons::Icon;

/// One filter the menu can switch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::collection) enum Pick {
    Show(InstallFilter),
    Only(Only),
    /// "Show hidden games".
    Hidden,
    /// A completion status id.
    Status(String),
    /// A category name (`categories::UNCATEGORIZED` included).
    Category(String),
    Facet(FacetKind, String),
}

/// A group of values: the "Only" switches, statuses, categories or a facet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::collection) enum Kind {
    Only,
    Status,
    Category,
    Facet(FacetKind),
}

impl Kind {
    /// In the order the results list them.
    pub const ALL: [Kind; 9] = [
        Kind::Only,
        Kind::Status,
        Kind::Category,
        Kind::Facet(FacetKind::Store),
        Kind::Facet(FacetKind::Genre),
        Kind::Facet(FacetKind::Developer),
        Kind::Facet(FacetKind::Publisher),
        Kind::Facet(FacetKind::Series),
        Kind::Facet(FacetKind::Platform),
    ];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Only => tr!("Only"),
            Kind::Status => tr!("Status"),
            Kind::Category => tr!("Category"),
            Kind::Facet(k) => facet_label(k),
        }
    }

    /// The English name typed before `:` (the label works too).
    fn key(self) -> &'static str {
        match self {
            Kind::Only => "only",
            Kind::Status => "status",
            Kind::Category => "category",
            Kind::Facet(k) => k.as_str(),
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Kind::Only => Icon::Star,
            Kind::Status => Icon::Flag,
            Kind::Category => Icon::Label,
            Kind::Facet(FacetKind::Store) => Icon::Storefront,
            Kind::Facet(FacetKind::Genre) => Icon::Palette,
            Kind::Facet(FacetKind::Developer) => Icon::Build,
            Kind::Facet(FacetKind::Publisher) => Icon::ShoppingBag,
            Kind::Facet(FacetKind::Series) => Icon::Folder,
            Kind::Facet(FacetKind::Platform) => Icon::SportsEsports,
        }
    }
}

/// A value of the menu: what it switches, how it reads and how many games
/// have it (`None` when not worth counting).
#[derive(Clone, Debug)]
pub(in crate::collection) struct Choice {
    pub pick: Pick,
    pub label: String,
    pub count: Option<usize>,
    pub on: bool,
    /// Status colour dot.
    pub color: Option<Rgba>,
}

/// Every value of one kind, in display order.
pub(in crate::collection) struct Section {
    pub kind: Kind,
    pub choices: Vec<Choice>,
}

/// A row of the results: a value, or "every match of this kind" (unscoped
/// searches show a few per kind).
#[derive(Clone, Debug)]
pub(in crate::collection) enum Row {
    Choice(Choice),
    All { kind: Kind, matches: usize },
}

/// Results of one kind.
pub(in crate::collection) struct Found {
    pub kind: Kind,
    pub rows: Vec<Row>,
}

/// Matches shown per kind until the search is narrowed to it.
const PER_KIND: usize = 4;

/// Splits `kind: text`; an unknown prefix is part of the text.
pub(in crate::collection) fn parse_query(raw: &str) -> (Option<Kind>, String) {
    if let Some((head, rest)) = raw.split_once(':') {
        let head = fold(head.trim());
        if let Some(kind) = Kind::ALL.into_iter().find(|k| k.key() == head || fold(k.label()) == head) {
            return (Some(kind), fold(rest.trim()));
        }
    }
    (None, fold(raw.trim()))
}

/// The search box's results, kind by kind. Values that start with the text
/// (or one of its words) come first.
pub(in crate::collection) fn find(sections: Vec<Section>, raw: &str) -> Vec<Found> {
    let (scope, text) = parse_query(raw);
    let mut out = Vec::new();
    for section in sections {
        if scope.is_some_and(|k| k != section.kind) {
            continue;
        }
        let mut hits: Vec<(u8, Choice)> = section
            .choices
            .into_iter()
            .filter_map(|c| {
                let label = fold(&c.label);
                let rank = if text.is_empty() || label.starts_with(&text) {
                    0
                } else if label.split([' ', '-', '/', '(']).any(|w| w.starts_with(&text)) {
                    1
                } else if label.contains(&text) {
                    2
                } else {
                    return None;
                };
                Some((rank, c))
            })
            .collect();
        if hits.is_empty() {
            continue;
        }
        hits.sort_by_key(|(rank, _)| *rank);
        let matches = hits.len();
        let take = if scope.is_some() { matches } else { PER_KIND };
        let mut rows: Vec<Row> = hits.into_iter().take(take).map(|(_, c)| Row::Choice(c)).collect();
        if matches > take {
            rows.push(Row::All { kind: section.kind, matches });
        }
        out.push(Found { kind: section.kind, rows });
    }
    out
}

/// The text that narrows the search to `kind`, keeping what was typed.
pub(in crate::collection) fn scoped_query(kind: Kind, raw: &str) -> String {
    let (_, text) = parse_query(raw);
    format!("{}: {text}", kind.label().to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(pick: Pick, label: &str) -> Choice {
        Choice { pick, label: label.into(), count: Some(1), on: false, color: None }
    }

    fn sections() -> Vec<Section> {
        let genres = ["Adventure", "Role-playing (RPG)", "Shooter", "Simulator", "Strategy", "Sport"];
        vec![
            Section { kind: Kind::Only, choices: vec![choice(Pick::Only(Only::Favourites), "Favourites")] },
            Section { kind: Kind::Status, choices: vec![choice(Pick::Status("playing".into()), "Playing")] },
            Section {
                kind: Kind::Facet(FacetKind::Genre),
                choices: genres.iter().map(|g| choice(Pick::Facet(FacetKind::Genre, g.to_string()), g)).collect(),
            },
        ]
    }

    fn labels(found: &[Found]) -> Vec<String> {
        found
            .iter()
            .flat_map(|f| f.rows.iter())
            .map(|r| match r {
                Row::Choice(c) => c.label.clone(),
                Row::All { matches, .. } => format!("all {matches}"),
            })
            .collect()
    }

    #[test]
    fn scope_prefix_is_the_kind_name() {
        assert_eq!(parse_query("Genre: rpg"), (Some(Kind::Facet(FacetKind::Genre)), "rpg".into()));
        assert_eq!(parse_query("status:"), (Some(Kind::Status), String::new()));
        assert_eq!(parse_query("re: zero"), (None, "re: zero".into()));
    }

    #[test]
    fn matches_rank_word_starts_and_cap_per_kind() {
        assert_eq!(labels(&find(sections(), "rpg")), ["Role-playing (RPG)"]);
        assert_eq!(labels(&find(sections(), "play")), ["Playing", "Role-playing (RPG)"]);
        // six genres have an "r": the one starting with it first, four shown, then "all"
        assert_eq!(labels(&find(sections(), "r")), ["Favourites", "Role-playing (RPG)", "Adventure", "Shooter", "Simulator", "all 6"]);
        assert!(find(sections(), "zzz").is_empty());
    }

    #[test]
    fn a_scoped_search_lists_every_value() {
        let found = find(sections(), "genre:");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].rows.len(), 6);
        assert_eq!(scoped_query(Kind::Facet(FacetKind::Genre), "st"), "genre: st");
    }
}
