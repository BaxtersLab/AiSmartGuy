//! A whole run -- the GUI's own manifest builder and `ui::commands::start_run`
//! -- when the GPU refuses the model. llama.cpp is replaced by a TEST DOUBLE
//! for exactly one thing: any pass that asks for GPU layers gets llama.cpp's
//! real failure log (in-app Vulkan build, GTX 1660 SUPER, 2026-09-27) and
//! exits 1; a CPU pass answers. Everything else is the product's.
//!
//! One test in its own binary: it points HOME and PATH at a temp directory,
//! which is process-wide.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const REAL_VULKAN_OOM_LOG: &str = "\
0.00.121.343 I llama_completion: llama backend init
0.00.121.351 I llama_completion: load the model and apply lora adapter, if any
ggml_vulkan: Memory allocation of size 191439360 failed.
ggml_vulkan: vk::Device::allocateMemory: ErrorOutOfDeviceMemory
0.01.086.726 E llama_model_load: error loading model: vk::Device::allocateMemory: ErrorOutOfDeviceMemory
0.01.086.734 E llama_model_load_from_file_impl: failed to load model
0.01.086.742 E llama_completion: error: unable to create context
";

/// Logs "<ngl> <lane>/<prompt file>" per call. FAKE_LLAMA_MODE=gpu-refuses:
/// GPU passes fail as above, CPU passes answer. =always-fails: every pass
/// fails for a reason that is not the GPU.
const FAKE_LLAMA: &str = r#"#!/bin/sh
here=$(dirname "$0"); ngl=; prompt=; prev=
for a in "$@"; do
  [ "$prev" = --n-gpu-layers ] && ngl=$a
  [ "$prev" = -f ] && prompt=$a
  prev=$a
done
which="$(basename "$(dirname "$prompt")")/$(basename "$prompt")"
echo "$ngl $which" >> "$here/calls.log"
if [ "$FAKE_LLAMA_MODE" = always-fails ]; then
  echo "llama_completion: error: unable to read prompt" >&2; exit 1
fi
if [ "$ngl" != 0 ]; then cat "$here/oom.log" >&2; exit 1; fi
# An answer that names nothing internal, so what the fold is fed is only
# what the pipeline itself added.
echo "CPU ANSWER: the argument is sound."
"#;

