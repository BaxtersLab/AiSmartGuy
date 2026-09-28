//! Downloading llama.cpp: pinned, verified, and only when the user confirms.
//!
//! The manager's ruling on ASG-Q2 (2026-09-27, `constitution/outbox/
//! 2026-09-27-supervisor-to-manager-aismartguy-llama-download.md`) replaced a
//! download of whatever GitHub called `releases/latest`, unchecked:
//! - immutable tag URLs, never `latest`;
//! - an exact allowlist of (os, arch, backend) → asset, SHA-256 and size;
//! - a private temporary file, the size enforced while streaming, the hash
//!   checked before a byte is extracted;
//! - only the pinned layout is extracted: unsafe paths, hardlinks, devices and
//!   anything not pinned are refused, and a link is accepted only if the pin
//!   names it with the same target (the link is then made from the PIN);
//! - installed atomically: the previous install survives any failure;
//! - no fallback, and no network unless the user confirmed the plan shown.
//!
//! The one caller is `cmd_install_llama`, behind the confirmation in the
//! window. Nothing here runs at start-up or at package install.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

/// The upstream release pinned. Changing it (or any hash) needs a new
/// AiSmartGuy version (ruling, "record the hash source and bump").
pub const TAG: &str = "b10238";
const BASE: &str = "https://github.com/ggml-org/llama.cpp/releases/download";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// A regular file in the release directory.
    File(&'static str),
    /// A symlink in the release directory and the (sibling) file it names.
    Link(&'static str, &'static str),
    /// An unversioned development link (`libllama.so -> libllama.so.0`): in
    /// the archive, checked against its pinned target, and NOT installed.
    /// Nothing in the release loads these names (no NEEDED entry, no dlopen
    /// string), and installing them would put a link-to-a-link in the tree.
    DevLink(&'static str, &'static str),
}

impl Entry {
    pub fn name(&self) -> &'static str {
        match self {
            Entry::File(n) | Entry::Link(n, _) | Entry::DevLink(n, _) => n,
        }
    }
}

#[derive(Debug)]
pub struct Pin {
    pub os: &'static str,
    pub arch: &'static str,
    pub backend: &'static str,
    pub asset: &'static str,
    pub sha256: &'static str,
    /// Exact size in bytes; more is cut off while streaming, less refused.
    pub bytes: u64,
    /// The one directory every entry sits in, inside the archive.
    pub dir: &'static str,
    pub layout: &'static [Entry],
}

// Hash source, 2026-09-28: both assets fetched from
// https://github.com/ggml-org/llama.cpp/releases/download/b10238/<asset>,
// hashed locally with sha256sum, and each hash equal to the `digest` GitHub
// publishes for the asset (releases/tags/b10238 API). Sizes are the bytes
// fetched. Layouts are the archives' own listings. b10238 is the build this
// estate ran on the GTX 1660 SUPER (2026-09-27).
// Only Linux x86_64 is pinned: nothing else is supported on Baxters OS, and
// an unpinned platform is refused (the distribution's llama.cpp-tools is
// found on PATH instead).
pub static PINS: &[Pin] = &[
    Pin {
        os: "linux",
        arch: "x86_64",
        backend: "vulkan",
        asset: "llama-b10238-bin-ubuntu-vulkan-x64.tar.gz",
        sha256: "22517e74c3b5f18412d19b213679715b8fc3606f104cca4a50763c1a4c7c31a2",
        bytes: 32_453_759,
        dir: "llama-b10238",
        layout: LAYOUT_LINUX_X64_VULKAN,
    },
    Pin {
        os: "linux",
        arch: "x86_64",
        backend: "cpu",
        asset: "llama-b10238-bin-ubuntu-x64.tar.gz",
        sha256: "e58fdf16587523699fe446246ed8d394074a3db6f7611de88ba76b7e3f56c847",
        bytes: 16_465_416,
        dir: "llama-b10238",
        layout: LAYOUT_LINUX_X64_CPU,
    },
];

