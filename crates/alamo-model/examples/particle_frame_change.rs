//! Which emitters a 90 degree turn of the particle frame about X moves: `particle_frame_change [file...]`.
//! Invariant under that turn are spheres, cubes (kind 2), zero exact values and boxes symmetric about the origin whose Y and Z extents
//! match; directed exact values, off-centre or uneven boxes and acceleration vectors move. Without files it counts per name prefix.
use std::collections::BTreeMap;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::ParticleSystem;
use alamo_model::particle::{Emitter, Shape};

fn moves(s: &Shape) -> bool {
    match s.kind {
        0 => s.params[7].abs() > 1e-3 || s.params[8].abs() > 1e-3,
        1 => {
            let c = |i: usize| (s.min[i] + s.max[i]) * 0.5;
            let h = |i: usize| (s.max[i] - s.min[i]).abs();
            c(1).abs() > 1e-3 || c(2).abs() > 1e-3 || (h(1) - h(2)).abs() > 1e-3
        }
        _ => false,
    }
}

fn reason(e: &Emitter) -> Option<String> {
    let mut r = Vec::new();
    if moves(&e.speed_group) {
        r.push("speed");
    }
    if moves(&e.position_group) {
        r.push("position");
    }
    if e.acceleration[1].abs() > 1e-3 || e.acceleration[2].abs() > 1e-3 {
        r.push("acceleration");
    }
    (!r.is_empty()).then(|| r.join("+"))
}

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    if !files.is_empty() {
        for f in &files {
            let path = if f.contains('/') { f.clone() } else { format!("Art/Models/{f}") };
            let Ok(s) = fs.read(&path).map_err(|e| e.to_string()).and_then(|d| ParticleSystem::parse(&d).map_err(|e| e.to_string())) else {
                println!("{f}: unreadable");
                continue;
            };
            let moved: Vec<String> = s.emitters.iter().filter_map(|e| reason(e).map(|r| format!("{} ({r})", e.name))).collect();
            println!("{f}: {} of {} emitters move: {}", moved.len(), s.emitters.len(), moved.join(", "));
        }
        return;
    }
    let mut by_prefix: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    for f in fs.list("Art") {
        if !f.to_ascii_uppercase().ends_with(".ALO") {
            continue;
        }
        let Ok(data) = fs.read(&f) else { continue };
        if !data.starts_with(&[0, 9, 0, 0]) {
            continue;
        }
        let Ok(s) = ParticleSystem::parse(&data) else { continue };
        let name = f.rsplit('/').next().unwrap_or(&f).to_ascii_lowercase();
        let prefix = name.split('_').next().unwrap_or("").to_string();
        let moved = s.emitters.iter().filter(|e| reason(e).is_some()).count();
        let entry = by_prefix.entry(prefix).or_default();
        entry.0 += 1;
        entry.1 += usize::from(moved > 0);
        entry.2 += moved;
    }
    for (p, (files, with, emitters)) in &by_prefix {
        println!("{p:<12} {files:5} files, {with:5} with moving emitters ({emitters} emitters)");
    }
}
