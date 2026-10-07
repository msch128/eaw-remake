//! Parses every `.alo` reachable in a game file system and reports success rate and shader usage.
//!
//! `parse_all [workshop_id|-] [dir]` (default: mod 1770851727, dir `Art`).
use std::collections::BTreeMap;
use std::panic::catch_unwind;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::{Error, Model};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let id = args.get(1).map_or("1770851727", String::as_str);
    let dir = args.get(2).map_or("Art", String::as_str);
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, (id != "-").then_some(id)).expect("fs");
    let (mut ok, mut particles, mut failed, mut panics) = (0, 0, 0, 0);
    let mut shaders: BTreeMap<String, (usize, usize)> = BTreeMap::new(); // sub-meshes, files
    let mut formats: BTreeMap<String, usize> = BTreeMap::new();
    let mut errors: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut billboards: BTreeMap<String, usize> = BTreeMap::new();
    let mut winding = (0usize, 0usize);
    let mut sanity = (0usize, 0usize, 0usize, 0usize);
    let (mut tris, mut skinned_files, mut lights) = (0usize, 0usize, 0usize);
    for f in fs.list(dir) {
        if !f.ends_with(".ALO") {
            continue;
        }
        let data = match fs.read(&f) {
            Ok(d) => d,
            Err(e) => {
                failed += 1;
                errors.entry(format!("io: {e}")).or_default().push(f);
                continue;
            }
        };
        match catch_unwind(|| Model::parse(&data)) {
            Ok(Ok(m)) => {
                ok += 1;
                let mut seen = std::collections::BTreeSet::new();
                let mut skinned = false;
                for b in &m.bones {
                    *billboards.entry(format!("{:?}", b.billboard)).or_default() += 1;
                }
                for mesh in &m.meshes {
                    for s in &mesh.sub_meshes {
                        let name = s.material.shader.to_ascii_lowercase();
                        shaders.entry(name.clone()).or_default().0 += 1;
                        seen.insert(name);
                        *formats.entry(s.vertex_format.clone()).or_default() += 1;
                        tris += s.indices.len() / 3;
                        skinned |= s.is_skinned();
                        if !s.material.shader.to_ascii_lowercase().contains("shadow") {
                            for t in s.indices.as_chunks::<3>().0 {
                                let (a, b, c) = (
                                    s.vertices[t[0] as usize],
                                    s.vertices[t[1] as usize],
                                    s.vertices[t[2] as usize],
                                );
                                let e1: Vec<f32> =
                                    (0..3).map(|i| b.position[i] - a.position[i]).collect();
                                let e2: Vec<f32> =
                                    (0..3).map(|i| c.position[i] - a.position[i]).collect();
                                let n = [
                                    e1[1] * e2[2] - e1[2] * e2[1],
                                    e1[2] * e2[0] - e1[0] * e2[2],
                                    e1[0] * e2[1] - e1[1] * e2[0],
                                ];
                                let d: f32 = (0..3)
                                    .map(|i| n[i] * (a.normal[i] + b.normal[i] + c.normal[i]))
                                    .sum();
                                if d > 1e-9 {
                                    winding.0 += 1
                                } else if d < -1e-9 {
                                    winding.1 += 1
                                }
                            }
                        }
                        for v in &s.vertices {
                            let l = v.normal.iter().map(|x| x * x).sum::<f32>().sqrt();
                            if (l - 1.0).abs() > 0.05 {
                                sanity.0 += 1;
                            }
                            if s.is_skinned() {
                                let w: f32 = v.bone_weights.iter().sum();
                                if (w - 1.0).abs() > 0.05 {
                                    sanity.1 += 1;
                                }
                                if v.bone_indices
                                    .iter()
                                    .zip(v.bone_weights)
                                    .any(|(&i, w)| w > 0.0 && i as usize >= s.bone_palette.len())
                                {
                                    sanity.2 += 1;
                                }
                            }
                            sanity.3 += 1;
                        }
                    }
                }
                for s in seen {
                    shaders.entry(s).or_default().1 += 1;
                }
                skinned_files += skinned as usize;
                lights += m.lights.len();
            }
            Ok(Err(Error::NotAModel(_))) => particles += 1,
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
    println!(
        "models ok: {ok}, particle systems (skipped): {particles}, failed: {failed}, panics: {panics}"
    );
    println!("triangles: {tris}, skinned files: {skinned_files}, lights: {lights}");
    println!(
        "vertices {}: bad normal len {}, bad weight sum {}, bone index outside palette {}",
        sanity.3, sanity.0, sanity.1, sanity.2
    );
    println!(
        "winding: normal agrees with CCW cross product {}, CW {}",
        winding.0, winding.1
    );
    for (e, files) in &errors {
        println!(
            "ERROR {e}: {} files, e.g. {:?}",
            files.len(),
            &files[..files.len().min(3)]
        );
    }
    let mut v: Vec<_> = shaders.into_iter().collect();
    v.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
    println!("\nshader (sub-meshes, files):");
    for (s, (n, f)) in v {
        println!("{s:40} {n:7} {f:6}");
    }
    println!("\nbillboard modes (bones): {billboards:?}");
    println!("\nvertex formats:");
    for (s, n) in formats {
        println!("{s:40} {n:7}");
    }
}