const LAYOUT_LINUX_X64_VULKAN: &[Entry] = &[
    Entry::File("LICENSE"),
    Entry::File("ggml-rpc-server"),
    Entry::DevLink("libggml-base.so", "libggml-base.so.0"),
    Entry::Link("libggml-base.so.0", "libggml-base.so.0.18.0"),
    Entry::File("libggml-base.so.0.18.0"),
    Entry::File("libggml-cpu-alderlake.so"),
    Entry::File("libggml-cpu-cannonlake.so"),
    Entry::File("libggml-cpu-cascadelake.so"),
    Entry::File("libggml-cpu-cooperlake.so"),
    Entry::File("libggml-cpu-haswell.so"),
    Entry::File("libggml-cpu-icelake.so"),
    Entry::File("libggml-cpu-ivybridge.so"),
    Entry::File("libggml-cpu-piledriver.so"),
    Entry::File("libggml-cpu-sandybridge.so"),
    Entry::File("libggml-cpu-sapphirerapids.so"),
    Entry::File("libggml-cpu-skylakex.so"),
    Entry::File("libggml-cpu-sse42.so"),
    Entry::File("libggml-cpu-x64.so"),
    Entry::File("libggml-cpu-zen4.so"),
    Entry::File("libggml-rpc.so"),
    Entry::File("libggml-vulkan.so"),
    Entry::DevLink("libggml.so", "libggml.so.0"),
    Entry::Link("libggml.so.0", "libggml.so.0.18.0"),
    Entry::File("libggml.so.0.18.0"),
    Entry::File("libllama-batched-bench-impl.so"),
    Entry::File("libllama-bench-impl.so"),
    Entry::File("libllama-cli-impl.so"),
    Entry::DevLink("libllama-common.so", "libllama-common.so.0"),
    Entry::Link("libllama-common.so.0", "libllama-common.so.0.0.10238"),
    Entry::File("libllama-common.so.0.0.10238"),
    Entry::File("libllama-completion-impl.so"),
    Entry::File("libllama-fit-params-impl.so"),
    Entry::File("libllama-perplexity-impl.so"),
    Entry::File("libllama-quantize-impl.so"),
    Entry::File("libllama-server-impl.so"),
    Entry::DevLink("libllama.so", "libllama.so.0"),
    Entry::Link("libllama.so.0", "libllama.so.0.0.10238"),
    Entry::File("libllama.so.0.0.10238"),
    Entry::DevLink("libmtmd.so", "libmtmd.so.0"),
    Entry::Link("libmtmd.so.0", "libmtmd.so.0.0.10238"),
    Entry::File("libmtmd.so.0.0.10238"),
    Entry::File("llama"),
    Entry::File("llama-batched-bench"),
    Entry::File("llama-bench"),
    Entry::File("llama-cli"),
    Entry::File("llama-completion"),
    Entry::File("llama-debug-template-parser"),
    Entry::File("llama-fit-params"),
    Entry::File("llama-gemma3-cli"),
    Entry::File("llama-gguf-split"),
    Entry::File("llama-imatrix"),
    Entry::File("llama-llava-cli"),
    Entry::File("llama-minicpmv-cli"),
    Entry::File("llama-mtmd-cli"),
    Entry::File("llama-mtmd-debug"),
    Entry::File("llama-perplexity"),
    Entry::File("llama-quantize"),
    Entry::File("llama-qwen2vl-cli"),
    Entry::File("llama-results"),
    Entry::File("llama-server"),
    Entry::File("llama-template-analysis"),
    Entry::File("llama-tokenize"),
    Entry::File("llama-tts"),
];

const LAYOUT_LINUX_X64_CPU: &[Entry] = &[
    Entry::File("LICENSE"),
    Entry::File("ggml-rpc-server"),
    Entry::DevLink("libggml-base.so", "libggml-base.so.0"),
    Entry::Link("libggml-base.so.0", "libggml-base.so.0.18.0"),
    Entry::File("libggml-base.so.0.18.0"),
    Entry::File("libggml-cpu-alderlake.so"),
    Entry::File("libggml-cpu-cannonlake.so"),
    Entry::File("libggml-cpu-cascadelake.so"),
    Entry::File("libggml-cpu-cooperlake.so"),
    Entry::File("libggml-cpu-haswell.so"),
    Entry::File("libggml-cpu-icelake.so"),
    Entry::File("libggml-cpu-ivybridge.so"),
    Entry::File("libggml-cpu-piledriver.so"),
    Entry::File("libggml-cpu-sandybridge.so"),
    Entry::File("libggml-cpu-sapphirerapids.so"),
    Entry::File("libggml-cpu-skylakex.so"),
    Entry::File("libggml-cpu-sse42.so"),
    Entry::File("libggml-cpu-x64.so"),
    Entry::File("libggml-cpu-zen4.so"),
    Entry::File("libggml-rpc.so"),
    Entry::DevLink("libggml.so", "libggml.so.0"),
    Entry::Link("libggml.so.0", "libggml.so.0.18.0"),
    Entry::File("libggml.so.0.18.0"),
    Entry::File("libllama-batched-bench-impl.so"),
    Entry::File("libllama-bench-impl.so"),
    Entry::File("libllama-cli-impl.so"),
    Entry::DevLink("libllama-common.so", "libllama-common.so.0"),
    Entry::Link("libllama-common.so.0", "libllama-common.so.0.0.10238"),
    Entry::File("libllama-common.so.0.0.10238"),
    Entry::File("libllama-completion-impl.so"),
    Entry::File("libllama-fit-params-impl.so"),
    Entry::File("libllama-perplexity-impl.so"),
    Entry::File("libllama-quantize-impl.so"),
    Entry::File("libllama-server-impl.so"),
    Entry::DevLink("libllama.so", "libllama.so.0"),
    Entry::Link("libllama.so.0", "libllama.so.0.0.10238"),
    Entry::File("libllama.so.0.0.10238"),
    Entry::DevLink("libmtmd.so", "libmtmd.so.0"),
    Entry::Link("libmtmd.so.0", "libmtmd.so.0.0.10238"),
    Entry::File("libmtmd.so.0.0.10238"),
    Entry::File("llama"),
    Entry::File("llama-batched-bench"),
    Entry::File("llama-bench"),
    Entry::File("llama-cli"),
    Entry::File("llama-completion"),
    Entry::File("llama-debug-template-parser"),
    Entry::File("llama-fit-params"),
    Entry::File("llama-gemma3-cli"),
    Entry::File("llama-gguf-split"),
    Entry::File("llama-imatrix"),
    Entry::File("llama-llava-cli"),
    Entry::File("llama-minicpmv-cli"),
    Entry::File("llama-mtmd-cli"),
    Entry::File("llama-mtmd-debug"),
    Entry::File("llama-perplexity"),
    Entry::File("llama-quantize"),
    Entry::File("llama-qwen2vl-cli"),
    Entry::File("llama-results"),
    Entry::File("llama-server"),
    Entry::File("llama-template-analysis"),
    Entry::File("llama-tokenize"),
    Entry::File("llama-tts"),
];

