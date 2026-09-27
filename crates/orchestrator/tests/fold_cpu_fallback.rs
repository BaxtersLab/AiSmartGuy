//! The fusion fold, run for real through `run_fusion`, when the GPU refuses
//! the model. llama.cpp is replaced by a TEST DOUBLE for exactly one thing:
//! any pass that asks for GPU layers gets llama.cpp's real failure log (in-app
//! Vulkan build, GTX 1660 SUPER, 2026-09-27) and exits 1. Everything else --
//! model_loader's process handling, the fold, the paths -- is the product's.
//!
//! One test in its own binary: it points HOME and PATH at a temp directory,
//! which is process-wide.
#![cfg(unix)]

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use orchestrator::fusion::run_fusion;
use orchestrator::FusionInput;

const REAL_VULKAN_OOM_LOG: &str = "\
0.00.121.343 I llama_completion: llama backend init
0.00.121.351 I llama_completion: load the model and apply lora adapter, if any
ggml_vulkan: Memory allocation of size 191439360 failed.
ggml_vulkan: vk::Device::allocateMemory: ErrorOutOfDeviceMemory
0.01.086.726 E llama_model_load: error loading model: vk::Device::allocateMemory: ErrorOutOfDeviceMemory
0.01.086.734 E llama_model_load_from_file_impl: failed to load model
0.01.086.742 E llama_completion: error: unable to create context
";

/// FAKE_LLAMA_MODE=gpu-refuses: GPU passes fail as above, CPU passes answer.
/// FAKE_LLAMA_MODE=always-fails: every pass fails for a reason that is not the GPU.
const FAKE_LLAMA: &str = r#"#!/bin/sh
here=$(dirname "$0"); ngl=; prev=
for a in "$@"; do [ "$prev" = --n-gpu-layers ] && ngl=$a; prev=$a; done
echo "$FAKE_LLAMA_MODE $ngl" >> "$here/calls.log"
if [ "$FAKE_LLAMA_MODE" = always-fails ]; then
  echo "llama_completion: error: unable to read prompt" >&2; exit 1
fi
if [ "$ngl" != 0 ]; then cat "$here/oom.log" >&2; exit 1; fi
echo "FOLDED ON THE CPU"
"#;

fn write_exe(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn calls(llama_dir: &Path) -> Vec<String> {
    std::fs::read_to_string(llama_dir.join("calls.log"))
        .unwrap_or_default().lines().map(String::from).collect()
}

#[test]
fn a_fold_the_gpu_refuses_finishes_on_the_cpu() {
    let root = std::env::temp_dir().join(format!("asg_fold_fallback_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let llama_dir = root.join("home/.aismartguy/llama-cpp");
    let bin = root.join("bin");
    std::fs::create_dir_all(&llama_dir).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    write_exe(&llama_dir.join("llama-completion"), FAKE_LLAMA);
    std::fs::write(llama_dir.join("oom.log"), REAL_VULKAN_OOM_LOG).unwrap();
    // A 6 GB card, as on the estate's dev box, so the fold asks for the GPU
    // on any machine.
    write_exe(&bin.join("nvidia-smi"), "#!/bin/sh\necho 6144\n");
    std::env::set_var("HOME", root.join("home"));
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.display()));

    // A 64 MB (sparse) model file: big enough that the GPU gets layers.
    let model = root.join("model.gguf");
    std::fs::File::create(&model).unwrap().set_len(64 << 20).unwrap();
    let config = manifest::ModelConfig {
        name: "fusion".into(),
        path: model.display().to_string(),
        gpu_usage: Some("GPU".into()),
        context_length: Some(4096),
        ..Default::default()
    };
    let input = FusionInput {
        model_outputs: HashMap::from([("model1".to_string(), vec!["Chapter one's analysis.".to_string()])]),
    };

    // The GPU refuses: the fold is rerun on the CPU and completes.
    std::env::set_var("FAKE_LLAMA_MODE", "gpu-refuses");
    let out = run_fusion(&config, &input, &root.join("run1"), 75);
    let seen = calls(&llama_dir);
    let out = out.unwrap_or_else(|e| panic!("the fold failed instead of moving to the CPU: {e}; llama calls {seen:?}"));
    assert_eq!(std::fs::read_to_string(out).unwrap().trim(), "FOLDED ON THE CPU");
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_ne!(seen[0], "gpu-refuses 0", "the first pass must ask for the GPU: {seen:?}");
    assert_eq!(seen[1], "gpu-refuses 0", "the retry must ask for no GPU layers: {seen:?}");

    // Control: a failure that is not the GPU is not retried on the CPU.
    std::fs::remove_file(llama_dir.join("calls.log")).unwrap();
    std::env::set_var("FAKE_LLAMA_MODE", "always-fails");
    assert!(run_fusion(&config, &input, &root.join("run2"), 75).is_err());
    assert_eq!(calls(&llama_dir).len(), 1, "{:?}", calls(&llama_dir));

    let _ = std::fs::remove_dir_all(&root);
}
