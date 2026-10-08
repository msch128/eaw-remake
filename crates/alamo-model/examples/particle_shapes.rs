//! Survey of emitter shape kinds and the speed property: `particle_shapes`.
use std::collections::BTreeMap;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::ParticleSystem;

fn main() {
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    let mut kinds: BTreeMap<(&str, u32), usize> = BTreeMap::new();
    let mut examples: BTreeMap<(&str, u32), String> = BTreeMap::new();
    let mut speed: BTreeMap<String, usize> = BTreeMap::new();
    for f in fs.list("Art") {
        if !f.ends_with(".ALO") {
            continue;
        }
        let Ok(data) = fs.read(&f) else { continue };
        if !data.starts_with(&[0, 9, 0, 0]) {
            continue;
        }
        let Ok(s) = ParticleSystem::parse(&data) else { continue };
        for e in &s.emitters {
            for (n, sh) in [("pos", &e.position), ("vel", &e.velocity), ("extra", &e.extra)] {
                *kinds.entry((n, sh.kind)).or_default() += 1;
                examples.entry((n, sh.kind)).or_insert_with(|| {
                    format!("{} {}: min {:?} max {:?} params {:?}", s.name, e.name, sh.min, sh.max, sh.params)
                });
            }
            let k = if e.speed == 0.0 { "0".to_string() } else if e.speed == 1.0 { "1".into() } else { "other".into() };
            *speed.entry(format!("vel kind {} speed {k}", e.velocity.kind)).or_default() += 1;
        }
    }
    for (k, v) in &kinds {
        println!("{k:?}: {v}  e.g. {}", examples[k]);
    }
    for (k, v) in &speed {
        println!("{k}: {v}");
    }
}