/// What the window shows before the user confirms.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Plan {
    pub tag: String,
    pub asset: String,
    pub backend: String,
    pub bytes: u64,
    pub url: String,
}

pub fn url_for(pin: &Pin) -> String {
    format!("{BASE}/{TAG}/{}", pin.asset)
}

pub fn plan(pin: &Pin) -> Plan {
    Plan {
        tag: TAG.to_string(),
        asset: pin.asset.to_string(),
        backend: pin.backend.to_string(),
        bytes: pin.bytes,
        url: url_for(pin),
    }
}

/// As before: an NVIDIA GPU takes the Vulkan build (upstream publishes no
/// CUDA build for Linux), anything else the CPU build.
pub fn backend_for(has_nvidia: bool) -> &'static str {
    if has_nvidia { "vulkan" } else { "cpu" }
}

/// The pinned asset for a platform, or why there is none. Refused before any
/// network access.
pub fn pin_for(pins: &'static [Pin], os: &str, arch: &str, backend: &str) -> Result<&'static Pin, String> {
    pins.iter()
        .find(|p| p.os == os && p.arch == arch && p.backend == backend)
        .ok_or_else(|| format!(
            "no verified llama.cpp build is pinned for {os}/{arch}/{backend}: nothing was downloaded. \
             Install the distribution's llama.cpp-tools instead."))
}

/// A pin the installer will use (supervisor's ruling on ASG-Q5): every link
/// targets a bare file name in the same directory (no `/`, so no `..` and no
/// absolute path), and that target is itself a pinned regular file (no
/// dangling link, no link to a link). Checked before any download.
pub fn pin_is_sound(pin: &Pin) -> Result<(), String> {
    let mut names = HashSet::new();
    for e in pin.layout {
        if e.name().is_empty() || e.name().contains('/') || !names.insert(e.name()) {
            return Err(format!("unsound pin for {}: bad or duplicate name {:?}", pin.asset, e.name()));
        }
    }
    for e in pin.layout {
        if let Entry::Link(name, target) = e {
            if target.is_empty() || target.contains('/') {
                return Err(format!("unsound pin for {}: link {name} -> {target} is not a bare file name", pin.asset));
            }
            if !pin.layout.contains(&Entry::File(target)) {
                return Err(format!("unsound pin for {}: link {name} -> {target} does not target a pinned file", pin.asset));
            }
        }
        if let Entry::DevLink(name, target) = e {
            if target.is_empty() || target.contains('/') || !names.contains(target) {
                return Err(format!("unsound pin for {}: development link {name} -> {target} is not a pinned bare name", pin.asset));
            }
        }
    }
    Ok(())
}

/// The window showed `asset`; refuse anything else, so a confirmation can
/// never be spent on a different download.
pub fn confirm(pin: &Pin, asset: &str) -> Result<(), String> {
    if pin.asset == asset {
        Ok(())
    } else {
        Err(format!(
            "the confirmed download ({asset}) is not the one pinned for this machine ({}); nothing was downloaded",
            pin.asset))
    }
}

/// A file removed when dropped, unless kept.
struct TempFile {
    path: PathBuf,
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A directory removed when dropped, unless disarmed.
struct TempDir {
    path: PathBuf,
    armed: bool,
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn unique(stem: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!(".{stem}.{}-{nanos}", std::process::id())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Stream `url` into a private temporary file in `dir`: cut off past the
/// pinned size, then length and hash checked. On any error the file is gone.
fn download(url: &str, pin: &Pin, dir: &Path, progress: &dyn Fn(u64)) -> Result<TempFile, String> {
    let resp = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(30))
        .timeout_read(std::time::Duration::from_secs(60))
        .build()
        .get(url)
        .call()
        .map_err(|e| format!("download failed: {e}"))?;
    if let Some(len) = resp.header("Content-Length").and_then(|v| v.trim().parse::<u64>().ok()) {
        if len != pin.bytes {
            return Err(format!(
                "the server offers {len} bytes, the pinned {} is {} bytes: refused before downloading",
                pin.asset, pin.bytes));
        }
    }
    let path = dir.join(unique(pin.asset));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&path)
        .map_err(|e| format!("cannot create a private temporary file in {}: {e}", dir.display()))?;
    let tmp = TempFile { path };

    let mut reader = resp.into_reader();
    let mut hasher = Sha256::new();
    let mut total: u64 = 0;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!(
            "download interrupted after {total} of {} bytes: {e}; nothing was installed", pin.bytes))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > pin.bytes {
            return Err(format!(
                "download cut off: more than the pinned {} bytes; nothing was installed", pin.bytes));
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| format!("cannot write the download: {e}"))?;
        progress(total);
    }
    if total != pin.bytes {
        return Err(format!(
            "incomplete download: {total} of {} bytes; nothing was installed", pin.bytes));
    }
    file.sync_all().map_err(|e| format!("cannot write the download: {e}"))?;
    let got = hex(&hasher.finalize());
    if got != pin.sha256 {
        return Err(format!(
            "hash mismatch for {}: got {got}, pinned {}; nothing was installed", pin.asset, pin.sha256));
    }
    Ok(tmp)
}

