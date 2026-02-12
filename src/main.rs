use clap::Parser;
use std::{error::Error, fmt};

mod autotune;
mod cli;
mod config;
mod debuglog;
mod gamescope;
mod hyprland;
mod picker;
mod tui_config;
mod types;
mod util;

use crate::cli::{Cli, Commands};
use crate::config::{
    apply_config, interactive_config, load_config, show_config, write_default_config,
};
use crate::debuglog::init_debug_logging;
use crate::gamescope::{gamescope_down, gamescope_up};

#[derive(Debug)]
struct MyError(String);

impl fmt::Display for MyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for MyError {}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    init_debug_logging(cli.debug, &cli.debug_log)?;
    let config = load_config(&cli.config)?;

    match &cli.command {
        Some(Commands::GamescopeUp {
            startup_timeout_secs,
            no_pin,
            pick,
            hide_waybar,
            pick_size,
            render_scale,
            virtual_width,
            virtual_height,
            gamescope_args,
        }) => {
            println!("Hyprfinity: Launching Gamescope span session...");
            let launch = apply_config(
                gamescope_args,
                *no_pin,
                *pick,
                *hide_waybar,
                *pick_size,
                *render_scale,
                *virtual_width,
                *virtual_height,
                *startup_timeout_secs,
                &config,
            );
            gamescope_up(
                &launch.args,
                launch.timeout,
                launch.no_pin,
                launch.pick,
                launch.hide_waybar,
                launch.pick_size,
                launch.render_scale,
                launch.virtual_width,
                launch.virtual_height,
                launch.output_width,
                launch.output_height,
                cli.verbose,
            )
        }
        None => {
            println!("Hyprfinity: Launching Gamescope span session...");
            let launch = apply_config(
                &[],
                false,
                false,
                false,
                false,
                None,
                None,
                None,
                10,
                &config,
            );
            gamescope_up(
                &launch.args,
                launch.timeout,
                launch.no_pin,
                launch.pick,
                launch.hide_waybar,
                launch.pick_size,
                launch.render_scale,
                launch.virtual_width,
                launch.virtual_height,
                launch.output_width,
                launch.output_height,
                cli.verbose,
            )
        }
        Some(Commands::Config) => interactive_config(&cli.config, cli.verbose),
        Some(Commands::GamescopeDown) => {
            println!("Hyprfinity: Tearing down Gamescope session...");
            gamescope_down()
        }
        Some(Commands::ConfigInit { force }) => {
            write_default_config(&cli.config, *force)?;
            Ok(())
        }
        Some(Commands::ConfigShow {
            no_pin,
            pick,
            hide_waybar,
            pick_size,
            render_scale,
            virtual_width,
            virtual_height,
            startup_timeout_secs,
            gamescope_args,
        }) => {
            show_config(
                &cli.config,
                gamescope_args,
                *no_pin,
                *pick,
                *hide_waybar,
                *pick_size,
                *render_scale,
                *virtual_width,
                *virtual_height,
                *startup_timeout_secs,
            )?;
            Ok(())
        }
    }
}
