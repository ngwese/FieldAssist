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
];

fn main() {
    println!("cargo:rerun-if-changed=assets/app-icon/app-icon.ico");
    println!("cargo:rerun-if-changed=assets/logo/04-bands.svg");
    println!("cargo:rerun-if-changed=dsp");
    for (stem, _) in CHAINS {
        println!("cargo:rerun-if-changed=dsp/{stem}.dsp");
    }
    println!("cargo:rerun-if-changed=dsp/headphone_crossfeed.lib");

    compile_faust_chains();
    embed_windows_icon();
}

fn compile_faust_chains() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let dsp_dir = manifest.join("dsp");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let committed = manifest.join("src").join("monitor").join("generated");
    let faust = faust_command();

    if let Some(faust) = faust.as_ref() {
        for (stem, class_name) in CHAINS {
            compile_one(faust, &dsp_dir, &out_dir, stem, class_name);
        }
        publish_generated(&out_dir, &committed);
    } else {
        for (stem, _) in CHAINS {
            let src_rs = committed.join(format!("{stem}.inc.rs"));
            let src_json = committed.join(format!("{stem}.json"));
            if !src_rs.is_file() || !src_json.is_file() {
                panic!(
                    "Faust compiler not found and missing {}; install Faust or restore generated files",
                    src_rs.display()
                );
            }
            fs::copy(&src_rs, out_dir.join(format!("{stem}.inc.rs")))
                .expect("copy committed Faust rust");
            let json = fs::read_to_string(&src_json).expect("read committed Faust json");
            fs::write(
                out_dir.join(format!("{stem}.json")),
                sanitize_faust_json(&json),
            )
            .expect("write Faust json");
        }
    }

    write_wrapper(&out_dir);
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

fn compile_one(faust: &Path, dsp_dir: &Path, out_dir: &Path, stem: &str, class_name: &str) {
    let dsp = dsp_dir.join(format!("{stem}.dsp"));
    let rs_out = out_dir.join(format!("{stem}.raw.rs"));
    let status = Command::new(faust)
        .arg("-I")
        .arg(dsp_dir)
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
    fs::write(
        out_dir.join(format!("{stem}.inc.rs")),
        strip_generated_helpers(&raw),
    )
    .expect("write stripped rust");

    let json_src = dsp_dir.join(format!("{stem}.dsp.json"));
    if json_src.is_file() {
        let json = fs::read_to_string(&json_src).expect("read Faust json");
        fs::write(
            out_dir.join(format!("{stem}.json")),
            sanitize_faust_json(&json),
        )
        .expect("write Faust json");
        let _ = fs::remove_file(&json_src);
    } else {
        panic!("faust -json did not write {}", json_src.display());
    }
}

fn sanitize_faust_json(raw: &str) -> String {
    // Faust embeds absolute include paths. On Windows those use backslashes,
    // which are not valid JSON escapes (`\P`, `\U`, …).
    raw.replace('\\', "/")
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
            let dst = committed.join(format!("{stem}.{ext}"));
            let Ok(bytes) = fs::read(&src) else {
                continue;
            };
            let same = fs::read(&dst)
                .ok()
                .is_some_and(|existing| existing == bytes);
            if !same {
                fs::write(&dst, bytes).expect("write committed Faust artifact");
            }
        }
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

    fs::write(out_dir.join("monitor_dsp.rs"), wrapper).expect("write Faust wrapper");
}

fn embed_windows_icon() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let icon = Path::new(&manifest_dir)
        .join("assets")
        .join("app-icon")
        .join("app-icon.ico");
    if !icon.is_file() {
        panic!(
            "missing {}; run python script/generate-app-icon.py",
            icon.display()
        );
    }

    let icon_escaped = icon.display().to_string().replace('\\', "/");
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR");
    let rc_path = Path::new(&out_dir).join("app_icon.rc");
    fs::write(&rc_path, format!("1 ICON \"{icon_escaped}\"\n")).expect("write app_icon.rc");

    embed_resource::compile(&rc_path, embed_resource::NONE)
        .manifest_optional()
        .expect("embed Windows app icon");
}
