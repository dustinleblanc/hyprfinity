use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub(crate) struct Cli {
    /// Enable verbose debug output.
    #[arg(long, global = true, default_value_t = false)]
    pub(crate) verbose: bool,
    /// Enable diagnostic logging to a file.
    #[arg(long, global = true, default_value_t = false)]
    pub(crate) debug: bool,
    /// Path to debug log file (used with --debug). Overrides HYPRFINITY_DEBUG_LOG.
    #[arg(long, global = true)]
    pub(crate) debug_log: Option<String>,
    /// Path to a config file (TOML). Defaults to $XDG_CONFIG_HOME/hyprfinity/config.toml.
    #[arg(long, global = true)]
    pub(crate) config: Option<String>,
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Commands {
    /// Launch and span a Gamescope session across all physical monitors.
    GamescopeUp {
        /// Seconds to wait for the Gamescope window to appear.
        #[arg(long, default_value_t = 10)]
        startup_timeout_secs: u64,
        /// Do not pin the Gamescope window to all workspaces.
        #[arg(long, default_value_t = false)]
        no_pin: bool,
        /// Open an interactive picker even if a game/app command is provided.
        #[arg(long, default_value_t = false)]
        pick: bool,
        /// Stop Waybar while Gamescope is active, then restore it on exit.
        #[arg(long, default_value_t = false)]
        hide_waybar: bool,
        /// Open an interactive picker for internal (virtual) render size.
        #[arg(long, default_value_t = false)]
        pick_size: bool,
        /// Scale internal (virtual) render size relative to monitor span (e.g. 0.75 for 75%).
        #[arg(long)]
        render_scale: Option<f32>,
        /// Internal (virtual) render width for Gamescope (-w).
        #[arg(long)]
        virtual_width: Option<i32>,
        /// Internal (virtual) render height for Gamescope (-h).
        #[arg(long)]
        virtual_height: Option<i32>,
        /// Arguments passed to gamescope. Use `--` to separate gamescope args from the game command.
        #[arg(trailing_var_arg = true)]
        gamescope_args: Vec<String>,
    },
    /// Tear down the active Gamescope session launched by GamescopeUp.
    GamescopeDown,
    /// Create a starter config file.
    ConfigInit {
        /// Overwrite existing config if present (skip overwrite prompt).
        #[arg(long, default_value_t = false)]
        force: bool,
    },
    /// Interactively configure output and internal render sizes.
    Config,
    /// Print resolved config (path + values).
    ConfigShow {
        /// Override no-pin in effective output.
        #[arg(long, default_value_t = false)]
        no_pin: bool,
        /// Override pick in effective output.
        #[arg(long, default_value_t = false)]
        pick: bool,
        /// Override hide-waybar in effective output.
        #[arg(long, default_value_t = false)]
        hide_waybar: bool,
        /// Override pick-size in effective output.
        #[arg(long, default_value_t = false)]
        pick_size: bool,
        /// Override render scale in effective output.
        #[arg(long)]
        render_scale: Option<f32>,
        /// Override virtual width in effective output.
        #[arg(long)]
        virtual_width: Option<i32>,
        /// Override virtual height in effective output.
        #[arg(long)]
        virtual_height: Option<i32>,
        /// Override startup timeout in effective output.
        #[arg(long, default_value_t = 10)]
        startup_timeout_secs: u64,
        /// Arguments passed to gamescope (for effective output). Use `--` to separate gamescope args.
        #[arg(trailing_var_arg = true)]
        gamescope_args: Vec<String>,
    },
}
