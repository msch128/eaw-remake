//! Where the particle emitters on model proxies point, in model space, under the two candidate particle
//! axis mappings: `id` (particle axes are the proxy bone's own axes) and `y-up` (particle `(x, y, z)` on bone
//! `(x, -z, y)`).
//!
//! `particle_axes <model.alo> [--prefix pe_]` prints every particle proxy of one model: the bone's model-space
//! position and axes, then per emitter the directed part of its speed and position groups (exact value or box
//! centre; spheres, cubes and cylinders are symmetric and give zero) mapped to model space.
//!
//! `particle_axes --survey [prefix...]` runs over every model of the game: for engine proxies (`pe_`, `pte_` by
//! default) it averages the emitters' directed speed and counts how often it leaves the stern (model `+Y`,
//! bows point `-Y`) under each mapping.
use std::collections::HashMap;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::particle::Shape;
use alamo_model::{Model, ParticleSystem};

fn directed(s: &Shape) -> [f32; 3] {
    match s.kind {
        0 => [s.params[6], s.params[7], s.params[8]],
        1 => [(s.min[0] + s.max[0]) * 0.5, (s.min[1] + s.max[1]) * 0.5, (s.min[2] + s.max[2]) * 0.5],
        _ => [0.0; 3],
    }
}

fn rot(m: &[[f32; 4]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn y_up(v: [f32; 3]) -> [f32; 3] {
    [v[0], -v[2], v[1]]
}

fn unit(v: [f32; 3]) -> Option<[f32; 3]> {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (l > 1e-3).then(|| [v[0] / l, v[1] / l, v[2] / l])
}

fn fmt(v: [f32; 3]) -> String {
    format!("({:8.1} {:8.1} {:8.1})", v[0], v[1], v[2])
}

fn one_model(fs: &GameFs, path: &str, prefix: Option<&str>) {
    let model = Model::parse(&fs.read(path).expect("read model")).expect("parse model");
    for p in &model.proxies {
        if prefix.is_some_and(|pre| !p.name.to_ascii_lowercase().starts_with(pre)) {
            continue;
        }
        let Ok(data) = fs.read(&format!("Art/Models/{}.alo", p.name)) else { continue };
        let Ok(sys) = ParticleSystem::parse(&data) else { continue };
        let w = model.world_transform(p.bone);
        println!(
            "{} on {} at {} bone X {} Y {} Z {}",
            p.name,
            model.bones[p.bone].name,
            fmt([w[0][3], w[1][3], w[2][3]]),
            fmt(rot(&w, [1.0, 0.0, 0.0])),
            fmt(rot(&w, [0.0, 1.0, 0.0])),
            fmt(rot(&w, [0.0, 0.0, 1.0]))
        );
        for e in &sys.emitters {
            let (v, x) = (directed(&e.speed_group), directed(&e.position_group));
            if v.iter().chain(&x).all(|c| c.abs() < 1e-3) {
                continue;
            }
            // Model-space point where a particle ends its life (spawn offset plus speed times life), per mapping.
            let end = |m: fn([f32; 3]) -> [f32; 3]| {
                let d = rot(&w, m([x[0] + v[0] * e.lifetime, x[1] + v[1] * e.lifetime, x[2] + v[2] * e.lifetime]));
                [w[0][3] + d[0], w[1][3] + d[1], w[2][3] + d[2]]
            };
            println!(
                "    {:<22} life {:5.2} speed {} pos {} | id: vel {} pos {} ends {} | y-up: vel {} pos {} ends {}",
                e.name,
                e.lifetime,
                fmt(v),
                fmt(x),
                fmt(rot(&w, v)),
                fmt(rot(&w, x)),
                fmt(end(|v| v)),
                fmt(rot(&w, y_up(v))),
                fmt(rot(&w, y_up(x))),
                fmt(end(y_up))
            );
        }
    }
}

fn survey(fs: &GameFs, prefixes: &[String]) {
    let mut systems: HashMap<String, Option<ParticleSystem>> = HashMap::new();
    let (mut proxies, mut id_back, mut yup_back, mut neither) = (0, 0, 0, 0);
    let mut odd = Vec::new();
    for f in fs.list("Art") {
        if !f.to_ascii_uppercase().ends_with(".ALO") {
            continue;
        }
        let Ok(data) = fs.read(&f) else { continue };
        if data.starts_with(&[0, 9, 0, 0]) {
            continue; // a particle system
        }
        let Ok(model) = Model::parse(&data) else { continue };
        for p in &model.proxies {
            let lower = p.name.to_ascii_lowercase();
            if !prefixes.iter().any(|pre| lower.starts_with(pre.as_str())) {
                continue;
            }
            let sys = systems.entry(lower.clone()).or_insert_with(|| {
                fs.read(&format!("Art/Models/{}.alo", p.name)).ok().and_then(|d| ParticleSystem::parse(&d).ok())
            });
            let Some(sys) = sys else { continue };
            // Mean directed speed of the emitters that move along an axis.
            let mut sum = [0.0f32; 3];
            for e in &sys.emitters {
                if let Some(u) = unit(directed(&e.speed_group)) {
                    (0..3).for_each(|i| sum[i] += u[i]);
                }
            }
            let Some(dir) = unit(sum) else { continue };
            let w = model.world_transform(p.bone);
            // Engines (`pe_`, `pte_`) should blow out of the stern (model +Y); anything else (damage fire) away from the hull's
            // long axis, judged in the cross-section (x, z) through the bone.
            let engine = lower.starts_with("pe") || lower.starts_with("pte");
            let out = unit([w[0][3], 0.0, w[2][3]]);
            let back = |v: [f32; 3]| {
                unit(rot(&w, v)).is_some_and(|m| if engine { m[1] > 0.7 } else { out.is_some_and(|o| m[0] * o[0] + m[2] * o[2] > 0.3) })
            };
            proxies += 1;
            match (back(dir), back(y_up(dir))) {
                (true, false) => id_back += 1,
                (false, true) => yup_back += 1,
                (true, true) => {}
                (false, false) => {
                    neither += 1;
                    if odd.len() < 12 {
                        odd.push(format!("{f} {} id {} y-up {}", p.name, fmt(rot(&w, dir)), fmt(rot(&w, y_up(dir)))));
                    }
                }
            }
        }
    }
    println!("{proxies} proxies with a directed stream: {id_back} point the expected way (engines out of the stern, others away from the hull axis) with id only, {yup_back} with y-up only, {neither} with neither");
    for o in odd {
        println!("  neither: {o}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    if args.first().is_some_and(|a| a == "--survey") {
        let mut prefixes: Vec<String> = args[1..].iter().map(|s| s.to_ascii_lowercase()).collect();
        if prefixes.is_empty() {
            prefixes = vec!["pe_".into(), "pte_".into()];
        }
        survey(&fs, &prefixes);
        return;
    }
    let path = args.first().expect("usage: particle_axes <model.alo> [--prefix p] | --survey [prefix...]");
    let prefix = args.iter().position(|a| a == "--prefix").and_then(|i| args.get(i + 1)).map(|s| s.to_ascii_lowercase());
    one_model(&fs, path, prefix.as_deref());
}
