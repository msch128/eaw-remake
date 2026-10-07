//! One line per emitter of a particle system: `particle_summary <path> [workshop_id|-]`.
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::ParticleSystem;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let install = GameInstall::detect().expect("install");
    let id = a.get(1).map_or("1770851727", String::as_str);
    let fs = GameFs::for_mod(&install, (id != "-").then_some(id)).expect("fs");
    let sys = ParticleSystem::parse(&fs.read(&a[0]).expect("read")).expect("parse");
    println!("system {} with {} emitters", sys.name, sys.emitters.len());
    for e in &sys.emitters {
        let c = |i: usize| e.color[i].at(0.0);
        println!(
            "{:<24} {:?} {} grid {} frames {}->{} life {} delay {} rate {} interval {} max {} loop {:#x} size {}->{} rgba0 ({:.2},{:.2},{:.2},{:.2}) world {} oriented {} pos {:?} {:?}..{:?} vel {:?} {:?}..{:?}",
            e.name,
            e.blend,
            e.texture,
            e.atlas_grid,
            e.frame.start,
            e.frame.end,
            e.lifetime,
            e.initial_delay,
            e.spawn_rate,
            e.spawn_interval,
            e.max_particles,
            e.loop_count,
            e.size.start,
            e.size.end,
            c(0),
            c(1),
            c(2),
            c(3),
            e.world_space,
            e.oriented,
            e.position.kind,
            e.position.min,
            e.position.max,
            e.velocity.kind,
            e.velocity.min,
            e.velocity.max,
        );
    }
}
