//! Survey of type-4 (light) emitters: size buckets, name prefixes, textures: `particle_lights`.
use std::collections::BTreeMap;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::ParticleSystem;
use alamo_model::particle::BlendMode;

fn main() {
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    let mut size: BTreeMap<&str, usize> = BTreeMap::new();
    let mut prefix: BTreeMap<String, usize> = BTreeMap::new();
    let mut tex: BTreeMap<String, usize> = BTreeMap::new();
    let mut world: BTreeMap<(bool, bool), usize> = BTreeMap::new();
    for f in fs.list("Art") {
        if !f.ends_with(".ALO") {
            continue;
        }
        let Ok(data) = fs.read(&f) else { continue };
        if !data.starts_with(&[0, 9, 0, 0]) {
            continue;
        }
        let Ok(s) = ParticleSystem::parse(&data) else { continue };
        for e in s.emitters.iter().filter(|e| e.blend == BlendMode::Light) {
            let big = e.size.keys.iter().map(|k| k.1).fold(e.size.start.max(e.size.end), f32::max);
            let b = match big {
                x if x <= 20.0 => "<=20",
                x if x <= 100.0 => "<=100",
                x if x <= 300.0 => "<=300",
                x if x <= 1000.0 => "<=1000",
                _ => ">1000",
            };
            *size.entry(b).or_default() += 1;
            let n = e.name.to_ascii_lowercase();
            let p: String = n.split(['_', ' ', '0', '1', '2', '3']).next().unwrap_or("").into();
            *prefix.entry(p).or_default() += 1;
            *tex.entry(e.texture.to_ascii_lowercase()).or_default() += 1;
            *world.entry((e.world_space, e.oriented)).or_default() += 1;
        }
    }
    println!("size {size:?}");
    let mut p: Vec<_> = prefix.into_iter().collect();
    p.sort_by_key(|x| std::cmp::Reverse(x.1));
    println!("prefix {:?}", &p[..p.len().min(15)]);
    let mut t: Vec<_> = tex.into_iter().collect();
    t.sort_by_key(|x| std::cmp::Reverse(x.1));
    println!("tex {:?}", &t[..t.len().min(8)]);
    println!("(world, oriented) {world:?}");
}
