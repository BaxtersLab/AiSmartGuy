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

/// Whether a package version agrees with the crate's. A Debian revision
/// (0.1.1-1: the same program, packaged again) is the package's own counter,
/// so what must agree is the upstream part -- the rule bxdeb's
/// bx_assert_versions_agree applies: an exact match, or a match once
/// everything after the LAST '-' is dropped.
fn agrees(control: &str, crate_v: &str) -> bool {
    control == crate_v || control.rsplit_once('-').is_some_and(|(upstream, _)| upstream == crate_v)
}

/// The rule itself, on numbers of its own: a revision passes, a different
/// upstream (with or without a revision) does not.
#[test]
fn the_version_rule_takes_a_revision_and_nothing_else() {
    assert!(agrees("0.1.1", "0.1.1"));
    assert!(agrees("0.1.1-1", "0.1.1"));
    assert!(agrees("0.1.1-12", "0.1.1"));
    assert!(agrees("0.1.1-rc1-2", "0.1.1-rc1"));   // only the LAST '-' starts the revision
    assert!(!agrees("0.1.2-1", "0.1.1"));
    assert!(!agrees("0.1.1.1", "0.1.1"));
    assert!(!agrees("0.1.1", "0.1.2"));
    assert!(!agrees("0.1-1", "0.1.1"));
}

/// The package, the Rust crate (which reports record as the engine version)
/// and the Tauri bundle carry one version: the control agrees with the crate
/// under the rule above, and the crate and the bundle agree exactly.
#[test]
fn one_version_everywhere() {
    let control = field(&read("packaging/DEBIAN/control"), "Version");
    let cargo = read("src-tauri/Cargo.toml");
    let cargo_v = cargo.lines().find_map(|l| l.strip_prefix("version = \"")).unwrap().trim_end_matches('"');
    let tauri = read("src-tauri/tauri.conf.json");
    let tauri_v: serde_json::Value = serde_json::from_str(&tauri).unwrap();
    assert!(agrees(&control, cargo_v), "control {control} vs src-tauri/Cargo.toml {cargo_v}");
    assert_eq!(cargo_v, tauri_v["version"].as_str().unwrap(), "src-tauri/Cargo.toml vs tauri.conf.json");
}
