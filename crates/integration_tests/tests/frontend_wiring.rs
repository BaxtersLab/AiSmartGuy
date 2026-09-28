//! The frontend has no JS test harness, and there is no JS runtime on the
//! build box, so this pins the structure; the behaviour was proven in the VM
//! harness (2026-09-27): before, one Browse click after two visits to the
//! Model Loader opened two folder pickers; after, one.

fn function_body<'a>(src: &'a str, name: &str) -> &'a str {
    let start = src.find(&format!("function {name}()")).unwrap_or_else(|| panic!("{name} not found"));
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
