//! Prints the decoded fields of every emitter of one particle file: `particle_one <path>`.
//!
//! Per emitter: spawn model (`inBursts`, per second, burst fields), sprite orientation, the colour, size, frame and spin tracks with their
//! keys, and the speed / lifetime / position property groups.
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::ParticleSystem;
use alamo_model::particle::{ColorTrack, FloatTrack};

fn color(name: &str, t: &ColorTrack) -> String {
    format!("{name} {:.3}->{:.3} {:?}", t.start, t.end, t.keys)
}

fn float(name: &str, t: &FloatTrack) -> String {
    format!("{name} {:.2}->{:.2} mode {} {:?}", t.start, t.end, t.mode, t.keys)
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: particle_one <path in the game files>");
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    let s = ParticleSystem::parse(&fs.read(&path).expect("read")).expect("parse");
    for e in &s.emitters {
        let burst = if e.in_bursts { "in bursts" } else { "continuous" };
        let loops = if e.loop_count == u32::MAX { "inf".to_string() } else { e.loop_count.to_string() };
        println!(
            "{:<22} {:?} tex {} grid {} life {} speed {} acc {:?} {} ({burst}: {} per second, per burst {}, every {} s, {loops} times, delay {})",
            e.name, e.blend, e.texture, e.atlas_grid, e.lifetime, e.speed, e.acceleration,
            if e.billboard { "billboard" } else { "flat sprite" },
            e.spawn_rate, e.max_particles, e.spawn_interval, e.initial_delay,
        );
        println!("    {}\n    {}\n    {}\n    {}", color("r", &e.color[0]), color("g", &e.color[1]), color("b", &e.color[2]), color("a", &e.color[3]));
        println!("    {}\n    {}\n    {}", float("size", &e.size), float("frame", &e.frame), float("spin", &e.spin));
        for (name, g) in [("speed", &e.speed_group), ("lifetime", &e.lifetime_group), ("position", &e.position_group)] {
            println!("    {name} group kind {} {:?}..{:?} {:?}", g.kind, g.min, g.max, g.params);
        }
        let props: Vec<String> = e
            .props
            .iter()
            .filter(|(k, _)| [0x0b, 0x0c, 0x12, 0x13, 0x17, 0x3c, 0x3e, 0x3f, 0x40, 0x42].contains(k))
            .map(|(k, v)| format!("{k:#x}={}", if v.len() == 4 { f32::from_le_bytes([v[0], v[1], v[2], v[3]]).to_string() } else { format!("{v:?}") }))
            .collect();
        println!("    props {props:?}");
    }
}
