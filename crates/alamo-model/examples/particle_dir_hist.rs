//! Histogram of the dominant axis of emitters' directed speed and position (exact value or box centre).
use std::collections::BTreeMap;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::ParticleSystem;
use alamo_model::particle::Shape;

fn directed(s: &Shape) -> [f32; 3] {
    match s.kind {
        0 => [s.params[6], s.params[7], s.params[8]],
        1 => [(s.min[0] + s.max[0]) * 0.5, (s.min[1] + s.max[1]) * 0.5, (s.min[2] + s.max[2]) * 0.5],
        _ => [0.0; 3],
    }
}

fn axis(v: [f32; 3]) -> Option<String> {
    let (i, m) = v.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))?;
    if m.abs() < 1e-3 {
        return None;
    }
    Some(format!("{}{}", if *m < 0.0 { "-" } else { "+" }, ["x", "y", "z"][i]))
}

fn main() {
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    let mut hist: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut ex: BTreeMap<(String, String), String> = BTreeMap::new();
    for f in fs.list("Art") {
        if !f.to_ascii_uppercase().ends_with(".ALO") {
            continue;
        }
        let Ok(data) = fs.read(&f) else { continue };
        if !data.starts_with(&[0, 9, 0, 0]) {
            continue;
        }
        let Ok(s) = ParticleSystem::parse(&data) else { continue };
        for e in &s.emitters {
            for (what, sh) in [("speed", &e.speed_group), ("pos", &e.position_group)] {
                if let Some(a) = axis(directed(sh)) {
                    let k = (what.to_string(), a);
                    *hist.entry(k.clone()).or_default() += 1;
                    ex.entry(k).or_insert_with(|| format!("{} {}", s.name, e.name));
                }
            }
        }
    }
    for (k, n) in &hist {
        println!("{:<6} {:<3} {:6}  e.g. {}", k.0, k.1, n, ex[k]);
    }
}
