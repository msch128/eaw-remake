//! Prints the decoded fields of every emitter of one particle file: `particle_one <path>`.
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::ParticleSystem;

fn main() {
    let path = std::env::args().nth(1).expect("path");
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    let s = ParticleSystem::parse(&fs.read(&path).expect("read")).expect("parse");
    for e in &s.emitters {
        println!(
            "{:<22} {:?} tex {} grid {} life {} speed {} acc {:?} size {:?}->{:?} keys {:?}\n    pos k{} {:?}..{:?} {:?}\n    vel k{} {:?}..{:?} {:?}\n    ext k{} {:?}..{:?} {:?}\n    props {:?}",
            e.name, e.blend, e.texture, e.atlas_grid, e.lifetime, e.speed, e.acceleration,
            e.size.start, e.size.end, e.size.keys,
            e.speed_group.kind, e.speed_group.min, e.speed_group.max, e.speed_group.params,
            e.lifetime_group.kind, e.lifetime_group.min, e.lifetime_group.max, e.lifetime_group.params,
            e.position_group.kind, e.position_group.min, e.position_group.max, e.position_group.params,
            e.props.iter().filter(|(k, _)| [0x0b, 0x17, 0x3c, 0x3e, 0x3f, 0x40, 0x42, 0x12, 0x13, 0x0c].contains(k))
                .map(|(k, v)| format!("{k:#x}={}", if v.len() == 4 { format!("{}", f32::from_le_bytes([v[0], v[1], v[2], v[3]])) } else { format!("{v:?}") })).collect::<Vec<_>>(),
        );
    }
}
