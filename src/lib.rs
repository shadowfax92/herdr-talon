use anyhow::Result;
use clap::{Parser, Subcommand};

pub mod actions;
pub mod app;
pub mod config;
pub mod document;
pub mod herdr;
pub mod hints;
pub mod keybindings;
pub mod matcher;
pub mod picker;
pub mod snapshot;
pub mod ui;

pub const PLUGIN_ID: &str = "shadowfax.talon";

#[derive(Debug, Parser)]
#[command(name = "herdr-talon", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Launch,
    Picker,
    InstallKeybindings,
}

pub fn run(command: Command) -> Result<()> {
    match command {
        Command::Launch => {
            snapshot::launch_from_environment()?;
            Ok(())
        }
        Command::Picker => picker::run_from_environment(),
        Command::InstallKeybindings => {
            keybindings::install_from_environment()?;
            Ok(())
        }
    }
}
