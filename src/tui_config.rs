use crate::config::Config;
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
use std::error::Error;
use std::time::Duration;

fn format_optional_size(width: Option<i32>, height: Option<i32>) -> String {
    match (width, height) {
        (Some(w), Some(h)) => format!("{}x{}", w, h),
        (Some(w), None) => format!("{}x(auto)", w),
        (None, Some(h)) => format!("(auto)x{}", h),
        (None, None) => "auto".to_string(),
    }
}

pub(crate) fn apply_editor_defaults(mut config: Config, auto_scale: f32) -> Config {
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

pub(crate) fn edit_config_tui(
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
