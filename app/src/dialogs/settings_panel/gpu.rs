//! "GPU" tab: which card draws the game, then what the cards of this
//! computer offer (`heronite_launch::gpu`): the NVIDIA driver's features,
//! the upscaler upgrades of GE-Proton and Proton-CachyOS, Mesa's switches,
//! and what DirectX games are told about the card. Only the options of the
//! card that draws the game show (a launch leaves the others out too, see
//! `Gpu::for_card`).
//!
//! The rows are built once ([`SettingsPanel::gpu_rows`]) and drawn by the
//! desktop tab here and by the console list (`console/gpu.rs`).

use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::gpu::detect::{Card, Machine, ProtonFeatures};
use heronite_launch::gpu::{self, DlssPreset, Gpu, Vendor, Vsync};
use serde_json::{json, Value};

use super::parts::{column, heading, help, rows, Rows};
use super::SettingsPanel;
use crate::ui::fields::SelectOption;

/// The cards do not change while the app runs: looked at once.
static MACHINE: OnceLock<Arc<Machine>> = OnceLock::new();

pub(super) type Pick = Rc<dyn Fn(&mut SettingsPanel, usize, &mut Context<SettingsPanel>)>;

/// One row of the tab.
pub(super) enum GpuRow {
    Header(&'static str),
    Toggle {
        key: &'static str,
        label: &'static str,
        desc: String,
        enabled: bool,
    },
    /// `options`: `(label, note)`.
    Choice {
        id: &'static str,
        label: &'static str,
        desc: String,
        options: Vec<(String, Option<String>)>,
        selected: usize,
        enabled: bool,
        pick: Pick,
    },
    Info {
        id: &'static str,
        label: String,
        value: String,
    },
}

impl SettingsPanel {
    /// Looks at the cards (once per run) and at the game's Proton.
    pub(super) fn load_gpu(&mut self, cx: &mut Context<Self>) {
        self.load_proton_features(cx);
        if let Some(m) = MACHINE.get() {
            self.machine = Some(m.clone());
            return;
        }
        // Debug captures only keep the first frames: look in place there.
        if crate::env::flag("HERONITE_DEBUG_DIALOG") {
            self.machine = Some(MACHINE.get_or_init(|| Arc::new(Machine::detect(&heronite_launch::SystemProbe))).clone());
            return;
        }
        cx.spawn(async move |this, cx| {
            let m = cx.background_spawn(async move { Machine::detect(&heronite_launch::SystemProbe) }).await;
            let m = MACHINE.get_or_init(|| Arc::new(m)).clone();
            this.update(cx, |this, cx| {
                this.machine = Some(m);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// What the game's Proton offers (Windows games run here; Steam's own
    /// Proton is not read).
    pub(super) fn load_proton_features(&mut self, cx: &mut Context<Self>) {
        if !self.is_game() || self.is_steam() || self.native() {
            return;
        }
        let w = self.wine_version();
        if w.kind != "proton" {
            self.proton_features = Some((w.bin, ProtonFeatures::default()));
            return;
        }
        if self.proton_features.as_ref().is_some_and(|(bin, _)| *bin == w.bin) {
            return;
        }
        let bin = w.bin.clone();
        cx.spawn(async move |this, cx| {
            let read = bin.clone();
            let f = cx.background_spawn(async move { gpu::detect::proton_features(&read, &heronite_launch::SystemProbe) }).await;
            this.update(cx, |this, cx| {
                this.proton_features = Some((bin, f));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn gpu_settings(&self) -> Gpu {
        Gpu::from_map(&self.settings)
    }

    /// Whether the game runs through Wine / Proton (Steam games too), where
    /// the Proton and DXVK rows apply.
    fn gpu_windows(&self) -> bool {
        !self.native()
    }

    /// The game's Proton has the upscaler upgrades (`None`: not known: a
    /// Steam game, the global defaults, or still being read).
    fn gpu_proton(&self) -> Option<ProtonFeatures> {
        if self.is_steam() || !self.is_game() {
            return None;
        }
        self.proton_features.as_ref().filter(|(bin, _)| *bin == self.wine_version().bin).map(|(_, f)| *f)
    }

    /// The rows of the tab. `disabled`: a Steam game whose launch options
    /// cannot change now.
    pub(super) fn gpu_rows(&self, disabled: bool) -> Vec<GpuRow> {
        let Some(m) = self.machine.clone() else {
            return vec![GpuRow::Info { id: "gpu-looking", label: tr!("Looking for graphics cards\u{2026}").into(), value: String::new() }];
        };
        let g = self.gpu_settings();
        let mut v = vec![GpuRow::Header(tr!("Graphics card"))];
        v.push(self.gpu_device_row(&m, &g, disabled));
        // No card found: every option shows, as every one applies.
        let target = gpu_target(&m, &g);
        let vendor = target.map(|c| c.vendor_id);
        let nvidia = vendor.is_none_or(|v| v == gpu::NVIDIA);
        if nvidia {
            v.extend(self.gpu_nvidia_rows(target, &m, &g, disabled));
        }
        if self.gpu_windows() {
            v.extend(self.gpu_upscaler_rows(vendor, &g, disabled));
        }
        if vendor.is_none_or(|v| v != gpu::NVIDIA) {
            v.push(GpuRow::Header(target.and_then(Card::vendor).map_or("Mesa", Vendor::name)));
            v.push(GpuRow::Toggle {
                key: gpu::KEY_ANTI_LAG,
                label: tr!("Anti-Lag"),
                desc: if m.anti_lag {
                    tr!("Less input lag, from Mesa's Anti-Lag layer").into()
                } else {
                    tr!("Needs a Mesa with its Anti-Lag layer").into()
                },
                enabled: !disabled && (m.anti_lag || g.anti_lag),
            });
            v.push(GpuRow::Toggle {
                key: gpu::KEY_GLTHREAD,
                label: tr!("Threaded OpenGL"),
                desc: tr!("Faster OpenGL games, like Minecraft and older Source games").into(),
                enabled: !disabled,
            });
        }
        v.extend(self.gpu_display_rows(&g, disabled));
        v
    }

    fn gpu_device_row(&self, m: &Machine, g: &Gpu, disabled: bool) -> GpuRow {
        let kind = |integrated: Option<bool>| match integrated {
            Some(true) => Some(tr!("integrated").to_string()),
            Some(false) => Some(tr!("dedicated").to_string()),
            None => None,
        };
        if m.cards.len() < 2 && g.device.is_empty() && !g.dedicated {
            let value = m.cards.first().map(|c| c.name.clone()).unwrap_or_else(|| tr!("None found").into());
            return GpuRow::Info { id: "gpu-card", label: tr!("Graphics card").into(), value };
        }
        let mut options: Vec<(String, Option<String>)> = vec![(tr!("The system's choice").into(), m.cards.first().map(|c| c.name.clone()))];
        // Heroic's switch is kept while it is the setting.
        let dedicated = g.dedicated && g.device.is_empty();
        if dedicated {
            options.push((tr!("The dedicated GPU").into(), Some("NVIDIA PRIME".into())));
        }
        let cards: Vec<String> = m.cards.iter().map(|c| c.slot.clone()).collect();
        options.extend(m.cards.iter().map(|c| (c.name.clone(), kind(c.integrated))));
        let missing = !g.device.is_empty() && m.card(&g.device).is_none();
        if missing {
            options.push((tr!("Card at {slot}", slot = g.device.as_str()), Some(tr!("not connected").into())));
        }
        let selected = if missing {
            options.len() - 1
        } else if let Some(i) = cards.iter().position(|s| *s == g.device) {
            i + 1 + usize::from(dedicated)
        } else {
            usize::from(dedicated)
        };
        let desc = if missing {
            tr!("This card is not in the computer now: the game gets the system's choice.")
        } else {
            tr!("For laptops and computers with two cards. The options below are the picked card's.")
        };
        GpuRow::Choice {
            id: "gpu-device",
            label: tr!("Draws the game"),
            desc: desc.into(),
            options,
            selected,
            enabled: !disabled,
            pick: Rc::new(move |this, i, cx| {
                let first_card = 1 + usize::from(dedicated);
                let (device, prime) = match i {
                    0 => (String::new(), false),
                    1 if dedicated => (String::new(), true),
                    i if i >= first_card && i - first_card < cards.len() => (cards[i - first_card].clone(), false),
                    _ => return,
                };
                this.set_all(vec![(gpu::KEY_DEVICE, json!(device)), (gpu::KEY_PRIME, Value::Bool(prime))], cx);
            }),
        }
    }

    fn gpu_nvidia_rows(&self, card: Option<&Card>, m: &Machine, g: &Gpu, disabled: bool) -> Vec<GpuRow> {
        let mut v = vec![GpuRow::Header("NVIDIA")];
        let smooth_desc = if !m.smooth_motion {
            tr!("Needs NVIDIA driver 575 or newer").to_string()
        } else if card.and_then(|c| c.smooth_motion_capable()) == Some(false) {
            tr!("Needs a GeForce RTX 40 series card or newer").to_string()
        } else if g.smooth_motion && self.lsfg_settings().enabled {
            tr!("Lossless Scaling frame generation is on too: use one of them").to_string()
        } else {
            tr!("The driver generates frames between the game's: about twice the FPS. Vulkan and DirectX games").to_string()
        };
        v.push(GpuRow::Toggle {
            key: gpu::KEY_SMOOTH_MOTION,
            label: tr!("Smooth Motion"),
            desc: smooth_desc,
            enabled: !disabled && (m.smooth_motion || g.smooth_motion),
        });
        if self.gpu_windows() {
            let has = self.gpu_proton().is_none_or(|f| f.reflex);
            v.push(GpuRow::Toggle {
                key: gpu::KEY_REFLEX,
                label: tr!("Reflex in Vulkan games"),
                desc: if has {
                    tr!("Less input lag in Vulkan games with Reflex. DirectX games have it already").into()
                } else {
                    tr!("Needs GE-Proton or Proton-CachyOS").into()
                },
                enabled: !disabled && (has || g.reflex),
            });
        }
        v.push(GpuRow::Toggle {
            key: gpu::KEY_SHADER_CACHE,
            label: tr!("Bigger shader cache"),
            desc: tr!("10 GB that is never trimmed: big games stop compiling shaders again").into(),
            enabled: !disabled,
        });
        v
    }

    /// `vendor`: the maker of the card that draws the game (`None`: unknown).
    fn gpu_upscaler_rows(&self, vendor: Option<u16>, g: &Gpu, disabled: bool) -> Vec<GpuRow> {
        let proton = self.gpu_proton();
        let has = proton.is_none_or(|f| f.upgrades);
        let enabled = |on: bool| !disabled && (has || on);
        let needs = tr!("Needs GE-Proton or Proton-CachyOS");
        let desc = |text: &str| if has { text.to_string() } else { needs.to_string() };
        let mut v = vec![GpuRow::Header(tr!("Upscalers"))];
        if vendor.is_none_or(|v| v == gpu::NVIDIA) {
            v.push(GpuRow::Toggle {
                key: gpu::KEY_DLSS_UPGRADE,
                label: tr!("Newest DLSS"),
                desc: desc(tr!("Games with DLSS get its newest version, downloaded into the prefix")),
                enabled: enabled(g.dlss_upgrade),
            });
            if g.dlss_upgrade {
                let names = [
                    (tr!("The game's"), None),
                    (tr!("Newest"), Some(tr!("recommended"))),
                    ("K", Some(tr!("transformer, DLSS 4"))),
                    ("E", Some(tr!("CNN, lighter"))),
                ];
                v.push(GpuRow::Choice {
                    id: "gpu-dlss-preset",
                    label: tr!("DLSS model"),
                    desc: tr!("The newer models look sharper and cost a little more").into(),
                    options: names.iter().map(|(n, note)| (n.to_string(), note.map(str::to_string))).collect(),
                    selected: DlssPreset::ALL.iter().position(|p| *p == g.dlss_preset).unwrap_or(0),
                    enabled: !disabled,
                    pick: Rc::new(|this, i, cx| this.set(gpu::KEY_DLSS_PRESET, json!(DlssPreset::ALL[i].id()), cx)),
                });
                v.push(GpuRow::Toggle {
                    key: gpu::KEY_DLSS_INDICATOR,
                    label: tr!("Show the DLSS indicator"),
                    desc: tr!("The DLSS version and model in a corner of the game").into(),
                    enabled: !disabled,
                });
            }
        }
        if vendor.is_none_or(|v| v == gpu::AMD) {
            v.push(GpuRow::Toggle {
                key: gpu::KEY_FSR4_UPGRADE,
                label: tr!("FSR 4"),
                desc: desc(tr!("Games with FSR 3.1 use FSR 4 (Radeon RX 9000)")),
                enabled: enabled(g.fsr4_upgrade),
            });
            if g.fsr4_upgrade {
                v.push(GpuRow::Toggle {
                    key: gpu::KEY_FSR4_INDICATOR,
                    label: tr!("Show the FSR indicator"),
                    desc: tr!("A mark in the game while FSR 4 runs").into(),
                    enabled: !disabled,
                });
            }
        }
        v.push(GpuRow::Toggle {
            key: gpu::KEY_XESS_UPGRADE,
            label: tr!("Newest XeSS"),
            desc: desc(tr!("Games with XeSS get its newest version (any card)")),
            enabled: enabled(g.xess_upgrade),
        });
        v
    }

    fn gpu_display_rows(&self, g: &Gpu, disabled: bool) -> Vec<GpuRow> {
        let mut v = vec![GpuRow::Header(tr!("All cards"))];
        let vsync_names = [tr!("The game's"), tr!("Always on"), tr!("Always off")];
        v.push(GpuRow::Choice {
            id: "gpu-vsync",
            label: tr!("VSync"),
            desc: tr!("Off: more FPS and less lag, with tearing. NVIDIA's Vulkan follows the game").into(),
            options: vsync_names.iter().map(|n| (n.to_string(), None)).collect(),
            selected: Vsync::ALL.iter().position(|s| *s == g.vsync).unwrap_or(0),
            enabled: !disabled,
            pick: Rc::new(|this, i, cx| this.set(gpu::KEY_VSYNC, json!(Vsync::ALL[i].id()), cx)),
        });
        if !self.gpu_windows() {
            return v;
        }
        let mut vendors: Vec<(String, Option<String>)> = vec![(tr!("The real one").into(), None)];
        vendors.extend(Vendor::ALL.iter().map(|x| (x.name().to_string(), None)));
        v.push(GpuRow::Choice {
            id: "gpu-vendor",
            label: tr!("Card maker the game sees"),
            desc: tr!("For DirectX games that refuse a card, or hide options from it").into(),
            options: vendors,
            selected: g.vendor.and_then(|x| Vendor::ALL.iter().position(|y| *y == x)).map_or(0, |i| i + 1),
            enabled: !disabled,
            pick: Rc::new(|this, i, cx| {
                let id = if i == 0 { "" } else { Vendor::ALL[i - 1].id() };
                this.set(gpu::KEY_VENDOR, json!(id), cx);
            }),
        });
        let mut sizes: Vec<(String, Option<String>)> = vec![(tr!("All of it").into(), None)];
        sizes.extend(gpu::VRAM_CHOICES.iter().map(|mb| (format!("{} GB", mb / 1024), None)));
        // A size set by hand (in the launch options) stays shown.
        let mut values: Vec<u32> = std::iter::once(0).chain(gpu::VRAM_CHOICES).collect();
        if g.vram > 0 && !values.contains(&g.vram) {
            sizes.push((format!("{} MB", g.vram), None));
            values.push(g.vram);
        }
        v.push(GpuRow::Choice {
            id: "gpu-vram",
            label: tr!("Video memory the game sees"),
            desc: tr!("For DirectX games that misjudge the memory and crash or refuse settings").into(),
            options: sizes,
            selected: values.iter().position(|x| *x == g.vram).unwrap_or(0),
            enabled: !disabled,
            pick: Rc::new(move |this, i, cx| {
                if let Some(mb) = values.get(i) {
                    this.set(gpu::KEY_VRAM, json!(mb), cx);
                }
            }),
        });
        v
    }

    /// The desktop tab: a caption and a card per header.
    pub(super) fn gpu_tab(&mut self, disabled: bool, cx: &mut Context<Self>) -> Div {
        let mut col = column();
        let mut card: Option<Rows> = None;
        for r in self.gpu_rows(disabled) {
            match r {
                GpuRow::Header(text) => {
                    if let Some(c) = card.take() {
                        col = col.child(c.card());
                    }
                    col = col.child(heading(text));
                    card = Some(rows());
                }
                row => {
                    let c = card.take().unwrap_or_default();
                    card = Some(self.gpu_desktop_row(c, row, cx));
                }
            }
        }
        if let Some(c) = card {
            col = col.child(c.card());
        }
        col
    }

    fn gpu_desktop_row(&self, c: Rows, row: GpuRow, cx: &mut Context<Self>) -> Rows {
        match row {
            GpuRow::Header(_) => c,
            GpuRow::Toggle { key, label, desc, enabled } => c.child(self.toggle_row(key, key, false, label, Some(&desc), !enabled, cx)),
            GpuRow::Info { label, value, .. } => c.child(crate::ui::controls::row(label, None, help(&value))),
            GpuRow::Choice { id, label, desc, options, selected, enabled, pick } => {
                let shown = options.get(selected).map(|(n, _)| n.clone()).unwrap_or_default();
                let options = options
                    .into_iter()
                    .enumerate()
                    .map(|(i, (label, note))| SelectOption {
                        label: label.into(),
                        selected: i == selected,
                        dot: None,
                        group: None,
                        note: note.map(Into::into),
                    })
                    .collect();
                let select = self.select_menu_row(id, Some(label), shown, options, !enabled, cx, move |this, i, _, cx| pick(this, i, cx));
                c.field_help(select, vec![help(&desc)])
            }
        }
    }
}

/// The card that draws the game: the picked one, the NVIDIA card for
/// Heroic's "dedicated GPU", else the one the screens start on (listed
/// first). `None`: no card found.
fn gpu_target<'a>(m: &'a Machine, g: &Gpu) -> Option<&'a Card> {
    if let Some(c) = m.card(&g.device) {
        return Some(c);
    }
    let nvidia = |offload: bool| m.cards.iter().find(|c| c.vendor_id == gpu::NVIDIA && (!offload || !c.primary));
    match g.device.is_empty() && g.dedicated {
        true => nvidia(true).or_else(|| nvidia(false)).or(m.cards.first()),
        false => m.cards.first(),
    }
}
