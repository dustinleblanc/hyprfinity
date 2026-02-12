# Hyprfinity (Gamescope Spanning)

Hyprfinity is a small CLI helper for Hyprland that launches a Gamescope session and spans it across all connected monitors. It computes the full monitor bounding box via `hyprctl`, launches `gamescope`, then floats, moves, and resizes the Gamescope window to cover the whole span.

## Requirements

- Hyprland (Wayland)
- `hyprctl` available in `PATH`
- `gamescope` available in `PATH`

## Usage

Default (same as `gamescope-up`):

```bash
hyprfinity
```

Launch with an explicit command:

```bash
hyprfinity gamescope-up -- -- steam -applaunch 620
```

Pick from a fuzzy TUI list of applications (when no command is provided):

```bash
hyprfinity gamescope-up
```

Force the picker even if a command is provided:

```bash
hyprfinity gamescope-up --pick -- -- steam -applaunch 620
```

Hide Waybar while Gamescope runs (restored on exit):

```bash
hyprfinity gamescope-up --hide-waybar -- -- steam -applaunch 620
```

Launch at 75% internal render scale (keeps full output span, lowers internal render cost):

```bash
hyprfinity gamescope-up --render-scale 0.75 -- -- steam -applaunch 620
```

Use an interactive monitor-aware picker for internal render size:

```bash
hyprfinity gamescope-up --pick-size -- -- steam -applaunch 620
```

Stop the active session:

```bash
hyprfinity gamescope-down
```

Run interactive configuration for output/internal sizing:

```bash
hyprfinity config
```

## Configuration

Hyprfinity reads a config file from:

- `$XDG_CONFIG_HOME/hyprfinity/config.toml`
- or `~/.config/hyprfinity/config.toml`

You can override the path with `--config /path/to/config.toml`.

Generate a starter config:

```bash
hyprfinity config-init
```

Overwrite if it already exists:

```bash
hyprfinity config-init --force
```

Show the resolved config:

```bash
hyprfinity config-show
```

Show effective values with CLI overrides:

```bash
hyprfinity config-show --no-pin -- -r 60
```

Example config:

```toml
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
```

## Notes

- Hyprfinity injects `-W/-H` defaults using the configured `output_width`/`output_height` when present, otherwise full monitor span.
- Hyprfinity injects `-w/-h` defaults using internal render settings: `virtual_width`/`virtual_height` (if set), otherwise `render_scale * output_size`.
- `hide_waybar` defaults to `true` to avoid top-bar overlay; set it to `false` if you want to keep your bar visible.
- `hyprfinity config` is an interactive wizard for setting output size and internal render mode (scale or explicit virtual size).
- `--pick-size` opens an interactive picker that detects monitors and offers internal size presets (native span, scaled percentages, common heights like 1080p-equivalent).
- Use `--no-pin` to avoid pinning the Gamescope window to all workspaces.
- Use `--verbose` to show `hyprctl` debug output and Gamescope logs.
- Press Ctrl+C during `gamescope-up` to tear down the Gamescope session.
