//! `A3S_ICON_SCALE` parsing (spec B21, Q3): garbage, non-finite and non-positive values fall
//! back to 1.5. The scale is read once per process, so each value runs in a child process.

use std::process::Command;

const CHILD: &str = "child_prints_scaled_sizes";

#[test]
fn icon_scale_env_values() {
    let exe = std::env::current_exe().expect("test binary path");
    for (value, expected) in [
        ("garbage", "18 24 96 65"),
        ("1,5", "18 24 96 65"),
        ("", "18 24 96 65"),
        ("NaN", "18 24 96 65"),
        ("inf", "18 24 96 65"),
        ("-Infinity", "18 24 96 65"),
        ("1e400", "18 24 96 65"),
        ("0", "18 24 96 65"),
        ("-1", "18 24 96 65"),
        ("0x2", "18 24 96 65"),
        // Java Double.parseDouble trims whitespace.
        (" 2 ", "24 32 128 65"),
        ("2", "24 32 128 65"),
        ("1.0", "12 16 64 65"),
        ("0.1", "8 8 8 65"),
        ("1.25", "15 20 80 65"),
    ] {
        let output = Command::new(&exe)
            .args([
                "--exact",
                CHILD,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("A3S_ICON_SCALE", value)
            .output()
            .expect("run child test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "child failed for {value:?}: {stdout}"
        );
        let line = stdout
            .lines()
            .find_map(|l| l.split_once("SCALED=").map(|(_, rest)| rest.trim()))
            .unwrap_or_else(|| panic!("no SCALED line for {value:?}: {stdout}"));
        assert_eq!(line, expected, "A3S_ICON_SCALE={value:?}");
    }
}

#[test]
#[ignore = "child process of icon_scale_env_values"]
fn child_prints_scaled_sizes() {
    let sizes = [12, 16, 64, 65].map(a3s_ui::icon::scaled_size);
    println!("SCALED={} {} {} {}", sizes[0], sizes[1], sizes[2], sizes[3]);
}
