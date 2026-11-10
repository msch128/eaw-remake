//! Which production queue every buildable kind lands in: `audit_queues [mod id] [campaign]`.
//! One line per group of build items that share role, object class, build tabs and behaviour: `count | role | class | tabs | behaviour | examples`.
use std::collections::BTreeMap;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn main() {
    let mut args = std::env::args().skip(1);
    let modid = args.next().unwrap_or_else(|| "1770851727".into());
    let campaign = args.next().unwrap_or_else(|| "GC_Full_Web_Way_Rebel".into());
    let install = GameInstall::detect().expect("game install");
    let fs = GameFs::for_mod(&install, Some(&modid)).expect("game files");
    let (data, _) = load_galaxy(&fs);
    let c = data.campaign(&campaign).expect("campaign");
    let sim = Sim::new(&data, c, SimConfig::default()).expect("sim");
    let w = sim.world();
    let describe = |name: &str| -> String {
        let Some(r) = data.objects.resolve(name) else { return "(no object) | - | - | base -".into() };
        let tabs: Vec<&str> = ["build_tab_space_units", "build_tab_space_station", "build_tab_land_units", "build_tab_special_structures", "build_tab_heroes", "build_tab_outpost", "build_tab_upgrades"]
            .into_iter()
            .filter(|t| r.flag(t).unwrap_or(false))
            .collect();
        let behaviour: Vec<String> = r.list("behavior").into_iter().filter(|b| b.to_ascii_uppercase().starts_with("DUMMY")).collect();
        format!("{} | {} | {} | base {}", r.kind(), tabs.join("+"), behaviour.join("+"), r.get("base_position").unwrap_or("-"))
    };
    let side = |kind: &ProductionKind| if w.on_ground_side(kind) { "ground" } else { "space" };
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut seen = std::collections::BTreeSet::new();
    for fi in 0..w.factions.len() {
        for &u in &w.buildable_units[fi] {
            if !seen.insert(("u", u)) {
                continue;
            }
            let ut = &w.units[u as usize];
            let role = match &ut.role {
                UnitRole::Ship if ut.hero => "hero ship".to_string(),
                UnitRole::Ship if !ut.members.is_empty() => "squadron".to_string(),
                UnitRole::Ship => "ship".to_string(),
                UnitRole::Research(_) => "research".to_string(),
                UnitRole::Upgrade { .. } => "vessel upgrade".to_string(),
                UnitRole::Dummy => "dummy purchase".to_string(),
                UnitRole::Agent => "agent".to_string(),
                UnitRole::StructureSwap { to, downgrade, .. } => format!("structure {} ({})", if *downgrade { "downgrade" } else { "upgrade" }, if w.structures[*to as usize].orbital { "orbital" } else { "ground" }),
            };
            groups.entry(format!("{} | {role} | {}", side(&ProductionKind::Unit(u)), describe(&ut.name))).or_default().push(ut.name.clone());
        }
        for &s in &w.buildable_structures[fi] {
            if !seen.insert(("s", s)) {
                continue;
            }
            let st = &w.structures[s as usize];
            groups.entry(format!("{} | structure ({}) | {}", side(&ProductionKind::Structure(s)), if st.orbital { "orbital" } else { "ground" }, describe(&st.name))).or_default().push(st.name.clone());
        }
        for (i, t) in w.factions[fi].tech_upgrades.iter().enumerate() {
            groups.entry(format!("{} | tech level | {}", side(&ProductionKind::Tech(i)), describe(&t.name))).or_default().push(t.name.clone());
        }
    }
    for (b, sb) in w.starbases.iter().enumerate() {
        if sb.cost > 0.0 {
            groups.entry(format!("{} | star base level | {}", side(&ProductionKind::Starbase(b as u32)), describe(&sb.name))).or_default().push(sb.name.clone());
        }
    }
    println!("count | queue | kind | class | build tabs | dummy behaviour | base position | examples");
    for (k, names) in &groups {
        println!("{} | {k} | {}", names.len(), names.iter().take(3).cloned().collect::<Vec<_>>().join(", "));
    }
}
