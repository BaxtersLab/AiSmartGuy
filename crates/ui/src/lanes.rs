//! Lane selections -> model configs -> the run manifest.
//!
//! Shared by the GUI (`src-tauri` `cmd_begin_run`) and the headless harness
//! (`examples/headless_run.rs`), so a run started without a window builds its
//! manifest with exactly the code a GUI run uses.

use std::path::{Path, PathBuf};

use manifest::{
    Manifest, ModelConfig, ModelSet, OptimizationState, RagPacketMap, ResourceThrottle, RunMode,
    SourcePdf,
};
use rag_engine::hitlist;

/// True for a llama.cpp vision projector (`mmproj-*.gguf`): a companion file
/// for a multimodal model, not a language model that can be run.
fn is_projector(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("mmproj")
}

/// The model file in a lane's folder: the first `.gguf` by name that is not a
/// vision projector. Picking the first `.gguf` of any kind loaded
/// `mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf` from the qwen2.5-omni-3b folder
/// -- it sorts before the model -- and the run died loading a projector.
pub fn first_gguf_in(folder: &str) -> Result<PathBuf, String> {
    let dir = PathBuf::from(folder);
    if !dir.is_dir() {
        return Err(format!("not a directory: {}", folder));
    }
    let mut ggufs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("gguf")).unwrap_or(false))
        .collect();
    ggufs.sort();
    let (projectors, models): (Vec<PathBuf>, Vec<PathBuf>) = ggufs.into_iter().partition(|p| {
        p.file_name().map(|n| is_projector(&n.to_string_lossy())).unwrap_or(false)
    });
    models.into_iter().next().ok_or_else(|| {
        if projectors.is_empty() {
            format!("no .gguf file found in {}", folder)
        } else {
            format!("no model .gguf in {} (only a vision projector, which cannot run on its own)", folder)
        }
    })
}

pub fn model_config_from_lane(folder: &str, ctx: u32) -> Result<ModelConfig, String> {
    let gguf_path = first_gguf_in(folder)?;
    let name = gguf_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    Ok(ModelConfig {
        name,
        path: gguf_path.to_string_lossy().into_owned(),
        quantization: String::new(),
        context_length: Some(ctx),
        gpu_usage: Some("CPU".to_string()),
        n_gpu_layers: None,
        active: true,
        revision: None,
        sha256: None,
    })
}

/// The manifest for one run, from the GUI's lane selections.
///
/// `mode` is the GUI's value: "1" = one model, "2" = one model + fusion,
/// "3" = two models + fusion, anything else ("full") = three + fusion.
/// `lanes` are the folders of lanes 2-5 (model 1, model 2, model 3, fusion).
#[allow(clippy::too_many_arguments)]
pub fn build_run_manifest(
    mode: &str,
    lanes: [&str; 4],
    ctx_size: Option<u32>,
    throttle_pct: Option<u32>,
    pdf_path: &Path,
    run_id: String,
    timestamp: String,
    engine_version: &str,
) -> Result<Manifest, String> {
    let [lane2, lane3, lane4, lane5] = lanes;
    let run_mode = match mode {
        "1" => RunMode::Single,
        "2" => RunMode::Dual,
        _ => RunMode::Full,
    };

    let ctx = ctx_size.unwrap_or(16384);
    let throttle = throttle_pct.unwrap_or(75).clamp(25, 100);

    let mut model1 = Some(model_config_from_lane(lane2, ctx)?);
    if let Some(ref mut m) = model1 { m.gpu_usage = Some("GPU".to_string()); }

    let mut model2 = if mode == "3" || mode == "full" {
        Some(model_config_from_lane(lane3, ctx)?)
    } else {
        None
    };
    if let Some(ref mut m) = model2 { m.gpu_usage = Some("GPU".to_string()); }

    let mut model3 = if mode == "full" {
        Some(model_config_from_lane(lane4, ctx)?)
    } else {
        None
    };
    if let Some(ref mut m) = model3 { m.gpu_usage = Some("GPU".to_string()); }

    let mut fusion = if mode != "1" {
        Some(model_config_from_lane(lane5, ctx)?)
    } else {
        None
    };
    if let Some(ref mut m) = fusion { m.gpu_usage = Some("GPU".to_string()); }

    let source_pdf = SourcePdf {
        filename: pdf_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        hash_sha256: None,
        page_count: None,
    };

    Ok(Manifest {
        manifest_version: "1.0".into(),
        engine_version: engine_version.into(),
        run_id,
        timestamp,
        source_pdf,
        mode: run_mode,
        models: ModelSet { model1, model2, model3, fusion },
        rag_packets_used: RagPacketMap::new(),
        categories_active: hitlist::active_slugs(),
        optimization_state: OptimizationState::default(),
        resource_throttle: ResourceThrottle { throttle_pct: throttle },
        partial_run: None,
        notes: None,
    })
}

