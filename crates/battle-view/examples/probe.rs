//! Developer probe: lists space maps and dumps model structure.
use alamo_fs::{FileSource, GameFs, GameInstall};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    match args.first().map(String::as_str) {
        Some("ls") => {
            for f in fs.list(&args[1]) {
                if f.to_lowercase().contains(&args[2].to_lowercase()) {
                    println!("{f}");
                }
            }
        }
        Some("bbox") => {
            let data = fs.read(&format!("Art/Models/{}", args[1])).expect("read");
            let m = alamo_model::Model::parse(&data).expect("parse");
            for me in &m.meshes {
                println!("{} lod {} bbox {:?} .. {:?} hidden {} shaders {:?}", me.name, me.lod, me.bbox_min, me.bbox_max, me.hidden, me.sub_meshes.iter().map(|s| s.material.shader.clone()).collect::<Vec<_>>());
            }
        }
        Some("maps") => {
            for f in fs.list("Art/Maps") {
                if f.to_lowercase().contains("space") {
                    println!("{f}");
                }
            }
        }
        Some("model") => {
            let data = fs.read(&format!("Art/Models/{}", args[1])).expect("read");
            let m = alamo_model::Model::parse(&data).expect("parse");
            println!("bones {}", m.bones.len());
            for (i, b) in m.bones.iter().enumerate() {
                println!("bone {i} {} parent {:?} vis {}", b.name, b.parent, b.visible);
            }
            for (i, me) in m.meshes.iter().enumerate() {
                println!("mesh {i} {} lod {} hidden {} bone {} subs {}", me.name, me.lod, me.hidden, m.bone_of_object(i), me.sub_meshes.len());
            }
            for p in &m.proxies {
                println!("proxy {} bone {} hidden {}", p.name, p.bone, p.hidden);
            }
        }
        Some("ship") => {
            let (data, _) = alamo_data::load_galaxy(&fs);
            let u = data.objects.space_unit(&args[1]).expect("unit");
            println!("model {:?} scale {} death_clone {:?} fields {:?}", u.space_model, u.scale_factor, u.death_clone, u.fields.iter().filter(|(k, _)| k.contains("death") || k.contains("explo") || k.contains("particle") || k.contains("scale") || k.contains("shield") || k.contains("model")).collect::<Vec<_>>());
            for h in data.objects.hardpoints_of(&u.hardpoints) {
                println!("  {} type {} bone {:?} fire {:?} hp {:?} destroyable {} coll {:?} fields {:?}", h.name, h.hp_type, h.attachment_bone, h.fire_bone_a, h.health, h.is_destroyable, h.collision_mesh, h.fields.keys().filter(|k| k.contains("model") || k.contains("particle") || k.contains("death") || k.contains("damage")).collect::<Vec<_>>());
            }
        }
        Some("hp") => {
            let (data, _) = alamo_data::load_galaxy(&fs);
            let h = data.objects.hardpoint(&args[1]).expect("hp");
            println!("{:#?}", h.fields);
        }
        Some("particles") => {
            let defs = alamo_render::XmlDefs::load(&fs);
            let mut v: Vec<_> = defs.particles.values().filter(|p| p.name.to_lowercase().contains(&args[1].to_lowercase())).collect();
            v.sort_by(|a, b| a.name.cmp(&b.name));
            for p in v { println!("{} -> {} scale {} life {:?}", p.name, p.model, p.scale, p.lifetime_frames); }
        }
        Some("unit") => {
            let (data, _) = alamo_data::load_galaxy(&fs);
            let r = data.objects.resolve(&args[1]).expect("obj");
            for e in r.elements() { let t = e.text_trimmed(); if t.len() < 200 { println!("{} = {}", e.name, t); } }
        }
        Some("proj") => {
            let (data, _) = alamo_data::load_galaxy(&fs);
            let p = data.objects.projectile(&args[1]).expect("proj");
            println!("{:?}", p);
        }
        _ => {}
    }
}






