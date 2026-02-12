use crate::hyprland::{compute_monitor_span, get_monitors};
use crate::types::AutoTuneProfile;
use std::process::Command;

pub(crate) fn detect_total_memory_gib() -> Option<f32> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = meminfo.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb = line
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<u64>().ok())?;
    Some(kb as f32 / 1024.0 / 1024.0)
}

pub(crate) fn detect_span_size() -> Option<(i32, i32)> {
    let monitors = get_monitors(false).ok()?;
    let (_, _, w, h) = compute_monitor_span(&monitors).ok()?;
    Some((w, h))
}

pub(crate) fn detect_span_pixels() -> Option<i64> {
    let (w, h) = detect_span_size()?;
    Some(i64::from(w) * i64::from(h))
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
    stdout
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
        .collect()
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
    detect_gpu_models()
        .into_iter()
        .max_by_key(|model| gpu_model_score(model))
}

fn detect_gpu_vram_gib() -> Option<f32> {
    let mut best_vram_bytes: Option<u64> = None;
    let entries = std::fs::read_dir("/sys/class/drm").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("card") || name.contains('-') {
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

pub(crate) fn detect_auto_tune_profile() -> AutoTuneProfile {
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
