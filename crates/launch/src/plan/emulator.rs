//! Plans for emulated games (port of Heroic's `emulation/launch.ts`).

use heronite_data::models::{GameInfo, LocalGameMeta};
use heronite_i18n::tr;

use super::{Command, LaunchOptions, LaunchPlan, PlanContext, ProcessPlan, StopSpec};
use crate::emulators::{
    emulator_definition, expand_argv, node_basename, node_dirname, resolve_profile_template, vars_for_rom, UserEmulator,
};
use crate::env::Env;
use crate::probe::Probe;
use crate::settings::GameSettings;

#[allow(clippy::large_enum_variant)]
pub(super) enum EmulatorOutcome {
    Plan(LaunchPlan),
    Wine { executable: String, args: Vec<String> },
}

/// `pickRomPath`.
pub fn pick_rom_path(meta: &LocalGameMeta, extra_args: &[String], probe: &dyn Probe) -> Option<String> {
    let requested =
        extra_args.first().filter(|s| !s.is_empty()).cloned().or_else(|| meta.selected_rom_path.clone().filter(|s| !s.is_empty()));
    if let Some(req) = &requested {
        if meta.roms.as_ref().map(|r| r.iter().any(|rom| &rom.path == req)).unwrap_or(false) {
            return Some(req.clone());
        }
        if probe.exists(req) {
            return Some(req.clone());
        }
    }
    meta.roms.as_ref()?.iter().find(|r| probe.exists(&r.path)).map(|r| r.path.clone())
}

/// `emulatorExecutablePath`: `flatpak` for flatpak emulators, else the
/// emulator's executable, else the game's remapped executable.
pub fn emulator_executable(ctx: &PlanContext, meta: &LocalGameMeta) -> Option<String> {
    let emulator = meta.emulator_id.as_deref().and_then(|id| ctx.emulators.get(id));
    if let Some(e) = emulator {
        if e.launch_kind == "flatpak" {
            return Some("flatpak".into());
        }
        if !e.executable.is_empty() {
            return Some(e.executable.clone());
        }
    }
    meta.remapped_executable.clone().filter(|s| !s.is_empty())
}

/// The emulator on its own, without a game (the Emulators page's "Open"):
/// `flatpak run <id>`, the Windows executable in the default Wine prefix, or
/// the executable from its folder. `settings` are the global defaults.
pub fn open_emulator_command(ctx: &PlanContext, probe: &dyn Probe, settings: &GameSettings, e: &UserEmulator) -> Result<Command, String> {
    if e.launch_kind == "flatpak" {
        let id = e.flatpak_id.clone().filter(|f| !f.is_empty()).ok_or_else(|| tr!("This emulator has no Flatpak id").to_string())?;
        return Ok(Command { program: "flatpak".into(), args: vec!["run".into(), id], env: Env::new(), cwd: None });
    }
    if e.executable.is_empty() || !probe.exists(&e.executable) {
        return Err(tr!("Emulator executable was not found").into());
    }
    if e.launch_kind == "wine" || e.executable.to_lowercase().ends_with(".exe") {
        return crate::wine_tools::run_exe_command(ctx, probe, settings, &e.executable, &[]).map(|t| t.command);
    }
    let cwd = e.install_dir.clone().filter(|d| !d.is_empty()).unwrap_or_else(|| node_dirname(&e.executable));
    Ok(Command { program: e.executable.clone(), args: vec![], env: Env::new(), cwd: Some(cwd) })
}

