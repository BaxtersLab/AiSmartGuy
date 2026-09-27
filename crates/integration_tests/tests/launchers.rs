//! The two launchers, run for real: `run.sh` (dev tree) and `packaging/run.sh`
//! (installed). Each is copied beside a stand-in for the app binary that
//! writes down the environment, working directory and arguments it received,
//! then launched through a symlink -- as a desktop entry or a user would --
//! with a clean environment and with a VS Code terminal's environment (snap
//! variables, some under $HOME/snap/ and some mid-list, and GDK_BACKEND).

use std::collections::HashMap;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

struct Launch {
    env: HashMap<String, String>,
    cwd: String,
    args: Vec<String>,
}

/// A temp app directory holding `launcher` and a stand-in binary at `bin`.
fn stage(tag: &str, launcher: &str, bin: &str, shim: bool) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("asg_launch_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let app = root.join("app");
    std::fs::create_dir_all(app.join(Path::new(bin).parent().unwrap())).unwrap();
    std::fs::copy(repo().join(launcher), app.join("run.sh")).unwrap();
    let fake = app.join(bin);
    std::fs::write(&fake, "#!/bin/sh\nenv -0 > \"$DUMP.env\"\npwd > \"$DUMP.cwd\"\nprintf '%s\\n' \"$@\" > \"$DUMP.args\"\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    if shim {
        std::fs::create_dir_all(app.join("schema-shim")).unwrap();
        std::fs::write(app.join("schema-shim/gschemas.compiled"), b"GVariant").unwrap();
    }
    let link_dir = root.join("elsewhere");
    std::fs::create_dir_all(&link_dir).unwrap();
    symlink(app.join("run.sh"), link_dir.join("aismartguy")).unwrap();
    (root, link_dir.join("aismartguy"))
}

