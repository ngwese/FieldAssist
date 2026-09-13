// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const CHAINS: &[(&str, &str)] = &[
    ("monitor_mono", "MonitorMono"),
    ("monitor_stereo", "MonitorStereo"),
    ("monitor_ms", "MonitorMs"),
    ("monitor_foa", "MonitorFoa"),
    ("monitor_foa_fuma", "MonitorFoaFuma"),
];

const DSP_LIBS: &[&str] = &["headphone_crossfeed.lib", "bformat.lib"];

fn main() {
    for (stem, _) in CHAINS {
        println!("cargo:rerun-if-changed=dsp/{stem}.dsp");
    }
    for lib in DSP_LIBS {
        println!("cargo:rerun-if-changed=dsp/{lib}");
    }
    println!("cargo:rerun-if-env-changed=FAUST");
    println!("cargo:rerun-if-env-changed=FAUST_REGENERATE");

    compile_faust_chains();
}

fn compile_faust_chains() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let dsp_dir = manifest.join("dsp");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let committed = manifest.join("src").join("generated");

    if regenerate_requested() {
        let faust = faust_command().unwrap_or_else(|| {
            panic!(
                "FAUST_REGENERATE is set but the Faust compiler was not found; \
                 install Faust or set FAUST to the binary path"
            );
        });
        let work_dir = prepare_faust_workdir(&dsp_dir, &out_dir);
        for (stem, class_name) in CHAINS {
            compile_one(&faust, &work_dir, &out_dir, stem, class_name);
        }
        publish_generated(&out_dir, &committed);
    } else {
        copy_committed(&committed, &out_dir);
    }

    write_wrapper(&out_dir);
}

fn regenerate_requested() -> bool {
    matches!(
        env::var("FAUST_REGENERATE").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

fn copy_committed(committed: &Path, out_dir: &Path) {
    for (stem, _) in CHAINS {
        let src_rs = committed.join(format!("{stem}.inc.rs"));
        let src_json = committed.join(format!("{stem}.json"));
        if !src_rs.is_file() || !src_json.is_file() {
            panic!(
                "missing {}; install Faust and run FAUST_REGENERATE=1 cargo build \
                 -p field-audio-monitor, or restore generated files",
                src_rs.display()
            );
        }
        write_if_changed(
            &out_dir.join(format!("{stem}.inc.rs")),
            &fs::read(&src_rs).unwrap(),
        );
        let json = fs::read_to_string(&src_json).expect("read committed Faust json");
        write_if_changed(
            &out_dir.join(format!("{stem}.json")),
            sanitize_faust_json(&json).as_bytes(),
        );
    }
}

fn faust_command() -> Option<PathBuf> {
    if let Ok(path) = env::var("FAUST") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    Command::new("faust")
        .arg("-v")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|_| PathBuf::from("faust"))
}

/// Copy DSP inputs under OUT_DIR so Faust's `-json` sidecar never touches
/// the watched source tree.
fn prepare_faust_workdir(dsp_dir: &Path, out_dir: &Path) -> PathBuf {
    let work_dir = out_dir.join("faust_dsp");
    let _ = fs::remove_dir_all(&work_dir);
    fs::create_dir_all(&work_dir).expect("create Faust workdir");

    for (stem, _) in CHAINS {
        let name = format!("{stem}.dsp");
        fs::copy(dsp_dir.join(&name), work_dir.join(&name)).unwrap_or_else(|err| {
            panic!("copy {}: {err}", dsp_dir.join(&name).display());
        });
    }
    for lib in DSP_LIBS {
        fs::copy(dsp_dir.join(lib), work_dir.join(lib)).unwrap_or_else(|err| {
            panic!("copy {}: {err}", dsp_dir.join(lib).display());
        });
    }
    work_dir
}

fn compile_one(faust: &Path, work_dir: &Path, out_dir: &Path, stem: &str, class_name: &str) {
    let dsp = work_dir.join(format!("{stem}.dsp"));
    let rs_out = out_dir.join(format!("{stem}.raw.rs"));
    let status = Command::new(faust)
        .arg("-I")
        .arg(work_dir)
        .args(["-lang", "rust", "-json", "-cn", class_name])
        .arg("-o")
        .arg(&rs_out)
        .arg(&dsp)
        .status()
        .unwrap_or_else(|err| panic!("failed to run faust: {err}"));
    if !status.success() {
        panic!("faust failed compiling {}", dsp.display());
    }

    let raw = fs::read_to_string(&rs_out).expect("read generated rust");
    write_if_changed(
        &out_dir.join(format!("{stem}.inc.rs")),
        strip_generated_helpers(&raw).as_bytes(),
    );

    let json_src = work_dir.join(format!("{stem}.dsp.json"));
    if json_src.is_file() {
        let json = fs::read_to_string(&json_src).expect("read Faust json");
        write_if_changed(
            &out_dir.join(format!("{stem}.json")),
            sanitize_faust_json(&json).as_bytes(),
        );
        let _ = fs::remove_file(&json_src);
    } else {
        panic!("faust -json did not write {}", json_src.display());
    }
}

fn sanitize_faust_json(raw: &str) -> String {
    // Faust embeds absolute include paths. On Windows those use backslashes,
    // which are not valid JSON escapes (`\P`, `\U`, …).
    let normalized = raw.replace('\\', "/");

    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&normalized) else {
        return normalized;
    };
    let Some(obj) = value.as_object_mut() else {
        return normalized;
    };

    if let Some(list) = obj.get_mut("library_list").and_then(|v| v.as_array_mut()) {
        for entry in list {
            if let Some(path) = entry.as_str() {
                *entry = serde_json::Value::String(stabilize_library_path(path));
            }
        }
    }
    if let Some(list) = obj
        .get_mut("include_pathnames")
        .and_then(|v| v.as_array_mut())
    {
        let mut seen = std::collections::BTreeSet::new();
        let mut stable = Vec::new();
        for entry in list.iter() {
            let Some(path) = entry.as_str() else {
                continue;
            };
            let name = stabilize_include_pathname(path);
            if seen.insert(name.clone()) {
                stable.push(serde_json::Value::String(name));
            }
        }
        *list = stable;
    }

    // Compact, deterministic JSON (no host-specific formatting drift).
    serde_json::to_string(&value).unwrap_or(normalized)
}

