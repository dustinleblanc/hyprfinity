use crate::MyError;
use crate::debuglog::debug_log_line;
use crate::types::{Client, Monitor};
use std::process::Command;
use std::thread;
use std::time::Duration;

pub(crate) fn execute_hyprctl(
    args: &[&str],
    verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
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

pub(crate) fn execute_hyprctl_output(
    args: &[&str],
    verbose: bool,
) -> Result<String, Box<dyn std::error::Error>> {
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

fn normalize_bind_token(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

pub(crate) fn bind_exists(
    mods: &str,
    key: &str,
    verbose: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    let stdout = execute_hyprctl_output(&["binds"], verbose)?;
    let needle = normalize_bind_token(&format!("{},{}", mods, key));
    for line in stdout.lines() {
        if normalize_bind_token(line).contains(&needle) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn get_monitors(verbose: bool) -> Result<Vec<Monitor>, Box<dyn std::error::Error>> {
    let stdout = execute_hyprctl_output(&["monitors", "-j"], verbose)?;
    debug_log_line(&format!("raw monitors json: {}", stdout.trim()));
    let monitors: Vec<Monitor> = serde_json::from_str(&stdout)
        .map_err(|e| MyError(format!("Failed to parse hyprctl output: {}", e)))?;

    if monitors.is_empty() {
        return Err(MyError("No monitors detected. Is Hyprland running?".to_string()).into());
    }
    Ok(monitors)
}

pub(crate) fn compute_monitor_span(
    monitors: &[Monitor],
) -> Result<(i32, i32, i32, i32), Box<dyn std::error::Error>> {
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

pub(crate) fn wait_for_client_pid(
    pid: u32,
    timeout_secs: u64,
    verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
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

fn primary_client_for_pid(clients: &[Client], pid: u32) -> Option<&Client> {
    clients
        .iter()
        .filter(|c| c.pid == pid as i32)
        .max_by_key(|c| {
            let (w, h) = c.size.map(|s| (s[0], s[1])).unwrap_or((0, 0));
            i64::from(w.max(0)) * i64::from(h.max(0))
        })
}

pub(crate) fn get_primary_window_selector(
    pid: u32,
    verbose: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let stdout = execute_hyprctl_output(&["clients", "-j"], verbose)?;
    let clients: Vec<Client> = serde_json::from_str(&stdout)
        .map_err(|e| MyError(format!("Failed to parse hyprctl clients output: {}", e)))?;
    let selector = primary_client_for_pid(&clients, pid)
        .and_then(|c| c.address.as_ref().map(|a| format!("address:{}", a)))
        .unwrap_or_else(|| format!("pid:{}", pid));
    Ok(selector)
}

type ClientGeometry = (i32, i32, i32, i32);

fn get_client_geometry(
    pid: u32,
    verbose: bool,
) -> Result<Option<ClientGeometry>, Box<dyn std::error::Error>> {
    let stdout = execute_hyprctl_output(&["clients", "-j"], verbose)?;
    let clients: Vec<Client> = serde_json::from_str(&stdout)
        .map_err(|e| MyError(format!("Failed to parse hyprctl clients output: {}", e)))?;
    let client = primary_client_for_pid(&clients, pid);
    if let Some(c) = client
        && let (Some(at), Some(size)) = (c.at, c.size)
    {
        return Ok(Some((at[0], at[1], size[0], size[1])));
    }
    Ok(None)
}

pub(crate) fn fit_window_to_span(
    pid: u32,
    window: &str,
    target_x: i32,
    target_y: i32,
    target_w: i32,
    target_h: i32,
    verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Client, Monitor};

    #[test]
    fn compute_monitor_span_combines_offsets() {
        let monitors = vec![
            Monitor {
                name: Some("left".to_string()),
                width: 1920,
                height: 1080,
                x: -1920,
                y: 0,
            },
            Monitor {
                name: Some("right".to_string()),
                width: 2560,
                height: 1440,
                x: 0,
                y: 0,
            },
        ];
        let (min_x, min_y, w, h) = compute_monitor_span(&monitors).unwrap();
        assert_eq!(min_x, -1920);
        assert_eq!(min_y, 0);
        assert_eq!(w, 4480);
        assert_eq!(h, 1440);
    }

    #[test]
    fn primary_client_for_pid_prefers_largest_area() {
        let clients = vec![
            Client {
                pid: 100,
                address: Some("0x1".to_string()),
                at: Some([0, 0]),
                size: Some([800, 600]),
            },
            Client {
                pid: 100,
                address: Some("0x2".to_string()),
                at: Some([0, 0]),
                size: Some([1920, 1080]),
            },
            Client {
                pid: 200,
                address: Some("0x3".to_string()),
                at: Some([0, 0]),
                size: Some([3840, 2160]),
            },
        ];
        let selected = primary_client_for_pid(&clients, 100).unwrap();
        assert_eq!(selected.address.as_deref(), Some("0x2"));
    }
}
