//! Reports on models referenced by XML tags: `xml_models <Tag> [workshop_id]`, e.g. `Galactic_Model_Name`.
use std::collections::BTreeSet;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::Model;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tag = args.first().map_or("Galactic_Model_Name", String::as_str);
    let id = args.get(1).map_or("1770851727", String::as_str);
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, (id != "-").then_some(id)).expect("fs");
    let open = format!("<{tag}>").to_ascii_lowercase();
    let close = format!("</{tag}>").to_ascii_lowercase();
    let mut names = BTreeSet::new();
    for f in fs.list("XML") {
        if !f.ends_with(".XML") {
            continue;
        }
        let Ok(bytes) = fs.read(&f) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        let lower = text.to_ascii_lowercase();
        let mut from = 0;
        while let Some(i) = lower[from..].find(&open) {
            let s = from + i + open.len();
            let Some(e) = lower[s..].find(&close) else {
                break;
            };
            let name = text[s..s + e].trim().to_string();
            if !name.is_empty() && !name.starts_with("<!") {
                names.insert(name);
            }
            from = s + e;
        }
    }
    println!("{} distinct {tag} values", names.len());
    for n in names {
        let path = format!("Art/Models/{n}");
        match fs
            .read(&path)
            .map_err(|e| e.to_string())
            .and_then(|d| Model::parse(&d).map_err(|e| e.to_string()))
        {
            Ok(m) => {
                let mut shaders = BTreeSet::new();
                let mut tex = BTreeSet::new();
                let (mut v, mut t) = (0, 0);
                for s in m.meshes.iter().flat_map(|x| &x.sub_meshes) {
                    shaders.insert(s.material.shader.clone());
                    for (_, p) in &s.material.params {
                        if let alamo_model::alo::ParamValue::Texture(x) = p {
                            tex.insert(x.clone());
                        }
                    }
                    v += s.vertices.len();
                    t += s.indices.len() / 3;
                }
                println!(
                    "{n}: bones {} meshes {} verts {v} tris {t} proxies {} shaders {shaders:?} textures {tex:?}",
                    m.bones.len(),
                    m.meshes.len(),
                    m.proxies.len()
                );
            }
            Err(e) => println!("{n}: ERROR {e}"),
        }
    }
}
