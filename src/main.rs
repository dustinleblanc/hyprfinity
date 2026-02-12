use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell as TuiCell, Paragraph, Row as TuiRow, Table as TuiTable},
};
use serde::{Deserialize, Serialize};
use skim::prelude::*;
use std::collections::BTreeSet;
use std::{
    error::Error,
    fmt,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
    sync::Mutex,
    sync::OnceLock,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
    time::SystemTime,
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
    /// Enable diagnostic logging to a file.
    #[arg(long, global = true, default_value_t = false)]
    debug: bool,
    /// Path to debug log file (used with --debug). Overrides HYPRFINITY_DEBUG_LOG.
    #[arg(long, global = true)]
    debug_log: Option<String>,
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
const DEBUG_LOG_ENV_VAR: &str = "HYPRFINITY_DEBUG_LOG";
const DEFAULT_DEBUG_LOG_PATH: &str = "/var/log/hyprfinity-debug.log";
const FALLBACK_DEBUG_LOG_PATH: &str = "/tmp/hyprfinity-debug.log";

static DEBUG_LOGGER: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

fn get_gamescope_state_file_path() -> Result<std::path::PathBuf, Box<dyn Error>> {
    let temp_dir = std::env::temp_dir();
    Ok(temp_dir.join(GAMESCOPE_STATE_FILE_NAME))
}

fn init_debug_logging(enabled: bool, path_override: &Option<String>) -> Result<(), Box<dyn Error>> {
    if !enabled {
        return Ok(());
    }
    let chosen_path = if let Some(p) = path_override.as_ref() {
        PathBuf::from(p)
    } else if let Ok(p) = std::env::var(DEBUG_LOG_ENV_VAR) {
        PathBuf::from(p)
    } else {
        PathBuf::from(DEFAULT_DEBUG_LOG_PATH)
    };

    let open_file = |path: &PathBuf| -> Result<std::fs::File, Box<dyn Error>> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        Ok(std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)?)
    };

    let (path, file) = match open_file(&chosen_path) {
        Ok(f) => (chosen_path, f),
        Err(e) => {
            let fallback = PathBuf::from(FALLBACK_DEBUG_LOG_PATH);
            eprintln!(
                "Hyprfinity: Failed to open debug log at {} ({}), falling back to {}",
                chosen_path.display(),
                e,
                fallback.display()
            );
            let f = open_file(&fallback)?;
            (fallback, f)
        }
    };

    let _ = DEBUG_LOGGER.set(Mutex::new(file));
    println!("Hyprfinity: Debug log enabled at {}", path.display());
    debug_log_line("debug logging initialized");
    Ok(())
}

fn debug_log_line(message: &str) {
    let Some(lock) = DEBUG_LOGGER.get() else {
        return;
    };
    let ts_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut file) = lock.lock() {
        let _ = writeln!(file, "[{}] {}", ts_ms, message);
    }
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

#[derive(Debug, Clone)]
struct AutoTuneProfile {
    render_scale: f32,
    reason: String,
}

fn detect_total_memory_gib() -> Option<f32> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = meminfo.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb = line
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<u64>().ok())?;
    Some(kb as f32 / 1024.0 / 1024.0)
}

fn detect_span_pixels() -> Option<i64> {
    let (w, h) = detect_span_size()?;
    Some(i64::from(w) * i64::from(h))
}

fn detect_span_size() -> Option<(i32, i32)> {
    let monitors = get_monitors(false).ok()?;
    let (_, _, w, h) = compute_monitor_span(&monitors).ok()?;
    Some((w, h))
}

fn detect_gpu_models() -> Vec<String> {
    let output = match Command::new("lspci").arg("-nn").output() {
        Ok(out) => out,
        Err(_) => return Vec::new(),
    };
    if !output.status.success() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let candidates: Vec<String> = stdout
        .lines()
        .filter(|line| {
            line.contains("VGA compatible controller")
                || line.contains("3D controller")
                || line.contains("Display controller")
        })
        .map(|line| {
            line.split_once(':')
                .map(|(_, rest)| rest.trim().to_string())
                .unwrap_or_else(|| line.trim().to_string())
        })
        .collect();
    candidates
}

