use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

#[test]
fn config_show_uses_cli_overrides_and_appends_default_command() {
    let dir = tempdir().expect("tempdir");
    let config_path = dir.path().join("config.toml");

    let config_contents = r#"
gamescope_args = ["-r", "60"]
default_command = ["steam", "-applaunch", "620"]
render_scale = 0.8
startup_timeout_secs = 15
"#;
    fs::write(&config_path, config_contents).expect("write config");

    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("hyprfinity");
    cmd.args([
        "--config",
        config_path.to_str().expect("config path"),
        "config-show",
        "--render-scale",
        "1.2",
        "--startup-timeout-secs",
        "20",
        "--",
        "-r",
        "120",
    ]);

    cmd.assert().success().stdout(
        predicate::str::contains("Hyprfinity: Config path:")
            .and(predicate::str::contains(config_path.to_str().unwrap()))
            .and(predicate::str::contains(
                "[\"-r\", \"120\", \"--\", \"steam\", \"-applaunch\", \"620\"]",
            ))
            .and(predicate::str::contains("startup_timeout_secs"))
            .and(predicate::str::contains("20"))
            .and(predicate::str::contains("render_scale"))
            .and(predicate::str::contains("1")),
    );
}
