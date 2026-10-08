//! Loads every model of the game file system and every texture they name, and lists what is missing or cannot be
//! decoded: `asset_scan [workshop_id]`. Missing textures show up as black in the game (with a WARN).
use std::collections::BTreeMap;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::Model;
use alamo_model::alo::ParamValue;
use alamo_render::texture::{image_from_bytes, read_texture_file};

fn main() {
    let id = std::env::args().nth(1).unwrap_or_else(|| "1770851727".into());
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some(&id)).expect("fs");
    let (mut models, mut bad_models) = (0, Vec::new());
    let mut tex: BTreeMap<String, String> = BTreeMap::new();
    for f in fs.list("Art/Models") {
        if !f.to_ascii_lowercase().ends_with(".alo") {
            continue;
        }
        let Ok(data) = fs.read(&f) else { continue };
        match Model::parse(&data) {
            Ok(m) => {
                models += 1;
                for mesh in &m.meshes {
                    for s in &mesh.sub_meshes {
                        for (_, v) in &s.material.params {
                            if let ParamValue::Texture(t) = v
                                && !t.trim().is_empty()
                            {
                                tex.entry(t.trim().to_ascii_lowercase()).or_insert_with(|| f.clone());
                            }
                        }
                    }
                }
            }
            Err(alamo_model::Error::NotAModel(_)) => {}
            Err(e) => bad_models.push(format!("{f}: {e}")),
        }
    }
    println!("{models} models parsed, {} unparsable, {} distinct textures", bad_models.len(), tex.len());
    for b in &bad_models {
        println!("  MODEL  {b}");
    }
    let (mut missing, mut undecodable) = (0, 0);
    for (t, first) in &tex {
        match read_texture_file(&fs, t) {
            None => {
                missing += 1;
                println!("  MISSING      {t}   (first used by {first})");
            }
            Some(d) => {
                if let Err(e) = image_from_bytes(&d) {
                    undecodable += 1;
                    println!("  UNDECODABLE  {t}: {e}   (first used by {first})");
                }
            }
        }
    }
    println!("{missing} missing, {undecodable} undecodable textures");

    // Model files named anywhere in the XML (`*_Model_Name`, props, planets, skydomes) that do not exist.
    let mut named: BTreeMap<String, String> = BTreeMap::new();
    for f in fs.list("XML") {
        if !f.to_ascii_lowercase().ends_with(".xml") {
            continue;
        }
        let Ok(bytes) = fs.read(&f) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        for tok in text.split(|c: char| c == '<' || c == '>' || c == '"' || c == ',') {
            let t = tok.trim();
            if t.to_ascii_lowercase().ends_with(".alo") && !t.contains(' ') && t.len() < 80 {
                named.entry(t.to_ascii_lowercase()).or_insert_with(|| f.clone());
            }
        }
    }
    let mut absent = 0;
    for (m, first) in &named {
        if !fs.exists(&format!("Art/Models/{m}")) {
            absent += 1;
            println!("  NO MODEL     {m}   (named in {first})");
        }
    }
    println!("{} models named in XML, {absent} do not exist", named.len());
}
