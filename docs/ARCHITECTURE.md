# Architecture

HeroniteRust is a GPUI rewrite of the Heronite launcher (a Heroic Games
Launcher fork). It is a separate app with its own data folder,
`~/.local/share/heronite/` (`DataPaths`; `HERONITE_DATA_DIR` overrides
it). The folder keeps Heroic's layout and electron-store formats, but nothing
in it is shared with Heroic or Heronite, and this app never reads or writes
Heroic's folders. UI state lives in `~/.config/heronite/` and caches in
`~/.cache/heronite/` (`AppPaths`).

On the first start, when the data folder does not exist yet,
`heronite_data::migrate` copies an existing Heroic setup into it once:
`~/.config/heroic` (minus Chromium's files), Heroic's game logs and the
legendary binary of the Heronite install (to `tools/bin/legendary`). Paths
into Heroic's folder are rewritten. Heroic's files are only read. The
UI state and cache folders of the old name (`heronite-rs`) are renamed.

`docs/CODE-STYLE.md` has the code rules; `docs/UI-CONVENTIONS.md` the look of
the UI.

## Crates

```
app (bin heronite-rs) ── GPUI UI, desktop integration
 ├─ heronite-i18n       interface language: tr!/trn! and the pt-BR strings (no GPUI)
 ├─ heronite-data       data folder files, paths, Heroic migration, safe writes, processes
 ├─ heronite-images     decode/resize/blur pipeline and thumbnail cache (no GPUI)
 ├─ heronite-launch     launch plans, game runtime, playtime, logs   → data
 ├─ heronite-epic       legendary CLI, Epic library, download queue  → data, launch
 ├─ heronite-gog        gogdl CLI + GOG web APIs: GOG account,
 │                      library, installs, updates, cloud saves      → data, launch, epic
 ├─ heronite-amazon     nile CLI: Amazon Games account, library,
 │                      installs, updates                            → data, launch, epic
 ├─ heronite-emulation  emulators, ROM scan, save states, Ludusavi   → data, launch
 ├─ heronite-metadata   IGDB/Steam/Lutris/HLTB metadata and artwork  → data
 ├─ heronite-media      screenshots: listing, thumbnails, capture,
 │                      lossless PNG → WebP compression              → data, images
 ├─ heronite-wine       Wine/Proton manager                          → data
 ├─ heronite-shortcuts  .desktop files and Steam shortcuts.vdf       → data
 ├─ heronite-steam      settings of Steam games (launch options,
 │                      Proton) via the client or its files         → data, launch, shortcuts
 └─ heronite-update     app updates: release check, signed download,
                        AppImage swap / package install, relaunch   → data
epic-login (bin)        WebKit window for the Epic, GOG (`--store gog`) and Amazon
                        (`--store amazon --url ..`) logins (keeps a web engine out of the app)
```

Library crates never depend on GPUI. Their functions are blocking; the app
calls them from `cx.background_spawn` or worker threads.

- **heronite-data:** the base. `paths` (`DataPaths`, `AppPaths`), `migrate`,
  `json_store` (electron-store read-modify-write: tab indent, atomic write,
  unknown keys kept), `temps` (atomic writes and the startup `.tmp` sweep),
  `proc` (`guard` and `Run`: helper processes die with the app), and one
  module per data file: `library`, `collection`, `sideload`, `game_config`,
  `settings`, `status`, `sessions`, `playtime`, `ui_state`, `appearance`,
  `language`, `steam_owned` (the cached Steam library and its twins: a game
  owned on Steam that the Collection has from another source joins that
  entry instead of getting a Steam entry of its own).
- **heronite-i18n:** English text is the key (`tr!("Play")`); Portuguese
  lives in `locales/pt-BR/*.json`, embedded at build time and loaded the
  first time it is needed. The language follows the system unless one is
  picked in Settings → General (`language` in `appearance.json`); switching
  redraws the windows. `tests/coverage.rs` scans the workspace: every key
  needs a translation and every translation a key.
- **heronite-launch:** `plan` turns a game plus settings into a `LaunchPlan`
  and does no I/O beyond reads. `runtime::Launcher` runs the plan through a
  `Spawner`, tracks the process group and emits `LaunchEvent`s. `record`
  writes sessions and playtime (with checkpoints while the game runs).
  `log_writer` writes the launcher and game logs. `prepare` does the work
  around a launch (cloud saves, runtimes); `first_launch` sets up a game the
  first time it runs in a Wine prefix. `lsfg` finds lsfg-vk (Lossless
  Scaling frame generation), the Lossless Scaling DLL and lsfg-vk's profiles,
  and adds the frame generation variables to a launch. `fps_limit` holds a
  game's one FPS limit and picks the one limiter that applies it (MangoHud,
  DXVK / VKD3D-Proton or gamescope; Heroic's separate limiters are read as
  a fallback). `gpu` holds a game's graphics card settings (the card that draws it,
  NVIDIA Smooth Motion and Reflex, the DLSS / FSR 4 / XeSS upgrades of GE-Proton and
  Proton-CachyOS, Mesa's Anti-Lag and glthread, vsync and what DXVK tells DirectX games)
  as variables, and `gpu::detect` finds the cards (sysfs, `vulkaninfo`, `pci.ids`) and the
  driver layers they need. `actions` turns a game's own actions (`heronite_data::game_actions`,
  `local_library/actions.json`) into something to open, spawn or launch as
  the game's session. `umu_ids` picks the
  `GAMEID` of a umu launch (protonfixes) from the user's id, the umu
  database (downloaded weekly), Heroic's cache, the metadata's Steam id or
  the title; `keep_awake` adds the logind lock the runtime holds while the
  game runs.
