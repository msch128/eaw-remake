//! Survey of the float tracks (size, frame, spin) of every particle emitter: `particle_tracks`.
use std::collections::BTreeMap;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::ParticleSystem;

fn main() {
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    // (track, mode, keys > 0, start == end) -> count
    let mut stats: BTreeMap<(&str, u32, bool, bool), usize> = BTreeMap::new();
    let mut frame_vs_cells: BTreeMap<String, usize> = BTreeMap::new();
    let mut frame_end: BTreeMap<i32, usize> = BTreeMap::new();
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
            for (name, t) in [("size", &e.size), ("frame", &e.frame), ("spin", &e.spin)] {
                *stats
                    .entry((name, t.mode, !t.keys.is_empty(), t.start == t.end))
                    .or_default() += 1;
            }
            *frame_end.entry(e.frame.end as i32).or_default() += 1;
            let cells = e.atlas_grid * e.atlas_grid;
            let hi = e.frame.start.max(e.frame.end) as u32;
            let key = format!(
                "blend {:?} cells {} {}",
                e.blend,
                if cells <= 1 { "1" } else { "many" },
                if hi >= cells { "frame>=cells" } else { "frame<cells" }
            );
            *frame_vs_cells.entry(key).or_default() += 1;
        }
    }
    for (k, v) in &stats {
        println!("{k:?}: {v}");
    }
    println!("frame end histogram: {frame_end:?}");
    for (k, v) in &frame_vs_cells {
        println!("{k}: {v}");
    }
}
