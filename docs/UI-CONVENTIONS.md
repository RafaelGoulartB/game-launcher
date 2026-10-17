# UI conventions (app/)

The Collection screen (list, cards, header bar, focus panel, stage) keeps the
look of the Electron Heronite Collection
([`src/frontend/screens/LocalLibrary`](https://github.com/RafaelGoulartB/HeroicGamesLauncher/tree/master/src/frontend/screens/LocalLibrary)).
Everything that floats over or replaces it (menus, dropdowns, context menus,
dialogs, drawers, pages, settings) follows the **Glass design system** below.

## Stack
- GPUI through `gpui-kit = 0.7.1` (`use gpui_kit::*; use gpui_kit::prelude::*;`).
  Sources: `~/.cargo/registry/src/*/gpui-pre-0.3.8`, `gpui-component-0.7.1`,
  `gpui-base-0.7.1`. Read them when unsure of an API.
- Reuse from gpui-component only behaviour-heavy widgets: `input::{Input, InputState}`
  (text fields), `slider::{Slider, SliderState}`. Style everything else by hand.

## Layout of the code
- `app/src/theme/` — tokens (`color`, `font`, `layout`, `glass`: nord-dark colours, fonts, status colours,
  `W_BOLD` = CSS 600/700, `W_500` = CSS 500, and the glass tokens: fills,
  borders, radii `R_*`, text colours, toggle / button / segmented / input
  values). Never hard-code a new colour twice; add it here.
- `app/src/ui/` — building blocks: `icon`, `tool_button`, `round_button`,
  `chip`, `scrollbar`, `dropdown::{dropdown, menu_panel, menu_item,
  menu_separator}`, `dialog::{dialog, drawer}`, `glass::*`,
  `controls::*` (see "Glass design system").
- `app/src/ui/fields/` — the ONE set of form helpers; import from
  `crate::ui::fields` only (never write a local label/input/select look):
  - inputs: `new_input` / `new_textarea` (state), `input_box(&input, disabled)`,
    `textarea_box`, `text_field(label, &input)`, `path_field` (clear/browse
    icon) + `choose_path` (file chooser), `search_box`, `on_commit`
    (blur/Enter commit, for debounced or deferred saves);
  - `labeled(label, control)` for any control under a label (an input next
    to a button, a select...), `field_label`, `paragraph`,
    `meta_text`, `panel`;
  - `select_field(id, label, value, disabled, open, options, cx, on_toggle,
    on_pick)`: label + 40px select + its open menu (the caller keeps the
    `open` flag and the picked value);
  - `TagField` (entity: chips + autocomplete, emits `TagEvent::Changed`).
  Inputs/selects/tags share `input_surface` (glass input: 40px, radius 12,
  `rgba(0,0,0,.25)` fill, 1px border, 14px body font; accent ring at 60% while
  focused).
- `app/src/collection/` — the main screen (`mod.rs` = `CollectionView`).
- `app/src/dialogs/<name>.rs` — one dialog per file. A dialog is an
  `Entity<V>` where `V: Render + EventEmitter<DialogEvent>`; build it with
  `DialogCtx` (paths + `Arc<Library>`). Open it from `CollectionView` with
  `self.open_dialog(cx.new(|cx| V::new(ctx, window, cx)), cx)`.
  Emit `DialogEvent::Changed` after writing data, `Close` to close.
- Settings: there is ONE place, the ☰ Settings page (`app/src/pages/settings/`, Ctrl ,). The left nav is grouped under captions
  (`Section::GROUPS`): App (General, Appearance, Advanced), Games (Game defaults, Downloads, Library),
  Collection (Metadata, Screenshots, Backups), Help (Logs, About). A section = title + lead (`Section::blurb`) +
  captioned cards of rows. The Collection sections live in `app/src/pages/settings/<section>.rs`
  (`appearance`, `metadata`, `screenshots`, `saves` + `backup` for Backups) with `State::new(&PageCtx, &EffectiveSettings, …)`
  and `render(&mut SettingsPage, …)`; save with `heronite_data::settings::*` and call `d.saved(result, cx)`
  (updates the page copy of `settings.json` and emits `PageEvent::LibraryChanged`). Work that spawns a process or
  reads folders (Ludusavi detection, screenshot cache size) starts in `SettingsPage::on_section_shown`, never at page
  creation. Open a section from code with `CollectionView::open_settings_section` (or `pages::settings::request_section`
  + navigate); add new sections to `Section` (label, icon, blurb, palette `keywords`, `parse`). Emulators and ROM
  folders are NOT a Settings section: they live on the ☰ Emulators page.
- Command palette (Ctrl K, `app/src/collection/palette/`): it reads the app's own lists, so a new feature reaches it
  without palette code. A game action goes in `collection/game_actions.rs` (`GameAction`, listed by
  `CollectionView::game_actions`, run by `run_game_action`): the card's context menu draws the same list. A page action
  goes in the page's `palette.rs` (`pages::commands::PaletteCommands`: list it, even before the page exists, from the
  services or `CommandCtx`; run it; say when the page has a dialog open with `page_modal`). Something that cannot run
  now is listed with `disabled(Some(reason))`, not left out.

## Glass design system (menus, dialogs, pages, settings)

Everything below keeps its old name/signature where one existed, so a caller
picks the new look automatically.

**Backdrop.** GPUI has no backdrop-filter. `CollectionView::glass_image`
(collection/mod.rs) asks the image pipeline for `Variant::Glass` of the selected
game's art (blur 30px, saturate 1.5, ~1/5 of the window, cached in the
`ImageStore` like the stage; costs ~+1 MB PSS) and publishes it every frame with
`ui::glass::set_backdrop`. Glass surfaces paint it aligned to the window and
clipped to their rounded bounds (`Window::paint_image` with a sub-rect), under a
dark dim (`theme::glass_art_dim`) and the translucent fill
(`rgba(22,27,36,.84)` by default). No art (no game selected, Solid) or still
loading: nearly opaque fill (`.94`). Never blur per frame; never request another
blur: reuse this one.