- **heronite-epic:** `cmd` runs legendary. `manager` + `queue` are the
  download queue (`store/download-manager.json`), shared by every store: an
  element's `params.runner` picks the job (legendary, gogdl or nile), and
  `progress` parses the reports of all three tools. `ops` covers install,
  repair, uninstall and import. `moves` is the resumable install move with
  its journal. `eos_overlay` manages the EOS overlay.
- **heronite-gog / heronite-amazon:** the same shape for GOG and Amazon
  Games. The tools (`gogdl`, `nile`) are found or downloaded into
  `<data>/tools/bin` by `heronite_launch::store_tools`. GOG's library comes
  from GOG's web APIs (token from gogdl's `gog_store/auth.json`) into
  `store_cache/gog_library.json`, installs are listed in
  `gog_store/installed.json`; Amazon's library and installs are nile's own
  files (`nile_config/nile/`), mirrored into `store_cache/nile_library.json`.
  Both libraries are read by the Collection like Epic's. Launching is in
  `heronite-launch` (`gog.rs`: the primary task of `goggame-<id>.info`, or
  `start.sh` for Linux builds; `nile.rs`: `fuel.json`), always through the
  Wine/Proton plan of local games; the first launch in a prefix runs the
  store's installers (`first_launch::StoreSetup`, Amazon's `PostInstall`).

- **heronite-steam:** Steam games keep launching through Steam
  (`steam://rungameid`), with Steam's Proton and prefix. Their game settings
  tabs edit what Steam stores per game instead: the compatibility tool and the
  launch options line. `options` maps the line to the panel's settings keys
  and back (lines with shell syntax are only edited as text). While Steam
  runs, changes go through its CEF debugging interface (`client`, via
  `heronite_shortcuts::steam_client`, whose port `debugger_addr` finds);
  with Steam closed, `files` edits `localconfig.vdf` and `config.vdf` in
  place (`vdf_edit` touches only the changed entry). `wrap` writes the script
  that runs the before/after scripts from the launch options. `library`
  reads the account's library from the client (games only: no tools,
  soundtracks, demos, test servers or games hidden in Steam), caches it and
  adds the games the Collection does not have yet; Steam's play time moves
  the Steam entries up when it is ahead.
- **heronite-wine:** the Wine Manager (`manager`: download, install, remove
  builds from `repos`) and `detect`, the Wine/Proton list the settings pick
  from. `default` keeps a global default that can run games: when none is
  set (a new setup) or it was removed, the newest Proton found becomes the
  default, a few seconds after start, after a Wine Manager install, and when
  a Windows game cannot start for lack of one.

## App (`app/src`)

