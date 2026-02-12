# Hyprfinity (Gamescope Spanning)

Hyprfinity is a small CLI helper for Hyprland that launches a Gamescope session and spans it across all connected monitors. It computes the full monitor bounding box via `hyprctl`, launches `gamescope`, then floats, moves, and resizes the Gamescope window to cover the whole span.

## Requirements

- Hyprland (Wayland)
- `hyprctl` available in `PATH`
- `gamescope` available in `PATH`

## Usage

Launch with an explicit command:

```bash
hyprfinity gamescope-up -- -- steam -applaunch 620
```

Pick from a terminal list of applications (when no command is provided):

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

## Notes

- Hyprfinity injects `-W/-H/-w/-h` defaults if you do not specify them in the Gamescope args.
- Use `--no-pin` to avoid pinning the Gamescope window to all workspaces.
- Use `--verbose` to show `hyprctl` debug output and Gamescope logs.
- Press Ctrl+C during `gamescope-up` to tear down the Gamescope session.
