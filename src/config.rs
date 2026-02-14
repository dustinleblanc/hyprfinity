use crate::MyError;
use crate::autotune::{detect_auto_tune_profile, detect_span_size};
use crate::hyprland::{compute_monitor_span, get_monitors};
use crate::tui_config::{apply_editor_defaults, edit_config_tui};
use crate::types::AutoTuneProfile;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::io::Write;

const DEFAULT_CONFIG_REL_PATH: &str = "hyprfinity/config.toml";

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub(crate) struct Config {
    pub(crate) gamescope_args: Option<Vec<String>>,
    pub(crate) default_command: Option<Vec<String>>,
    pub(crate) no_pin: Option<bool>,
    pub(crate) pick: Option<bool>,
    pub(crate) hide_waybar: Option<bool>,
    pub(crate) pick_size: Option<bool>,
    pub(crate) render_scale: Option<f32>,
    pub(crate) virtual_width: Option<i32>,
    pub(crate) virtual_height: Option<i32>,
    pub(crate) output_width: Option<i32>,
    pub(crate) output_height: Option<i32>,
    pub(crate) startup_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct LaunchSettings {
    pub(crate) args: Vec<String>,
    pub(crate) no_pin: bool,
    pub(crate) pick: bool,
    pub(crate) hide_waybar: bool,
    pub(crate) pick_size: bool,
    pub(crate) render_scale: f32,
    pub(crate) virtual_width: Option<i32>,
    pub(crate) virtual_height: Option<i32>,
    pub(crate) output_width: Option<i32>,
    pub(crate) output_height: Option<i32>,
    pub(crate) timeout: u64,
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

fn resolve_config_path(
    path_override: &Option<String>,
) -> Result<std::path::PathBuf, Box<dyn Error>> {
    if let Some(path) = path_override {
        Ok(std::path::PathBuf::from(path))
    } else {
        resolve_default_config_path()
    }
}

pub(crate) fn load_config(path_override: &Option<String>) -> Result<Config, Box<dyn Error>> {
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

pub(crate) fn write_default_config(
    path_override: &Option<String>,
    force: bool,
) -> Result<(), Box<dyn Error>> {
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn show_config(
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

pub(crate) fn interactive_config(
    path_override: &Option<String>,
    verbose: bool,
) -> Result<(), Box<dyn Error>> {
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_config(
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

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config() -> Config {
        Config {
            gamescope_args: Some(vec!["-r".to_string(), "60".to_string()]),
            default_command: Some(vec![
                "steam".to_string(),
                "-applaunch".to_string(),
                "620".to_string(),
            ]),
            no_pin: Some(false),
            pick: Some(false),
            hide_waybar: Some(true),
            pick_size: Some(false),
            render_scale: Some(0.9),
            virtual_width: Some(1280),
            virtual_height: Some(720),
            output_width: Some(3840),
            output_height: Some(1080),
            startup_timeout_secs: Some(15),
        }
    }

    #[test]
    fn apply_config_uses_config_defaults_and_appends_default_command() {
        let config = base_config();
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

        assert_eq!(
            launch.args,
            vec!["-r", "60", "--", "steam", "-applaunch", "620"]
        );
        assert!(!launch.no_pin);
        assert!(!launch.pick);
        assert!(launch.hide_waybar);
        assert!(!launch.pick_size);
        assert_eq!(launch.render_scale, 0.9);
        assert_eq!(launch.virtual_width, Some(1280));
        assert_eq!(launch.virtual_height, Some(720));
        assert_eq!(launch.output_width, Some(3840));
        assert_eq!(launch.output_height, Some(1080));
        assert_eq!(launch.timeout, 15);
    }

    #[test]
    fn apply_config_cli_overrides_and_clamps_render_scale() {
        let config = base_config();
        let launch = apply_config(
            &["-r".to_string(), "120".to_string()],
            true,
            true,
            true,
            true,
            Some(2.0),
            Some(1600),
            None,
            25,
            &config,
        );

        assert_eq!(
            launch.args,
            vec!["-r", "120", "--", "steam", "-applaunch", "620"]
        );
        assert!(launch.no_pin);
        assert!(launch.pick);
        assert!(launch.hide_waybar);
        assert!(launch.pick_size);
        assert_eq!(launch.render_scale, 1.0);
        assert_eq!(launch.virtual_width, Some(1600));
        assert_eq!(launch.virtual_height, Some(720));
        assert_eq!(launch.timeout, 25);
    }
}
