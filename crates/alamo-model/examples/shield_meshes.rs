//! Which `MeshShield` meshes the models hide at rest (shield bubbles that flash on a hit) and which are visible all the time (engine
//! glow cones, sails, waterfalls, hangar force fields): `shield_meshes [name-filter]`.
//!
//! Prints the totals and, per mesh name, the colour, base texture and model of the first mesh with that name. Findings (2026-10-17):
//! 272 shield-shader sub-meshes are hidden at rest, 374 are visible, 208 of those named `Engine_Glow*` in 144 models.
use std::collections::BTreeMap;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::Model;

fn main() {
    let filter = std::env::args().nth(1).map(|f| f.to_ascii_lowercase());
    let install = GameInstall::detect().expect("Empire at War install not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("file system");
    let (mut hidden, mut visible, mut glow, mut glow_models) = (0, 0, 0, 0);
    // (hidden, mesh name, colour, base texture) -> (count, first model)
    let mut kinds: BTreeMap<(bool, String, String, String), (usize, String)> = BTreeMap::new();
    for path in fs.list("Art") {
        if !path.to_ascii_lowercase().ends_with(".alo") {
            continue;
        }
        let Ok(data) = fs.read(&path) else { continue };
        if data.starts_with(&[0, 9, 0, 0]) {
            continue;
        }
        let Ok(model) = Model::parse(&data) else { continue };
        let mut has_glow = false;
        for mesh in &model.meshes {
            for sub in &mesh.sub_meshes {
                if !sub.material.shader.to_ascii_lowercase().contains("shield") {
                    continue;
                }
                let name = mesh.name.to_ascii_lowercase();
                if mesh.hidden {
                    hidden += 1;
                } else {
                    visible += 1;
                    if name.contains("engine") {
                        glow += 1;
                        has_glow = true;
                    }
                }
                if filter.as_ref().is_some_and(|f| !name.contains(f.as_str())) {
                    continue;
                }
                let colour = sub.material.float4("Color").map_or("-".into(), |c| format!("({:.2},{:.2},{:.2})", c[0], c[1], c[2]));
                let base = sub.material.texture("BaseTexture").unwrap_or("-").to_string();
                kinds.entry((mesh.hidden, name, colour, base)).or_insert((0, path.clone())).0 += 1;
            }
        }
        glow_models += usize::from(has_glow);
    }
    println!("shield-shader sub-meshes: {hidden} hidden at rest, {visible} visible ({glow} named engine*, in {glow_models} models)");
    let mut rows: Vec<_> = kinds.into_iter().collect();
    rows.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
    for ((hidden, name, colour, base), (n, model)) in rows.into_iter().take(25) {
        println!("{n:4} {} {name:28} colour {colour:18} base {base:32} e.g. {model}", if hidden { "hidden " } else { "visible" });
    }
}
