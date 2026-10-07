//! Dumps game object statistics and fully resolved objects.
//!
//! Usage: `cargo run -p alamo-data --example object_dump -- [workshop_id] [names...]`
//!
//! Without names: per-kind counts, space data summary and two sample capital ships.
//! A name prints the typed view with hardpoints and projectiles; `raw:NAME` prints every
//! resolved tag instead and `?text` searches object names by substring.

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (workshop, names) = match args.split_first() {
        Some((first, rest)) if first.chars().all(|c| c.is_ascii_digit()) => {
            (Some(first.as_str()), rest)
        }
        _ => (Some("1770851727"), &args[..]),
    };
    let install = GameInstall::detect().expect("game install not found");
    let fs = GameFs::for_mod(&install, workshop).expect("cannot open game files");
    let t = std::time::Instant::now();
    let (data, warnings) = load_galaxy(&fs);
    eprintln!("loaded in {:?}, {} warnings", t.elapsed(), warnings.len());

    if names.is_empty() {
        summary(&data);
        for sample in ["Star_Destroyer", "MC-80_Star_Cruiser"] {
            ship(&data, sample);
        }
        return;
    }
    for name in names {
        if let Some(pat) = name.strip_prefix('?') {
            let pat = pat.to_ascii_lowercase();
            for o in data
                .objects
                .iter()
                .filter(|o| o.name.to_ascii_lowercase().contains(&pat))
            {
                println!("{:<18} {:<48} {}", o.kind, o.name, o.source_file);
            }
        } else if let Some(raw) = name.strip_prefix("raw:") {
            raw_dump(&data, raw);
        } else {
            ship(&data, name);
        }
    }
}

fn summary(data: &GameData) {
    println!("{} objects", data.objects.len());
    for (kind, n) in data.objects.count_by_kind() {
        println!("  {n:>5}  {kind}");
    }
    let units = data.objects.space_units();
    println!("\n{} space units/squadrons; by category:", units.len());
    let mut by_cat = std::collections::BTreeMap::new();
    for u in &units {
        *by_cat.entry(format!("{:?}", u.category)).or_insert(0usize) += 1;
    }
    println!("  {by_cat:?}");
    println!(
        "{} star bases, {} damage types, {} armor types, {} matrix entries",
        data.objects.star_bases().len(),
        data.combat.damage_types.len(),
        data.combat.armor_types.len(),
        data.combat.len()
    );

    for c in data.campaigns.iter().filter(|c| c.has_galaxy()).take(1) {
        let owners = data.campaign_planet_owners(c);
        let mut per_faction = std::collections::BTreeMap::new();
        for f in owners.values() {
            *per_faction.entry(f.as_str()).or_insert(0usize) += 1;
        }
        println!("\nowners in {}: {per_faction:?}", c.name);
        println!("conflicts: {:?}", data.campaign_ownership_conflicts(c));
    }
}

fn ship(data: &GameData, name: &str) {
    let objs = &data.objects;
    let Some(u) = objs.space_unit(name) else {
        println!("{name}: not a space unit (try raw:{name})");
        return;
    };
    println!(
        "\n== {} <{}> {:?} from {}",
        u.name, u.kind, u.category, u.source_file
    );
    println!(
        "  text: {} / {}",
        u.text_id.as_deref().unwrap_or("-"),
        u.text_id
            .as_deref()
            .and_then(|t| data.text(t))
            .unwrap_or("-")
    );
    println!(
        "  affiliation {:?} mask {:?} model {:?}",
        u.affiliation, u.category_mask, u.space_model
    );
    println!(
        "  hull {} shield {} (+{}/s) armor {:?} / {:?}",
        u.hull, u.shield_points, u.shield_refresh_rate, u.armor_type, u.shield_armor_type
    );
    println!(
        "  speed {} turn {} accel {} hyperspace {}",
        u.max_speed, u.max_rate_of_turn, u.acceleration, u.hyperspace_speed
    );
    println!(
        "  cost {} time {} tech {} pop {} starbase>={} needs {:?} planets {:?}",
        u.build_cost,
        u.build_time,
        u.tech_level,
        u.population_value,
        u.required_star_base_level,
        u.required_special_structures,
        u.required_planets
    );
    println!(
        "  upkeep {} abilities {:?}",
        u.upkeep(),
        u.abilities
            .iter()
            .map(|a| a.kind.as_str())
            .collect::<Vec<_>>()
    );
    if !u.squadron_units.is_empty() {
        println!("  squadron of {:?}", u.squadron_units);
    }
    if !u.starting_spawned_units.is_empty() {
        println!("  carries {:?}", u.starting_spawned_units);
    }
    let hps = objs.hardpoints_of(&u.hardpoints);
    println!(
        "  {} hardpoints listed, {} resolved",
        u.hardpoints.len(),
        hps.len()
    );
    let mut seen = std::collections::HashSet::new();
    for hp in &hps {
        let key = (hp.hp_type.clone(), hp.projectile.clone());
        if !seen.insert(key) {
            continue;
        }
        println!(
            "    {:<26} {:<32} hp={:?} range={} cone={}x{} recharge={}-{} pulses={}",
            hp.name,
            hp.hp_type,
            hp.health,
            hp.fire_range,
            hp.fire_cone_width,
            hp.fire_cone_height,
            hp.fire_min_recharge,
            hp.fire_max_recharge,
            hp.fire_pulse_count
        );
        if let Some(p) = hp.projectile.as_deref().and_then(|p| objs.projectile(p)) {
            let mult = u
                .armor_type
                .as_deref()
                .zip(p.damage_type.as_deref())
                .map(|(a, d)| data.combat.multiplier(d, a));
            println!(
                "      -> {} {:?} dmg {} {:?} speed {} shield={} hull={} vs own armor x{:?}",
                p.name,
                p.category,
                p.damage,
                p.damage_type,
                p.max_speed,
                p.does_shield_damage,
                p.does_hitpoint_damage,
                mult
            );
        }
    }
}

fn raw_dump(data: &GameData, name: &str) {
    let Some(r) = data.objects.resolve(name) else {
        println!("{name}: not found");
        return;
    };
    let chain: Vec<_> = r.chain.iter().map(|o| o.name.as_str()).collect();
    println!(
        "== {} <{}> chain: {}",
        r.name(),
        r.kind(),
        chain.join(" -> ")
    );
    for e in r.elements().iter().filter(|e| !e.name.contains("sfx")) {
        println!(
            "  {} = {}",
            e.raw_name,
            e.text_trimmed().chars().take(200).collect::<String>()
        );
        print_children(e, 2);
    }
}

/// Prints sub-elements recursively (for `SubObjectList` tags such as `Abilities`).
fn print_children(e: &alamo_data::Element, depth: usize) {
    for c in e.children.iter().filter(|c| !c.name.contains("sfx")) {
        let attrs: String = c
            .attrs
            .iter()
            .map(|(k, v)| format!(" {k}=\"{v}\""))
            .collect();
        println!(
            "{}<{}{attrs}> {}",
            "  ".repeat(depth + 1),
            c.raw_name,
            c.text_trimmed().chars().take(120).collect::<String>()
        );
        print_children(c, depth + 1);
    }
}
