//! How particle emitters spawn and how their sprites are oriented, over every particle system of the game: `particle_spawn`.
//!
//! Prints the emitters grouped by `inBursts` (property 0x07), `numBursts` (0x27, `inf` = `u32::MAX`), `numParticlesPerBurst` (0x26)
//! and by `isNotBillboarded` (0x2e) per blend mode, then the steady live particle count (`numParticlesPerSecond * lifetime`) of the
//! engine systems (`pe_`, `pte_`). Property names follow https://modtools.petrolution.net/docs/ParticleFileFormat; ids are the
//! doc's decimal numbers in hex here. Findings (2026-10-17): 1071 emitters loop for ever without bursts and carry
//! `numParticlesPerBurst` 1 (the editor default; they run at their per-second rate), 429 of 8,251 are not billboarded, and an engine
//! proxy keeps 19 particles alive on average (132 at most).
use std::collections::BTreeMap;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::ParticleSystem;

fn class(n: u32) -> &'static str {
    match n {
        0 => "0",
        1 => "1",
        u32::MAX => "inf",
        _ => "many",
    }
}

fn flag(e: &alamo_model::particle::Emitter, id: u8) -> String {
    e.props.get(&id).and_then(|p| p.first()).map_or("-".into(), |b| b.to_string())
}

fn main() {
    let install = GameInstall::detect().expect("Empire at War install not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("file system");
    // key -> (count, first example)
    let mut spawn: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut flat: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut engines: Vec<(f32, String)> = Vec::new();
    let mut total = 0;
    for f in fs.list("Art") {
        if !f.to_ascii_lowercase().ends_with(".alo") {
            continue;
        }
        let Ok(data) = fs.read(&f) else { continue };
        if !data.starts_with(&[0, 9, 0, 0]) {
            continue;
        }
        let Ok(system) = ParticleSystem::parse(&data) else { continue };
        let file = f.to_ascii_lowercase();
        let engine = file.rsplit('/').next().is_some_and(|n| n.starts_with("pe_") || n.starts_with("pte_"));
        let mut live = 0.0;
        for e in &system.emitters {
            total += 1;
            let example = format!("{} / {}", system.name, e.name);
            let key = format!("inBursts {} numBursts {:>4} perBurst {:>4}", flag(e, 0x07), class(e.loop_count), class(e.max_particles));
            let entry = spawn.entry(key).or_insert((0, example.clone()));
            entry.0 += 1;
            let key = format!("notBillboarded {} {:?}", flag(e, 0x2e), e.blend);
            let entry = flat.entry(key).or_insert((0, example));
            entry.0 += 1;
            if engine && !e.in_bursts {
                live += e.spawn_rate * e.lifetime;
            }
        }
        if engine {
            engines.push((live, system.name.clone()));
        }
    }
    println!("{total} emitters");
    for (title, table) in [("spawn model", spawn), ("sprite orientation", flat)] {
        println!("\n{title}:");
        let mut rows: Vec<_> = table.into_iter().collect();
        rows.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
        for (key, (n, example)) in rows {
            println!("{n:5} {key:50} e.g. {example}");
        }
    }
    engines.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mean = engines.iter().map(|e| e.0).sum::<f32>() / engines.len().max(1) as f32;
    println!("\n{} engine systems keep {mean:.1} particles alive on average; the largest:", engines.len());
    for (live, name) in engines.iter().take(5) {
        println!("{live:6.0} {name}");
    }
}