**Interface vs Collection appearance** (Settings → Appearance, two independent
groups). *Collection background* = the stage art behind the list/details: on/off,
blur, darkening, colour, edge shading in Heronite's `local_library/settings.json`
(`background`), plus Opacity (`collectionOpacity`, default 100). *Interface
background* = every glass surface (menus, ☰, context menus, dropdowns, dialogs,
drawer, pages, palette): `interfaceMode` `transparent` (default) | `solid`,
`interfaceOpacity` 50-100 (default 84, maps to `glass_fill`; the art dim rises
from .5 to 1 above 84 so 100 is fully opaque), `interfaceBlur` 0-60 in steps of 5
(default 30), `interfaceSolidTint` slate (default) | graphite | midnight | plum.
These live in this app's `appearance.json` (`AppPaths::appearance()`, next to
`ui-state.json`; `heronite_data::appearance::Appearance`), NOT in the data folder.
At runtime `theme::appearance()/set_appearance()` (a process-wide value) feeds
`theme::{glass_fill, glass_fill_solid, glass_art_dim, page_backdrop,
glass_blur_px, glass_shows_art}`; never hard-code those numbers. The glass
backdrop is generated whenever `glass_shows_art()` (Transparent and < 100%),
independent of the Collection art; in Solid or at 100% nothing is generated and
`ImageStore::forget_prefix("g:")` frees the cached texture (same for the stage
texture "s:" when the Collection art is off). Pages sit on `theme::page_backdrop()`
(opaque page colour when Solid). Sliders call `set_appearance` live and save on
release (off the UI thread).

**Surfaces** (`ui::glass`): `glass_panel(radius)` (big shadow; popovers 20, sheets
26, drawers 24), `glass_menu_surface(radius)` (menus/dropdown lists, radius 14). Children added after the constructor paint above the layers. Children
are not clipped to the rounded corners: give edge-to-edge children their own radius.

