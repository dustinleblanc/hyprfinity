use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use skim::prelude::*;
use std::collections::BTreeSet;
use std::{
    error::Error,
    fmt,
    io::Write,
    process::{Command, Stdio},
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

#[derive(Debug)]
struct MyError(String);

impl fmt::Display for MyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for MyError {}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Enable verbose debug output.
    #[arg(long, global = true, default_value_t = false)]
    verbose: bool,
    /// Path to a config file (TOML). Defaults to $XDG_CONFIG_HOME/hyprfinity/config.toml.
    #[arg(long, global = true)]
    config: Option<String>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
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
        /// Overwrite existing config if present.
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

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Monitor {
    name: Option<String>,
    width: i32,
    height: i32,
    x: i32,
    y: i32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct GamescopeState {
    gamescope_pid: u32,
    span_x: i32,
    span_y: i32,
    span_width: i32,
    span_height: i32,
    gamescope_args: Vec<String>,
    waybar_was_stopped: bool,
}

const GAMESCOPE_STATE_FILE_NAME: &str = "hyprfinity_gamescope_state.json";
const DEFAULT_CONFIG_REL_PATH: &str = "hyprfinity/config.toml";

fn get_gamescope_state_file_path() -> Result<std::path::PathBuf, Box<dyn Error>> {
    let temp_dir = std::env::temp_dir();
    Ok(temp_dir.join(GAMESCOPE_STATE_FILE_NAME))
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
struct Config {
    gamescope_args: Option<Vec<String>>,
    default_command: Option<Vec<String>>,
    no_pin: Option<bool>,
    pick: Option<bool>,
    hide_waybar: Option<bool>,
    pick_size: Option<bool>,
    render_scale: Option<f32>,
    virtual_width: Option<i32>,
    virtual_height: Option<i32>,
    output_width: Option<i32>,
    output_height: Option<i32>,
    startup_timeout_secs: Option<u64>,
}

fn resolve_default_config_path() -> Result<std::path::PathBuf, Box<dyn Error>> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Ok(std::path::PathBuf::from(xdg).join(DEFAULT_CONFIG_REL_PATH));
    }
    if let Ok(home) = std::env::var("HOME") {
        return Ok(std::path::PathBuf::from(home)
            .join(".config")
            .join(DEFAULT_CONFIG_REL_PATH));
    }
    Err(
        MyError("Unable to resolve config path (HOME and XDG_CONFIG_HOME are unset).".to_string())
            .into(),
    )
}

fn load_config(path_override: &Option<String>) -> Result<Config, Box<dyn Error>> {
    let path = if let Some(path) = path_override {
        std::path::PathBuf::from(path)
    } else {
        resolve_default_config_path()?
    };

    if !path.exists() {
        return Ok(Config::default());
    }

    let contents = std::fs::read_to_string(&path)?;
    let config: Config = toml::from_str(&contents)
        .map_err(|e| MyError(format!("Failed to parse config {}: {}", path.display(), e)))?;
    Ok(config)
}

fn resolve_config_path(
    path_override: &Option<String>,
) -> Result<std::path::PathBuf, Box<dyn Error>> {
    if let Some(path) = path_override {
        Ok(std::path::PathBuf::from(path))
    } else {
        resolve_default_config_path()
    }
}

fn write_default_config(path_override: &Option<String>, force: bool) -> Result<(), Box<dyn Error>> {
    let path = resolve_config_path(path_override)?;

    if path.exists() && !force {
        return Err(MyError(format!(
            "Config already exists at {} (use --force to overwrite).",
            path.display()
        ))
        .into());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let contents = r#"# Hyprfinity config

# Default gamescope args (used when no args are provided on the CLI)
gamescope_args = ["-r", "60"]

# Default game/app command (appended if no `--` command is provided)
default_command = ["steam", "-applaunch", "620"]

# Defaults for CLI flags
no_pin = false
pick = false
hide_waybar = true
pick_size = false
# Internal render scale relative to output span; 1.0 = native span.
render_scale = 1.0
# Optional explicit internal render size (when set, these take precedence over render_scale).
# virtual_width = 5760
# virtual_height = 1080
# Optional explicit output size for Gamescope (-W/-H). Default is full monitor span.
# output_width = 7680
# output_height = 1440
startup_timeout_secs = 10
"#;

    std::fs::write(&path, contents)?;
    println!("Hyprfinity: Wrote config to {}", path.display());
    Ok(())
}

fn show_config(
    path_override: &Option<String>,
    cli_args: &[String],
    cli_no_pin: bool,
    cli_pick: bool,
    cli_hide_waybar: bool,
    cli_pick_size: bool,
    cli_render_scale: Option<f32>,
    cli_virtual_width: Option<i32>,
    cli_virtual_height: Option<i32>,
    cli_timeout: u64,
) -> Result<(), Box<dyn Error>> {
    let path = resolve_config_path(path_override)?;
    let config = load_config(path_override)?;

    let launch = apply_config(
        cli_args,
        cli_no_pin,
        cli_pick,
        cli_hide_waybar,
        cli_pick_size,
        cli_render_scale,
        cli_virtual_width,
        cli_virtual_height,
        cli_timeout,
        &config,
    );

    println!("Hyprfinity: Config path: {}", path.display());
    println!("Hyprfinity: Effective values (after CLI overrides):");
    println!("  gamescope_args = {:?}", launch.args);
    println!("  no_pin = {}", launch.no_pin);
    println!("  pick = {}", launch.pick);
    println!("  hide_waybar = {}", launch.hide_waybar);
    println!("  pick_size = {}", launch.pick_size);
    println!("  render_scale = {}", launch.render_scale);
    println!("  virtual_width = {:?}", launch.virtual_width);
    println!("  virtual_height = {:?}", launch.virtual_height);
    println!("  output_width = {:?}", launch.output_width);
    println!("  output_height = {:?}", launch.output_height);
    println!("  startup_timeout_secs = {}", launch.timeout);

    println!("Hyprfinity: Raw config values:");
    println!(
        "  gamescope_args = {:?}",
        config.gamescope_args.unwrap_or_default()
    );
    println!(
        "  default_command = {:?}",
        config.default_command.unwrap_or_default()
    );
    println!("  no_pin = {}", config.no_pin.unwrap_or(false));
    println!("  pick = {}", config.pick.unwrap_or(false));
    println!("  hide_waybar = {}", config.hide_waybar.unwrap_or(true));
    println!("  pick_size = {}", config.pick_size.unwrap_or(false));
    println!("  render_scale = {}", config.render_scale.unwrap_or(1.0));
    println!("  virtual_width = {:?}", config.virtual_width);
    println!("  virtual_height = {:?}", config.virtual_height);
    println!("  output_width = {:?}", config.output_width);
    println!("  output_height = {:?}", config.output_height);
    println!(
        "  startup_timeout_secs = {}",
        config.startup_timeout_secs.unwrap_or(10)
    );
    Ok(())
}

fn save_gamescope_state(state: &GamescopeState) -> Result<(), Box<dyn Error>> {
    let path = get_gamescope_state_file_path()?;
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(&path, json)?;
    println!("Hyprfinity: Saved Gamescope state to {:?}", path);
    Ok(())
}

fn load_gamescope_state() -> Result<GamescopeState, Box<dyn Error>> {
    let path = get_gamescope_state_file_path()?;
    let json = std::fs::read_to_string(&path)?;
    let state: GamescopeState = serde_json::from_str(&json)?;
    println!("Hyprfinity: Loaded Gamescope state from {:?}", path);
    Ok(state)
}

fn execute_hyprctl(args: &[&str], verbose: bool) -> Result<(), Box<dyn Error>> {
    if verbose {
        println!(
            "Hyprfinity (DEBUG): Executing hyprctl with args: {:?}",
            args
        );
    }
    let output = Command::new("hyprctl").args(args).output()?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if verbose {
        println!("Hyprfinity (DEBUG): hyprctl stdout: {}", stdout.trim());
        println!("Hyprfinity (DEBUG): hyprctl stderr: {}", stderr.trim());
        println!("Hyprfinity (DEBUG): hyprctl exit status: {}", output.status);
    }

    if !output.status.success() {
        return Err(MyError(format!("hyprctl failed for args {:?}: {}", args, stderr)).into());
    }
    Ok(())
}

fn execute_hyprctl_output(args: &[&str], verbose: bool) -> Result<String, Box<dyn Error>> {
    if verbose {
        println!(
            "Hyprfinity (DEBUG): Executing hyprctl with args: {:?}",
            args
        );
    }
    let output = Command::new("hyprctl").args(args).output()?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if verbose {
        println!("Hyprfinity (DEBUG): hyprctl stdout: {}", stdout.trim());
        println!("Hyprfinity (DEBUG): hyprctl stderr: {}", stderr.trim());
        println!("Hyprfinity (DEBUG): hyprctl exit status: {}", output.status);
    }

    if !output.status.success() {
        return Err(MyError(format!("hyprctl failed for args {:?}: {}", args, stderr)).into());
    }
    Ok(stdout)
}

fn get_monitors(verbose: bool) -> Result<Vec<Monitor>, Box<dyn Error>> {
    let stdout = execute_hyprctl_output(&["monitors", "-j"], verbose)?;
    let monitors: Vec<Monitor> = serde_json::from_str(&stdout)
        .map_err(|e| MyError(format!("Failed to parse hyprctl output: {}", e)))?;

    if monitors.is_empty() {
        return Err(MyError("No monitors detected. Is Hyprland running?".to_string()).into());
    }
    Ok(monitors)
}

fn compute_monitor_span(monitors: &[Monitor]) -> Result<(i32, i32, i32, i32), Box<dyn Error>> {
    if monitors.is_empty() {
        return Err(MyError("No monitors detected.".to_string()).into());
    }

    let min_x = monitors.iter().map(|m| m.x).min().unwrap_or(0);
    let min_y = monitors.iter().map(|m| m.y).min().unwrap_or(0);
    let max_x = monitors.iter().map(|m| m.x + m.width).max().unwrap_or(0);
    let max_y = monitors.iter().map(|m| m.y + m.height).max().unwrap_or(0);

    let span_width = max_x - min_x;
    let span_height = max_y - min_y;

    Ok((min_x, min_y, span_width, span_height))
}

#[derive(Debug, Deserialize)]
struct Client {
    pid: i32,
    #[serde(default)]
    at: Option<[i32; 2]>,
    #[serde(default)]
    size: Option<[i32; 2]>,
}

fn wait_for_client_pid(pid: u32, timeout_secs: u64, verbose: bool) -> Result<(), Box<dyn Error>> {
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
    while std::time::Instant::now() < deadline {
        let stdout = execute_hyprctl_output(&["clients", "-j"], verbose)?;
        let clients: Vec<Client> = serde_json::from_str(&stdout)
            .map_err(|e| MyError(format!("Failed to parse hyprctl clients output: {}", e)))?;

        if clients.iter().any(|c| c.pid == pid as i32) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }

    Err(MyError(format!(
        "Timed out waiting for Gamescope window (PID {}).",
        pid
    ))
    .into())
}

fn get_client_geometry(
    pid: u32,
    verbose: bool,
) -> Result<Option<(i32, i32, i32, i32)>, Box<dyn Error>> {
    let stdout = execute_hyprctl_output(&["clients", "-j"], verbose)?;
    let clients: Vec<Client> = serde_json::from_str(&stdout)
        .map_err(|e| MyError(format!("Failed to parse hyprctl clients output: {}", e)))?;
    let client = clients.iter().find(|c| c.pid == pid as i32);
    if let Some(c) = client {
        if let (Some(at), Some(size)) = (c.at, c.size) {
            return Ok(Some((at[0], at[1], size[0], size[1])));
        }
    }
    Ok(None)
}

fn fit_window_to_span(
    pid: u32,
    window: &str,
    target_x: i32,
    target_y: i32,
    target_w: i32,
    target_h: i32,
    verbose: bool,
) -> Result<(), Box<dyn Error>> {
    let mut req_w = target_w;
    let mut req_h = target_h;

    for attempt in 1..=4 {
        let move_params = format!("exact {} {}", target_x, target_y);
        execute_hyprctl(
            &[
                "dispatch",
                "movewindowpixel",
                &format!("{},{}", move_params, window),
            ],
            verbose,
        )?;

        let resize_params = format!("exact {} {}", req_w, req_h);
        execute_hyprctl(
            &[
                "dispatch",
                "resizewindowpixel",
                &format!("{},{}", resize_params, window),
            ],
            verbose,
        )?;

        thread::sleep(Duration::from_millis(80));

        let Some((x, y, w, h)) = get_client_geometry(pid, verbose)? else {
            continue;
        };
        let pos_ok = (x - target_x).abs() <= 1 && (y - target_y).abs() <= 1;
        let size_ok = (w - target_w).abs() <= 1 && (h - target_h).abs() <= 1;
        if pos_ok && size_ok {
            if verbose {
                println!(
                    "Hyprfinity (DEBUG): Window fit success on attempt {}: at=({}, {}), size={}x{}",
                    attempt, x, y, w, h
                );
            }
            return Ok(());
        }

        // Compensate for decorations/size hints by increasing requested size by observed delta.
        req_w = (req_w + (target_w - w)).max(2);
        req_h = (req_h + (target_h - h)).max(2);
        if verbose {
            println!(
                "Hyprfinity (DEBUG): Window fit attempt {} mismatch: at=({}, {}), size={}x{}, target=({}, {}) {}x{}, next request={}x{}",
                attempt, x, y, w, h, target_x, target_y, target_w, target_h, req_w, req_h
            );
        }
    }

    if let Some((x, y, w, h)) = get_client_geometry(pid, verbose)? {
        eprintln!(
            "Hyprfinity: Warning: Gamescope window may not fully cover span (actual at=({}, {}), size={}x{}; target at=({}, {}), size={}x{}).",
            x, y, w, h, target_x, target_y, target_w, target_h
        );
    } else {
        eprintln!("Hyprfinity: Warning: Unable to verify final Gamescope window geometry.");
    }
    Ok(())
}

fn has_arg(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| {
        arg == flag
            || arg.starts_with(&format!("{flag}="))
            || (flag.len() == 2 && arg.starts_with(flag) && arg.len() > 2)
    })
}

fn build_gamescope_args_with_internal(
    args: &[String],
    span_width: i32,
    span_height: i32,
    internal_width: i32,
    internal_height: i32,
) -> Vec<String> {
    let mut pre: Vec<String> = Vec::new();
    let mut post: Vec<String> = Vec::new();

    if let Some(idx) = args.iter().position(|a| a == "--") {
        pre.extend(args[..idx].iter().cloned());
        post.extend(args[idx..].iter().cloned());
    } else {
        pre.extend(args.iter().cloned());
    }

    let has_output_w = has_arg(&pre, "-W") || has_arg(&pre, "--output-width");
    let has_output_h = has_arg(&pre, "-H") || has_arg(&pre, "--output-height");
    let has_nested_w = has_arg(&pre, "-w") || has_arg(&pre, "--nested-width");
    let has_nested_h = has_arg(&pre, "-h") || has_arg(&pre, "--nested-height");

    if !has_output_w {
        pre.push("-W".to_string());
        pre.push(span_width.to_string());
    }
    if !has_output_h {
        pre.push("-H".to_string());
        pre.push(span_height.to_string());
    }
    if !has_nested_w {
        pre.push("-w".to_string());
        pre.push(internal_width.to_string());
    }
    if !has_nested_h {
        pre.push("-h".to_string());
        pre.push(internal_height.to_string());
    }

    pre.extend(post.into_iter());
    pre
}

#[derive(Debug, Clone)]
struct LaunchSettings {
    args: Vec<String>,
    no_pin: bool,
    pick: bool,
    hide_waybar: bool,
    pick_size: bool,
    render_scale: f32,
    virtual_width: Option<i32>,
    virtual_height: Option<i32>,
    output_width: Option<i32>,
    output_height: Option<i32>,
    timeout: u64,
}

#[derive(Debug, Clone)]
struct SizePreset {
    label: String,
    width: i32,
    height: i32,
}

fn clamp_i32(v: i32, min: i32, max: i32) -> i32 {
    v.max(min).min(max)
}

fn even_floor(v: i32) -> i32 {
    if v <= 2 {
        2
    } else if v % 2 == 0 {
        v
    } else {
        v - 1
    }
}

fn scaled_dimensions(span_width: i32, span_height: i32, scale: f32) -> (i32, i32) {
    let w = (span_width as f32 * scale).round() as i32;
    let h = (span_height as f32 * scale).round() as i32;
    let w = even_floor(clamp_i32(w, 2, span_width));
    let h = even_floor(clamp_i32(h, 2, span_height));
    (w, h)
}

fn derive_internal_size(
    span_width: i32,
    span_height: i32,
    render_scale: f32,
    virtual_width: Option<i32>,
    virtual_height: Option<i32>,
) -> (i32, i32) {
    match (virtual_width, virtual_height) {
        (Some(w), Some(h)) => (
            even_floor(clamp_i32(w, 2, span_width)),
            even_floor(clamp_i32(h, 2, span_height)),
        ),
        (Some(w), None) => {
            let w = even_floor(clamp_i32(w, 2, span_width));
            let h = ((w as f32 * span_height as f32) / span_width as f32).round() as i32;
            (w, even_floor(clamp_i32(h, 2, span_height)))
        }
        (None, Some(h)) => {
            let h = even_floor(clamp_i32(h, 2, span_height));
            let w = ((h as f32 * span_width as f32) / span_height as f32).round() as i32;
            (even_floor(clamp_i32(w, 2, span_width)), h)
        }
        (None, None) => scaled_dimensions(span_width, span_height, render_scale),
    }
}

fn derive_output_size(
    span_width: i32,
    span_height: i32,
    output_width: Option<i32>,
    output_height: Option<i32>,
) -> (i32, i32) {
    match (output_width, output_height) {
        (Some(w), Some(h)) => (
            even_floor(clamp_i32(w, 2, span_width)),
            even_floor(clamp_i32(h, 2, span_height)),
        ),
        (Some(w), None) => {
            let w = even_floor(clamp_i32(w, 2, span_width));
            let h = ((w as f32 * span_height as f32) / span_width as f32).round() as i32;
            (w, even_floor(clamp_i32(h, 2, span_height)))
        }
        (None, Some(h)) => {
            let h = even_floor(clamp_i32(h, 2, span_height));
            let w = ((h as f32 * span_width as f32) / span_height as f32).round() as i32;
            (even_floor(clamp_i32(w, 2, span_width)), h)
        }
        (None, None) => (span_width, span_height),
    }
}

fn build_size_presets(span_width: i32, span_height: i32) -> Vec<SizePreset> {
    let mut options: Vec<SizePreset> = Vec::new();
    let mut seen: BTreeSet<(i32, i32)> = BTreeSet::new();

    let mut add = |label: String, width: i32, height: i32| {
        if width <= 0 || height <= 0 || width > span_width || height > span_height {
            return;
        }
        if seen.insert((width, height)) {
            options.push(SizePreset {
                label,
                width,
                height,
            });
        }
    };

    add(
        format!("Native span: {}x{} (100%)", span_width, span_height),
        span_width,
        span_height,
    );
    for scale in [0.9_f32, 0.85, 0.8, 0.75, 0.67, 0.6, 0.5] {
        let (w, h) = scaled_dimensions(span_width, span_height, scale);
        add(
            format!("Scaled: {}x{} ({}%)", w, h, (scale * 100.0).round() as i32),
            w,
            h,
        );
    }

    for target_h in [1440_i32, 1200, 1080, 900, 720] {
        if target_h >= span_height {
            continue;
        }
        let w = ((target_h as f32 * span_width as f32) / span_height as f32).round() as i32;
        let w = even_floor(clamp_i32(w, 2, span_width));
        add(
            format!("Common height: {}x{} (~{}p tall)", w, target_h, target_h),
            w,
            target_h,
        );
    }

    options
}

fn pick_internal_size(
    monitors: &[Monitor],
    span_width: i32,
    span_height: i32,
) -> Result<Option<(i32, i32)>, Box<dyn Error>> {
    let monitor_summary = monitors
        .iter()
        .enumerate()
        .map(|(idx, m)| {
            format!(
                "{}:{}x{}@{},{}",
                m.name
                    .clone()
                    .unwrap_or_else(|| format!("monitor{}", idx + 1)),
                m.width,
                m.height,
                m.x,
                m.y
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    println!("Hyprfinity: Detected monitors: {}", monitor_summary);

    let options_data = build_size_presets(span_width, span_height);
    if options_data.is_empty() {
        return Ok(None);
    }

    let options = SkimOptionsBuilder::default()
        .height(Some("70%"))
        .prompt(Some("Select internal size> "))
        .reverse(true)
        .multi(false)
        .build()
        .map_err(|e| MyError(format!("Failed to build skim options: {}", e)))?;

    let input = options_data
        .iter()
        .map(|opt| opt.label.clone())
        .collect::<Vec<String>>()
        .join("\n");
    let reader = SkimItemReader::default();
    let items = reader.of_bufread(std::io::Cursor::new(input));
    let selected = Skim::run_with(&options, Some(items))
        .map(|out| out.selected_items)
        .unwrap_or_default();

    if selected.is_empty() {
        return Ok(None);
    }

    let selected_label = selected[0].output().to_string();
    let selected_opt = options_data
        .iter()
        .find(|o| o.label == selected_label)
        .ok_or_else(|| MyError("Selected size option not found.".to_string()))?;
    Ok(Some((selected_opt.width, selected_opt.height)))
}

#[derive(Debug, Clone)]
struct DesktopApp {
    name: String,
    exec: String,
}

fn list_desktop_apps() -> Result<Vec<DesktopApp>, Box<dyn Error>> {
    let mut apps: Vec<DesktopApp> = Vec::new();

    let mut dirs: Vec<std::path::PathBuf> = vec![
        std::path::PathBuf::from("/usr/share/applications"),
        std::path::PathBuf::from("/usr/local/share/applications"),
    ];
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(std::path::PathBuf::from(home).join(".local/share/applications"));
    }

    for dir in dirs {
        if !dir.exists() {
            continue;
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let mut in_desktop_entry = false;
            let mut name: Option<String> = None;
            let mut exec: Option<String> = None;
            let mut hidden = false;

            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('[') && line.ends_with(']') {
                    in_desktop_entry = line == "[Desktop Entry]";
                    continue;
                }
                if !in_desktop_entry || line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some(rest) = line.strip_prefix("Name=") {
                    if !rest.is_empty() {
                        name = Some(rest.to_string());
                    }
                } else if let Some(rest) = line.strip_prefix("Exec=") {
                    if !rest.is_empty() {
                        exec = Some(rest.to_string());
                    }
                } else if let Some(rest) = line.strip_prefix("NoDisplay=") {
                    if rest.eq_ignore_ascii_case("true") {
                        hidden = true;
                    }
                } else if let Some(rest) = line.strip_prefix("Hidden=") {
                    if rest.eq_ignore_ascii_case("true") {
                        hidden = true;
                    }
                }
            }

            if hidden {
                continue;
            }

            if let (Some(name), Some(exec)) = (name, exec) {
                apps.push(DesktopApp { name, exec });
            }
        }
    }

    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(apps)
}

fn sanitize_exec(exec: &str) -> String {
    let mut cleaned = exec.to_string();
    for token in [
        "%U", "%u", "%F", "%f", "%i", "%c", "%k", "%d", "%D", "%n", "%N", "%v", "%m", "%M", "%r",
        "%R",
    ] {
        cleaned = cleaned.replace(token, "");
    }
    cleaned.trim().to_string()
}

fn pick_desktop_app_command() -> Result<Vec<String>, Box<dyn Error>> {
    let apps = list_desktop_apps()?;
    if apps.is_empty() {
        return Err(MyError("No desktop applications found.".to_string()).into());
    }

    let options = SkimOptionsBuilder::default()
        .height(Some("70%"))
        .prompt(Some("Select app> "))
        .reverse(true)
        .multi(false)
        .build()
        .map_err(|e| MyError(format!("Failed to build skim options: {}", e)))?;

    let input = apps
        .iter()
        .map(|app| app.name.clone())
        .collect::<Vec<String>>()
        .join("\n");

    let reader = SkimItemReader::default();
    let items = reader.of_bufread(std::io::Cursor::new(input));

    let selected = Skim::run_with(&options, Some(items))
        .map(|out| out.selected_items)
        .unwrap_or_default();

    if selected.is_empty() {
        return Err(MyError("User cancelled selection.".to_string()).into());
    }

    let selected_name = selected[0].output().to_string();
    let app = apps
        .iter()
        .find(|a| a.name == selected_name)
        .ok_or_else(|| MyError("Selected app not found.".to_string()))?;

    let exec = sanitize_exec(&app.exec);
    let args = shell_words::split(&exec)
        .map_err(|e| MyError(format!("Failed to parse Exec for {}: {}", app.name, e)))?;
    if args.is_empty() {
        return Err(MyError(format!("No executable found for {}.", app.name)).into());
    }
    Ok(args)
}

fn ensure_game_command(
    mut gamescope_args: Vec<String>,
    pick: bool,
) -> Result<Vec<String>, Box<dyn Error>> {
    let mut need_pick = pick;
    if let Some(idx) = gamescope_args.iter().position(|a| a == "--") {
        if idx == gamescope_args.len() - 1 {
            need_pick = true;
        }
    } else {
        need_pick = true;
    }

    if need_pick {
        let cmd = pick_desktop_app_command()?;
        gamescope_args.push("--".to_string());
        gamescope_args.extend(cmd);
    }

    Ok(gamescope_args)
}

fn apply_config(
    cli_args: &[String],
    cli_no_pin: bool,
    cli_pick: bool,
    cli_hide_waybar: bool,
    cli_pick_size: bool,
    cli_render_scale: Option<f32>,
    cli_virtual_width: Option<i32>,
    cli_virtual_height: Option<i32>,
    cli_timeout: u64,
    config: &Config,
) -> LaunchSettings {
    let mut args = if cli_args.is_empty() {
        config.gamescope_args.clone().unwrap_or_default()
    } else {
        cli_args.to_vec()
    };

    if args.is_empty() {
        args = Vec::new();
    }

    let no_pin = if cli_no_pin {
        true
    } else {
        config.no_pin.unwrap_or(false)
    };

    let pick = if cli_pick {
        true
    } else {
        config.pick.unwrap_or(false)
    };

    let hide_waybar = if cli_hide_waybar {
        true
    } else {
        config.hide_waybar.unwrap_or(true)
    };

    let pick_size = if cli_pick_size {
        true
    } else {
        config.pick_size.unwrap_or(false)
    };

    let mut render_scale = cli_render_scale.or(config.render_scale).unwrap_or(1.0);
    if !(0.1..=1.0).contains(&render_scale) {
        eprintln!(
            "Hyprfinity: render_scale {} is out of range; clamping to [0.1, 1.0].",
            render_scale
        );
        render_scale = render_scale.clamp(0.1, 1.0);
    }

    let virtual_width = cli_virtual_width.or(config.virtual_width);
    let virtual_height = cli_virtual_height.or(config.virtual_height);
    let output_width = config.output_width;
    let output_height = config.output_height;

    let timeout = if cli_timeout != 10 {
        cli_timeout
    } else {
        config.startup_timeout_secs.unwrap_or(10)
    };

    if config.default_command.is_some() && !args.iter().any(|a| a == "--") {
        args.push("--".to_string());
        args.extend(config.default_command.clone().unwrap_or_default());
    }

    LaunchSettings {
        args,
        no_pin,
        pick,
        hide_waybar,
        pick_size,
        render_scale,
        virtual_width,
        virtual_height,
        output_width,
        output_height,
        timeout,
    }
}

fn gamescope_up(
    gamescope_args: &[String],
    startup_timeout_secs: u64,
    no_pin: bool,
    pick: bool,
    hide_waybar: bool,
    pick_size: bool,
    render_scale: f32,
    virtual_width: Option<i32>,
    virtual_height: Option<i32>,
    output_width: Option<i32>,
    output_height: Option<i32>,
    verbose: bool,
) -> Result<(), Box<dyn Error>> {
    let monitors = get_monitors(verbose)?;
    let (span_x, span_y, span_width, span_height) = compute_monitor_span(&monitors)?;

    println!(
        "Hyprfinity: Computed monitor span: origin=({}, {}), size={}x{}",
        span_x, span_y, span_width, span_height
    );

    let waybar_was_stopped = if hide_waybar {
        maybe_stop_waybar(verbose)?
    } else {
        false
    };

    let gamescope_args = ensure_game_command(gamescope_args.to_vec(), pick)?;
    let output = derive_output_size(span_width, span_height, output_width, output_height);
    let mut internal = derive_internal_size(
        output.0,
        output.1,
        render_scale,
        virtual_width,
        virtual_height,
    );
    if pick_size {
        if let Some(selected) = pick_internal_size(&monitors, span_width, span_height)? {
            internal = selected;
        } else {
            println!("Hyprfinity: Internal size picker cancelled, using configured/default size.");
        }
    }

    println!(
        "Hyprfinity: Internal render size: {}x{} (output span {}x{})",
        internal.0, internal.1, output.0, output.1
    );

    let final_args = build_gamescope_args_with_internal(
        &gamescope_args,
        output.0,
        output.1,
        internal.0,
        internal.1,
    );
    println!(
        "Hyprfinity: Launching gamescope with args: {:?}",
        final_args
    );

    let mut cmd = Command::new("gamescope");
    cmd.args(&final_args);
    if !verbose {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    let mut child = cmd.spawn()?;

    let gamescope_pid = child.id();
    println!("Hyprfinity: gamescope started with PID {}.", gamescope_pid);

    wait_for_client_pid(gamescope_pid, startup_timeout_secs, verbose)?;

    let window = format!("pid:{}", gamescope_pid);
    execute_hyprctl(&["dispatch", "setfloating", &window], verbose)?;
    fit_window_to_span(
        gamescope_pid,
        &window,
        span_x,
        span_y,
        span_width,
        span_height,
        verbose,
    )?;

    if !no_pin {
        execute_hyprctl(&["dispatch", "pin", &window], verbose)?;
    }

    let state = GamescopeState {
        gamescope_pid,
        span_x,
        span_y,
        span_width,
        span_height,
        gamescope_args: final_args,
        waybar_was_stopped,
    };
    save_gamescope_state(&state)?;

    let shutting_down = Arc::new(AtomicBool::new(false));
    {
        let shutting_down = Arc::clone(&shutting_down);
        ctrlc::set_handler(move || {
            if shutting_down.swap(true, Ordering::SeqCst) {
                return;
            }
            println!("\nHyprfinity: Ctrl+C received, tearing down Gamescope session...");
            if let Err(e) = gamescope_down() {
                eprintln!("Hyprfinity: Failed to tear down Gamescope session: {}", e);
            }
            std::process::exit(130);
        })?;
    }

    println!("Hyprfinity: Gamescope is running. Press Ctrl+C to stop.");
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            println!("Hyprfinity: Gamescope exited with status {}.", status);
            if waybar_was_stopped {
                maybe_start_waybar(verbose)?;
            }
            let state_file_path = get_gamescope_state_file_path()?;
            let _ = std::fs::remove_file(&state_file_path);
            break;
        }
        thread::sleep(Duration::from_secs(1));
    }

    Ok(())
}

fn gamescope_down() -> Result<(), Box<dyn Error>> {
    let state = load_gamescope_state()?;
    println!(
        "Hyprfinity: Stopping gamescope PID {}...",
        state.gamescope_pid
    );
    match Command::new("kill")
        .arg(state.gamescope_pid.to_string())
        .status()
    {
        Ok(status) => {
            if status.success() {
                println!("Hyprfinity: Gamescope process killed.");
            } else {
                eprintln!(
                    "Hyprfinity: Failed to kill gamescope process. Status: {}",
                    status
                );
            }
        }
        Err(e) => eprintln!("Hyprfinity: Error killing gamescope process: {}", e),
    }

    let state_file_path = get_gamescope_state_file_path()?;
    std::fs::remove_file(&state_file_path)?;
    println!(
        "Hyprfinity: Cleaned up Gamescope state file {:?}",
        state_file_path
    );
    if state.waybar_was_stopped {
        maybe_start_waybar(false)?;
    }
    Ok(())
}

fn maybe_stop_waybar(verbose: bool) -> Result<bool, Box<dyn Error>> {
    let status = Command::new("pgrep").args(["-x", "waybar"]).status()?;
    if !status.success() {
        return Ok(false);
    }
    let kill_status = Command::new("pkill").args(["-x", "waybar"]).status()?;
    if !kill_status.success() {
        return Err(MyError("Failed to stop waybar with pkill -x waybar.".to_string()).into());
    }
    if verbose {
        println!("Hyprfinity (DEBUG): Stopped waybar for fullscreen coverage.");
    }
    Ok(true)
}

fn maybe_start_waybar(verbose: bool) -> Result<(), Box<dyn Error>> {
    let status = Command::new("pgrep").args(["-x", "waybar"]).status()?;
    if status.success() {
        return Ok(());
    }
    Command::new("waybar")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| MyError(format!("Failed to start waybar: {}", e)))?;
    if verbose {
        println!("Hyprfinity (DEBUG): Restarted waybar.");
    }
    Ok(())
}

