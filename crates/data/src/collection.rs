//! Pure functions over the loaded games: the filtering, sorting, grouping,
//! facets and related games of `Collection/index.tsx`, `collectionFacets.ts`,
//! `collectionTags.ts`, `relatedGames.ts` and `playtime.ts`.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use heronite_i18n::tr;
use serde::{Deserialize, Serialize};

use crate::library::CollectionGame;
use crate::models::{CollectionGameMetadata, CompletionStatus, GameInfo};
use crate::status::{resolve_status_id, status_color, status_label, UNKNOWN_GROUP_ID};
use crate::text::{facet_key, locale_cmp, locale_cmp_base, split_names, unique_names};

pub type Game = Arc<CollectionGame>;

/// `InstallFilter` (stored as `all` / `installed` / `uninstalled`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallFilter {
    #[default]
    All,
    Installed,
    Uninstalled,
}

impl InstallFilter {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "all" => Some(Self::All),
            "installed" => Some(Self::Installed),
            "uninstalled" => Some(Self::Uninstalled),
            _ => None,
        }
    }

    pub fn matches(self, installed: bool) -> bool {
        match self {
            Self::All => true,
            Self::Installed => installed,
            Self::Uninstalled => !installed,
        }
    }
}

/// `CollectionSort` (stored as `title` / `lastPlayed` / `playtime`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SortMode {
    #[default]
    #[serde(rename = "title")]
    Title,
    #[serde(rename = "lastPlayed")]
    LastPlayed,
    #[serde(rename = "playtime")]
    Playtime,
}

impl SortMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "title" => Some(Self::Title),
            "lastPlayed" => Some(Self::LastPlayed),
            "playtime" => Some(Self::Playtime),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FacetKind {
    Series,
    Developer,
    Publisher,
    Genre,
    Platform,
    /// Where the game comes from (Steam, Epic Games, added manually...);
    /// a game also owned on Steam is under Steam too.
    Store,
}

/// `FACET_KINDS`, in the order the filter menu lists them.
pub const FACET_KINDS: [FacetKind; 6] =
    [FacetKind::Store, FacetKind::Series, FacetKind::Developer, FacetKind::Publisher, FacetKind::Genre, FacetKind::Platform];

impl FacetKind {
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Series => "series",
            Self::Developer => "developer",
            Self::Publisher => "publisher",
            Self::Genre => "genre",
            Self::Platform => "platform",
            Self::Store => "store",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        FACET_KINDS.into_iter().find(|k| k.as_str() == value)
    }
}

/// `CollectionFacetFilters`, with several values per kind: a game matches a
/// kind when it has any of its values (Heronite kept one per kind).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FacetFilters {
    pub series: Vec<String>,
    pub developer: Vec<String>,
    pub publisher: Vec<String>,
    pub genre: Vec<String>,
    pub platform: Vec<String>,
    pub store: Vec<String>,
}

impl FacetFilters {
    pub fn get(&self, kind: FacetKind) -> &[String] {
        match kind {
            FacetKind::Series => &self.series,
            FacetKind::Developer => &self.developer,
            FacetKind::Publisher => &self.publisher,
            FacetKind::Genre => &self.genre,
            FacetKind::Platform => &self.platform,
            FacetKind::Store => &self.store,
        }
    }

    fn slot(&mut self, kind: FacetKind) -> &mut Vec<String> {
        match kind {
            FacetKind::Series => &mut self.series,
            FacetKind::Developer => &mut self.developer,
            FacetKind::Publisher => &mut self.publisher,
            FacetKind::Genre => &mut self.genre,
            FacetKind::Platform => &mut self.platform,
            FacetKind::Store => &mut self.store,
        }
    }

    /// `hasFacetFilters`.
    pub fn is_active(&self) -> bool {
        FACET_KINDS.iter().any(|k| !self.get(*k).is_empty())
    }