/// Unpack a verified archive into the empty directory `into`, flattened, and
/// only what the pin lists. Each refusal names the check that refused.
fn extract(archive: &Path, pin: &Pin, into: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("cannot open the download: {e}"))?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut seen: HashSet<&'static str> = HashSet::new();
    for entry in ar.entries().map_err(|e| format!("archive unreadable: {e}"))? {
        let mut entry = entry.map_err(|e| format!("archive unreadable: {e}"))?;
        let raw = entry.path().map_err(|e| format!("archive unreadable: {e}"))?.into_owned();
        if raw.is_absolute() || raw.components().any(|c| !matches!(c, Component::Normal(_))) {
            return Err(format!("unsafe path in the archive: {}", raw.display()));
        }
        let kind = entry.header().entry_type();
        if kind.is_hard_link() {
            return Err(format!("hardlink refused: {}", raw.display()));
        }
        if !(kind.is_file() || kind.is_symlink() || kind.is_dir()) {
            return Err(format!("entry type {kind:?} refused: {}", raw.display()));
        }
        let parts: Vec<&str> = raw.components()
            .map(|c| c.as_os_str().to_str().unwrap_or("\u{fffd}"))
            .collect();
        if parts.first() != Some(&pin.dir) || parts.len() > 2 {
            return Err(format!("outside the pinned layout: {}", raw.display()));
        }
        if parts.len() == 1 {
            if kind.is_dir() {
                continue;
            }
            return Err(format!("outside the pinned layout: {}", raw.display()));
        }
        let name = parts[1];
        let pinned = pin.layout.iter().find(|e| e.name() == name)
            .ok_or_else(|| format!("not in the pinned layout: {name}"))?;
        if !seen.insert(pinned.name()) {
            return Err(format!("listed twice in the archive: {name}"));
        }
        let out = into.join(pinned.name());
        match *pinned {
            Entry::File(_) if kind.is_file() => {
                let mode = entry.header().mode().unwrap_or(0o644) & 0o755;
                let mut f = std::fs::File::create(&out).map_err(|e| format!("cannot write {name}: {e}"))?;
                std::io::copy(&mut entry, &mut f).map_err(|e| format!("cannot write {name}: {e}"))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode))
                        .map_err(|e| format!("cannot set the mode of {name}: {e}"))?;
                }
                #[cfg(not(unix))]
                let _ = mode;
            }
            Entry::Link(_, target) if kind.is_symlink() => {
                let named = entry.link_name().map_err(|e| format!("archive unreadable: {e}"))?;
                if named.as_deref() != Some(Path::new(target)) {
                    return Err(format!("link {name} points to {:?}, the pin says {target}", named));
                }
                #[cfg(unix)]
                std::os::unix::fs::symlink(target, &out)
                    .map_err(|e| format!("cannot make link {name}: {e}"))?;
                #[cfg(not(unix))]
                return Err(format!("links are not supported here: {name}"));
            }
            Entry::DevLink(_, target) if kind.is_symlink() => {
                let named = entry.link_name().map_err(|e| format!("archive unreadable: {e}"))?;
                if named.as_deref() != Some(Path::new(target)) {
                    return Err(format!("link {name} points to {:?}, the pin says {target}", named));
                }
                // Verified, and deliberately not installed.
            }
            Entry::File(_) if kind.is_symlink() => return Err(format!("link not in the pinned layout: {name}")),
            Entry::Link(..) if kind.is_file() => return Err(format!("pinned as a link, the archive has a file: {name}")),
            _ => return Err(format!("entry type {kind:?} does not match the pin: {name}")),
        }
    }
    if let Some(missing) = pin.layout.iter().find(|e| !seen.contains(e.name())) {
        return Err(format!("missing from the archive: {}", missing.name()));
    }
    Ok(())
}