fn launch(root: &Path, link: &Path, env: &[(&str, String)]) -> Launch {
    let dump = root.join("dump");
    let mut cmd = Command::new("bash");
    cmd.arg(link).arg("--flag").env_clear().current_dir(root);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.env("DUMP", &dump);
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "launcher failed: {}", String::from_utf8_lossy(&out.stderr));
    let raw = std::fs::read(dump.with_extension("env")).unwrap();
    let env = raw
        .split(|b| *b == 0)
        .filter(|e| !e.is_empty())
        .filter_map(|e| {
            let s = String::from_utf8_lossy(e);
            s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect();
    Launch {
        env,
        cwd: std::fs::read_to_string(dump.with_extension("cwd")).unwrap().trim().to_string(),
        args: std::fs::read_to_string(dump.with_extension("args")).unwrap().lines().map(String::from).collect(),
    }
}

/// What a VS Code (snap) integrated terminal exports on this box, reduced to
/// the shapes that matter.
fn vscode_env(home: &Path) -> Vec<(&'static str, String)> {
    let h = home.display();
    vec![
        ("HOME", h.to_string()),
        ("PATH", "/snap/bin:/usr/local/bin:/usr/bin:/bin".into()),
        ("GDK_BACKEND", "x11".into()),
        ("ELECTRON_RUN_AS_NODE", "1".into()),
        ("LOCPATH", "/snap/core20/current/usr/lib/locale".into()),
        ("GSETTINGS_SCHEMA_DIR", format!("{h}/snap/code/265/.local/share/glib-2.0/schemas")),
        ("GTK_PATH", "/usr/lib/x86_64-linux-gnu/gtk-3.0:/snap/code/265/usr/lib/gtk-3.0".into()),
        ("XDG_CONFIG_DIRS", format!("/etc/xdg:{h}/snap/code/265/etc/xdg")),
        ("XDG_DATA_HOME", format!("{h}/snap/code/265/.local/share")),
        ("XDG_DATA_DIRS", format!("{h}/snap/code/265/.local/share:/snap/code/265/usr/share:/usr/share/ubuntu:/var/lib/snapd/desktop:/usr/share")),
        ("KEEP_ME", "/usr/share/keep".into()),
        ("NOT_A_SNAP", "/snapshots/backup".into()),
    ]
}

fn assert_scrubbed(l: &Launch) {
    for gone in ["LOCPATH", "GSETTINGS_SCHEMA_DIR", "GTK_PATH", "XDG_CONFIG_DIRS", "XDG_DATA_HOME", "ELECTRON_RUN_AS_NODE"] {
        assert!(!l.env.contains_key(gone), "{gone} survived the scrub: {:?}", l.env.get(gone));
    }
    assert_eq!(l.env["XDG_DATA_DIRS"], "/usr/share/ubuntu:/var/lib/snapd/desktop:/usr/share");
    assert_eq!(l.env["KEEP_ME"], "/usr/share/keep");
    assert_eq!(l.env["NOT_A_SNAP"], "/snapshots/backup");
    assert_eq!(l.args, ["--flag"]);
}

// ── dev run.sh ───────────────────────────────────────────────────────────────

#[test]
fn dev_launcher_scrubs_a_vscode_environment_in_a_fresh_clone() {
    // Fresh clone: schema-shim/gschemas.compiled is git-ignored, so absent.
    let (root, link) = stage("dev_vscode", "run.sh", "target/release/aismartguy-app", false);
    let l = launch(&root, &link, &vscode_env(&root));
    let _ = std::fs::remove_dir_all(&root);
    assert_scrubbed(&l);
    assert_eq!(l.env["GDK_BACKEND"], "x11");
    assert_eq!(l.env["PATH"], "/snap/bin:/usr/local/bin:/usr/bin:/bin", "the dev launcher leaves PATH alone");
    assert!(l.cwd.ends_with("/app"), "run.sh must cd to its real directory: {}", l.cwd);
}

#[test]
fn dev_launcher_points_gsettings_at_the_shim_when_it_is_compiled() {
    let (root, link) = stage("dev_shim", "run.sh", "target/release/aismartguy-app", true);
    let l = launch(&root, &link, &vscode_env(&root));
    let _ = std::fs::remove_dir_all(&root);
    assert!(l.env["GSETTINGS_SCHEMA_DIR"].ends_with("/app/schema-shim"), "{:?}", l.env.get("GSETTINGS_SCHEMA_DIR"));
}

#[test]
fn dev_launcher_from_a_clean_environment() {
    let (root, link) = stage("dev_clean", "run.sh", "target/release/aismartguy-app", false);
    let l = launch(&root, &link, &[("HOME", root.display().to_string()), ("PATH", "/usr/bin:/bin".into())]);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(l.env["GDK_BACKEND"], "x11", "set explicitly, not inherited");
    assert!(!l.env.contains_key("GSETTINGS_SCHEMA_DIR"));
    assert_eq!(l.args, ["--flag"]);
}

#[test]
fn dev_launcher_overrides_an_inherited_wayland_backend() {
    let (root, link) = stage("dev_wl", "run.sh", "target/release/aismartguy-app", false);
    let l = launch(&root, &link, &[("HOME", root.display().to_string()), ("PATH", "/usr/bin:/bin".into()),
                                   ("GDK_BACKEND", "wayland".into())]);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(l.env["GDK_BACKEND"], "x11");
}

// ── installed packaging/run.sh ───────────────────────────────────────────────

#[test]
fn installed_launcher_scrubs_a_vscode_environment() {
    // Installed layout: the binary sits beside the launcher.
    let (root, link) = stage("pkg_vscode", "packaging/run.sh", "aismartguy-app", false);
    let l = launch(&root, &link, &vscode_env(&root));
    let _ = std::fs::remove_dir_all(&root);
    assert_scrubbed(&l);
    assert_eq!(l.env["GDK_BACKEND"], "x11");
    assert!(l.cwd.ends_with("/app"));
}

#[test]
fn installed_launcher_from_a_clean_environment() {
    let (root, link) = stage("pkg_clean", "packaging/run.sh", "aismartguy-app", false);
    let l = launch(&root, &link, &[("HOME", root.display().to_string()), ("PATH", "/usr/bin:/bin".into())]);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(l.args, ["--flag"]);
    assert!(!l.env.contains_key("ELECTRON_RUN_AS_NODE"));
    assert_eq!(l.env["GDK_BACKEND"], "x11", "the close button needs mutter's titlebar");
}

#[test]
fn installed_launcher_overrides_an_inherited_wayland_backend() {
    let (root, link) = stage("pkg_wl", "packaging/run.sh", "aismartguy-app", false);
    let l = launch(&root, &link, &[("HOME", root.display().to_string()), ("PATH", "/usr/bin:/bin".into()),
                                   ("GDK_BACKEND", "wayland".into())]);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(l.env["GDK_BACKEND"], "x11");
}