| Module | Role |
|---|---|
| `main.rs` | logging, CLI, single instance, signals, the GPUI loop |
| `env.rs` | `HERONITE_*` switches (`flag`, `value`, `parse`) |
| `system/` | lifecycle `hub/` (window, quit, launch requests, tray events), `cli`, `single_instance`, `signals`, `tray`, `notify`, `shortcuts` |
| `services/` | long-lived entities shared by every screen: `downloads/` (the store service: Epic, GOG, Amazon Games), `session` (post-game work), `compression` (background lossless compression of the captures), `update/` (app updates: checks, download, restart), `steam_library` (reads the Steam library while Steam runs), `widgets` |
| `runtime.rs` | bridge between `heronite-launch` and the UI |
| `collection/` | the main screen (`CollectionView`): list, header and its View menu (`view_menu`: filter search, views, sort, grouping), focus panel, grid view (`grid`, `grid_drawer`), stage, context menu, command palette, screenshots, and the glue to dialogs and services; `welcome` is the empty Collection of a new setup (it checks Steam every few seconds while it shows); `onboarding` hosts the getting-started checklist (`dialogs/onboarding`: steps, looping previews drawn from the user's covers), which opens by itself once per setup and from the bulb in the header (`onboarding.json` in the app state) |
| `dialogs/` | one view per dialog; they talk back through `DialogEvent` |
| `pages/` | ☰ pages (Settings, Downloads, Epic, GOG, Amazon Games, Steam, Wine Manager, Emulators); they talk back through `PageEvent` |
| `ui/` | building blocks: `controls`, `fields`, `dialog`/`drawer`, `dropdown`, `glass`, `scrollbar` |
| `theme/` | colour, font, layout and glass tokens, plus the runtime appearance |
| `images.rs` | UI side of the image pipeline (texture cache, eviction) |

### Patterns
- **One window, one root view.** `CollectionView` owns the open dialog, page,
  menus and palette, and draws them over the list. A big view is a
  directory: `mod.rs` holds the struct and `Render`, and sibling files add
  `impl` blocks (`actions.rs`, `load.rs`, `render_*.rs`, `debug.rs`).
- **Services are entities** kept in GPUI globals and created on first use. Views `observe` them or
  `subscribe` to their events. They own background work, so closing the
  window (to the tray) does not stop a download or a post-game task.
- **No I/O in `render`.** Loads run in the background and land in the entity
  with `this.update(..)`. Long-lived channels are drained by "pump" tasks
  (`collection/pumps.rs`).
- **Debug hooks.** `HERONITE_DEBUG_*` variables open a page, dialog, menu or
  state at startup, so `scripts/shot.sh` can capture it without clicks. They
  live in each area's `debug.rs`, and every read goes through `crate::env`.
- **Safe shutdown.** `system/hub/quit.rs` is the single quit path (window,
  tray, signals): checkpoint playtime, pause the Epic queue, release the
  socket and the tray, then end the loop. Games are separate process groups
  and keep running. "Restart now" after an update goes through the same path
  (`restart_into`, refused while a game runs) and `main` starts the new
  AppImage once the socket is gone.
- **App updates.** `heronite_update` reads the latest GitHub release and
  checks every download against its minisign signature (key in
  `crates/update/src/lib.rs`, trusted comment `version:<X.Y.Z>`). From an
  AppImage (`$APPIMAGE`) it downloads `Heronite-x86_64.AppImage` next to the
  running one and renames it over the old file: the running app keeps the old
  inode, and nothing is written into it. From our .deb / .rpm / Arch package
  (marker `/usr/share/heronite/package-format`, written by
  `scripts/packages.sh` from `packaging/nfpm.yaml`) it downloads the package
  of the same format and runs apt-get / dnf / pacman through `pkexec`. Any
  other build only checks when asked. The release workflow signs with the
  `MINISIGN_SECRET_KEY` secret and checks the result against the key in the
  code.

## Tests
- Unit tests sit next to the code. Integration tests are in
  `crates/*/tests`, and some need `HERONITE_FIXTURE=/tmp/heronite-fixture`
  (a copy of a data folder).
- `cargo test --workspace` and `cargo clippy --workspace --all-targets` must
  both be clean.
- UI checks use `scripts/shot.sh` on a fixture copy (see "Verifying visually"
  in `docs/UI-CONVENTIONS.md`).
