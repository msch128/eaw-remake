//! Census of the sound tags of unit abilities in a mod's data.
//!
//! ```text
//! ability_sfx [workshop_id]     (default 1770851727, the Remake)
//! ```
//!
//! Goes through every object of the loaded object files with its `Variant_Of_Existing_Type` tags merged in and lists, for each
//! `Unit_Ability` of its `Unit_Abilities_Data` (per ability `Type`) and for each ability object of its `Abilities` list, the tags that
//! name a sound, with the number of space units (objects with a `Space_Model_Name`) and other objects that carry it, the events it names,
//! and whether the game code plays the tag (`alamo_audio::hooks::WIRED_TAGS`). Events of the loop tag are listed with their voice rules.

use std::collections::{BTreeMap, BTreeSet};

use alamo_audio::{SfxLibrary, hooks};
use alamo_data::objects::GameObjects;
use alamo_fs::{GameFs, GameInstall};

/// Who carries a tag and which events it names.
#[derive(Default)]
struct Row {
    space: BTreeSet<String>,
    other: BTreeSet<String>,
    events: BTreeMap<String, usize>,
    /// Space objects per event.
    space_by_event: BTreeMap<String, BTreeSet<String>>,
}

fn is_sound_tag(tag: &str) -> bool {
    tag.contains("sfx") || tag.contains("sound") || tag.contains("speech")
}

fn main() {
    let id = std::env::args().nth(1).unwrap_or_else(|| "1770851727".into());
    let install = GameInstall::detect().expect("game not found");
    let fs = GameFs::for_mod(&install, Some(&id)).expect("fs");
    let mut w = Vec::new();
    let objects = GameObjects::load(&fs, &mut w);
    let sfx = SfxLibrary::load(&fs, &mut w);

    // (where, ability type or object kind, tag) -> row
    let mut rows: BTreeMap<(String, String, String), Row> = BTreeMap::new();
    let mut abilities_by_type: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    let mut repeats: Vec<String> = Vec::new();
    for o in objects.iter() {
        let Some(r) = objects.resolve(&o.name) else { continue };
        let space = r.get("space_model_name").is_some_and(|s| !s.is_empty());
        let mut note = |place: &str, kind: &str, tag: &str, event: &str| {
            let row = rows.entry((place.to_string(), kind.to_string(), tag.to_string())).or_default();
            if space { row.space.insert(o.name.clone()) } else { row.other.insert(o.name.clone()) };
            *row.events.entry(event.trim().to_string()).or_default() += 1;
            if space {
                row.space_by_event.entry(event.trim().to_string()).or_default().insert(o.name.clone());
            }
        };
        if let Some(data) = r.element("unit_abilities_data") {
            let mut seen: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for a in data.children_named("unit_ability") {
                let kind = a.child_text("type").unwrap_or("?").to_ascii_uppercase();
                let by_type = abilities_by_type.entry(kind.clone()).or_default();
                if space { by_type.0.insert(o.name.clone()) } else { by_type.1.insert(o.name.clone()) };
                let mut events = Vec::new();
                for c in a.children.iter().filter(|c| is_sound_tag(&c.name)) {
                    note("Unit_Ability", &kind, &c.raw_name, c.text_trimmed());
                    events.push(format!("{}={}", c.raw_name, c.text_trimmed()));
                }
                seen.entry(kind).or_default().push(events.join(" "));
            }
            // A type that appears twice in one unit: the view must tell its blocks apart.
            for (kind, blocks) in seen.iter().filter(|(_, b)| b.len() > 1) {
                if space {
                    repeats.push(format!("{} has {} {kind} blocks: {}", o.name, blocks.len(), blocks.join(" | ")));
                }
            }
        }
        for list in r.elements_named("abilities") {
            for sub in &list.children {
                for c in sub.children.iter().filter(|c| is_sound_tag(&c.name)) {
                    note("Abilities", &sub.raw_name, &c.raw_name, c.text_trimmed());
                }
            }
        }
    }

    // `ability_sfx <id> <text>`: who carries the rows whose "place/type/tag" contains the text, and the events they name.
    if let Some(filter) = std::env::args().nth(2).map(|f| f.to_ascii_lowercase()) {
        for ((place, kind, tag), row) in &rows {
            if !format!("{place}/{kind}/{tag}").to_ascii_lowercase().contains(&filter) {
                continue;
            }
            println!("{place}/{kind}/{tag}: events {:?}", row.events);
            for (event, who) in &row.space_by_event {
                println!("  space, {:?}: {} objects: {}", event, who.len(), who.iter().take(40).cloned().collect::<Vec<_>>().join(", "));
            }
            println!("  other: {}", row.other.iter().take(8).cloned().collect::<Vec<_>>().join(", "));
        }
        return;
    }

    println!("Unit_Ability types (space units / other objects):");
    for (kind, (space, other)) in &abilities_by_type {
        println!("  {kind:<28} {:>4} / {:>4}", space.len(), other.len());
    }
    println!();
    println!("{:<13} {:<28} {:<46} {:>5} {:>5} {:>4}  game code", "place", "type / kind", "tag", "space", "other", "evts");
    for ((place, kind, tag), row) in &rows {
        let wired = hooks::is_wired(&tag.to_ascii_lowercase());
        println!("{place:<13} {kind:<28} {tag:<46} {:>5} {:>5} {:>4}  {}", row.space.len(), row.other.len(), row.events.len(), if wired { "yes" } else { "NO" });
    }

    println!();
    println!("events of the ability loop tag, with their voice rules:");
    for ((place, kind, tag), row) in &rows {
        if !tag.eq_ignore_ascii_case("SFXEvent_Special_Ability_Loop") {
            continue;
        }
        for (event, n) in &row.events {
            let rules = sfx.get(event).map_or("UNKNOWN EVENT".to_string(), |e| {
                format!(
                    "loop {} 3D {} max_instances {:?} priority {} volume {:?} saturation {:?} fade {:?} samples {}",
                    e.looping, !e.is_2d, e.max_instances, e.priority, e.volume, e.saturation_distance, e.loop_fade, e.samples.len()
                )
            });
            println!("  {place}/{kind}: {event} ({n}): {rules}");
        }
    }

    println!();
    println!("space units with the same ability type twice ({}):", repeats.len());
    for r in repeats.iter().take(12) {
        println!("  {r}");
    }
    println!();
    println!("space units that carry an ability loop tag:");
    for ((place, kind, tag), row) in &rows {
        if tag.eq_ignore_ascii_case("SFXEvent_Special_Ability_Loop") {
            println!("  {place}/{kind}: {}", row.space.iter().cloned().collect::<Vec<_>>().join(", "));
        }
    }
}