    /// Selected values, all kinds together.
    pub fn len(&self) -> usize {
        FACET_KINDS.iter().map(|k| self.get(*k).len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        !self.is_active()
    }

    /// Whether `value` is selected (same [`facet_key`]).
    pub fn contains(&self, kind: FacetKind, value: &str) -> bool {
        let key = facet_key(value);
        self.get(kind).iter().any(|v| facet_key(v) == key)
    }

    /// `toggleFacetFilter`: adds the value, or removes it when selected.
    pub fn toggled(&self, kind: FacetKind, value: &str) -> Self {
        let mut next = self.clone();
        let key = facet_key(value);
        let slot = next.slot(kind);
        let before = slot.len();
        slot.retain(|v| facet_key(v) != key);
        if slot.len() == before && !key.is_empty() {
            slot.push(value.trim().to_owned());
        }
        next
    }

    pub fn clear_kind(&mut self, kind: FacetKind) {
        self.slot(kind).clear();
    }

    /// The same values per kind, in any order or spelling.
    pub fn same_as(&self, other: &Self) -> bool {
        let keys = |f: &Self, k: FacetKind| f.get(k).iter().map(|v| facet_key(v)).collect::<HashSet<_>>();
        FACET_KINDS.iter().all(|k| keys(self, *k) == keys(other, *k))
    }
}

/// One facet value of a game with its normalised key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetItem {
    pub value: String,
    pub key: String,
}

/// Facet values of a game, indexed by [`FacetKind::index`].
pub type GameFacets = [Vec<FacetItem>; 6];

fn items(values: Vec<String>) -> Vec<FacetItem> {
    values.into_iter().map(|value| FacetItem { key: facet_key(&value), value }).collect()
}

