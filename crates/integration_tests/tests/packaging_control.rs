//! What the .deb declares (packaging/DEBIAN/control), read from the source
//! tree. The estate gate checks the control inside the BUILT package; this
//! catches the same mistakes before anything is built.

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{}/../../{rel}", env!("CARGO_MANIFEST_DIR")))
        .unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// A control field, continuation lines folded in.
fn field(control: &str, name: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in control.lines() {
        if let Some(rest) = line.strip_prefix(&format!("{name}:")) {
            inside = true;
            out.push_str(rest.trim());
        } else if inside && line.starts_with(' ') {
            out.push(' ');
            out.push_str(line.trim());
        } else {
            inside = false;
        }
    }
    out
}

/// Without a llama.cpp the installed app can analyse nothing. The archive's
/// llama.cpp-tools puts llama-completion on PATH, where the app finds it
/// (supervisor ASG-Q1, approved 2026-09-27).
#[test]
fn the_package_depends_on_the_archive_llama() {
    let depends = field(&read("packaging/DEBIAN/control"), "Depends");
    let names: Vec<&str> = depends.split(',').map(|d| d.trim().split(' ').next().unwrap()).collect();
    assert!(names.contains(&"llama.cpp-tools"), "Depends: {depends}");
}

/// The package, the Rust crate (which reports record as the engine version)
/// and the Tauri bundle carry one version.
#[test]
fn one_version_everywhere() {
    let control = field(&read("packaging/DEBIAN/control"), "Version");
    let cargo = read("src-tauri/Cargo.toml");
    let cargo_v = cargo.lines().find_map(|l| l.strip_prefix("version = \"")).unwrap().trim_end_matches('"');
    let tauri = read("src-tauri/tauri.conf.json");
    let tauri_v: serde_json::Value = serde_json::from_str(&tauri).unwrap();
    assert_eq!(control, cargo_v, "control vs src-tauri/Cargo.toml");
    assert_eq!(control, tauri_v["version"].as_str().unwrap(), "control vs tauri.conf.json");
}