fn gpu_model_score(model: &str) -> i32 {
    let lc = model.to_lowercase();
    let mut score = 0;

    if lc.contains("nvidia") || lc.contains("geforce") || lc.contains("rtx") || lc.contains("gtx") {
        score += 50;
    }
    if lc.contains("amd")
        || lc.contains("ati")
        || lc.contains("radeon")
        || lc.contains("rx ")
        || lc.contains("rx5")
        || lc.contains("rx 5")
        || lc.contains("rx6")
        || lc.contains("rx 6")
        || lc.contains("rx7")
        || lc.contains("rx 7")
    {
        score += 45;
    }
    if lc.contains("intel") {
        score += 10;
        if lc.contains("arc") {
            score += 20;
        } else {
            score -= 8;
        }
    }

    if lc.contains("uhd")
        || lc.contains("hd graphics")
        || lc.contains("iris")
        || lc.contains("vega 8")
        || lc.contains("vega 11")
    {
        score -= 6;
    }

    if lc.contains("rx 580") || lc.contains("rx580") {
        score += 5;
    }

    score
}

fn detect_gpu_model() -> Option<String> {
    let candidates = detect_gpu_models();
    candidates
        .into_iter()
        .max_by_key(|model| gpu_model_score(model))
}

fn detect_gpu_vram_gib() -> Option<f32> {
    let mut best_vram_bytes: Option<u64> = None;
    let entries = std::fs::read_dir("/sys/class/drm").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("card") {
            continue;
        }
        if name.contains('-') {
            continue;
        }
        let vram_path = entry.path().join("device/mem_info_vram_total");
        let value = std::fs::read_to_string(vram_path).ok();
        if let Some(v) = value.and_then(|s| s.trim().parse::<u64>().ok()) {
            best_vram_bytes = Some(best_vram_bytes.map_or(v, |b| b.max(v)));
        }
    }
    best_vram_bytes.map(|bytes| bytes as f32 / 1024.0 / 1024.0 / 1024.0)
}

fn gpu_scale_adjustment(
    gpu_model: Option<&str>,
    gpu_vram_gib: Option<f32>,
    span_pixels: Option<i64>,
) -> (f32, String) {
    let mut delta = 0.0_f32;
    let mut reasons: Vec<String> = Vec::new();

    if let Some(vram) = gpu_vram_gib {
        if vram <= 4.0 {
            delta -= 0.20;
            reasons.push(format!("VRAM {:.1}GiB (very low)", vram));
        } else if vram <= 6.0 {
            delta -= 0.15;
            reasons.push(format!("VRAM {:.1}GiB (low)", vram));
        } else if vram <= 8.0 {
            delta -= 0.10;
            reasons.push(format!("VRAM {:.1}GiB (mid)", vram));
        } else if vram >= 16.0 {
            delta += 0.08;
            reasons.push(format!("VRAM {:.1}GiB (high)", vram));
        } else if vram >= 12.0 {
            delta += 0.05;
            reasons.push(format!("VRAM {:.1}GiB (good)", vram));
        }
    }

    if let Some(model) = gpu_model {
        let lc = model.to_lowercase();
        if lc.contains("rx 580")
            || lc.contains("rx580")
            || lc.contains("rx 570")
            || lc.contains("rx570")
            || lc.contains("rx 560")
            || lc.contains("rx560")
            || lc.contains("rx 480")
            || lc.contains("rx480")
            || lc.contains("rx 470")
            || lc.contains("rx470")
            || lc.contains("rx 460")
            || lc.contains("rx460")
        {
            delta -= 0.15;
            reasons.push("older AMD Polaris class".to_string());
        } else if lc.contains("intel") && !lc.contains("arc") {
            delta -= 0.12;
            reasons.push("integrated Intel graphics".to_string());
        } else if lc.contains("vega 8") || lc.contains("vega 11") {
            delta -= 0.10;
            reasons.push("integrated Vega graphics".to_string());
        } else if lc.contains("rtx 40") || lc.contains("rx 7") {
            delta += 0.08;
            reasons.push("newer high-end GPU tier".to_string());
        }
    }

    if span_pixels.unwrap_or(0) > 10_000_000 && delta < 0.0 {
        delta -= 0.05;
        reasons.push("large multi-monitor span".to_string());
    }

    delta = delta.clamp(-0.35, 0.12);

    let reason = if reasons.is_empty() {
        "no strong GPU adjustment".to_string()
    } else {
        reasons.join(", ")
    };
    (delta, reason)
}