fn write_exe(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// (gpu layers, "lane/prompt") per llama call, in order.
fn calls(llama_dir: &Path) -> Vec<(u32, String)> {
    std::fs::read_to_string(llama_dir.join("calls.log")).unwrap_or_default().lines()
        .map(|l| {
            let (n, w) = l.split_once(' ').unwrap();
            (n.parse().unwrap(), w.to_string())
        })
        .collect()
}

fn run(root: &Path, tag: &str) -> (Result<(), String>, std::path::PathBuf) {
    let pdf = root.join("book.pdf");
    let models = root.join("models").display().to_string();
    let run_id = ui::lanes::new_run_id();
    let manifest = ui::lanes::build_run_manifest(
        "2", [&models, "", "", &models], None, None, &pdf,
        run_id.clone(), ui::lanes::run_timestamp(), "test",
    ).unwrap();
    let run_dir = root.join(tag).join(&run_id);
    std::fs::create_dir_all(&run_dir).unwrap();
    let manifest_path = run_dir.join("manifest.json");
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();

    let state = ui::state::new_shared_state();
    {
        let mut s = state.lock().unwrap();
        s.pdf_path = Some(pdf.to_string_lossy().into_owned());
        s.pdf_loaded = true;
        s.manifest = Some(manifest);
        s.config_detected = true;
    }
    model_loader::cancel::clear();
    let r = ui::commands::start_run(state, manifest_path, run_dir.clone()).map_err(|e| format!("{e:?}"));
    (r, run_dir)
}

#[test]
fn a_run_the_gpu_refuses_finishes_on_the_cpu_with_its_synthesis() {
    let root = std::env::temp_dir().join(format!("asg_run_gpu_fallback_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    let llama_dir = home.join(".aismartguy/llama-cpp");
    let rag = home.join(".aismartguy/rag_defaults");
    let bin = root.join("bin");
    for d in [&llama_dir, &rag, &bin, &root.join("models")] {
        std::fs::create_dir_all(d).unwrap();
    }
    write_exe(&llama_dir.join("llama-completion"), FAKE_LLAMA);
    std::fs::write(llama_dir.join("oom.log"), REAL_VULKAN_OOM_LOG).unwrap();
    // A 6 GB card, as on the estate's dev box, so every pass asks for the
    // GPU on any machine.
    write_exe(&bin.join("nvidia-smi"), "#!/bin/sh\necho 6144\n");
    // The RAG packets the GUI seeds at start-up.
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/rag_defaults");
    for e in std::fs::read_dir(&assets).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), rag.join(e.file_name())).unwrap();
    }
    std::env::set_var("HOME", &home);
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.display()));
    // Prose mode, as this test was written for: structured findings are now
    // the default wherever llama supports grammars (manager ruling F1).
    std::env::set_var("ASG_STRUCTURED_FINDINGS", "0");

    // A 64 MB (sparse) model, big enough that the GPU gets layers.
    std::fs::File::create(root.join("models/model.gguf")).unwrap().set_len(64 << 20).unwrap();
    // A one-chapter book, written by the app's own PDF writer.
    let text = "Chapter One\n\nThe argument of this chapter is that every reader should \
                think for themselves, weigh the evidence, and distrust any claim that \
                arrives without its reasons.\n".repeat(20);
    pdf_io::write_final_pdf(&root.join("unused.pdf"), &text, "{}", &root.join("book.pdf")).unwrap();

    // ── The GPU refuses every model: the run finishes on the CPU. ──
    std::env::set_var("FAKE_LLAMA_MODE", "gpu-refuses");
    let (result, run_dir) = run(&root, "refused");
    let seen = calls(&llama_dir);
    assert!(result.is_ok(), "the run failed: {result:?}; llama calls {seen:?}");
    let gpu: Vec<usize> = (0..seen.len()).filter(|&i| seen[i].0 > 0).collect();
    // One refusal per stage -- model1's chapters, fusion's chapters, the
    // fold -- and never twice: once moved, a stage stays on the CPU.
    assert_eq!(gpu.len(), 3, "GPU attempts: {seen:?}");
    for &i in &gpu {
        assert_eq!(seen.get(i + 1), Some(&(0, seen[i].1.clone())),
                   "a refused pass must be rerun at once on the CPU: {seen:?}");
    }
    assert!(seen[gpu[0]].1.starts_with("model1/"), "{seen:?}");
    assert!(seen[gpu[1]].1.starts_with("fusion/chapter_"), "{seen:?}");
    assert!(seen[gpu[2]].1.starts_with("fusion/pass"), "{seen:?}");
    let synthesis = std::fs::read_to_string(run_dir.join("outputs/fusion/fusion_output.txt")).unwrap();
    assert!(synthesis.contains("CPU ANSWER"), "no synthesis: {synthesis:?}");
    // Nothing internal reaches a model as content: no lane or stage names in
    // any prompt of the run (Hermes took "fusion · chapter 1" for the title).
    let mut prompts = 0;
    for lane in ["model1", "fusion"] {
        for e in std::fs::read_dir(run_dir.join("prompts").join(lane)).unwrap() {
            let text = std::fs::read_to_string(e.unwrap().path()).unwrap();
            prompts += 1;
            for label in ["model1", "model2", "model3", "fusion", "findings ·", " · chapter"] {
                assert!(!text.contains(label), "a prompt contains the internal label {label:?}");
            }
        }
    }
    assert!(prompts >= 3, "premise: chapter and fold prompts were written ({prompts})");
    let report = std::fs::read_dir(&run_dir).unwrap()
        .filter_map(|e| e.ok()).any(|e| e.file_name().to_string_lossy().ends_with("_AiSmartGuy_Report.pdf"));
    assert!(report, "no report PDF in {}", run_dir.display());

    // ── Control: a failure that is not the GPU is never moved to the CPU. ──
    std::fs::remove_file(llama_dir.join("calls.log")).unwrap();
    std::env::set_var("FAKE_LLAMA_MODE", "always-fails");
    let _ = run(&root, "unrelated");
    let seen = calls(&llama_dir);
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|c| c.0 > 0), "moved to the CPU on an unrelated failure: {seen:?}");

    let _ = std::fs::remove_dir_all(&root);
}
