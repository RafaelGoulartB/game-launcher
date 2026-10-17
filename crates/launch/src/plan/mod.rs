//! Launch plans: pure data describing *how* a game would be started.
//!
//! Building a plan never spawns anything. The [`crate::runtime::Launcher`]
//! executes plans through a [`crate::runtime::Spawner`].

mod build;
mod context;
mod emulator;
mod types;

#[cfg(test)]
mod tests;

pub use build::{build_plan, build_plan_with_settings};
pub(crate) use build::{native_plan, split_args};
pub use context::{LaunchOptions, PlanContext};
pub use emulator::{emulator_executable, open_emulator_command, pick_rom_path};
pub use types::{CloudSaves, Command, CompanionPlan, FirstLaunch, LaunchPlan, PrepSpec, ProcessPlan, SetupStep, StopSpec, WineRuntime};