**Collection header** (`collection/header.rs`, the one place the main screen departs from Heronite, by user
decision): faint, no bar fill. Over the list column: ☰ (`tool_button`, radius 10, icon `theme::tool_icon()`), the
search pill (`search_fill`, accent ring while focused, clear button) and the View button (`Icon::Tune`, accent while
anything is filtered). Its menu (`collection/view_menu/`) starts with a search box: the active filters sit in it as
tokens (click or Backspace removes one) and typing lists matching values of every kind (Only, Status, Category,
Store, Genre, Developer, Publisher, Series, Platform) with counts, four per kind until `kind:` narrows it; ↑ ↓ and
Enter pick. Below, when nothing is typed: Presets (a `select_menu_field` showing the preset on screen, else "Custom"; a preset applies
everything the menu sets: filters, sort, grouping, hidden games; four built-in ones are added to the user's list once
and are then ordinary presets; under them "Save as preset…" while none matches and "Edit presets…", which lists them
with move, store current state, rename and delete, plus "Restore default presets"; stored as `savedViews`), Sort
(segmented + direction), Group by (None | Status), Show (segmented with counts), Only (Favourites, Update, Offline,
Hidden tiles) and Browse (one chip per kind, fills `kind:`); the footer has the count, Reset all and Statuses…. Several
values of a kind add up (any of them); kinds combine. While its box has the keyboard the menu stays open until Esc or
a click outside. Each active filter also appears as a removable `filter_chip` after the button. Right: in the grid the cover size slider and the "Aa" titles switch, the
Focus / Grid switch (`grid.rs`), bulk metadata, refresh, close focus (Focus view only). Header menus open on click or
after a 140ms hover (`OPEN_DELAY`) and close 180ms after the pointer leaves. The ☰ menu (`nav.rs`) is a list:
pages grouped (Collection, Downloads, Statistics | Stores, Wine Manager, Emulators), then Add game, Console mode,
then Settings and Command palette with their keys; each row = icon square + title + meta on the right. "Stores" is one
row for Epic Games, GOG and Amazon Games: each store keeps its own page, titled "Stores", and the three switch with the
tabs of `pages::stores` (LB / RB in console mode); the row (and the console sidebar's) opens the store shown last.

**Focus panel** (`collection/focus/`, the second place that departs from Heronite, by user decision): a fixed left column
(`focus_left.rs`: big cover, Play, status selector + favourite / metadata / settings, star rating, facts card with score,
library, save backup and Linux) next to the scrolling details; no footer. The title + meta line sit above both, fixed and
aligned with the cover's left edge, so the cover, "Screenshots" and "Details" start on one line; the gap left column → main
equals main → side (`COL_GAP`). `focus::layout(panel_w)` picks the widths: narrow = everything scrolls in one column with the
left block as a row under the title; otherwise left column + main/side (stacked when tight), centred as a group in wide panels. The main column holds Screenshots, Overview, **Timeline** (`timeline.rs`: days of play with
the screenshots of their sessions, thumbnails only) and Related. Facts that are not in the Library load in `game_facts.rs`.
Clickable facts: "Save backup" backs up now, "On Linux" opens the ProtonDB page, the Details folder rows open the file manager;
rows that act get `focus_left::clickable` (pointer + faint highlight). Folder rows are resolved on disk, never shown from
imported fields (`open_links::game_folders`): Steam games → the folder of their `appmanifest`; emulated games → "Emulator
Folder" + "ROM Folder"; others → the stored path only when it exists (Playnite's Windows paths are dropped). ProtonDB needs a
Steam id: the game's, the metadata's, else a strict Steam search by title (`covers::steam_app_id_exact`, same name or name +
edition words, cached in `~/.cache/heronite/protondb/titles.json`); emulated games have no "On Linux" row.

**Grid view** (`collection/grid.rs`, `grid_drawer.rs`, by user decision): the header's Focus / Grid switch (also in the
palette) shows the covers across the whole window instead of the list next to the focus panel. Same groups, sticky
headers, cards, context menu and stage; the cards share each row's width (`grid_columns`: at least `gridCoverWidth`,
110-240, set by the header slider; covers are requested in 16px steps). The selected game opens in a glass drawer on the
right (460px at most, inset 10px, the grid narrows beside it): sharp hero art on top with a close button, the cover
reaching into it next to the title and meta line, then the focus panel's own pieces (Play, status row, rating, facts
card), three tiles (Played, Last played, Sessions) and the details stacked in one column (`focus::stacked_details`),
ending in "Open game page" (back to the Focus view). Keys while nothing floats over it: arrows move the selection
(the drawer follows), Enter plays an installed game, Esc closes the drawer (the selection stays; selecting a game opens
it again). `ui-state.json` keeps `gridView`, `gridCoverWidth` and `gridTitles`. Console mode is not affected.