fn detect_auto_tune_profile() -> AutoTuneProfile {
    let cpu_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let mem_gib = detect_total_memory_gib();
    let span_pixels = detect_span_pixels();
    let gpu_model = detect_gpu_model();
    let gpu_vram_gib = detect_gpu_vram_gib();

    let mut scale = match span_pixels {
        Some(p) if p > 16_000_000 => 0.60_f32,
        Some(p) if p > 12_000_000 => 0.67_f32,
        Some(p) if p > 8_500_000 => 0.75_f32,
        Some(p) if p > 5_500_000 => 0.85_f32,
        _ => 1.0_f32,
    };

    let mem = mem_gib.unwrap_or(16.0);
    if cpu_threads >= 16 && mem >= 32.0 {
        scale += 0.10;
    } else if cpu_threads >= 12 && mem >= 24.0 {
        scale += 0.05;
    } else if cpu_threads <= 4 || mem < 8.0 {
        scale -= 0.15;
    } else if cpu_threads <= 6 || mem < 12.0 {
        scale -= 0.10;
    }

    let (gpu_delta, gpu_reason) =
        gpu_scale_adjustment(gpu_model.as_deref(), gpu_vram_gib, span_pixels);
    scale += gpu_delta;

    scale = (scale * 100.0).round() / 100.0;
    scale = scale.clamp(0.50, 1.0);

    let reason = format!(
        "auto-tuned using CPU threads={}, RAM={} GiB, span_pixels={}, GPU='{}', GPU_VRAM={} GiB, gpu_adjustment={:+.2} ({})",
        cpu_threads,
        mem_gib
            .map(|v| format!("{:.1}", v))
            .unwrap_or_else(|| "unknown".to_string()),
        span_pixels
            .map(|v| v.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        gpu_model.unwrap_or_else(|| "unknown".to_string()),
        gpu_vram_gib
            .map(|v| format!("{:.1}", v))
            .unwrap_or_else(|| "unknown".to_string()),
        gpu_delta,
        gpu_reason
    );

    AutoTuneProfile {
        render_scale: scale,
        reason,
    }
}

fn default_config_values(auto: &AutoTuneProfile) -> Config {
    Config {
        gamescope_args: Some(vec!["-r".to_string(), "60".to_string()]),
        default_command: None,
        no_pin: Some(false),
        pick: Some(false),
        hide_waybar: Some(true),
        pick_size: Some(false),
        render_scale: Some(auto.render_scale),
        virtual_width: None,
        virtual_height: None,
        output_width: None,
        output_height: None,
        startup_timeout_secs: Some(10),
    }
}

fn format_toml_string_array(values: &[String]) -> String {
    values
        .iter()
        .map(|s| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string()))
        .collect::<Vec<String>>()
        .join(", ")
}

