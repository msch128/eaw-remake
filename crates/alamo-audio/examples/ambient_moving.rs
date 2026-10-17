//! Census of `SFXEvent_Ambient_Moving` (a moving unit's random flyby / chatter sound) in a mod's data.
//!
//! ```text
//! ambient_moving [workshop_id]     (default 1770851727, the Remake)
//! ```
//!
//! Counts the raw XML references (every file under `Xml/`, loaded or not), the objects of the loaded object
//! files that name the tag themselves, and the objects that get a non-empty event through `Variant_Of_Existing_Type`;
//! then lists the space units among them (objects with a `Space_Model_Name`) with their event, delays and whether
//! the event has samples.

use std::collections::BTreeMap;

use alamo_audio::SfxLibrary;
use alamo_data::objects::GameObjects;
use alamo_fs::{GameFs, GameInstall};

fn main() {
    let id = std::env::args().nth(1).unwrap_or_else(|| "1770851727".into());
    let install = GameInstall::detect().expect("game not found");
    let fs = GameFs::for_mod(&install, Some(&id)).expect("fs");
    let refs = alamo_audio::hooks::scan_hooks(&fs);
    let raw = refs.iter().filter(|r| r.tag == "sfxevent_ambient_moving").count();
    let named = refs.iter().filter(|r| r.tag == "sfxevent_ambient_moving" && !r.event.trim().is_empty()).count();
    println!("raw XML references: {raw} SFXEvent_Ambient_Moving tags in all Xml files ({named} name an event)");

    let mut w = Vec::new();
    let objects = GameObjects::load(&fs, &mut w);
    let sfx = SfxLibrary::load(&fs, &mut w);
    let own = objects
        .iter()
        .filter(|o| o.element.children.iter().any(|c| c.name == "sfxevent_ambient_moving" && !c.text_trimmed().is_empty()))
        .count();
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut space = Vec::new();
    for o in objects.iter() {
        let Some(r) = objects.resolve(&o.name) else { continue };
        let Some(event) = r.string("sfxevent_ambient_moving") else { continue };
        *by_kind.entry(o.kind.clone()).or_default() += 1;
        if r.string("space_model_name").is_some() {
            let min = r.num::<f32>("sfxevent_ambient_moving_min_delay_seconds");
            let max = r.num::<f32>("sfxevent_ambient_moving_max_delay_seconds");
            let playable = sfx.get(&event).is_some_and(|e| !e.samples.is_empty() || e.chained.is_some());
            space.push(format!("{:<44} {:<12} {:<34} delay {:?}..{:?} {}", o.name, o.kind, event, min, max, if playable { "samples" } else { "NO SAMPLES" }));
        }
    }
    let resolved: usize = by_kind.values().sum();
    println!("loaded objects naming it themselves: {own}; with a non-empty event after inheritance: {resolved}");
    for (k, n) in &by_kind {
        println!("  {k:<24} {n}");
    }
    println!("space objects (with a Space_Model_Name): {}", space.len());
    for s in &space {
        println!("  {s}");
    }
}