pub(super) fn emulator_plan(
    ctx: &PlanContext,
    probe: &dyn Probe,
    game: &GameInfo,
    meta: &LocalGameMeta,
    _settings: &GameSettings,
    opts: &LaunchOptions,
) -> EmulatorOutcome {
    let unavailable = |s: &str| EmulatorOutcome::Plan(LaunchPlan::Unavailable(s.to_string()));
    let emulator = meta.emulator_id.as_deref().and_then(|id| ctx.emulators.get(id));
    let profile = emulator.and_then(|e| e.profile(meta.emulator_profile_id.as_deref().unwrap_or("")));
    let template = emulator.zip(profile).map(|(e, p)| resolve_profile_template(e, p));

    let rom_path = pick_rom_path(meta, &opts.extra_args, probe);
    let is_flatpak = emulator.map(|e| e.launch_kind == "flatpak").unwrap_or(false);
    let Some(executable) = emulator_executable(ctx, meta) else {
        return unavailable(tr!("Emulator executable was not found"));
    };
    if !is_flatpak && !probe.exists(&executable) {
        return unavailable(tr!("Emulator executable was not found"));
    }
    let has_roms = meta.roms.as_ref().map(|r| !r.is_empty()).unwrap_or(false);
    if has_roms && rom_path.is_none() {
        return unavailable(tr!("ROM file was not found"));
    }

    let emulator_dir = emulator.and_then(|e| e.install_dir.clone().filter(|d| !d.is_empty())).unwrap_or_else(|| node_dirname(&executable));
    let core_path = template.as_ref().and_then(|t| t.core_path.clone()).or_else(|| profile.and_then(|p| p.core_path.clone()));
    let vars = vars_for_rom(rom_path.as_deref().unwrap_or(""), Some(&emulator_dir), core_path.as_deref(), &meta.title);
    let argv_template = template
        .as_ref()
        .map(|t| t.arguments.clone())
        .filter(|s| !s.is_empty())
        .or_else(|| meta.launcher_args.clone().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "{ImagePath}".to_string());
    let mut argv = expand_argv(&argv_template, &vars);
    if let Some(rom) = &rom_path {
        for r in meta.roms.iter().flatten() {
            for part in argv.iter_mut() {
                if *part == r.path {
                    *part = rom.clone();
                }
            }
        }
        if !argv.contains(rom) && !argv_template.contains("{Image") {
            argv.push(rom.clone());
        }
    }

    if !opts.state_args.is_empty() || opts.state_replaces_image {
        if opts.state_replaces_image {
            argv.retain(|a| Some(a) != rom_path.as_ref());
        }
        argv.splice(0..0, opts.state_args.iter().cloned());
    }

    let use_wine = emulator.map(|e| e.launch_kind == "wine").unwrap_or(false) || executable.to_lowercase().ends_with(".exe");
    if use_wine {
        return EmulatorOutcome::Wine { executable, args: argv };
    }

    let (program, args, dir) = match emulator {
        Some(e) if e.launch_kind == "flatpak" && e.flatpak_id.as_deref().map(|f| !f.is_empty()).unwrap_or(false) => {
            let mut a = vec!["run".to_string(), e.flatpak_id.clone().unwrap()];
            a.extend(argv);
            ("flatpak".to_string(), a, String::new())
        }
        _ => {
            let dir = node_dirname(&executable);
            (executable.clone(), argv, dir)
        }
    };
    let cwd = if template.as_ref().and_then(|t| t.working_dir.as_deref()) == Some("rom") && rom_path.is_some() {
        Some(node_dirname(rom_path.as_deref().unwrap()))
    } else {
        emulator.and_then(|e| e.install_dir.clone()).filter(|d| !d.is_empty()).or_else(|| (!dir.is_empty()).then_some(dir))
    };

    // stopPattern: catalog pattern, else the emulator binary's name.
    let definition = emulator.and_then(|e| emulator_definition(&e.definition_id));
    let pattern = definition
        .and_then(|d| d.stop_pattern.map(str::to_string))
        .or_else(|| emulator.filter(|e| !e.executable.is_empty()).map(|e| node_basename(&e.executable)));
    let mut kill_patterns: Vec<String> = pattern.into_iter().collect();
    if let Some(e) = emulator.filter(|e| e.launch_kind == "flatpak") {
        if let Some(last) = e.flatpak_id.as_deref().and_then(|f| f.rsplit('.').next()).filter(|s| !s.is_empty()) {
            kill_patterns.push(last.to_string());
        }
    }

    EmulatorOutcome::Plan(LaunchPlan::Emulator {
        process: ProcessPlan {
            command: Command { program, args, env: Env::new(), cwd },
            stop: StopSpec { kill_patterns, commands: vec![] },
            ..Default::default()
        },
        emulator: emulator
            .map(|e| e.name.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| meta.emulator_name.clone().filter(|n| !n.is_empty()))
            .unwrap_or_else(|| game.title.clone()),
        rom: rom_path,
    })
}
