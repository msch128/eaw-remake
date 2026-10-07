//! Parses every `.ala` and validates it: `parse_all_ala [workshop_id|-]`.
//!
//! Checks quaternion norms, finite values, and that bone names match the skeleton of the model
//! with the same base name (animation file `X_IDLE_00.ALA` -> model `X.ALO`).
use std::collections::BTreeMap;
use std::panic::catch_unwind;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::ala::Track;
use alamo_model::{Animation, Model};

fn main() {
    let id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "1770851727".into());
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, (id != "-").then_some(id.as_str())).expect("fs");
    let (mut ok, mut failed, mut panics) = (0, 0, 0);
    let mut errors: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let (mut bad_quat, mut nonfinite, mut keys_total, mut matched, mut unmatched, mut no_model) =
        (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let (mut perbone, mut with_vis, mut with_unk, mut animated_scale) =
        (0usize, 0usize, 0usize, 0usize);
    let mut fps: BTreeMap<String, usize> = BTreeMap::new();
    let mut models: BTreeMap<String, Option<Model>> = BTreeMap::new();
    for f in fs.list("Art") {
        if !f.ends_with(".ALA") {
            continue;
        }
        let Ok(data) = fs.read(&f) else { continue };
        match catch_unwind(|| Animation::parse(&data)) {
            Ok(Ok(a)) => {
                ok += 1;
                *fps.entry(format!("{}", a.fps)).or_default() += 1;
                // find model: strip suffix _<STATE>_<NN>.ALA progressively
                let stem = f.trim_end_matches(".ALA");
                let mut cand = stem.to_string();
                let mut model_key = None;
                while let Some(i) = cand.rfind('_') {
                    cand.truncate(i);
                    if fs.exists(&format!("{cand}.ALO")) {
                        model_key = Some(cand.clone());
                        break;
                    }
                }
                for b in &a.bones {
                    let mut chk = |q: [f32; 4]| {
                        let n: f32 = q.iter().map(|x| x * x).sum::<f32>().sqrt();
                        if (n - 1.0).abs() > 0.02 {
                            bad_quat += 1;
                        }
                    };
                    match &b.rotation {
                        Track::Constant(q) => chk(*q),
                        Track::Keys(k) => {
                            keys_total += k.len();
                            k.iter().for_each(|q| chk(*q));
                        }
                    }
                    let t_ok = |v: &[f32; 3]| v.iter().all(|x| x.is_finite());
                    let all_t = match &b.translation {
                        Track::Constant(v) => t_ok(v),
                        Track::Keys(k) => k.iter().all(t_ok),
                    };
                    if !all_t {
                        nonfinite += 1;
                    }
                    if b.scale.is_animated() {
                        animated_scale += 1;
                    }
                    with_vis += b.visibility.is_some() as usize;
                    with_unk += b.unknown_bits.is_some() as usize;
                }
                perbone += a.bones.iter().any(|b| b.rotation.is_animated()) as usize;
                if let Some(k) = model_key {
                    let m = models.entry(k.clone()).or_insert_with(|| {
                        fs.read(&format!("{k}.ALO"))
                            .ok()
                            .and_then(|d| Model::parse(&d).ok())
                    });
                    if let Some(m) = m {
                        for b in &a.bones {
                            if m.bone_index(&b.name).is_some() {
                                matched += 1
                            } else {
                                unmatched += 1
                            }
                        }
                    } else {
                        no_model += 1;
                    }
                } else {
                    no_model += 1;
                }
            }
            Ok(Err(e)) => {
                failed += 1;
                errors.entry(e.to_string()).or_default().push(f);
            }
            Err(_) => {
                panics += 1;
                errors.entry("PANIC".into()).or_default().push(f);
            }
        }
    }
    println!("animations ok: {ok}, failed: {failed}, panics: {panics}");
    println!(
        "rotation keys {keys_total}, non-unit quaternions {bad_quat}, non-finite translations {nonfinite}"
    );
    println!(
        "bone tracks matching skeleton by name: {matched}, unmatched: {unmatched}; animations without resolvable model: {no_model}"
    );
    println!(
        "animated scale tracks {animated_scale}, visibility tracks {with_vis}, 0x1008 tracks {with_unk}, files with animated rotation {perbone}"
    );
    println!("fps: {fps:?}");
    for (e, files) in &errors {
        println!(
            "ERROR {e}: {} files, e.g. {:?}",
            files.len(),
            &files[..files.len().min(3)]
        );
    }
}