fn render_config_template(config: &Config, auto_reason: &str) -> String {
    let gamescope_args = config
        .gamescope_args
        .clone()
        .unwrap_or_else(|| vec!["-r".to_string(), "60".to_string()]);
    let default_command_line = config
        .default_command
        .clone()
        .map(|cmd| format!("default_command = [{}]", format_toml_string_array(&cmd)))
        .unwrap_or_else(|| "# default_command = [\"steam\", \"-applaunch\", \"620\"]".to_string());
    let no_pin = config.no_pin.unwrap_or(false);
    let pick = config.pick.unwrap_or(false);
    let hide_waybar = config.hide_waybar.unwrap_or(true);
    let pick_size = config.pick_size.unwrap_or(false);
    let render_scale = config.render_scale.unwrap_or(1.0);
    let startup_timeout_secs = config.startup_timeout_secs.unwrap_or(10);

    let virtual_width_line = config
        .virtual_width
        .map(|v| format!("virtual_width = {}", v))
        .unwrap_or_else(|| "# virtual_width = 5760".to_string());
    let virtual_height_line = config
        .virtual_height
        .map(|v| format!("virtual_height = {}", v))
        .unwrap_or_else(|| "# virtual_height = 1080".to_string());
    let output_width_line = config
        .output_width
        .map(|v| format!("output_width = {}", v))
        .unwrap_or_else(|| "# output_width = 7680".to_string());
    let output_height_line = config
        .output_height
        .map(|v| format!("output_height = {}", v))
        .unwrap_or_else(|| "# output_height = 1440".to_string());

    format!(
        r#"# Hyprfinity config

# Default gamescope args (used when no args are provided on the CLI)
gamescope_args = [{gamescope_args}]

# Optional default game/app command (appended if no `--` command is provided)
{default_command_line}

# Defaults for CLI flags
no_pin = {no_pin}
pick = {pick}
hide_waybar = {hide_waybar}
pick_size = {pick_size}
# Internal render scale relative to output span; 1.0 = native span.
# {auto_reason}
render_scale = {render_scale}
# Optional explicit internal render size (when set, these take precedence over render_scale).
{virtual_width_line}
{virtual_height_line}
# Optional explicit output size for Gamescope (-W/-H). Default is full monitor span.
{output_width_line}
{output_height_line}
startup_timeout_secs = {startup_timeout_secs}
"#,
        gamescope_args = format_toml_string_array(&gamescope_args),
        default_command_line = default_command_line,
        no_pin = no_pin,
        pick = pick,
        hide_waybar = hide_waybar,
        pick_size = pick_size,
        auto_reason = auto_reason,
        render_scale = render_scale,
        virtual_width_line = virtual_width_line,
        virtual_height_line = virtual_height_line,
        output_width_line = output_width_line,
        output_height_line = output_height_line,
        startup_timeout_secs = startup_timeout_secs,
    )
}

fn prompt_yes_no(prompt: &str, default: bool) -> Result<bool, Box<dyn Error>> {
    loop {
        let hint = if default { "Y/n" } else { "y/N" };
        print!("{} [{}]: ", prompt, hint);
        std::io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let normalized = input.trim().to_lowercase();

        if normalized.is_empty() {
            return Ok(default);
        }
        if normalized == "y" || normalized == "yes" {
            return Ok(true);
        }
        if normalized == "n" || normalized == "no" {
            return Ok(false);
        }

        println!("Please answer y/yes or n/no.");
    }
}

fn format_optional_size(width: Option<i32>, height: Option<i32>) -> String {
    match (width, height) {
        (Some(w), Some(h)) => format!("{}x{}", w, h),
        (Some(w), None) => format!("{}x(auto)", w),
        (None, Some(h)) => format!("(auto)x{}", h),
        (None, None) => "auto".to_string(),
    }
}

fn print_kv_table(title: &str, rows: Vec<(&str, String)>) {
    println!("Hyprfinity: {}", title);
    let key_width = rows
        .iter()
        .map(|(k, _)| k.len())
        .max()
        .unwrap_or(3)
        .max("Key".len());
    let val_width = rows
        .iter()
        .map(|(_, v)| v.len())
        .max()
        .unwrap_or(5)
        .max("Value".len());

    let sep = format!("+-{}-+-{}-+", "-".repeat(key_width), "-".repeat(val_width));
    println!("{}", sep);
    println!(
        "| {:<key_width$} | {:<val_width$} |",
        "Key",
        "Value",
        key_width = key_width,
        val_width = val_width
    );
    println!("{}", sep);
    for (k, v) in rows {
        println!(
            "| {:<key_width$} | {:<val_width$} |",
            k,
            v,
            key_width = key_width,
            val_width = val_width
        );
    }
    println!("{}", sep);
}

fn print_config_table(title: &str, config: &Config) {
    print_kv_table(
        title,
        vec![
            (
                "gamescope_args",
                format!("{:?}", config.gamescope_args.clone().unwrap_or_default()),
            ),
            (
                "default_command",
                format!("{:?}", config.default_command.clone().unwrap_or_default()),
            ),
            ("no_pin", config.no_pin.unwrap_or(false).to_string()),
            ("pick", config.pick.unwrap_or(false).to_string()),
            (
                "hide_waybar",
                config.hide_waybar.unwrap_or(true).to_string(),
            ),
            ("pick_size", config.pick_size.unwrap_or(false).to_string()),
            (
                "render_scale",
                config.render_scale.unwrap_or(1.0).to_string(),
            ),
            (
                "virtual_size",
                format_optional_size(config.virtual_width, config.virtual_height),
            ),
            (
                "output_size",
                format_optional_size(config.output_width, config.output_height),
            ),
            (
                "startup_timeout_secs",
                config.startup_timeout_secs.unwrap_or(10).to_string(),
            ),
        ],
    );
}

