//! The frontend has no JS test harness, and there is no JS runtime on the
//! build box, so this pins the structure; the behaviour was proven in the VM
//! harness (2026-09-27): before, one Browse click after two visits to the
//! Model Loader opened two folder pickers; after, one.

fn function_body<'a>(src: &'a str, name: &str) -> &'a str {
    let start = src.find(&format!("function {name}(")).unwrap_or_else(|| panic!("{name} not found"));
    let open = start + src[start..].find('{').unwrap();
    let mut depth = 0usize;
    for (i, c) in src[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => { depth -= 1; if depth == 0 { return &src[open..open + i + 1]; } }
            _ => {}
        }
    }
    panic!("{name}: unbalanced braces")
}

/// `wireLibraryButtons` runs on every visit to the Model Loader, so it must
/// return before adding a listener when it has already wired them.
#[test]
fn the_library_buttons_are_wired_once() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../frontend/index.html")).unwrap();
    assert!(function_body(&src, "showModelLoader").contains("wireLibraryButtons();"),
            "premise: it is called on every visit");
    let body = function_body(&src, "wireLibraryButtons");
    let guard = body.find("if (browseBtn.dataset.wired) return;").expect("no once-guard");
    let first_listener = body.find("addEventListener").expect("premise: it adds listeners");
    assert!(guard < first_listener, "the guard must come before the first listener");
}

/// llama.cpp is downloaded only from the confirmation panel, which shows the
/// pinned plan first (manager's ruling on ASG-Q2): the one call to
/// `cmd_install_llama` is inside `showLlamaConfirm` and passes the asset it
/// showed; the "not installed" path asks for the plan, never installs; and
/// start-up only detects.
#[test]
fn llama_is_downloaded_only_after_the_user_confirms_the_plan() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../frontend/index.html")).unwrap();
    assert_eq!(src.matches("invoke('cmd_install_llama'").count(), 1, "exactly one install call");
    let confirm = function_body(&src, "showLlamaConfirm");
    assert!(confirm.contains("invoke('cmd_install_llama', { asset: plan.asset })"),
            "the install call passes the asset the panel showed");
    for label in ["plan.tag", "plan.asset", "plan.backend", "plan.bytes"] {
        assert!(confirm.contains(label), "the panel shows {label}");
    }
    let handler_at = src.find("indexOf('LLAMA_NOT_INSTALLED')").expect("the not-installed handler");
    let handler = &src[handler_at..handler_at + src[handler_at..].find("return;\n          }").unwrap()];
    assert!(handler.contains("invoke('cmd_llama_install_plan')"), "{handler}");
    assert!(!handler.contains("cmd_install_llama"), "no download without confirmation: {handler}");
    let startup = function_body(&src, "checkLlama");
    assert!(startup.contains("invoke('cmd_detect_llama')"), "start-up detects");
    assert!(!startup.contains("invoke('cmd_install_llama'") && !startup.contains("invoke('cmd_llama_install_plan'"),
            "start-up never downloads");
}

/// Lock for 10 Runs forces Full mode and cannot be released before 10 runs:
/// locked with an empty lane, no run could start until a restart. The
/// handler refuses unless all four lanes are chosen, before it locks.
#[test]
fn the_lock_is_refused_while_a_lane_is_empty() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../frontend/index.html")).unwrap();
    let at = src.find("mlLockBtn.addEventListener('click'").expect("the lock handler");
    let handler = &src[at..at + src[at..].find("\n      });").unwrap()];
    let check = handler.find("lanes.some(function (sel) { return !sel.value; })").expect("an empty-lane check");
    let refuse = check + handler[check..].find("return;").expect("a refusal");
    let lock = handler.find("modeLocked = true;").expect("premise: it locks");
    assert!(refuse < lock, "the refusal must come before the lock");
    assert!(handler.contains("choose a model in all four lanes first"), "and it says why");
}

/// The start-up splash's listener must stop after start-up: a finished run
/// also emits pipeline-progress 100%, and it showed the Ready card over the
/// completion screen after every run.
#[test]
fn the_startup_listener_ignores_runs() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../frontend/index.html")).unwrap();
    let at = src.find("listen('pipeline-progress'").expect("the start-up listener");
    let body = &src[at..at + src[at..].find("showReady, 800").unwrap()];
    let guard = body.find("if (startupDone) return;").expect("a once-guard");
    let set = body.find("startupDone = true;").expect("set when start-up completes");
    assert!(guard < set);
    // main.rs does emit pipeline-progress 100 at the end of a run (the premise).
    let main = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../src-tauri/src/main.rs")).unwrap();
    assert!(main.contains("\"pipeline-progress\", PipelineProgress {\n                    percent: 100.0,"));
}