/// Points where a test can inject a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Extracted,
    Swapping,
}

/// Download, verify and install `pin` from `url` into `install_dir`. The
/// previous install is untouched until the new one is complete, and restored
/// if the swap fails.
pub fn install(
    pin: &Pin,
    url: &str,
    install_dir: &Path,
    progress: &dyn Fn(u64),
    fail_at: &dyn Fn(Stage) -> Result<(), String>,
) -> Result<PathBuf, String> {
    pin_is_sound(pin)?;
    let parent = install_dir.parent().ok_or("the install directory has no parent")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    let archive = download(url, pin, parent, progress)?;

    let staged = TempDir { path: parent.join(unique("llama-cpp.new")), armed: true };
    std::fs::create_dir(&staged.path).map_err(|e| format!("cannot create the staging directory: {e}"))?;
    extract(&archive.path, pin, &staged.path)?;
    let bin = staged.path.join(model_loader::LLAMA_BIN);
    if !bin.is_file() {
        return Err(format!("{} is not in the archive", model_loader::LLAMA_BIN));
    }
    fail_at(Stage::Extracted)?;

    let old = parent.join(unique("llama-cpp.old"));
    let had_old = install_dir.exists();
    if had_old {
        std::fs::rename(install_dir, &old)
            .map_err(|e| format!("cannot move the previous install aside: {e}"))?;
    }
    let swapped = fail_at(Stage::Swapping).and_then(|()| std::fs::rename(&staged.path, install_dir)
        .map_err(|e| format!("cannot move the new install into place: {e}")));
    if let Err(e) = swapped {
        if had_old {
            let _ = std::fs::rename(&old, install_dir);
        }
        return Err(format!("{e}; the previous install is unchanged"));
    }
    let mut staged = staged;
    staged.armed = false;
    if had_old {
        let _ = std::fs::remove_dir_all(&old);
    }
    Ok(install_dir.join(model_loader::LLAMA_BIN))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // ── crafted archives: raw headers, so unsafe names can be written ──────

    enum T<'a> {
        File(&'a str, &'a [u8], u32),
        Link(&'a str, &'a str),
        Hard(&'a str, &'a str),
        Fifo(&'a str),
        Dir(&'a str),
    }

    fn header(name: &str, kind: tar::EntryType, size: u64, mode: u32, link: &str) -> tar::Header {
        let mut h = tar::Header::new_old();
        {
            let old = h.as_old_mut();
            old.name[..name.len()].copy_from_slice(name.as_bytes());
            old.linkname[..link.len()].copy_from_slice(link.as_bytes());
        }
        h.set_entry_type(kind);
        h.set_size(size);
        h.set_mode(mode);
        h.set_mtime(0);
        h.set_cksum();
        h
    }

    fn targz(entries: &[T]) -> Vec<u8> {
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        for e in entries {
            let (h, data): (tar::Header, &[u8]) = match *e {
                T::File(n, d, m) => (header(n, tar::EntryType::Regular, d.len() as u64, m, ""), d),
                T::Link(n, l) => (header(n, tar::EntryType::Symlink, 0, 0o777, l), &[]),
                T::Hard(n, l) => (header(n, tar::EntryType::Link, 0, 0o644, l), &[]),
                T::Fifo(n) => (header(n, tar::EntryType::Fifo, 0, 0o644, ""), &[]),
                T::Dir(n) => (header(n, tar::EntryType::Directory, 0, 0o755, ""), &[]),
            };
            b.append(&h, data).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    const BIN: &[u8] = b"#!/bin/sh\necho llama\n";
    static LAYOUT: &[Entry] = &[
        Entry::File("llama-completion"),
        Entry::File("libggml.so.0.1"),
        Entry::Link("libggml.so.0", "libggml.so.0.1"),
        Entry::DevLink("libggml.so", "libggml.so.0"),
    ];

    fn good() -> Vec<T<'static>> {
        vec![
            T::Dir("llama-test/"),
            // setuid in the archive: the install must strip it.
            T::File("llama-test/llama-completion", BIN, 0o4755),
            T::File("llama-test/libggml.so.0.1", b"lib", 0o644),
            T::Link("llama-test/libggml.so.0", "libggml.so.0.1"),
            T::Link("llama-test/libggml.so", "libggml.so.0"),
        ]
    }

    /// A test-only pin for exactly these bytes, so only the check under test
    /// can refuse.
    fn pin_of(archive: &[u8]) -> &'static Pin {
        Box::leak(Box::new(Pin {
            os: "test",
            arch: "test",
            backend: "test",
            asset: "llama-test-bin.tar.gz",
            sha256: Box::leak(hex(&Sha256::digest(archive)).into_boxed_str()),
            bytes: archive.len() as u64,
            dir: "llama-test",
            layout: LAYOUT,
        }))
    }

    // ── a local HTTP server ────────────────────────────────────────────────

    enum Serve {
        Full,
        Truncated(usize),
        Endless,
    }

    /// Serve `body` to every request; returns the URL and a request count.
    fn serve(body: Vec<u8>, how: Serve, content_length: Option<usize>) -> (String, Arc<AtomicUsize>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/llama-test-bin.tar.gz", l.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        std::thread::spawn(move || {
            for stream in l.incoming() {
                let Ok(mut s) = stream else { return };
                h.fetch_add(1, Ordering::SeqCst);
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                while r.read_line(&mut line).map(|n| n > 0).unwrap_or(false) && line != "\r\n" {
                    line.clear();
                }
                let mut head = String::from("HTTP/1.1 200 OK\r\nConnection: close\r\n");
                if let Some(n) = content_length {
                    head += &format!("Content-Length: {n}\r\n");
                }
                head += "\r\n";
                let _ = s.write_all(head.as_bytes());
                match how {
                    Serve::Full => { let _ = s.write_all(&body); }
                    Serve::Truncated(n) => { let _ = s.write_all(&body[..n]); }
                    Serve::Endless => {
                        // 16 MB with no length: far past any test pin, and
                        // bounded, so a client with no cut-off fails instead
                        // of hanging.
                        let chunk = vec![0u8; 64 * 1024];
                        for _ in 0..256 {
                            if s.write_all(&chunk).is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });
        (url, hits)
    }

    // ── a scratch HOME with a previous install ──────────────────────────────

    struct Home {
        root: PathBuf,
        install: PathBuf,
    }

    impl Home {
        fn new(tag: &str) -> Home {
            let root = std::env::temp_dir().join(format!("asg_llama_install_{tag}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let install = root.join(".aismartguy/llama-cpp");
            std::fs::create_dir_all(&install).unwrap();
            std::fs::write(install.join("previous"), b"the previous verified install").unwrap();
            Home { root, install }
        }

        /// The previous install exactly as it was, and nothing left behind.
        fn untouched(&self) {
            let names: Vec<String> = std::fs::read_dir(&self.install).unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
            assert_eq!(names, ["previous"], "the previous install changed");
            assert_eq!(std::fs::read(self.install.join("previous")).unwrap(), b"the previous verified install");
            self.no_leftovers();
        }

        fn no_leftovers(&self) {
            let names: Vec<String> = std::fs::read_dir(self.install.parent().unwrap()).unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
            assert_eq!(names, ["llama-cpp"], "temporary files or directories left behind");
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn no_progress(_: u64) {}
    fn no_fault(_: Stage) -> Result<(), String> { Ok(()) }

    fn run(home: &Home, pin: &Pin, url: &str) -> Result<PathBuf, String> {
        install(pin, url, &home.install, &no_progress, &no_fault)
    }

    // ── the conditions ───────────────────────────────────────────────────────

    /// Positive control: the pinned bytes install, flat, with modes and links.
    #[test]
    fn a_pinned_archive_installs_flat_with_its_links_and_modes() {
        let home = Home::new("accept");
        let body = targz(&good());
        let pin = pin_of(&body);
        let (url, hits) = serve(body.clone(), Serve::Full, Some(body.len()));
        let bin = run(&home, pin, &url).unwrap();
        assert_eq!(bin, home.install.join("llama-completion"));
        assert_eq!(std::fs::read(&bin).unwrap(), BIN);
        assert_eq!(std::fs::metadata(&bin).unwrap().permissions().mode() & 0o7777, 0o755, "setuid stripped");
        assert_eq!(std::fs::metadata(home.install.join("libggml.so.0.1")).unwrap().permissions().mode() & 0o777, 0o644);
        assert_eq!(std::fs::read_link(home.install.join("libggml.so.0")).unwrap(), Path::new("libggml.so.0.1"));
        assert!(std::fs::symlink_metadata(home.install.join("libggml.so")).is_err(),
                "a development link is verified but never installed");
        assert!(!home.install.join("previous").exists(), "the new install replaces the previous one");
        home.no_leftovers();
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    /// One changed byte: refused before extraction, and no other URL tried.
    #[test]
    fn a_tampered_byte_is_refused_and_nothing_else_is_fetched() {
        let home = Home::new("tamper");
        let body = targz(&good());
        let pin = pin_of(&body);
        let mut bad = body.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 0x01;
        let (url, hits) = serve(bad, Serve::Full, Some(body.len()));
        let err = run(&home, pin, &url).unwrap_err();
        assert!(err.contains("hash mismatch"), "{err}");
        home.untouched();
        assert_eq!(hits.load(Ordering::SeqCst), 1, "no fallback download");
    }

    /// Unknown platforms and assets are refused before any network access.
    #[test]
    fn an_unpinned_platform_or_asset_is_refused_before_any_download() {
        for (os, arch, backend) in [("windows", "x86_64", "cpu"), ("linux", "aarch64", "cpu"), ("linux", "x86_64", "rocm")] {
            let err = pin_for(PINS, os, arch, backend).unwrap_err();
            assert!(err.contains("no verified llama.cpp build is pinned") && err.contains("nothing was downloaded"), "{err}");
        }
        let pin = pin_for(PINS, "linux", "x86_64", "vulkan").unwrap();
        assert!(confirm(pin, pin.asset).is_ok(), "control: the shown asset is accepted");
        let err = confirm(pin, "llama-b10238-bin-ubuntu-rocm-7.2-x64.tar.gz").unwrap_err();
        assert!(err.contains("not the one pinned"), "{err}");
    }

    /// A stream longer than the pin is cut off while it streams.
    #[test]
    fn an_oversized_download_is_cut_off_while_streaming() {
        let home = Home::new("endless");
        let body = targz(&good());
        let pin = pin_of(&body);
        let (url, _) = serve(body, Serve::Endless, None);
        let err = run(&home, pin, &url).unwrap_err();
        assert!(err.contains("cut off"), "{err}");
        home.untouched();
    }

    /// A server announcing the wrong size is refused before the body.
    #[test]
    fn a_wrong_announced_size_is_refused_before_downloading() {
        let home = Home::new("length");
        let body = targz(&good());
        let pin = pin_of(&body);
        let (url, _) = serve(body.clone(), Serve::Full, Some(body.len() + 1));
        let err = run(&home, pin, &url).unwrap_err();
        assert!(err.contains("refused before downloading"), "{err}");
        home.untouched();
    }

    /// A download that stops early leaves no partial file and no install.
    #[test]
    fn an_interrupted_download_leaves_nothing() {
        let home = Home::new("cut");
        let body = targz(&good());
        let pin = pin_of(&body);
        let (url, _) = serve(body.clone(), Serve::Truncated(body.len() / 2), Some(body.len()));
        let err = run(&home, pin, &url).unwrap_err();
        assert!(err.contains("interrupted") || err.contains("incomplete"), "{err}");
        home.untouched();
    }

    /// Each unsafe or unpinned entry is refused by its own check, with the
    /// archive's hash pinned so the hash check cannot be what refused.
    #[test]
    fn unsafe_or_unpinned_archive_entries_are_refused_each_by_its_check() {
        let cases: Vec<(&str, T, &str)> = vec![
            ("absolute", T::File("/llama-test/extra", b"x", 0o644), "unsafe path"),
            ("dotdot", T::File("llama-test/../extra", b"x", 0o644), "unsafe path"),
            ("outside", T::File("elsewhere/llama-completion", b"x", 0o755), "outside the pinned layout"),
            ("nested", T::File("llama-test/sub/extra", b"x", 0o644), "outside the pinned layout"),
            ("unpinned file", T::File("llama-test/extra", b"x", 0o644), "not in the pinned layout"),
            ("unpinned link", T::Link("llama-test/evil", "/etc/passwd"), "not in the pinned layout"),
            ("link for a file", T::Link("llama-test/llama-completion", "libggml.so.0.1"), "link not in the pinned layout"),
            ("link retargeted", T::Link("llama-test/libggml.so.0", "../../../../etc/passwd"), "points to"),
            ("hardlink", T::Hard("llama-test/libggml.so.0", "llama-test/libggml.so.0.1"), "hardlink refused"),
            ("fifo", T::Fifo("llama-test/libggml.so.0"), "entry type Fifo refused"),
            ("unpinned fifo", T::Fifo("llama-test/pipe"), "entry type Fifo refused"),
            ("file for a link", T::File("llama-test/libggml.so.0", b"x", 0o644), "pinned as a link"),
            ("dev link retargeted", T::Link("llama-test/libggml.so", "/etc/passwd"), "points to"),
        ];
        for (what, bad, says) in cases {
            let home = Home::new(&what.replace(' ', "_"));
            let mut entries = good();
            if matches!(what, "link retargeted" | "hardlink" | "fifo" | "file for a link" | "link for a file" | "dev link retargeted") {
                // replace the good entry of that name, so it is not "listed twice"
                let name = match &bad { T::File(n, ..) | T::Link(n, _) | T::Hard(n, _) | T::Fifo(n) | T::Dir(n) => *n };
                entries.retain(|e| !matches!(e, T::File(n, ..) | T::Link(n, _) if *n == name));
            }
            entries.push(bad);
            let body = targz(&entries);
            let pin = pin_of(&body);
            let (url, _) = serve(body.clone(), Serve::Full, Some(body.len()));
            let err = run(&home, pin, &url).expect_err(what);
            assert!(err.contains(says), "{what}: expected \"{says}\", got: {err}");
            home.untouched();
        }
    }

    /// Every pinned entry must be in the archive.
    #[test]
    fn an_archive_missing_a_pinned_entry_is_refused() {
        let home = Home::new("missing");
        let entries: Vec<T> = good().into_iter()
            .filter(|e| !matches!(e, T::Link(n, _) if *n == "llama-test/libggml.so.0")).collect();
        let body = targz(&entries);
        let pin = pin_of(&body);
        let (url, _) = serve(body.clone(), Serve::Full, Some(body.len()));
        let err = run(&home, pin, &url).unwrap_err();
        assert!(err.contains("missing from the archive: libggml.so.0"), "{err}");
        home.untouched();
    }

    /// ASG-Q5: an unsound pin is refused before anything is downloaded. A
    /// link must target a bare, pinned regular file.
    #[test]
    fn an_unsound_pin_is_refused_before_any_download() {
        let bad: [(&str, &'static [Entry], &str); 4] = [
            ("path in target", &[Entry::File("llama-completion"), Entry::File("lib.so.1"), Entry::Link("lib.so", "../lib.so.1")],
             "is not a bare file name"),
            ("link to a link", &[Entry::File("llama-completion"), Entry::File("lib.so.1"),
                                 Entry::Link("lib.so.0", "lib.so.1"), Entry::Link("lib.so", "lib.so.0")],
             "does not target a pinned file"),
            ("dangling link", &[Entry::File("llama-completion"), Entry::Link("lib.so", "lib.so.1")],
             "does not target a pinned file"),
            ("dev link off the pin", &[Entry::File("llama-completion"), Entry::DevLink("lib.so", "/usr/lib/x")],
             "is not a pinned bare name"),
        ];
        for (what, layout, says) in bad {
            let home = Home::new(&what.replace(' ', "_"));
            let body = targz(&good());
            let mut pin = Pin { ..*pin_of(&body) };
            pin.layout = layout;
            let (url, hits) = serve(body.clone(), Serve::Full, Some(body.len()));
            let err = run(&home, &pin, &url).expect_err(what);
            assert!(err.contains("unsound pin") && err.contains(says), "{what}: expected {says:?}, got: {err}");
            assert_eq!(hits.load(Ordering::SeqCst), 0, "{what}: refused before any download");
            home.untouched();
        }
        assert!(pin_is_sound(pin_of(&targz(&good()))).is_ok(), "control: the test layout is sound");
    }

    /// ASG-Q5: a pinned link whose target file is missing from the archive is
    /// refused, not installed dangling.
    #[test]
    fn a_link_whose_target_is_missing_from_the_archive_is_refused() {
        let home = Home::new("dangling");
        let entries: Vec<T> = good().into_iter()
            .filter(|e| !matches!(e, T::File(n, ..) if *n == "llama-test/libggml.so.0.1")).collect();
        let body = targz(&entries);
        let pin = pin_of(&body);
        let (url, _) = serve(body.clone(), Serve::Full, Some(body.len()));
        let err = run(&home, pin, &url).unwrap_err();
        assert!(err.contains("missing from the archive: libggml.so.0.1"), "{err}");
        home.untouched();
    }

    /// A failure mid-install leaves the previous install as it was.
    #[test]
    fn a_failure_mid_install_keeps_the_previous_install() {
        for stage in [Stage::Extracted, Stage::Swapping] {
            let home = Home::new(&format!("{stage:?}"));
            let body = targz(&good());
            let pin = pin_of(&body);
            let (url, _) = serve(body.clone(), Serve::Full, Some(body.len()));
            let fail = |s: Stage| if s == stage { Err(format!("injected at {s:?}")) } else { Ok(()) };
            let err = install(pin, &url, &home.install, &no_progress, &fail).unwrap_err();
            assert!(err.contains("injected"), "{err}");
            home.untouched();
        }
    }

    /// The real table: immutable tag URLs, exact sizes and hashes, complete
    /// layouts with llama-completion, links only to pinned files.
    #[test]
    fn the_real_pins_are_immutable_exact_and_complete() {
        assert!(!PINS.is_empty());
        for p in PINS {
            assert_eq!(p.sha256.len(), 64, "{}", p.asset);
            assert!(p.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{}", p.asset);
            assert!(p.bytes > 0);
            assert!(p.asset.starts_with(&format!("llama-{TAG}-")), "{}", p.asset);
            assert_eq!(p.dir, format!("llama-{TAG}"));
            let url = url_for(p);
            assert!(url.contains(&format!("/releases/download/{TAG}/")) && !url.contains("latest"), "{url}");
            assert!(p.layout.contains(&Entry::File(model_loader::LLAMA_BIN)), "{}", p.asset);
            pin_is_sound(p).unwrap();
        }
        assert!(pin_for(PINS, "linux", "x86_64", "vulkan").is_ok() && pin_for(PINS, "linux", "x86_64", "cpu").is_ok());
    }

    /// Never "latest", never the API that resolves it.
    #[test]
    fn nothing_asks_for_the_latest_release() {
        for f in ["src/main.rs", "src/llama_install.rs"] {
            let src = std::fs::read_to_string(format!("{}/{f}", env!("CARGO_MANIFEST_DIR"))).unwrap();
            let code: String = src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
            let code = code.split("#[cfg(all(test, unix))]").next().unwrap();
            assert!(!code.contains("releases/latest") && !code.contains("api.github.com"), "{f}");
        }
    }
}