fn print_effective_launch_table(title: &str, launch: &LaunchSettings) {
    print_kv_table(
        title,
        vec![
            ("gamescope_args", format!("{:?}", launch.args)),
            ("no_pin", launch.no_pin.to_string()),
            ("pick", launch.pick.to_string()),
            ("hide_waybar", launch.hide_waybar.to_string()),
            ("pick_size", launch.pick_size.to_string()),
            ("render_scale", launch.render_scale.to_string()),
            (
                "virtual_size",
                format_optional_size(launch.virtual_width, launch.virtual_height),
            ),
            (
                "output_size",
                format_optional_size(launch.output_width, launch.output_height),
            ),
            ("startup_timeout_secs", launch.timeout.to_string()),
        ],
    );
}

fn apply_editor_defaults(mut config: Config, auto_scale: f32) -> Config {
    if config.gamescope_args.is_none() {
        config.gamescope_args = Some(vec!["-r".to_string(), "60".to_string()]);
    }
    if config.no_pin.is_none() {
        config.no_pin = Some(false);
    }
    if config.pick.is_none() {
        config.pick = Some(false);
    }
    if config.hide_waybar.is_none() {
        config.hide_waybar = Some(true);
    }
    if config.pick_size.is_none() {
        config.pick_size = Some(false);
    }
    if config.render_scale.is_none() {
        config.render_scale = Some(auto_scale);
    }
    if config.startup_timeout_secs.is_none() {
        config.startup_timeout_secs = Some(10);
    }
    config
}

fn push_unique_size_option(options: &mut Vec<Option<(i32, i32)>>, candidate: Option<(i32, i32)>) {
    if !options.contains(&candidate) {
        options.push(candidate);
    }
}

fn output_size_options(span: Option<(i32, i32)>) -> Vec<Option<(i32, i32)>> {
    let mut options = vec![None];
    push_unique_size_option(&mut options, span);
    push_unique_size_option(&mut options, Some((1920, 1080)));
    push_unique_size_option(&mut options, Some((2560, 1440)));
    push_unique_size_option(&mut options, Some((3440, 1440)));
    push_unique_size_option(&mut options, Some((3840, 2160)));
    options
}

fn virtual_size_options(span: Option<(i32, i32)>) -> Vec<Option<(i32, i32)>> {
    let mut options = vec![None];
    push_unique_size_option(&mut options, Some((1280, 720)));
    push_unique_size_option(&mut options, Some((1600, 900)));
    push_unique_size_option(&mut options, Some((1920, 1080)));
    if let Some((sw, sh)) = span {
        push_unique_size_option(&mut options, Some((sw, sh)));
    }
    options
}

fn cycle_size_setting(
    width: &mut Option<i32>,
    height: &mut Option<i32>,
    options: &[Option<(i32, i32)>],
    forward: bool,
) {
    let current = match (*width, *height) {
        (Some(w), Some(h)) => Some((w, h)),
        _ => None,
    };
    let idx = options.iter().position(|o| *o == current).unwrap_or(0);
    let next_idx = if forward {
        (idx + 1) % options.len()
    } else if idx == 0 {
        options.len() - 1
    } else {
        idx - 1
    };
    match options[next_idx] {
        Some((w, h)) => {
            *width = Some(w);
            *height = Some(h);
        }
        None => {
            *width = None;
            *height = None;
        }
    }
}

