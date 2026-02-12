use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fmt,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    sync::Arc,
    thread,
    time::Duration,
};
use skim::prelude::*;

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
    /// Print resolved config (path + values).
    ConfigShow {
        /// Override no-pin in effective output.
        #[arg(long, default_value_t = false)]
        no_pin: bool,
        /// Override pick in effective output.
        #[arg(long, default_value_t = false)]
        pick: bool,
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
}

const GAMESCOPE_STATE_FILE_NAME: &str = "hyprfinity_gamescope_state.json";
const DEFAULT_CONFIG_REL_PATH: &str = "hyprfinity/config.toml";

fn get_gamescope_state_file_path() -> Result<std::path::PathBuf, Box<dyn Error>> {
    let temp_dir = std::env::temp_dir();
    Ok(temp_dir.join(GAMESCOPE_STATE_FILE_NAME))
}

#[derive(Debug, Deserialize, Default, Clone)]
struct Config {
    gamescope_args: Option<Vec<String>>,
    default_command: Option<Vec<String>>,
    no_pin: Option<bool>,
    pick: Option<bool>,
    startup_timeout_secs: Option<u64>,
}

fn resolve_default_config_path() -> Result<std::path::PathBuf, Box<dyn Error>> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Ok(std::path::PathBuf::from(xdg).join(DEFAULT_CONFIG_REL_PATH));
    }
    if let Ok(home) = std::env::var("HOME") {
        return Ok(std::path::PathBuf::from(home).join(".config").join(DEFAULT_CONFIG_REL_PATH));
    }
    Err(MyError("Unable to resolve config path (HOME and XDG_CONFIG_HOME are unset).".to_string()).into())
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

fn resolve_config_path(path_override: &Option<String>) -> Result<std::path::PathBuf, Box<dyn Error>> {
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
    cli_timeout: u64,
) -> Result<(), Box<dyn Error>> {
    let path = resolve_config_path(path_override)?;
    let config = load_config(path_override)?;

    let (args, no_pin, pick, timeout) =
        apply_config(cli_args, cli_no_pin, cli_pick, cli_timeout, &config);

    println!("Hyprfinity: Config path: {}", path.display());
    println!("Hyprfinity: Effective values (after CLI overrides):");
    println!("  gamescope_args = {:?}", args);
    println!("  no_pin = {}", no_pin);
    println!("  pick = {}", pick);
    println!("  startup_timeout_secs = {}", timeout);

    println!("Hyprfinity: Raw config values:");
    println!("  gamescope_args = {:?}", config.gamescope_args.unwrap_or_default());
    println!("  default_command = {:?}", config.default_command.unwrap_or_default());
    println!("  no_pin = {}", config.no_pin.unwrap_or(false));
    println!("  pick = {}", config.pick.unwrap_or(false));
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
        println!("Hyprfinity (DEBUG): Executing hyprctl with args: {:?}", args);
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
        println!("Hyprfinity (DEBUG): Executing hyprctl with args: {:?}", args);
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
    let max_x = monitors
        .iter()
        .map(|m| m.x + m.width)
        .max()
        .unwrap_or(0);
    let max_y = monitors
        .iter()
        .map(|m| m.y + m.height)
        .max()
        .unwrap_or(0);

    let span_width = max_x - min_x;
    let span_height = max_y - min_y;

    Ok((min_x, min_y, span_width, span_height))
}

#[derive(Debug, Deserialize)]
struct Client {
    pid: i32,
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

    Err(MyError(format!("Timed out waiting for Gamescope window (PID {}).", pid)).into())
}

fn has_arg(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| {
        arg == flag
            || arg.starts_with(&format!("{flag}="))
            || (flag.len() == 2 && arg.starts_with(flag) && arg.len() > 2)
    })
}

fn build_gamescope_args(args: &[String], span_width: i32, span_height: i32) -> Vec<String> {
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
        pre.push(span_width.to_string());
    }
    if !has_nested_h {
        pre.push("-h".to_string());
        pre.push(span_height.to_string());
    }

    pre.extend(post.into_iter());
    pre
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
        "%U", "%u", "%F", "%f", "%i", "%c", "%k", "%d", "%D", "%n", "%N", "%v", "%m", "%M",
        "%r", "%R",
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

fn ensure_game_command(mut gamescope_args: Vec<String>, pick: bool) -> Result<Vec<String>, Box<dyn Error>> {
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
    cli_timeout: u64,
    config: &Config,
) -> (Vec<String>, bool, bool, u64) {
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

    let timeout = if cli_timeout != 10 {
        cli_timeout
    } else {
        config.startup_timeout_secs.unwrap_or(10)
    };

    if config.default_command.is_some() && !args.iter().any(|a| a == "--") {
        args.push("--".to_string());
        args.extend(config.default_command.clone().unwrap_or_default());
    }

    (args, no_pin, pick, timeout)
}

fn gamescope_up(
    gamescope_args: &[String],
    startup_timeout_secs: u64,
    no_pin: bool,
    pick: bool,
    verbose: bool,
) -> Result<(), Box<dyn Error>> {
    let monitors = get_monitors(verbose)?;
    let (span_x, span_y, span_width, span_height) = compute_monitor_span(&monitors)?;

    println!(
        "Hyprfinity: Computed monitor span: origin=({}, {}), size={}x{}",
        span_x, span_y, span_width, span_height
    );

    let gamescope_args = ensure_game_command(gamescope_args.to_vec(), pick)?;
    let final_args = build_gamescope_args(&gamescope_args, span_width, span_height);
    println!("Hyprfinity: Launching gamescope with args: {:?}", final_args);

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

    let move_params = format!("exact {} {}", span_x, span_y);
    execute_hyprctl(
        &["dispatch", "movewindowpixel", &format!("{},{}", move_params, window)],
        verbose,
    )?;

    let resize_params = format!("exact {} {}", span_width, span_height);
    execute_hyprctl(
        &["dispatch", "resizewindowpixel", &format!("{},{}", resize_params, window)],
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
    println!("Hyprfinity: Stopping gamescope PID {}...", state.gamescope_pid);
    match Command::new("kill").arg(state.gamescope_pid.to_string()).status() {
        Ok(status) => {
            if status.success() {
                println!("Hyprfinity: Gamescope process killed.");
            } else {
                eprintln!("Hyprfinity: Failed to kill gamescope process. Status: {}", status);
            }
        }
        Err(e) => eprintln!("Hyprfinity: Error killing gamescope process: {}", e),
    }

    let state_file_path = get_gamescope_state_file_path()?;
    std::fs::remove_file(&state_file_path)?;
    println!("Hyprfinity: Cleaned up Gamescope state file {:?}", state_file_path);
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
            gamescope_args,
        }) => {
            println!("Hyprfinity: Launching Gamescope span session...");
            let (args, no_pin, pick, timeout) = apply_config(
                gamescope_args,
                *no_pin,
                *pick,
                *startup_timeout_secs,
                &config,
            );
            gamescope_up(&args, timeout, no_pin, pick, cli.verbose)
        }
        None => {
            println!("Hyprfinity: Launching Gamescope span session...");
            let (args, no_pin, pick, timeout) =
                apply_config(&[], false, false, 10, &config);
            gamescope_up(&args, timeout, no_pin, pick, cli.verbose)
        }
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
            startup_timeout_secs,
            gamescope_args,
        }) => {
            show_config(
                &cli.config,
                gamescope_args,
                *no_pin,
                *pick,
                *startup_timeout_secs,
            )?;
            Ok(())
        }
    }
}
