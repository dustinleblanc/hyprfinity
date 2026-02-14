use crate::MyError;
use crate::debuglog::debug_log_line;
use crate::hyprland::{
    bind_exists, compute_monitor_span, execute_hyprctl, fit_window_to_span, get_monitors,
    get_primary_window_selector, wait_for_client_pid,
};
use crate::picker::{pick_desktop_app_command, pick_internal_size};
use crate::util::{clamp_i32, even_floor, scaled_dimensions};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct GamescopeState {
    gamescope_pid: u32,
    span_x: i32,
    span_y: i32,
    span_width: i32,
    span_height: i32,
    gamescope_args: Vec<String>,
    waybar_was_stopped: bool,
    #[serde(default)]
    exit_hotkey: Option<ExitHotkey>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ExitHotkey {
    mods: String,
    key: String,
}

const GAMESCOPE_STATE_FILE_NAME: &str = "hyprfinity_gamescope_state.json";
const DEFAULT_EXIT_HOTKEY_MODS: &str = "SUPER SHIFT";
const DEFAULT_EXIT_HOTKEY_KEY: &str = "F12";

fn get_gamescope_state_file_path() -> Result<std::path::PathBuf, Box<dyn Error>> {
    let temp_dir = std::env::temp_dir();
    Ok(temp_dir.join(GAMESCOPE_STATE_FILE_NAME))
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

    pre.extend(post);
    pre
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

fn register_exit_hotkey(verbose: bool) -> Result<Option<ExitHotkey>, Box<dyn Error>> {
    let mods = DEFAULT_EXIT_HOTKEY_MODS;
    let key = DEFAULT_EXIT_HOTKEY_KEY;
    if bind_exists(mods, key, verbose)? {
        println!(
            "Hyprfinity: Exit hotkey {}+{} is already bound; skipping.",
            mods, key
        );
        return Ok(None);
    }

    let binding = format!("{mods}, {key}, exec, hyprfinity gamescope-down");
    execute_hyprctl(&["keyword", "bind", &binding], verbose)?;
    println!(
        "Hyprfinity: Exit hotkey bound: {}+{} (runs `hyprfinity gamescope-down`).",
        mods, key
    );
    Ok(Some(ExitHotkey {
        mods: mods.to_string(),
        key: key.to_string(),
    }))
}

fn unregister_exit_hotkey(hotkey: &ExitHotkey, verbose: bool) {
    let binding = format!("{}, {}", hotkey.mods, hotkey.key);
    let _ = execute_hyprctl(&["keyword", "unbind", &binding], verbose);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn gamescope_up(
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
    let mut waybar_was_stopped = false;
    let mut exit_hotkey: Option<ExitHotkey> = None;

    let result = (|| -> Result<(), Box<dyn Error>> {
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
                println!(
                    "Hyprfinity: Internal size picker cancelled, using configured/default size."
                );
            }
        }

        println!(
            "Hyprfinity: Internal render size: {}x{} (output span {}x{})",
            internal.0, internal.1, output.0, output.1
        );

        if hide_waybar {
            waybar_was_stopped = maybe_stop_waybar(verbose)?;
        }

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

        match register_exit_hotkey(verbose) {
            Ok(hotkey) => exit_hotkey = hotkey,
            Err(e) => eprintln!("Hyprfinity: Failed to register exit hotkey: {}", e),
        }

        let state = GamescopeState {
            gamescope_pid,
            span_x,
            span_y,
            span_width,
            span_height,
            gamescope_args: final_args,
            waybar_was_stopped,
            exit_hotkey: exit_hotkey.clone(),
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
                if let Some(hotkey) = exit_hotkey.as_ref() {
                    unregister_exit_hotkey(hotkey, verbose);
                }
                let state_file_path = get_gamescope_state_file_path()?;
                let _ = std::fs::remove_file(&state_file_path);
                break;
            }

            if reflow_tick.is_multiple_of(2)
                && let Ok(window) = get_primary_window_selector(gamescope_pid, verbose)
            {
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
            reflow_tick = reflow_tick.wrapping_add(1);
            thread::sleep(Duration::from_secs(1));
        }

        Ok(())
    })();

    if result.is_err() && waybar_was_stopped {
        let _ = maybe_start_waybar(verbose);
    }
    if result.is_err()
        && let Some(hotkey) = exit_hotkey.as_ref()
    {
        unregister_exit_hotkey(hotkey, verbose);
    }

    result
}

pub(crate) fn gamescope_down() -> Result<(), Box<dyn Error>> {
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
    if let Some(hotkey) = state.exit_hotkey.as_ref() {
        unregister_exit_hotkey(hotkey, false);
    }
    Ok(())
}
