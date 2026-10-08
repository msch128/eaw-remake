//! Survey of one emitter property id by emitter-name keyword: `particle_prop <hex id>`.
use std::collections::BTreeMap;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::ParticleSystem;

fn main() {
    let id = u8::from_str_radix(std::env::args().nth(1).expect("id").trim_start_matches("0x"), 16).unwrap();
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    let mut by_val: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_kw: BTreeMap<(String, String), usize> = BTreeMap::new();
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
            let Some(p) = e.props.get(&id) else { continue };
            let v = match p.len() {
                1 => format!("u8 {}", p[0]),
                4 => format!("f32 {:.2}", f32::from_le_bytes([p[0], p[1], p[2], p[3]])),
                n => format!("len {n}"),
            };
            *by_val.entry(v.clone()).or_default() += 1;
            let n = e.name.to_ascii_lowercase();
            for kw in ["smoke", "spark", "debr", "fire", "glow", "trail", "engine", "shock", "dust"] {
                if n.contains(kw) {
                    *by_kw.entry((kw.into(), v.clone())).or_default() += 1;
                }
            }
        }
    }
    let mut v: Vec<_> = by_val.into_iter().collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.1));
    println!("values: {:?}", &v[..v.len().min(25)]);
    let mut k: Vec<_> = by_kw.into_iter().collect();
    k.sort_by_key(|x| std::cmp::Reverse(x.1));
    for ((kw, val), n) in k.iter().take(30) {
        println!("{kw:8} {val:14} {n}");
    }
}