fn write_config(path_override: &Option<String>, config: &Config) -> Result<(), Box<dyn Error>> {
    let path = resolve_config_path(path_override)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let toml_str = toml::to_string_pretty(config)
        .map_err(|e| MyError(format!("Failed to serialize config: {}", e)))?;
    std::fs::write(&path, toml_str)?;
    println!("Hyprfinity: Wrote config to {}", path.display());
    Ok(())
}

fn prompt_line(prompt: &str, default: &str) -> Result<String, Box<dyn Error>> {
    print!("{} [{}]: ", prompt, default);
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn parse_resolution(input: &str) -> Result<(i32, i32), Box<dyn Error>> {
    let normalized = input.replace('x', " ");
    let parts: Vec<&str> = normalized.split_whitespace().collect();
    if parts.len() != 2 {
        return Err(MyError("Expected resolution format WIDTHxHEIGHT.".to_string()).into());
    }
    let w: i32 = parts[0]
        .parse()
        .map_err(|e| MyError(format!("Invalid width '{}': {}", parts[0], e)))?;
    let h: i32 = parts[1]
        .parse()
        .map_err(|e| MyError(format!("Invalid height '{}': {}", parts[1], e)))?;
    if w <= 0 || h <= 0 {
        return Err(MyError("Width and height must be positive.".to_string()).into());
    }
    Ok((w, h))
}

fn interactive_config(path_override: &Option<String>, verbose: bool) -> Result<(), Box<dyn Error>> {
    let path = resolve_config_path(path_override)?;
    let mut config = load_config(path_override)?;
    println!("Hyprfinity: Interactive config at {}", path.display());

    let span = match get_monitors(verbose) {
        Ok(monitors) => match compute_monitor_span(&monitors) {
            Ok((_, _, w, h)) => {
                println!("Hyprfinity: Detected monitor span {}x{}.", w, h);
                Some((w, h))
            }
            Err(e) => {
                eprintln!("Hyprfinity: Could not compute monitor span: {}", e);
                None
            }
        },
        Err(e) => {
            eprintln!("Hyprfinity: Could not detect monitors (continuing): {}", e);
            None
        }
    };

    println!(
        "Output size mode:\n1) Keep current ({:?}x{:?})\n2) Use auto span\n3) Set explicit output size",
        config.output_width, config.output_height
    );
    let out_mode = prompt_line("Choose output mode", "1")?;
    match out_mode.as_str() {
        "2" => {
            config.output_width = None;
            config.output_height = None;
        }
        "3" => {
            let default_output =
                if let (Some(w), Some(h)) = (config.output_width, config.output_height) {
                    format!("{}x{}", w, h)
                } else if let Some((w, h)) = span {
                    format!("{}x{}", w, h)
                } else {
                    "1920x1080".to_string()
                };
            let value = prompt_line("Output resolution (WIDTHxHEIGHT)", &default_output)?;
            let (w, h) = parse_resolution(&value)?;
            config.output_width = Some(w);
            config.output_height = Some(h);
        }
        _ => {}
    }

    println!(
        "Internal render mode:\n1) Keep current (scale {:?}, virtual {:?}x{:?})\n2) Use render scale\n3) Use explicit virtual size",
        config.render_scale, config.virtual_width, config.virtual_height
    );
    let in_mode = prompt_line("Choose internal mode", "1")?;
    match in_mode.as_str() {
        "2" => {
            let default_scale = config.render_scale.unwrap_or(1.0).to_string();
            let value = prompt_line("Render scale (0.1 - 1.0)", &default_scale)?;
            let mut scale: f32 = value
                .parse()
                .map_err(|e| MyError(format!("Invalid render scale '{}': {}", value, e)))?;
            scale = scale.clamp(0.1, 1.0);
            config.render_scale = Some(scale);
            config.virtual_width = None;
            config.virtual_height = None;
        }
        "3" => {
            let default_virtual =
                if let (Some(w), Some(h)) = (config.virtual_width, config.virtual_height) {
                    format!("{}x{}", w, h)
                } else {
                    let (w, h) = span.unwrap_or((1920, 1080));
                    format!("{}x{}", w, h)
                };
            let value = prompt_line("Virtual resolution (WIDTHxHEIGHT)", &default_virtual)?;
            let (w, h) = parse_resolution(&value)?;
            config.virtual_width = Some(w);
            config.virtual_height = Some(h);
        }
        _ => {}
    }

    write_config(path_override, &config)?;
    println!("Hyprfinity: Done. Use `hyprfinity config-show` to inspect effective values.");
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
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