/// `gameFacets`: the Collection metadata wins, the store's own info fills the
/// gaps (developer, genres).
pub fn game_facets(game: &GameInfo, metadata: Option<&CollectionGameMetadata>) -> GameFacets {
    let series: Vec<String> =
        metadata.and_then(|m| m.series.as_deref()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).into_iter().collect();
    let developers = match metadata.filter(|m| !m.developers.is_empty()) {
        Some(m) => unique_names(m.developers.iter().map(String::as_str)),
        None => unique_names(split_names(game.developer.as_deref()).iter().map(String::as_str)),
    };
    let publishers = unique_names(metadata.map(|m| m.publishers.as_slice()).unwrap_or(&[]).iter().map(String::as_str));
    let genres = match metadata.filter(|m| !m.genres.is_empty()) {
        Some(m) => unique_names(m.genres.iter().map(String::as_str)),
        None => {
            let from_extra: Vec<String> = game
                .extra
                .as_ref()
                .and_then(|e| e.get("genres"))
                .and_then(|g| g.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            unique_names(from_extra.iter().map(String::as_str))
        }
    };
    let platforms = unique_names(metadata.map(|m| m.platforms.as_slice()).unwrap_or(&[]).iter().map(String::as_str));
    // The stores are known from the overlay (`store_facet`).
    [items(series), items(developers), items(publishers), items(genres), items(platforms), Vec::new()]
}

/// Values of the Store facet: the game's source (a Steam entry, `steam-uri`,
/// is a Steam game whatever imported it), plus Steam for a game also owned
/// there. Brand names stay in English.
pub fn store_facet(source: Option<&str>, runner: &str, steam_entry: bool, also_on_steam: bool) -> Vec<FacetItem> {
    let own = match (source.unwrap_or(""), runner) {
        _ if steam_entry => "Steam",
        ("steam", _) => "Steam",
        ("epic", _) | (_, "legendary") => "Epic Games",
        ("gog", _) | (_, "gog") => "GOG",
        ("amazon", _) | (_, "nile") => "Amazon Games",
        ("xbox", _) => "Xbox",
        (_, "zoom") => "Zoom Platform",
        ("emulator", _) => heronite_i18n::tr!("Emulated"),
        ("other", _) => heronite_i18n::tr!("Other"),
        _ => heronite_i18n::tr!("Added manually"),
    };
    let mut values = vec![own.to_string()];
    if also_on_steam && own != "Steam" {
        values.push("Steam".into());
    }
    items(values)
}

/// `matchesFacets`: every active kind must match one of the game's values
/// (any of the selected ones).
pub fn matches_facets(game: &CollectionGame, filters: &FacetFilters) -> bool {
    if !filters.is_active() {
        return true;
    }
    FACET_KINDS.iter().all(|kind| {
        let selected = filters.get(*kind);
        selected.is_empty()
            || selected.iter().any(|s| {
                let wanted = facet_key(s);
                game.facets[kind.index()].iter().any(|item| item.key == wanted)
            })
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetOption {
    pub value: String,
    pub count: usize,
}

/// Value lists with counts for the filter menu, per kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FacetOptions {
    pub series: Vec<FacetOption>,
    pub developer: Vec<FacetOption>,
    pub publisher: Vec<FacetOption>,
    pub genre: Vec<FacetOption>,
    pub platform: Vec<FacetOption>,
    pub store: Vec<FacetOption>,
}

impl FacetOptions {
    pub fn get(&self, kind: FacetKind) -> &[FacetOption] {
        match kind {
            FacetKind::Series => &self.series,
            FacetKind::Developer => &self.developer,
            FacetKind::Publisher => &self.publisher,
            FacetKind::Genre => &self.genre,
            FacetKind::Platform => &self.platform,
            FacetKind::Store => &self.store,
        }
    }
}

/// `collectFacetOptions`: first spelling wins, sorted with `localeCompare`.
pub fn collect_facet_options(games: &[Game]) -> FacetOptions {
    let mut buckets: [HashMap<&str, (usize, &str)>; 6] = Default::default();
    // Preserve first-seen order only for the spelling; the output is sorted.
    for game in games {
        for kind in FACET_KINDS {
            let bucket = &mut buckets[kind.index()];
            for item in &game.facets[kind.index()] {
                let entry = bucket.entry(item.key.as_str()).or_insert((0, item.value.as_str()));
                entry.0 += 1;
            }
        }
    }
    let finish = |bucket: &HashMap<&str, (usize, &str)>| -> Vec<FacetOption> {
        let mut out: Vec<FacetOption> =
            bucket.values().map(|(count, value)| FacetOption { value: (*value).to_owned(), count: *count }).collect();
        out.sort_by(|a, b| locale_cmp(&a.value, &b.value));
        out
    };
    FacetOptions {
        series: finish(&buckets[0]),
        developer: finish(&buckets[1]),
        publisher: finish(&buckets[2]),
        genre: finish(&buckets[3]),
        platform: finish(&buckets[4]),
        store: finish(&buckets[5]),
    }
}

/// Tag kinds of `collectionTags.ts`.
pub const TAG_KINDS: [&str; 8] = ["series", "developers", "publishers", "genres", "themes", "gameModes", "platforms", "features"];

/// `collectTagOptions`: every known value per tag kind (for the tag editors),
/// sorted; in [`TAG_KINDS`] order.
pub fn collect_tag_options(games: &[Game]) -> Vec<(&'static str, Vec<String>)> {
    let mut buckets: Vec<HashMap<String, String>> = vec![HashMap::new(); TAG_KINDS.len()];
    for game in games {
        let md = game.metadata.as_deref();
        let lists: [Vec<String>; 8] = [
            game.facets[0].iter().map(|i| i.value.clone()).collect(),
            game.facets[1].iter().map(|i| i.value.clone()).collect(),
            game.facets[2].iter().map(|i| i.value.clone()).collect(),
            game.facets[3].iter().map(|i| i.value.clone()).collect(),
            unique_names(md.map(|m| m.themes.as_slice()).unwrap_or(&[]).iter().map(String::as_str)),
            unique_names(md.map(|m| m.game_modes.as_slice()).unwrap_or(&[]).iter().map(String::as_str)),
            unique_names(md.map(|m| m.platforms.as_slice()).unwrap_or(&[]).iter().map(String::as_str)),
            unique_names(md.map(|m| m.features.as_slice()).unwrap_or(&[]).iter().map(String::as_str)),
        ];
        for (bucket, list) in buckets.iter_mut().zip(lists) {
            for value in list {
                bucket.entry(facet_key(&value)).or_insert(value);
            }
        }
    }
    TAG_KINDS
        .iter()
        .zip(buckets)
        .map(|(kind, bucket)| {
            let mut values: Vec<String> = bucket.into_values().collect();
            values.sort_by(|a, b| locale_cmp(a, b));
            (*kind, values)
        })
        .collect()
}

/// The toolbar state that decides which games the list shows.
#[derive(Debug, Clone, Default)]
pub struct ViewQuery {
    pub search: String,
    pub install: InstallFilter,
    pub facets: FacetFilters,
    pub sort: SortMode,
    /// The sort's order turned around (Z-A, least played, played longest ago).
    pub descending: bool,
    pub favourites_only: bool,
    /// Games that run without a connection ([`runs_offline`]).
    pub offline_only: bool,
}

/// Whether the game plays without a connection: the library's
/// `canRunOffline` (set per game for Epic, always on for GOG, Amazon and
/// local programs, off for browser games). Emulated games, which have no
/// such flag, run offline.
pub fn runs_offline(game: &CollectionGame) -> bool {
    match game.game.rest.get("canRunOffline").and_then(serde_json::Value::as_bool) {
        Some(flag) => flag,
        None => game.launch_kind.as_deref() == Some("emulator"),
    }
}

/// Search of the Collection screen: every word of the query must match the
/// title or the store's own title, ignoring case and accents, with the
/// palette's tiers (exact, prefix, word start, inside a word, letters in
/// order). `None` when it does not match; higher is better.
pub fn search_score(game: &CollectionGame, tokens: &[&str]) -> Option<i32> {
    let title = crate::fuzzy::fold(&game.title);
    let store_title = crate::fuzzy::fold(&game.game.title);
    let alt = if store_title == title { "" } else { store_title.as_str() };
    crate::fuzzy::score_entry(tokens, &crate::fuzzy::Haystack { title: &title, alt, keywords: "" })
}

/// The sort comparator of `index.tsx`:
/// - `playtime`: more minutes first; ties by title;
/// - `lastPlayed`: most recent first, games never played last (ties by title);
/// - always: title with `localeCompare`.
///
/// `descending` turns the sort's own key around; ties stay A-Z and games
/// never played stay last.
pub fn compare_games(a: &CollectionGame, b: &CollectionGame, sort: SortMode, descending: bool) -> Ordering {
    let turn = |o: Ordering| if descending { o.reverse() } else { o };
    if sort == SortMode::Playtime {
        let delta = turn(b.playtime_minutes.cmp(&a.playtime_minutes));
        if delta != Ordering::Equal {
            return delta;
        }
    }
    if sort == SortMode::LastPlayed {
        let a_last = a.last_played.as_deref().unwrap_or("");
        let b_last = b.last_played.as_deref().unwrap_or("");
        if !a_last.is_empty() || !b_last.is_empty() {
            if a_last.is_empty() {
                return Ordering::Greater;
            }
            if b_last.is_empty() {
                return Ordering::Less;
            }
            let delta = turn(b_last.cmp(a_last));
            if delta != Ordering::Equal {
                return delta;
            }
        }
    }
    let by_title = locale_cmp(&a.title, &b.title);
    if sort == SortMode::Title {
        turn(by_title)
    } else {
        by_title
    }
}

/// Install filter + search + facets + the "only" switches, then sort
/// (stable).
pub fn filter_games(games: &[Game], query: &ViewQuery) -> Vec<Game> {
    filter_games_where(games, query, &|_| true)
}

/// [`filter_games`] with one more condition from the app (categories, games
/// with an update). While searching, the best matches come first and the
/// sort orders the ties.
pub fn filter_games_where(games: &[Game], query: &ViewQuery, keep: &dyn Fn(&CollectionGame) -> bool) -> Vec<Game> {
    let folded = crate::fuzzy::fold(query.search.trim());
    let tokens = crate::fuzzy::tokens(&folded);
    let mut out: Vec<(i32, Game)> = games
        .iter()
        .filter(|g| {
            query.install.matches(g.installed)
                && (!query.favourites_only || g.favourite)
                && (!query.offline_only || runs_offline(g))
                && matches_facets(g, &query.facets)
                && keep(g)
        })
        .filter_map(|g| {
            let score = if tokens.is_empty() { Some(0) } else { search_score(g, &tokens) };
            score.map(|s| (s, g.clone()))
        })
        .collect();
    out.sort_by(|(sa, a), (sb, b)| sb.cmp(sa).then_with(|| compare_games(a, b, query.sort, query.descending)));
    out.into_iter().map(|(_, g)| g).collect()
}

/// A section of the grouped list.
#[derive(Debug, Clone)]
pub struct StatusGroup {
    /// Status id, or [`UNKNOWN_GROUP_ID`] for the extra "Other" group.
    pub id: String,
    pub name: String,
    /// Status slug; `None` for "Other" (it has no color dot).
    pub slug: Option<String>,
    pub color: Option<&'static str>,
    pub games: Vec<Game>,
}

impl StatusGroup {
    pub fn is_other(&self) -> bool {
        self.id == UNKNOWN_GROUP_ID && self.slug.is_none()
    }
}

/// `grouped` + the render loop of `index.tsx`: one group per status in
/// `statuses` order (empty ones omitted), then "Other" for games whose status
/// id matches no status. Games keep the order of `filtered`. `statuses` must
/// already be in display order ([`crate::status::load_statuses`]).
pub fn group_by_status(filtered: &[Game], statuses: &[CompletionStatus]) -> Vec<StatusGroup> {
    let mut buckets: Vec<Vec<Game>> = vec![Vec::new(); statuses.len()];
    // Same as `new Map(status.id -> [])`: a repeated id shares the last bucket.
    let mut by_id: HashMap<&str, usize> = HashMap::new();
    for (i, s) in statuses.iter().enumerate() {
        by_id.insert(s.id.as_str(), i);
    }
    let mut unknown = Vec::new();
    for game in filtered {
        let id = resolve_status_id(game.status_raw.as_deref(), statuses);
        match by_id.get(id.as_str()) {
            Some(i) => buckets[*i].push(game.clone()),
            None => unknown.push(game.clone()),
        }
    }
    let mut groups = Vec::new();
    for (i, status) in statuses.iter().enumerate() {
        if by_id.get(status.id.as_str()) != Some(&i) || buckets[i].is_empty() {
            continue;
        }
        groups.push(StatusGroup {
            id: status.id.clone(),
            name: status_label(&status.name).to_string(),
            slug: Some(status.slug.clone()),
            color: status_color(&status.slug),
            games: std::mem::take(&mut buckets[i]),
        });
    }
    if !unknown.is_empty() {
        groups.push(StatusGroup { id: UNKNOWN_GROUP_ID.into(), name: tr!("Other").into(), slug: None, color: None, games: unknown });
    }
    groups
}

/// Items per group.
pub const RELATED_MAX_ITEMS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelatedKind {
    Series,
    Developer,
}

#[derive(Debug, Clone)]
pub struct RelatedGroup {
    pub kind: RelatedKind,
    /// Sorted by title (`sensitivity: base`), at most [`RELATED_MAX_ITEMS`].
    pub games: Vec<Game>,
}

fn has_overlap(current_keys: &HashSet<String>, candidates: &[FacetItem]) -> bool {
    !current_keys.is_empty() && candidates.iter().any(|c| current_keys.contains(&c.key))
}

/// `collectRelatedGames` with explicit series/developers of the current game
/// (the Focus passes developers that may come from Steam details).
pub fn related_games(current: &CollectionGame, series: &[String], developers: &[String], games: &[Game]) -> Vec<RelatedGroup> {
    let keys =
        |values: &[String]| -> HashSet<String> { values.iter().map(|v| v.trim()).filter(|v| !v.is_empty()).map(facet_key).collect() };
    let series = keys(series);
    let developers = keys(developers);
    let mut series_items: Vec<Game> = Vec::new();
    let mut developer_items: Vec<Game> = Vec::new();
    for game in games {
        if game.key == current.key {
            continue;
        }
        if has_overlap(&series, &game.facets[FacetKind::Series.index()]) {
            series_items.push(game.clone());
        }
        if has_overlap(&developers, &game.facets[FacetKind::Developer.index()]) {
            developer_items.push(game.clone());
        }
    }
    let finish = |mut list: Vec<Game>| {
        list.sort_by(|a, b| locale_cmp_base(&a.title, &b.title));
        list.truncate(RELATED_MAX_ITEMS);
        list
    };
    let mut groups = Vec::new();
    if !series_items.is_empty() {
        groups.push(RelatedGroup { kind: RelatedKind::Series, games: finish(series_items) });
    }
    if !developer_items.is_empty() {
        groups.push(RelatedGroup { kind: RelatedKind::Developer, games: finish(developer_items) });
    }
    groups
}

/// [`related_games`] using the game's own metadata (no Steam fallback).
pub fn related_games_for(current: &CollectionGame, games: &[Game]) -> Vec<RelatedGroup> {
    let series: Vec<String> = current.details.series.iter().cloned().collect();
    related_games(current, &series, &current.details.developers, games)
}

/// `formatPlaytimeMinutes`: `0m`, `45m`, `2h`, `2h 5m`.
pub fn format_playtime_minutes(minutes: u64) -> String {
    if minutes == 0 {
        return tr!("0m").into();
    }
    let (hours, mins) = (minutes / 60, minutes % 60);
    match (hours, mins) {
        (0, m) => tr!("{m}m", m),
        (h, 0) => tr!("{h}h", h),
        (h, m) => tr!("{h}h {m}m", h, m),
    }
}

/// Label under a card: the formatted playtime, or "Not Played".
pub fn playtime_label(minutes: u64) -> String {
    if minutes > 0 {
        format_playtime_minutes(minutes)
    } else {
        tr!("Not Played").into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests_support::game;
    use crate::status::default_statuses;

    fn g(app: &str, title: &str, f: impl FnOnce(&mut crate::library::tests_support::Spec)) -> Game {
        game(app, title, f)
    }

    #[test]
    fn playtime_formatting() {
        assert_eq!(format_playtime_minutes(0), "0m");
        assert_eq!(format_playtime_minutes(45), "45m");
        assert_eq!(format_playtime_minutes(120), "2h");
        assert_eq!(format_playtime_minutes(125), "2h 5m");
        assert_eq!(playtime_label(0), "Not Played");
        assert_eq!(playtime_label(61), "1h 1m");
    }

    #[test]
    fn sort_modes_follow_the_tie_breaks() {
        let games = vec![
            g("a", "Zelda", |s| {
                s.minutes = 10;
                s.last_played = Some("2026-01-01T00:00:00.000Z".into());
            }),
            g("b", "alpha", |s| {
                s.minutes = 10;
            }),
            g("c", "Beta", |s| {
                s.minutes = 300;
                s.last_played = Some("2026-05-01T00:00:00.000Z".into());
            }),
            g("d", "Delta", |s| {
                s.minutes = 0;
            }),
        ];
        let titles = |sort| {
            let q = ViewQuery { sort, ..Default::default() };
            filter_games(&games, &q).iter().map(|g| g.title.clone()).collect::<Vec<_>>()
        };
        assert_eq!(titles(SortMode::Title), ["alpha", "Beta", "Delta", "Zelda"]);
        // more minutes first; 10m tie goes by title; 0 last
        assert_eq!(titles(SortMode::Playtime), ["Beta", "alpha", "Zelda", "Delta"]);
        // recent first, never played last by title
        assert_eq!(titles(SortMode::LastPlayed), ["Beta", "Zelda", "alpha", "Delta"]);
    }

    #[test]
    fn search_install_and_facet_filters_combine() {
        let games = vec![
            g("a", "Half-Life", |s| {
                s.installed = true;
                s.developers = vec!["Valve".into()];
                s.genres = vec!["Shooter".into()];
            }),
            g("b", "Portal", |s| {
                s.developers = vec!["Valve".into()];
                s.genres = vec!["Puzzle".into()];
            }),
            g("c", "Ōkami", |s| {
                s.installed = true;
                s.developers = vec!["Clover".into()];
            }),
        ];
        let run = |q: ViewQuery| filter_games(&games, &q).iter().map(|g| g.app_name.clone()).collect::<Vec<_>>();
        assert_eq!(run(ViewQuery { search: "  PORT ".into(), ..Default::default() }), ["b"]);
        assert_eq!(run(ViewQuery { install: InstallFilter::Installed, ..Default::default() }), ["a", "c"]);
        assert_eq!(run(ViewQuery { install: InstallFilter::Uninstalled, ..Default::default() }), ["b"]);
        let facets = FacetFilters::default().toggled(FacetKind::Developer, "valve");
        assert_eq!(run(ViewQuery { facets: facets.clone(), ..Default::default() }), ["a", "b"]);
        let both = facets.toggled(FacetKind::Genre, "Puzzle");
        assert_eq!(run(ViewQuery { facets: both.clone(), ..Default::default() }), ["b"]);
        // several values of a kind add up; toggling one again removes it
        let either = both.toggled(FacetKind::Genre, "Shooter");
        assert_eq!(run(ViewQuery { facets: either.clone(), ..Default::default() }), ["a", "b"]);
        assert_eq!(either.len(), 3);
        assert!(either.contains(FacetKind::Genre, " shooter "));
        assert!(either.same_as(&facets.toggled(FacetKind::Genre, "shooter").toggled(FacetKind::Genre, "PUZZLE")));
        assert!(both.toggled(FacetKind::Genre, " puzzle ").genre.is_empty());
        assert!(!FacetFilters::default().is_active());

        let options = collect_facet_options(&games);
        assert_eq!(
            options.developer,
            vec![FacetOption { value: "Clover".into(), count: 1 }, FacetOption { value: "Valve".into(), count: 2 }]
        );
        assert_eq!(options.genre.len(), 2);
        assert!(options.series.is_empty());
    }

    #[test]
    fn descending_turns_the_sort_key_around() {
        let games = vec![
            g("a", "Zelda", |s| {
                s.minutes = 10;
                s.last_played = Some("2026-01-01T00:00:00.000Z".into());
            }),
            g("b", "alpha", |s| s.minutes = 10),
            g("c", "Beta", |s| {
                s.minutes = 300;
                s.last_played = Some("2026-05-01T00:00:00.000Z".into());
            }),
        ];
        let titles = |sort| {
            let q = ViewQuery { sort, descending: true, ..Default::default() };
            filter_games(&games, &q).iter().map(|g| g.title.clone()).collect::<Vec<_>>()
        };
        assert_eq!(titles(SortMode::Title), ["Zelda", "Beta", "alpha"]);
        // least played first; the 10m tie stays A-Z
        assert_eq!(titles(SortMode::Playtime), ["alpha", "Zelda", "Beta"]);
        // played longest ago first; never played still last
        assert_eq!(titles(SortMode::LastPlayed), ["Zelda", "Beta", "alpha"]);
    }

    #[test]
    fn search_forgives_case_accents_and_missing_letters() {
        let games = vec![
            g("w", "The Witcher 3: Wild Hunt", |_| {}),
            g("o", "Ōkami HD", |_| {}),
            g("p", "Pokémon Legends", |_| {}),
            g("h", "Hollow Knight", |s| s.store_title = Some("HK Game of the Year".into())),
            g("x", "Hitman", |_| {}),
        ];
        let run = |q: &str| {
            filter_games(&games, &ViewQuery { search: q.into(), ..Default::default() })
                .iter()
                .map(|g| g.app_name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(run("okami"), ["o"]);
        assert_eq!(run("POKEMON"), ["p"]);
        assert_eq!(run("witcr"), ["w"], "letters in order");
        assert_eq!(run("wild witcher"), ["w"], "every word, any order");
        assert_eq!(run("year"), ["h"], "the store's own title too");
        assert!(run("zzz").is_empty());
        // best match first, whatever the sort says: prefixes (shorter
        // first), then word starts
        assert_eq!(run("h"), ["x", "h", "o", "w"]);
    }

    #[test]
    fn only_switches_and_an_app_condition() {
        let games = vec![
            g("fav", "A", |s| {
                s.favourite = true;
                s.offline = Some(false);
            }),
            g("off", "B", |s| s.offline = Some(true)),
            g("plain", "C", |_| {}),
        ];
        let run = |q: ViewQuery| filter_games(&games, &q).iter().map(|g| g.app_name.clone()).collect::<Vec<_>>();
        assert_eq!(run(ViewQuery { favourites_only: true, ..Default::default() }), ["fav"]);
        assert_eq!(run(ViewQuery { offline_only: true, ..Default::default() }), ["off"]);
        let kept = filter_games_where(&games, &ViewQuery::default(), &|g| g.app_name != "off");
        assert_eq!(kept.iter().map(|g| g.app_name.as_str()).collect::<Vec<_>>(), ["fav", "plain"]);
    }

    #[test]
    fn grouping_uses_status_order_and_an_other_group() {
        let statuses = {
            let mut s = default_statuses();
            crate::status::sort_statuses(&mut s);
            s
        };
        let games = vec![
            g("a", "A", |s| s.status = Some("beaten".into())),
            g("b", "B", |s| s.status = None),
            g("c", "C", |s| s.status = Some("playing".into())),
            g("d", "D", |s| s.status = Some("ghost".into())),
            g("e", "E", |s| s.status = Some("beaten".into())),
        ];
        let groups = group_by_status(&games, &statuses);
        let summary: Vec<_> = groups.iter().map(|g| (g.id.as_str(), g.games.len())).collect();
        assert_eq!(summary, [("beaten", 2), ("playing", 1), ("not-played", 1), ("ungrouped", 1)]);
        assert_eq!(groups[0].color, Some("#f4d35e"));
        assert!(groups[3].is_other());
        assert_eq!(groups[3].name, "Other");
        assert_eq!(groups[0].games[0].app_name, "a");
    }

    #[test]
    fn related_games_share_series_or_developer() {
        let games = vec![
            g("cur", "Portal 2", |s| {
                s.series = Some("Portal".into());
                s.developers = vec!["Valve".into()];
            }),
            g("p1", "portal", |s| {
                s.series = Some("PORTAL".into());
                s.developers = vec!["Valve".into()];
            }),
            g("h1", "Half-Life", |s| s.developers = vec!["Valve".into()]),
            g("o", "Other", |s| s.developers = vec!["Someone".into()]),
        ];
        let groups = related_games_for(&games[0], &games);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].kind, RelatedKind::Series);
        assert_eq!(groups[0].games.iter().map(|g| g.app_name.as_str()).collect::<Vec<_>>(), ["p1"]);
        assert_eq!(groups[1].kind, RelatedKind::Developer);
        // sorted by title: Half-Life before portal
        assert_eq!(groups[1].games.iter().map(|g| g.app_name.as_str()).collect::<Vec<_>>(), ["h1", "p1"]);
    }

    #[test]
    fn related_games_are_capped() {
        let mut games = vec![g("cur", "Cur", |s| s.developers = vec!["Dev".into()])];
        for i in 0..30 {
            games.push(g(&format!("x{i}"), &format!("Game {i:02}"), |s| s.developers = vec!["Dev".into()]));
        }
        let groups = related_games_for(&games[0], &games);
        assert_eq!(groups[0].games.len(), RELATED_MAX_ITEMS);
        assert_eq!(groups[0].games[0].title, "Game 00");
    }

    #[test]
    fn tag_options_are_unique_and_sorted() {
        let games = vec![g("a", "A", |s| s.genres = vec!["rpg".into(), "Action".into()]), g("b", "B", |s| s.genres = vec!["RPG".into()])];
        let tags = collect_tag_options(&games);
        let genres = &tags.iter().find(|(k, _)| *k == "genres").unwrap().1;
        assert_eq!(genres, &["Action", "rpg"]);
    }
}
