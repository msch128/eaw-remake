//! Prints the chunk tree and a decoded summary of a `.ted` map.
//!
//! ```text
//! ted_dump [--tree] [--mod <workshop_id>] <file.ted | Art/Maps/Name.ted>
//! ```
//! A path that is not an existing file is looked up in the layered game file system (optionally with a
//! Workshop mod on top). Object names are resolved from the game's XML when the game is installed.

use std::collections::BTreeMap;

use alamo_data::GameObjects;
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_map::chunk::{Chunk, Minis, chunks};
use alamo_map::{ObjectClass, SpaceMap};

fn hex(d: &[u8]) -> String {
    let s: Vec<String> = d.iter().take(24).map(|b| format!("{b:02x}")).collect();
    s.join(" ") + if d.len() > 24 { " .." } else { "" }
}

fn tree(c: &Chunk, depth: usize, out: &mut Vec<String>) {
    let pad = "  ".repeat(depth);
    if c.is_container {
        out.push(format!("{pad}[{:#x}] container {}B", c.id, c.data.len()));
        for ch in c.children() {
            match ch {
                Ok(ch) => tree(&ch, depth + 1, out),
                Err(e) => out.push(format!("{pad}  ERR {e}")),
            }
        }
    } else if c.data.len() < 4000
        && let Ok(m) = Minis::parse(c.data)
    {
        let f: Vec<String> = m.0.iter().map(|(i, d)| format!("{i}:{}", hex(d))).collect();
        out.push(format!("{pad}[{:#x}] mini {}", c.id, f.join(" | ")));
    } else {
        out.push(format!(
            "{pad}[{:#x}] raw {}B: {}",
            c.id,
            c.data.len(),
            hex(c.data)
        ));
    }
}

fn print_tree(data: &[u8]) {
    let mut out = Vec::new();
    for c in chunks(data) {
        match c {
            Ok(c) => tree(&c, 0, &mut out),
            Err(e) => out.push(format!("ERR {e}")),
        }
    }
    let mut i = 0;
    while i < out.len() {
        let mut j = i;
        while j + 1 < out.len() && out[j + 1] == out[i] {
            j += 1;
        }
        let line: String = out[i].chars().take(1500).collect();
        if j > i {
            println!("{line}  (x{})", j - i + 1);
        } else {
            println!("{line}");
        }
        i = j + 1;
    }
}

fn v3(v: [f32; 3]) -> String {
    format!("({:.1}, {:.1}, {:.1})", v[0], v[1], v[2])
}

fn summary(map: &SpaceMap) {
    let h = &map.header;
    println!(
        "== {} (key {}) version {:#x} size {:?}",
        h.name, h.text_key, h.version, h.size
    );
    let b = &map.bounds;
    println!(
        "bounds {} .. {}  inner {} .. {}",
        v3(b.min),
        v3(b.max),
        v3(b.inner_min),
        v3(b.inner_max)
    );
    let e = &map.environment;
    println!("environment '{}'", e.name);
    for (i, l) in e.lights.iter().enumerate() {
        println!(
            "  light{i}: color {} yaw {:.3} pitch {:.3} scalar {:.2} dir {}",
            v3(l.color),
            l.yaw,
            l.pitch,
            l.scalar,
            v3(l.direction_to_light())
        );
    }
    println!(
        "  ambient {} sun_specular {}",
        v3(e.ambient),
        v3(e.sun_specular)
    );
    println!("  fog {:?}", e.fog);
    println!(
        "  skydome '{}' rot {:?}, nebula '{}' rot {:?}, clouds '{}'",
        e.skydome, e.skydome_rotation_deg, e.nebula, e.nebula_rotation_deg, e.cloud_texture
    );
    println!(
        "objects: {}  defender slots: {}  unknown chunks: {}",
        map.objects.len(),
        map.defender_slots.len(),
        map.unknown.len()
    );
    let mut by: BTreeMap<String, usize> = BTreeMap::new();
    for o in &map.objects {
        *by.entry(
            o.type_name
                .clone()
                .unwrap_or_else(|| format!("?{:08x}", o.type_hash)),
        )
        .or_default() += 1;
    }
    println!("types: {by:?}");
    for o in map
        .objects
        .iter()
        .filter(|o| o.class() != ObjectClass::Asteroid)
    {
        println!(
            "  #{:<6} {:<36} pos {} rot {} {:?}",
            o.id,
            o.type_name
                .clone()
                .unwrap_or_else(|| format!("?{:08x}", o.type_hash)),
            v3(o.position),
            v3(o.rotation_deg),
            o.class()
        );
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let show_tree = args
        .iter()
        .position(|a| a == "--tree")
        .map(|i| args.remove(i))
        .is_some();
    let mod_id = args.iter().position(|a| a == "--mod").map(|i| {
        args.remove(i);
        args.remove(i)
    });
    let Some(path) = args.first() else {
        eprintln!("usage: ted_dump [--tree] [--mod <workshop_id>] <path>");
        std::process::exit(2);
    };
    let install = GameInstall::detect();
    let fs = install
        .as_ref()
        .and_then(|i| GameFs::for_mod(i, mod_id.as_deref()).ok());
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => match fs.as_ref().map(|f| f.read(path)) {
            Some(Ok(d)) => d,
            _ => {
                eprintln!("cannot read {path}");
                std::process::exit(1);
            }
        },
    };
    if show_tree {
        print_tree(&data);
    }
    match SpaceMap::parse(&data) {
        Ok(mut map) => {
            if let Some(fs) = &fs {
                let mut warnings = Vec::new();
                let objs = GameObjects::load(fs, &mut warnings);
                let n = map.resolve_names(objs.iter().map(|o| o.name.as_str()));
                eprintln!("resolved {n}/{} object names", map.objects.len());
            }
            summary(&map);
        }
        Err(e) => {
            eprintln!("parse error: {e}");
            std::process::exit(1);
        }
    }
}