/// The GUI's run id: `Run_YYYY-MM-DD_HH-MM-SS` (UTC). The manifest validator
/// requires the `Run_` prefix.
pub fn new_run_id() -> String {
    let secs = now_secs();
    let (y, mo, day) = epoch_days_to_ymd((secs / 86400) as i64);
    format!("Run_{:04}-{:02}-{:02}_{:02}-{:02}-{:02}", y, mo, day, (secs / 3600) % 24, (secs / 60) % 60, secs % 60)
}

/// The run's ISO-8601 UTC timestamp.
pub fn run_timestamp() -> String {
    let secs = now_secs();
    let (y, mo, day) = epoch_days_to_ymd((secs / 86400) as i64);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, day, (secs / 3600) % 24, (secs / 60) % 60, secs % 60)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
fn epoch_days_to_ymd(mut days: i64) -> (i64, i64, i64) {
    days += 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(tag: &str, files: &[&str]) -> PathBuf {
        let d = std::env::temp_dir().join(format!("asg_lane_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for f in files {
            std::fs::write(d.join(f), b"GGUF").unwrap();
        }
        d
    }

    /// The real qwen2.5-omni-3b folder holds the model AND a vision projector
    /// that sorts before it.
    #[test]
    fn a_vision_projector_is_never_picked_as_the_model() {
        let d = lane("omni", &["mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf", "qwen2.5-omni-3b-q4_k_m.gguf"]);
        let got = first_gguf_in(d.to_str().unwrap());
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(got.unwrap().file_name().unwrap(), "qwen2.5-omni-3b-q4_k_m.gguf");
    }

    #[test]
    fn a_folder_with_only_a_projector_says_so() {
        let d = lane("onlyproj", &["mmproj-model-f16.gguf", "README.md"]);
        let got = first_gguf_in(d.to_str().unwrap());
        let _ = std::fs::remove_dir_all(&d);
        assert!(got.unwrap_err().contains("only a vision projector"));
    }

    #[test]
    fn the_first_model_by_name_is_picked_and_other_files_ignored() {
        let d = lane("plain", &["notes.txt", "b-model.gguf", "a-model.GGUF"]);
        let got = first_gguf_in(d.to_str().unwrap());
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(got.unwrap().file_name().unwrap(), "a-model.GGUF");
    }

    #[test]
    fn the_manifest_follows_the_guis_mode_rules() {
        let a = lane("m_a", &["alpha.gguf"]);
        let f = lane("m_f", &["fusion.gguf"]);
        let (a_s, f_s) = (a.to_str().unwrap(), f.to_str().unwrap());
        let single = build_run_manifest("1", [a_s, "", "", ""], None, None, Path::new("/x/book.pdf"),
                                        "Run_1".into(), "t".into(), "0.1.0").unwrap();
        let dual = build_run_manifest("2", [a_s, "", "", f_s], Some(8192), Some(10), Path::new("/x/book.pdf"),
                                      "Run_2".into(), "t".into(), "0.1.0").unwrap();
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&f);
        assert!(single.models.model1.is_some() && single.models.fusion.is_none());
        assert_eq!(single.models.model1.as_ref().unwrap().context_length, Some(16384), "GUI default ctx");
        assert_eq!(single.resource_throttle.throttle_pct, 75, "GUI default throttle");
        assert_eq!(dual.models.fusion.as_ref().unwrap().name, "fusion");
        assert_eq!(dual.models.model1.as_ref().unwrap().context_length, Some(8192));
        assert_eq!(dual.resource_throttle.throttle_pct, 25, "throttle is clamped to 25-100");
        assert_eq!(dual.source_pdf.filename, "book.pdf");
        assert!(!dual.categories_active.is_empty());
    }

    #[test]
    fn run_ids_are_the_guis_and_pass_the_manifest_validator() {
        let id = new_run_id();
        assert!(id.starts_with("Run_") && id.len() == "Run_2026-09-27_13-05-09".len(), "{id}");
        assert_eq!(epoch_days_to_ymd(0), (1970, 1, 1));
        assert_eq!(epoch_days_to_ymd(20_723), (2026, 9, 27));
        assert_eq!(epoch_days_to_ymd(19_417), (2023, 3, 1), "the day after a non-leap February");
    }
}
