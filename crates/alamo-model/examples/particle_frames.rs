//! Lists emitters whose atlas frame end value is a real range: `particle_frames`.
use std::collections::BTreeMap;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::ParticleSystem;

fn main() {
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
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
            if e.frame.end > 2.0 && e.frame.end != e.frame.start {
                let k = format!(
                    "{:?} tex {} grid {} frames {}->{} life {} (e.g. {} / {})",
                    e.blend, e.texture, e.atlas_grid, e.frame.start, e.frame.end, e.lifetime, s.name, e.name
                );
                *seen.entry(k).or_default() += 1;
            }
        }
    }
    let mut v: Vec<_> = seen.into_iter().collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.1));
    for (k, n) in v.iter().take(40) {
        println!("{n:4} {k}");
    }
}