fn edit_config_tui(
    title: &str,
    config: Config,
    auto_reason: &str,
    span: Option<(i32, i32)>,
) -> Result<Option<Config>, Box<dyn Error>> {
    let mut config = config;
    let mut selected: usize = 0;
    let output_opts = output_size_options(span);
    let virtual_opts = virtual_size_options(span);

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = (|| -> Result<Option<Config>, Box<dyn Error>> {
        loop {
            terminal.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(4),
                        Constraint::Min(8),
                        Constraint::Length(3),
                    ])
                    .split(f.area());

                let header = Paragraph::new(format!(
                    "{}\nAuto recommendation: {}\nSpan: {}",
                    title,
                    auto_reason,
                    span.map(|(w, h)| format!("{}x{}", w, h))
                        .unwrap_or_else(|| "unknown".to_string())
                ))
                .block(Block::default().borders(Borders::ALL).title("Context"));
                f.render_widget(header, chunks[0]);

                let rows = vec![
                    (
                        "render_scale",
                        format!("{:.2}", config.render_scale.unwrap_or(1.0)),
                    ),
                    (
                        "hide_waybar",
                        config.hide_waybar.unwrap_or(true).to_string(),
                    ),
                    ("pick_size", config.pick_size.unwrap_or(false).to_string()),
                    (
                        "output_size",
                        format_optional_size(config.output_width, config.output_height),
                    ),
                    (
                        "virtual_size",
                        format_optional_size(config.virtual_width, config.virtual_height),
                    ),
                    ("save", "Write config and exit".to_string()),
                    ("cancel", "Discard changes".to_string()),
                ];

                let table_rows = rows
                    .into_iter()
                    .enumerate()
                    .map(|(idx, (k, v))| {
                        let style = if idx == selected {
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        };
                        TuiRow::new(vec![TuiCell::from(k), TuiCell::from(v)]).style(style)
                    })
                    .collect::<Vec<_>>();

                let table =
                    TuiTable::new(table_rows, [Constraint::Length(18), Constraint::Min(24)])
                        .header(
                            TuiRow::new(vec!["Field", "Value"])
                                .style(Style::default().add_modifier(Modifier::BOLD)),
                        )
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title("Config Editor"),
                        );
                f.render_widget(table, chunks[1]);

                let footer = Paragraph::new(
                    "Keys: ↑/↓ select  ←/→ change  Enter activate/toggle  s save  q/Esc cancel",
                )
                .block(Block::default().borders(Borders::ALL).title("Help"));
                f.render_widget(footer, chunks[2]);
            })?;

            if event::poll(Duration::from_millis(200))? {
                let ev = event::read()?;
                if let Event::Key(key) = ev {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
                        KeyCode::Char('s') => return Ok(Some(config.clone())),
                        KeyCode::Down => selected = (selected + 1) % 7,
                        KeyCode::Up => {
                            selected = if selected == 0 { 6 } else { selected - 1 };
                        }
                        KeyCode::Left => match selected {
                            0 => {
                                let s = (config.render_scale.unwrap_or(1.0) - 0.05).clamp(0.1, 1.0);
                                config.render_scale = Some((s * 100.0).round() / 100.0);
                            }
                            1 => config.hide_waybar = Some(!config.hide_waybar.unwrap_or(true)),
                            2 => config.pick_size = Some(!config.pick_size.unwrap_or(false)),
                            3 => cycle_size_setting(
                                &mut config.output_width,
                                &mut config.output_height,
                                &output_opts,
                                false,
                            ),
                            4 => cycle_size_setting(
                                &mut config.virtual_width,
                                &mut config.virtual_height,
                                &virtual_opts,
                                false,
                            ),
                            _ => {}
                        },
                        KeyCode::Right => match selected {
                            0 => {
                                let s = (config.render_scale.unwrap_or(1.0) + 0.05).clamp(0.1, 1.0);
                                config.render_scale = Some((s * 100.0).round() / 100.0);
                            }
                            1 => config.hide_waybar = Some(!config.hide_waybar.unwrap_or(true)),
                            2 => config.pick_size = Some(!config.pick_size.unwrap_or(false)),
                            3 => cycle_size_setting(
                                &mut config.output_width,
                                &mut config.output_height,
                                &output_opts,
                                true,
                            ),
                            4 => cycle_size_setting(
                                &mut config.virtual_width,
                                &mut config.virtual_height,
                                &virtual_opts,
                                true,
                            ),
                            _ => {}
                        },
                        KeyCode::Enter => match selected {
                            1 => config.hide_waybar = Some(!config.hide_waybar.unwrap_or(true)),
                            2 => config.pick_size = Some(!config.pick_size.unwrap_or(false)),
                            3 => cycle_size_setting(
                                &mut config.output_width,
                                &mut config.output_height,
                                &output_opts,
                                true,
                            ),
                            4 => cycle_size_setting(
                                &mut config.virtual_width,
                                &mut config.virtual_height,
                                &virtual_opts,
                                true,
                            ),
                            5 => return Ok(Some(config.clone())),
                            6 => return Ok(None),
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }
        }
    })();

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn write_default_config(path_override: &Option<String>, force: bool) -> Result<(), Box<dyn Error>> {
    let path = resolve_config_path(path_override)?;

    if path.exists() && !force {
        let should_overwrite = prompt_yes_no(
            &format!("Config already exists at {}. Overwrite it?", path.display()),
            false,
        )?;
        if !should_overwrite {
            println!("Hyprfinity: Keeping existing config unchanged.");
            return Ok(());
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let auto = detect_auto_tune_profile();
    let mut config = apply_editor_defaults(default_config_values(&auto), auto.render_scale);
    let span = detect_span_size();

    if !force {
        match edit_config_tui("Config Init", config.clone(), &auto.reason, span)? {
            Some(edited) => config = apply_editor_defaults(edited, auto.render_scale),
            None => {
                println!("Hyprfinity: Config init cancelled.");
                return Ok(());
            }
        }
    }

    let contents = render_config_template(&config, &auto.reason);

    std::fs::write(&path, contents)?;
    println!("Hyprfinity: Wrote config to {}", path.display());
    print_config_table("Final Config Defaults", &config);
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
    print_effective_launch_table("Effective Values (after CLI overrides)", &launch);
    print_config_table("Raw Config Values", &config);
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
    debug_log_line(&format!("hyprctl {:?} (void)", args));
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
    debug_log_line(&format!(
        "hyprctl status={} stdout='{}' stderr='{}'",
        output.status,
        stdout.trim(),
        stderr.trim()
    ));

    if !output.status.success() {
        return Err(MyError(format!("hyprctl failed for args {:?}: {}", args, stderr)).into());
    }
    Ok(())
}

fn execute_hyprctl_output(args: &[&str], verbose: bool) -> Result<String, Box<dyn Error>> {
    debug_log_line(&format!("hyprctl {:?} (capture)", args));
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
    debug_log_line(&format!(
        "hyprctl status={} stdout='{}' stderr='{}'",
        output.status,
        stdout.trim(),
        stderr.trim()
    ));

    if !output.status.success() {
        return Err(MyError(format!("hyprctl failed for args {:?}: {}", args, stderr)).into());
    }
    Ok(stdout)
}

fn get_monitors(verbose: bool) -> Result<Vec<Monitor>, Box<dyn Error>> {
    let stdout = execute_hyprctl_output(&["monitors", "-j"], verbose)?;
    debug_log_line(&format!("raw monitors json: {}", stdout.trim()));
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
    let monitor_dump = monitors
        .iter()
        .map(|m| {
            format!(
                "{}:{}x{}@{},{}",
                m.name.clone().unwrap_or_else(|| "unknown".to_string()),
                m.width,
                m.height,
                m.x,
                m.y
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    debug_log_line(&format!(
        "compute_span monitors=[{}] => origin=({}, {}), size={}x{}",
        monitor_dump, min_x, min_y, span_width, span_height
    ));

    Ok((min_x, min_y, span_width, span_height))
}

#[derive(Debug, Deserialize)]
struct Client {
    pid: i32,
    #[serde(default)]
    address: Option<String>,
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

fn primary_client_for_pid<'a>(clients: &'a [Client], pid: u32) -> Option<&'a Client> {
    clients
        .iter()
        .filter(|c| c.pid == pid as i32)
        .max_by_key(|c| {
            let (w, h) = c.size.map(|s| (s[0], s[1])).unwrap_or((0, 0));
            i64::from(w.max(0)) * i64::from(h.max(0))
        })
}

fn get_primary_window_selector(pid: u32, verbose: bool) -> Result<String, Box<dyn Error>> {
    let stdout = execute_hyprctl_output(&["clients", "-j"], verbose)?;
    let clients: Vec<Client> = serde_json::from_str(&stdout)
        .map_err(|e| MyError(format!("Failed to parse hyprctl clients output: {}", e)))?;
    let selector = primary_client_for_pid(&clients, pid)
        .and_then(|c| c.address.as_ref().map(|a| format!("address:{}", a)))
        .unwrap_or_else(|| format!("pid:{}", pid));
    Ok(selector)
}

fn get_client_geometry(
    pid: u32,
    verbose: bool,
) -> Result<Option<(i32, i32, i32, i32)>, Box<dyn Error>> {
    let stdout = execute_hyprctl_output(&["clients", "-j"], verbose)?;
    let clients: Vec<Client> = serde_json::from_str(&stdout)
        .map_err(|e| MyError(format!("Failed to parse hyprctl clients output: {}", e)))?;
    let client = primary_client_for_pid(&clients, pid);
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
    debug_log_line("gamescope_up begin");
    let waybar_was_stopped = if hide_waybar {
        maybe_stop_waybar(verbose)?
    } else {
        false
    };

    let monitors = get_monitors(verbose)?;
    let (span_x, span_y, span_width, span_height) = compute_monitor_span(&monitors)?;

    println!(
        "Hyprfinity: Computed monitor span: origin=({}, {}), size={}x{}",
        span_x, span_y, span_width, span_height
    );
    debug_log_line(&format!(
        "computed span origin=({}, {}), size={}x{}",
        span_x, span_y, span_width, span_height
    ));

    let gamescope_args = ensure_game_command(gamescope_args.to_vec(), pick)?;
    let output = derive_output_size(span_width, span_height, output_width, output_height);
    debug_log_line(&format!(
        "derived output size={}x{} from span={}x{} with config output={:?}x{:?}",
        output.0, output.1, span_width, span_height, output_width, output_height
    ));
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
    debug_log_line(&format!("gamescope final args: {:?}", final_args));

    let mut cmd = Command::new("gamescope");
    cmd.args(&final_args);
    if !verbose {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    let mut child = cmd.spawn()?;

    let gamescope_pid = child.id();
    println!("Hyprfinity: gamescope started with PID {}.", gamescope_pid);

    wait_for_client_pid(gamescope_pid, startup_timeout_secs, verbose)?;

    let window = get_primary_window_selector(gamescope_pid, verbose)
        .unwrap_or_else(|_| format!("pid:{}", gamescope_pid));
    debug_log_line(&format!("initial window selector: {}", window));
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
    let mut reflow_tick: u64 = 0;
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

        // Gamescope can recreate/reconfigure clients after startup.
        // Periodically re-target the primary client and enforce full-span geometry.
        if reflow_tick % 2 == 0 {
            if let Ok(window) = get_primary_window_selector(gamescope_pid, verbose) {
                debug_log_line(&format!("reflow window selector: {}", window));
                let _ = execute_hyprctl(&["dispatch", "setfloating", &window], verbose);
                let _ = fit_window_to_span(
                    gamescope_pid,
                    &window,
                    span_x,
                    span_y,
                    span_width,
                    span_height,
                    verbose,
                );
                if !no_pin {
                    let _ = execute_hyprctl(&["dispatch", "pin", &window], verbose);
                }
            }
        }
        reflow_tick = reflow_tick.wrapping_add(1);
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

fn interactive_config(path_override: &Option<String>, verbose: bool) -> Result<(), Box<dyn Error>> {
    let path = resolve_config_path(path_override)?;
    println!("Hyprfinity: Interactive config at {}", path.display());
    let auto = detect_auto_tune_profile();
    let config = apply_editor_defaults(load_config(path_override)?, auto.render_scale);

    let span = match get_monitors(verbose) {
        Ok(monitors) => compute_monitor_span(&monitors)
            .ok()
            .map(|(_, _, w, h)| (w, h)),
        Err(_) => None,
    };

    match edit_config_tui("Config Editor", config, &auto.reason, span)? {
        Some(edited) => {
            write_config(path_override, &edited)?;
            println!("Hyprfinity: Done. Use `hyprfinity config-show` to inspect effective values.");
        }
        None => println!("Hyprfinity: Config update cancelled."),
    }
    Ok(())
}

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