**Menus** (`ui::dropdown`): `menu_panel(min_w)`, `menu_item(id, active)` (radius 10,
14px, hover .07, active = accent tint), `menu_separator()`, `dropdown(..)`. The
Collection's context menu (`collection/context_menu.rs::item`/`paper`) uses the
same look; other context-menu rows must call `context_menu::item`.

**Dialogs** (`ui::dialog`): `dialog(DialogProps, body, footer, on_close)` = glass
modal (radius 26, title Rubik 22, round close button, Collection darkened by
`rgba(0,0,0,.55)`); `drawer(DrawerProps { id, width }, header, body, footer,
on_close)` = right-hand sheet inset 10px, radius 24, with fixed header/footer
slots (build the header with `kicker`, `dialog_title`, `close_button`).

**Controls** (`ui::controls`):
- `toggle(id, on, disabled)` 40x24 switch (spring-animated knob unless
  `HERONITE_NO_ANIMATIONS`), `toggle_row(id, label, desc, on)`, legacy
  `checkbox(id, checked, label)` (= toggle row) and `mini_checkbox` (30x18 switch).
  The caller adds `.on_click`.
- `button(id, kind, label)` / `button_sm` / `action_button(.., enabled, on_click)`:
  pill buttons, sentence case. `ButtonKind::Glass` = secondary, `Primary` = accent
  fill, `Ghost` = no fill, `Danger` = red tint. Disabled = 45%.
- `tabs(id, items, active, on_select)` = segmented pills (scrolls horizontally).
- `card(rows)` / `card_box()` + `row(label, desc, control)` (11px padding,
  separators between rows), `caption(text)`, `page_heading`
  (Rubik 26), `dialog_title` (Rubik 22), `help_text` (lead), `kicker` (the only
  uppercase text), `kbd("Ctrl K")`, `close_button(id)`, `nav_pill(id, icon, label, active)`.
- Fields (`ui::fields`): `path_field` shows a folder icon and "Clear"/"Browse" in accent;
  `select_field`'s list is a scrollable glass menu; labels are 12px muted.

**Game settings drawer** (`dialogs/game_settings/`): the first `drawer` caller. Tabs are
segmented pills in the header slot; each tab is a stack of `caption` + `card`. The tabs are the same for
every game, by what the user wants to change: Game (images + details), Launch (Play, the actions,
the panel's launch settings), Compatibility (Wine / Proton), Performance (overlays, FPS limit, frame
generation, image, system) and Saves (Ludusavi backups + cloud saves); a tab only goes away when the
game has nothing for it. Descriptions are one short line saying the effect; paths and rare warnings
only show when they matter. The Heroic
panels (`dialogs/settings_panel`) build their cards with `parts::rows()` (`.child(toggle_row)`,
`.field(input)`, `.field_help(input, notes)`, `.card()`); they assume no container, so Settings →
Game defaults can host them too. A setting that differs from the game defaults gets a 6px accent dot
in the card's left margin (it turns into a reset arrow under the pointer), and the dialog a
"Reset this tab" button next to Done while the tab has changes (`settings_panel/changes.rs`;
console: a dot after the label, Y resets, a "Reset this tab" row). A new setting is one entry of its
`GROUPS`, keyed by the row, input or setting id. The Launch tab ends with "How the game starts"
(`launch_preview.rs`): the command and variables of a launch built from the settings on screen, grouped
by the setting each comes from (`HERONITE_DEBUG_PREVIEW=1` opens it).

