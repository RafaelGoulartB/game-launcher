//! Loading the library off the UI thread and applying it to the view.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::hltb::WikiCache;
use heronite_data::{art, ArtContext, DataPaths, Library};
use heronite_i18n::tr;
use heronite_images::ImageRef;

use super::{stage, CollectionView};
use crate::images::to_chain;

/// Everything loaded from disk off the UI thread.
struct Loaded {
    lib: Library,
    wiki: WikiCache,
    categories: heronite_data::categories::Categories,
    actions: HashMap<String, Vec<heronite_data::game_actions::Action>>,
    covers: HashMap<SharedString, Vec<ImageRef>>,
    stages: HashMap<SharedString, Vec<ImageRef>>,
}

fn load_all(data_paths: &DataPaths, include_hidden: bool) -> anyhow::Result<Loaded> {
    let t = Instant::now();
    let lib = Library::load_with(data_paths, include_hidden)?;
    let wiki = WikiCache::load(data_paths);
    let categories = heronite_data::categories::Categories::load(data_paths);
    let actions = heronite_data::game_actions::load(data_paths);
    let shots = (!lib.settings.screenshots.folder.is_empty()).then(|| std::path::PathBuf::from(&lib.settings.screenshots.folder));
    let ctx = ArtContext::new(data_paths, shots.as_deref());
    let mut covers = HashMap::new();
    let mut stages = HashMap::new();
    for g in &lib.games {
        let key: SharedString = g.key.clone().into();
        covers.insert(key.clone(), to_chain(&art::card_cover(&ctx, g)));
        stages.insert(key, to_chain(&art::stage_art(&ctx, g)));
    }
    log::info!("library: {} games loaded in {:?}", lib.games.len(), t.elapsed());
    Ok(Loaded { lib, wiki, categories, actions, covers, stages })
}

impl CollectionView {
    pub(super) fn spawn_load(&self, cx: &mut Context<Self>) {
        let data_paths = self.data_paths.clone();
        let include_hidden = self.ui.show_hidden;
        cx.spawn(async move |this, cx| {
            let loaded = cx.background_spawn(async move { load_all(&data_paths, include_hidden) }).await;
            this.update(cx, |this, cx| {
                this.refreshing = false;
                match loaded {
                    Ok(l) => {
                        this.welcome.load_error = None;
                        this.apply_loaded(l, cx);
                        this.nav_library_loaded(cx);
                    }
                    Err(err) => {
                        log::error!("loading library: {err:#}");
                        if this.lib.is_some() {
                            this.show_toast(tr!("Could not reload the library: {err}", err = format!("{err:#}")), true, cx);
                        } else {
                            this.welcome.load_error = Some(format!("{err:#}"));
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        self.images.clear_failed();
        self.spawn_load(cx);
        cx.notify();
    }

    fn apply_loaded(&mut self, l: Loaded, cx: &mut Context<Self>) {
        let first = self.lib.is_none();
        let s = &l.lib.settings;
        let bg = &s.background;
        self.scrim = stage::Scrim { dim: bg.dimming.get() as f32 / 100., scrim: bg.scrim.get() as f32 / 100. };
        self.blur_px = bg.blur.get() as f32;
        self.saturation = bg.saturation.get() as f32 / 100.;
        self.stage_enabled = bg.enabled;
        self.grey_uninstalled = s.grey_uninstalled_games && !s.all_tiles_in_color;
        // Textures are keyed by game, not by image: a game whose art changed
        // (custom cover/background, metadata) must load it again.
        let mut stale: Vec<String> = Vec::new();
        for (key, chain) in &self.cover_chains {
            if l.covers.get(key) != Some(chain) {
                stale.push(format!("c:{key}:"));
            }
        }
        for (key, chain) in &self.stage_chains {
            if l.stages.get(key) != Some(chain) {
                stale.extend([format!("s:{key}:"), format!("g:{key}:")]);
            }
        }
        if !stale.is_empty() {
            self.images.invalidate(&stale);
        }
        self.cover_chains = l.covers;
        self.stage_chains = l.stages;
        self.wiki = Arc::new(l.wiki);
        self.categories = l.categories;
        self.user_actions = l.actions;
        self.prune_category_filter();
        if !first {
            self.refresh_launch_options(cx);
        }
        // The first games of a new setup (Steam import) get a selection too.
        let before = self.lib.as_ref().map_or(0, |lib| lib.games.len());
        let was_empty = before == 0;
        self.lib = Some(Arc::new(l.lib));
        if let Some(lib) = self.lib.clone() {
            self.prune_status_filter(&lib);
            self.seed_default_views(&lib);
        }
        if self.lib.as_ref().is_some_and(|lib| lib.games.is_empty()) {
            self.start_welcome_watch(cx);
        }
        // New games (a Steam import, an added game, a ROM scan) get their
        // metadata now; the first load leaves it to the idle fill.
        if !first && self.lib.as_ref().is_some_and(|lib| lib.games.len() > before) {
            self.fill_metadata_now(cx);
        }
        // Rebuilding the rows resets the list to the top; a reload (a dialog
        // saved, the files changed on disk) keeps the user's place.
        let scroll = (!first).then(|| self.list_state.logical_scroll_top());
        self.refresh_groups();
        if let (Some(top), Some(last)) = (scroll, self.items.len().checked_sub(1)) {
            self.list_state.scroll_to(ListOffset { item_ix: top.item_ix.min(last), offset_in_item: top.offset_in_item });
        }
        let focused = if first {
            crate::env::value("HERONITE_DEBUG_SELECT").or_else(|| self.ui.focused_game.clone())
        } else {
            self.selected.as_ref().map(|s| s.to_string())
        };
        // The remembered game, else (first start, a removed game) the first
        // card, so the panel on the right is never empty when there are games.
        let exists = |k: &String| self.lib.as_ref().is_some_and(|lib| lib.get(k).is_some());
        let focused = focused.filter(exists).or_else(|| {
            (first || was_empty).then(|| self.groups.iter().flat_map(|g| g.cards.iter()).next().map(|c| c.key.to_string())).flatten()
        });
        if let Some(key) = focused {
            self.select(key.into(), false);
            if first {
                self.reveal_selected();
            }
        }
        if first {
            log::info!("library on screen after {:?}", self.started.elapsed());
            self.apply_debug_env();
            self.maybe_open_onboarding(cx);
            crate::trim_heap_later();
        }
    }
}
