use std::process::Command;
use crate::llama_detect::resolve_llama_path;
use crate::types::{InferenceRequest, ModelInstance};

/// Builds the `llama` subprocess `Command` for the given instance and request.
///
/// The resulting `Command` is ready to be spawned — it is not yet executed.
pub fn build_command(instance: &ModelInstance, request: &InferenceRequest) -> Command {
    let llama_path = resolve_llama_path();
    let mut cmd = Command::new(&llama_path);

    cmd.arg("--model")
        .arg(&instance.model_path)
        .arg("--ctx-size")
        .arg(instance.context_length.to_string())
        .arg("--n-gpu-layers")
        .arg(instance.n_gpu_layers.to_string())
        .arg("--threads")
        .arg(instance.threads.to_string())
        .arg("--temp")
        .arg("0.2")
        .arg("--top-k")
        .arg("40")
        .arg("--top-p")
        .arg("0.9")
        .arg("--repeat-penalty")
        .arg("1.1")
        .arg("-f")
        .arg(&request.prompt_path)
        .arg("--no-display-prompt")
        // One chat turn through the model's own chat template, then exit.
        // Raw completion (-no-cnv) hands an instruct model a book that ends
        // mid-passage, and it continues the book: on a real run (2026-09-27,
        // Qwen2.5-1.5B-Instruct) every pass came back as more of the source
        // essay, even with the instruction placed before the passage. The
        // same prompt as one chat turn came back as analysis, twice in two.
        .arg("-cnv")
        .arg("-st")
        .arg("-n")
        .arg("2048");

    // Phase 2: constrain output to the structured-findings grammar when set.
    if let Some(grammar) = &request.grammar_file {
        cmd.arg("--grammar-file").arg(grammar);
    }

    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ModelConfig, ModelInstance};
    use std::path::PathBuf;

    fn make_instance() -> ModelInstance {
        let config = ModelConfig {
            model_path: PathBuf::from("/models/test.gguf"),
            context_length: 2048,
            gpu_setting: "GPU".to_string(),
        };
        ModelInstance::new(config, 16)
    }

    #[test]
    fn test_command_contains_model_path() {
        let inst = make_instance();
        let req = InferenceRequest {
            chunk_id: 0,
            prompt_path: PathBuf::from("/tmp/prompt.txt"),
            output_path: PathBuf::from("/tmp/output.txt"),
            log_path: PathBuf::from("/tmp/run.log"),
            grammar_file: None,
        };
        let cmd = build_command(&inst, &req);
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.contains(&"--model".to_string()));
        assert!(args.contains(&"/models/test.gguf".to_string()));
        assert!(args.contains(&"--n-gpu-layers".to_string()));
        assert!(args.contains(&"16".to_string()));
        assert!(args.contains(&"--threads".to_string()));
        assert!(args.contains(&"4".to_string())); // default threads from ModelInstance::new
        // No grammar arg when grammar_file is None.
        assert!(!args.contains(&"--grammar-file".to_string()));
    }

    /// The prompt goes in as one chat turn, never as raw text to continue.
    #[test]
    fn the_prompt_is_one_chat_turn_not_raw_completion() {
        let inst = make_instance();
        let req = InferenceRequest {
            chunk_id: 0,
            prompt_path: PathBuf::from("/tmp/prompt.txt"),
            output_path: PathBuf::from("/tmp/output.txt"),
            log_path: PathBuf::from("/tmp/run.log"),
            grammar_file: None,
        };
        let args: Vec<_> = build_command(&inst, &req).get_args()
            .map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.iter().any(|a| a == "-cnv"), "{args:?}");
        assert!(args.iter().any(|a| a == "-st"), "{args:?}");
        assert!(!args.iter().any(|a| a == "-no-cnv"), "{args:?}");
    }

    #[test]
    fn test_command_emits_grammar_file_when_set() {
        let inst = make_instance();
        let req = InferenceRequest {
            chunk_id: 0,
            prompt_path: PathBuf::from("/tmp/prompt.txt"),
            output_path: PathBuf::from("/tmp/output.txt"),
            log_path: PathBuf::from("/tmp/run.log"),
            grammar_file: Some(PathBuf::from("/tmp/findings.gbnf")),
        };
        let cmd = build_command(&inst, &req);
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        let i = args.iter().position(|a| a == "--grammar-file").expect("grammar flag present");
        assert_eq!(args[i + 1], "/tmp/findings.gbnf");
    }
}
