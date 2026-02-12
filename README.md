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

Stop the active session:

```bash
hyprfinity gamescope-down
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
startup_timeout_secs = 10
```

## Notes

- Hyprfinity injects `-W/-H/-w/-h` defaults if you do not specify them in the Gamescope args.
- Use `--no-pin` to avoid pinning the Gamescope window to all workspaces.
- Use `--verbose` to show `hyprctl` debug output and Gamescope logs.
- Press Ctrl+C during `gamescope-up` to tear down the Gamescope session.
