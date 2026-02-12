use crate::MyError;
use crate::types::{DesktopApp, Monitor, SizePreset};
use crate::util::{clamp_i32, even_floor, scaled_dimensions};
use skim::prelude::*;
use std::collections::BTreeSet;

pub(crate) fn build_size_presets(span_width: i32, span_height: i32) -> Vec<SizePreset> {
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

pub(crate) fn pick_internal_size(
    monitors: &[Monitor],
    span_width: i32,
    span_height: i32,
) -> Result<Option<(i32, i32)>, Box<dyn std::error::Error>> {
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

pub(crate) fn list_desktop_apps() -> Result<Vec<DesktopApp>, Box<dyn std::error::Error>> {
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

pub(crate) fn sanitize_exec(exec: &str) -> String {
    let mut cleaned = exec.to_string();
    for token in [
        "%U", "%u", "%F", "%f", "%i", "%c", "%k", "%d", "%D", "%n", "%N", "%v", "%m", "%M", "%r",
        "%R",
    ] {
        cleaned = cleaned.replace(token, "");
    }
    cleaned.trim().to_string()
}

pub(crate) fn pick_desktop_app_command() -> Result<Vec<String>, Box<dyn std::error::Error>> {
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