fn stabilize_library_path(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    // Crate-local libs stay distinguishable from Faust stdlibs.
    if matches!(name, "headphone_crossfeed.lib" | "bformat.lib") {
        format!("dsp/{name}")
    } else {
        name.to_string()
    }
}

fn stabilize_include_pathname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    let name = trimmed.rsplit('/').next().unwrap_or(trimmed);
    // Source tree is `dsp/`; Faust workdir under OUT_DIR is `faust_dsp/`.
    if name == "dsp" || name == "faust_dsp" || trimmed.ends_with("/dsp") {
        "dsp".to_string()
    } else if name == "faust" || trimmed.ends_with("/share/faust") {
        "faust".to_string()
    } else {
        name.to_string()
    }
}

fn strip_generated_helpers(src: &str) -> String {
    let start = src.find("#[cfg(not(target_arch = \"wasm32\"))]");
    let end = src.find("impl FaustDsp for");
    match (start, end) {
        (Some(start), Some(end)) if start < end => {
            let mut out = String::new();
            out.push_str(&src[..start]);
            out.push_str(&src[end..]);
            out
        }
        _ => src.to_string(),
    }
}

fn publish_generated(out_dir: &Path, committed: &Path) {
    let _ = fs::create_dir_all(committed);
    for (stem, _) in CHAINS {
        for ext in ["inc.rs", "json"] {
            let src = out_dir.join(format!("{stem}.{ext}"));
            let Ok(bytes) = fs::read(&src) else {
                continue;
            };
            write_if_changed(&committed.join(format!("{stem}.{ext}")), &bytes);
        }
    }
}

fn write_if_changed(path: &Path, bytes: &[u8]) {
    let same = fs::read(path)
        .ok()
        .is_some_and(|existing| existing == bytes);
    if !same {
        fs::write(path, bytes).unwrap_or_else(|err| {
            panic!("write {}: {err}", path.display());
        });
    }
}

fn write_wrapper(out_dir: &Path) {
    let mut wrapper = String::from(
        r#"// @generated by build.rs — Faust DSP wrappers

pub type F32 = f32;
pub type FaustFloat = f32;

#[derive(Copy, Clone)]
pub struct ParamIndex(pub i32);

pub trait Meta {
    fn declare(&mut self, key: &str, value: &str);
}

pub trait UI<T> {
    fn open_tab_box(&mut self, label: &str);
    fn open_horizontal_box(&mut self, label: &str);
    fn open_vertical_box(&mut self, label: &str);
    fn close_box(&mut self);
    fn add_button(&mut self, label: &str, param: ParamIndex);
    fn add_check_button(&mut self, label: &str, param: ParamIndex);
    fn add_vertical_slider(&mut self, label: &str, param: ParamIndex, init: T, min: T, max: T, step: T);
    fn add_horizontal_slider(&mut self, label: &str, param: ParamIndex, init: T, min: T, max: T, step: T);
    fn add_num_entry(&mut self, label: &str, param: ParamIndex, init: T, min: T, max: T, step: T);
    fn add_horizontal_bargraph(&mut self, label: &str, param: ParamIndex, min: T, max: T);
    fn add_vertical_bargraph(&mut self, label: &str, param: ParamIndex, min: T, max: T);
    fn declare(&mut self, param: Option<ParamIndex>, key: &str, value: &str);
}

pub trait FaustDsp {
    type T;
    fn new() -> Self where Self: Sized;
    fn metadata(&self, m: &mut dyn Meta);
    fn get_sample_rate(&self) -> i32;
    fn get_num_inputs(&self) -> i32;
    fn get_num_outputs(&self) -> i32;
    fn class_init(sample_rate: i32) where Self: Sized;
    fn instance_reset_params(&mut self);
    fn instance_clear(&mut self);
    fn instance_constants(&mut self, sample_rate: i32);
    fn instance_init(&mut self, sample_rate: i32);
    fn init(&mut self, sample_rate: i32);
    fn build_user_interface(&self, ui_interface: &mut dyn UI<Self::T>);
    fn build_user_interface_static(ui_interface: &mut dyn UI<Self::T>) where Self: Sized;
    fn get_param(&self, param: ParamIndex) -> Option<Self::T>;
    fn set_param(&mut self, param: ParamIndex, value: Self::T);
    fn compute(&mut self, count: i32, inputs: &[&[Self::T]], outputs: &mut [&mut [Self::T]]);
}

fn remainderf(from: f32, to: f32) -> f32 {
    from - (from / to).round() * to
}

fn rintf(val: f32) -> f32 {
    val.round_ties_even()
}

"#,
    );

    for (stem, class_name) in CHAINS {
        wrapper.push_str(&format!(
            "pub mod {stem} {{\n    use super::*;\n    include!(\"{stem}.inc.rs\");\n}}\n"
        ));
        wrapper.push_str(&format!("pub use {stem}::{class_name};\n"));
        wrapper.push_str(&format!(
            "pub const {}_JSON: &str = include_str!(\"{stem}.json\");\n\n",
            stem.to_ascii_uppercase()
        ));
    }

    write_if_changed(out_dir.join("monitor_dsp.rs").as_path(), wrapper.as_bytes());
}