**Pages** (`pages/mod.rs`, ☰ pages other than Settings): `sheet(id, title, lead, actions, max_w, window, close, column)`
= glass sheet inset 12px under the slim header (title Rubik 26 + lead, optional header actions, round close =
`page_close(cx)` in the sheet's top-right corner, which navigates back to the Collection, then a scrolling body). Title
and body share one column of at most `max_w`, **centred**: never leave content stuck to the left of a wide window.
Use the width instead of a narrow strip: pick the layout from `content_width(window)` — `columns(wide, [a, b])` puts
blocks side by side (stacked when narrow), a `grid().grid_cols(n)` holds repeated cards (Emulators), and a fixed side
column + the main list suits browsers (Wine Manager). Check new pages at ~1000px and on an ultrawide. Fill the column
with `caption` + `card`/`card_box` + `row`. List helpers: `list_item(leading, content, trailing)` (card row with cover/icon),
`icon_action(id, icon, enabled)` (round 32px list button), `status_pill(text, colour)` / `tag_pill(text)`,
`caption_with(text, action)` (caption + action on the right), `empty_row(text)`. Progress bars: `services::widgets::progress_bar`.
Confirmations inside a page are inline rows of a card (text + ghost "Cancel" + danger button), not modals.
Statistics (`pages/stats.rs`, Ctrl 6) only reads: the Collection hands it the loaded library
(`StatsPage::set_library`, on navigate and after each reload) and the numbers come from `heronite_data::stats`
(sessions cut into hours; sessions under 2 min are failed launches and ignored). Charts are plain divs: bars stacked
in `theme::chart_series(i)` for the top 5 games of the period (`Stats::series`), the rest `theme::chart_other()`; a
game keeps its colour in every block of the page. Hovering a bar / heatmap cell updates the line above it (no tooltips).
`HERONITE_DEBUG_PAGE=stats:week|month|year|all` opens it on a span.

**Console mode** (`app/src/console/`, shell in `collection/console/`): an extra controller-first TV mode
entered from ☰ › Console mode or the palette,
remembered across restarts (`ui-state.json` `consoleMode`). The desktop look must not change: every page and dialog
keeps its desktop render and, when `crate::console::active()`, returns its console layout instead
(`render_console`, usually in a child module `console.rs`) and implements `console::PadTarget` (`on_pad` + `hints`).
- Input: `console::input` reads controllers with gilrs on a thread that only runs in console mode (presses only count
  while the window is focused); the keyboard maps to the same `Pad`s (arrows, Enter = A, Esc = B, X, Y, Q/E = LB/RB, M = menu).
- Hosting: pages register their hook in `nav.rs` (`pages.hooks`); dialogs are found by `console::registry::hook_for`
  (add new console-capable dialogs there). Overlays owned by a view use `console::Layer` (drawn by the owner; for a
  view embedded in a smaller area wrap it with `kit::window_layer`).
- Building blocks: `console::kit` (sizes through `u()`, scaled to the window height; `row_base`, `toggle_pill`, `choice`,
  `tabs`, `glyph`, `hint_bar`, `modal`), `console::list` (`Row`s + `ListNav`: one focused row, values changed with ◀ ▶,
  `Outcome` tells the owner what happened; `simple_dialog` for confirmations), `console::sheet::Sheet`,
  `console::osk::Osk` (on-screen keyboard; never use text inputs in console layouts), `console::browser::Browser`
  (folder / file picker; the portal file chooser cannot be driven by a controller).
- Motion (`console::motion`): scroll areas that follow the focus use a `Glide` (`go(Goal::Reveal{..})` / `Start` /
  `by(px)`, its `driver()` as the last child of the area) instead of `scroll_to_item`, so they glide on a spring;
  `row_base` focus pills pop in on every press; covers lift with `motion::spring`; screens, pages, dialogs, the menu
  and panels (`kit::panel_in`, `kit::scrim`) fade / slide in with `motion::enter` (new id = replay). Only moving
  things request frames; `HERONITE_NO_ANIMATIONS=1` jumps instead. Off-screen covers are empty boxes of the same size.
- Layout: top bar = `kit::wordmark` + LB/RB `kit::tabs` + `kit::clock`, content padded `u(46.)`, lists about `u(900.)`
  wide; the shell draws the hint bar from `hints()` (it adds Back / Menu). B goes back; Start opens the menu.
- Debug: `HERONITE_DEBUG_CONSOLE=1` (or `=game:<key>` for a game page) starts in console mode without saving it;
  `HERONITE_DEBUG_PAD=down,a,wait:500,b` plays presses (`up down left right a b x y lb rb lt rt start select sup sdown`).

**Do not** use uppercase labels, outlined buttons, underlined tabs, square
checkboxes or flat grey modals in new UI. Colours come from `theme` (`text_primary`,
`text_muted`, `text_dim`, `accent_tint`, ...).

**Verifying**: captures with the app on a hidden workspace only show what was
drawn before the compositor stopped sending frames, so late images (art, covers)
are missing. Take a first capture to wake the window, wait a second, then take
the real one (a modified copy of `scripts/shot.sh` doing `grim -T` twice).
`HERONITE_DEBUG_DROPDOWN=nav|view|view:<text>`, `HERONITE_DEBUG_MENU=<key>`,
`HERONITE_DEBUG_DIALOG=...`, `HERONITE_DEBUG_PAGE=...`. Copy
`~/.local/share/heronite/images-cache` into the fixture or no art appears at all.

## Rules
- No I/O on the UI thread except tiny reads; use `cx.background_spawn` and
  update the entity when done. Never block render.
- Data folder files (`~/.local/share/heronite`) are written only through
  `heronite_data::json_store` or the backend crates (`crates/*`), which keep
  the electron-store format.
- Images: request through `CollectionView::cover` / `ImageStore` (worker
  threads, LRU of GPU textures). For one-off rasters use
  `heronite_images::*` in `background_spawn` and wrap with
  `RenderImage::new(smallvec![image::Frame::new(decoded.into_image_buffer())])`.
- Icons: SVGs in `app/assets/icons` (MUI/FontAwesome names). Add new ones by
  taking the path of `<Name>` from the
  [`@mui/icons-material`](https://github.com/mui/material-ui/tree/master/packages/mui-icons-material)
  package, register them in `icons.rs` and list the source in
  `THIRD-PARTY-NOTICES.md` when it is a new one.
- Text is written in English (Heronite's `en` locale strings in
  [`public/locales/en/translation.json`](https://github.com/RafaelGoulartB/HeroicGamesLauncher/blob/master/public/locales/en/translation.json))
  and every user-visible string goes through `heronite_i18n`: `tr!("Play")`,
  `tr!("{n} of {total}", n, total)`, `trn!(count, "{n} game", "{n} games")`,
  `n_!` for constant tables. Never build a sentence from fragments
  (word order differs); never translate ids, keys or values that are saved or
  parsed. Add the Portuguese to `crates/i18n/locales/pt-BR/<area>.json`;
  `cargo test -p heronite-i18n` fails on a missing or unused key. Portuguese
  runs ~25% longer than English: leave room in buttons and pills.

## Verifying visually (mandatory for UI work)
- Build: `cargo build -p heronite`.
- `scripts/shot.sh /tmp/x.png [wait_s]` opens the app **on Hyprland
  workspace 2 silently**, captures the window with `grim -T` (works
  off-screen), prints PSS and closes it. Never open windows anywhere else,
  never switch workspaces, never move/click the real mouse.
- Debug hooks (env, via `SHOT_ENV="..."`): `HERONITE_DEBUG_SELECT=<runner_appName>`,
  `HERONITE_DEBUG_MENU=<key>` (context menu), `HERONITE_DEBUG_DROPDOWN=view|view:<text>|nav`,
  `HERONITE_DEBUG_DIALOG=settings|statuses|gamesettings:<tab>|onboarding[:<step>]` (extend the match in
  `CollectionView::render` for new dialogs; `settings:<section>` opens that Settings section,
  `onboarding:<step>` the getting-started checklist on a step such as `palette` or `screenshots`).
  `scripts/shot.sh` sets `HERONITE_NO_ONBOARDING=1`: the checklist does not open by itself on a new state folder.
- Main Collection screen: compare with the reference PNG (crop the same region with PIL, view both). Dialogs/menus/settings: one sanity check is enough — they need to work and be consistent, not pixel-identical (user decision).
- Anything that WRITES data must be exercised only against a copy:
  `HERONITE_DATA_DIR=/tmp/heronite-fixture-<you> scripts/shot.sh …`
  (copy `/tmp/heronite-fixture`, which mirrors the data folder's JSON).
- Never launch games, emulators, Steam or Ludusavi backups for real.
