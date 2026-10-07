//! Parses every particle `.alo`: `parse_all_particles [workshop_id|-]`.
use std::collections::BTreeMap;
use std::panic::catch_unwind;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::ParticleSystem;
use alamo_model::particle::BlendMode;

fn main() {
    let id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "1770851727".into());
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, (id != "-").then_some(id.as_str())).expect("fs");
    let (mut ok, mut failed, mut panics, mut emitters) = (0, 0, 0, 0);
    let mut blends: BTreeMap<String, usize> = BTreeMap::new();
    let mut textures: BTreeMap<String, usize> = BTreeMap::new();
    let mut errors: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in fs.list("Art") {
        if !f.ends_with(".ALO") {
            continue;
        }
        let Ok(data) = fs.read(&f) else { continue };
        if !data.starts_with(&[0, 9, 0, 0]) {
            continue;
        }
        match catch_unwind(|| ParticleSystem::parse(&data)) {
            Ok(Ok(s)) => {
                ok += 1;
                emitters += s.emitters.len();
                for e in &s.emitters {
                    let b = match e.blend {
                        BlendMode::Other(n) => format!("Other({n})"),
                        b => format!("{b:?}"),
                    };
                    *blends.entry(b).or_default() += 1;
                    *textures.entry(e.texture.to_ascii_lowercase()).or_default() += 1;
                }
            }
            Ok(Err(e)) => {
                failed += 1;
                errors.entry(e.to_string()).or_default().push(f);
            }
            Err(_) => {
                panics += 1;
                errors.entry("PANIC".into()).or_default().push(f);
            }
        }
    }
    println!("particle systems ok: {ok}, failed: {failed}, panics: {panics}, emitters: {emitters}");
    println!("blend modes: {blends:?}");
    let mut t: Vec<_> = textures.into_iter().collect();
    t.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!("top textures: {:?}", &t[..t.len().min(15)]);
    for (e, files) in &errors {
        println!(
            "ERROR {e}: {} files, e.g. {:?}",
            files.len(),
            &files[..files.len().min(3)]
        );
    }
}
