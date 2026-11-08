//! Clone Wars submod (workshop 2794270450 on top of 3229239424, stacked as the `fft` alias): Republic and CIS are
//! playable galactic conquest factions and, played by the AI, build ships and expand. Skipped when a folder is missing.

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn load() -> Option<GameData> {
    let install = GameInstall::detect()?;
    for id in ["3229239424", "2794270450", "1770851727"] {
        if !install.mod_data(id).is_dir() {
            eprintln!("skipping: workshop mod {id} not found");
            return None;
        }
    }
    let fs = GameFs::for_mod(&install, Some("fft")).ok()?;
    Some(load_galaxy(&fs).0)
}

fn campaigns_of(data: &GameData, faction: &str) -> Vec<String> {
    let suffix = format!("_{faction}").to_ascii_lowercase();
    data.campaigns
        .iter()
        .filter(|c| c.has_galaxy() && c.name.to_ascii_lowercase().ends_with(&suffix))
        .map(|c| c.name.clone())
        .collect()
}

#[test]
fn republic_and_cis_campaigns_are_listed() {
    let Some(data) = load() else { return };
    for f in ["Republic", "CIS"] {
        let list = campaigns_of(&data, f);
        println!("{f}: {} campaigns, e.g. {:?}", list.len(), list.iter().take(4).collect::<Vec<_>>());
        assert!(list.len() >= 4, "{f} campaigns: {list:?}");
    }
}

#[test]
fn republic_and_cis_build_and_expand_in_twenty_weeks() {
    let Some(data) = load() else { return };
    for human in ["Republic", "CIS"] {
        let name = format!("GC_Full_Web_Way_{human}");
        let campaign = data.campaign(&name).expect("campaign");
        let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, auto_resolve_battles: true, ..SimConfig::default() })
            .expect("sim");
        let w = sim.world().clone();
        let me = w.faction_id(human).unwrap_or_else(|| panic!("{human} is not a faction of {name}"));
        assert_eq!(w.victory.active_player, Some(me), "{name}: human faction");
        // Republic and CIS both play: AI on every faction, the player's included.
        sim.set_ai_factions(0..w.factions.len() as FactionId);
        let other = w.faction_id(if human == "Republic" { "CIS" } else { "Republic" }).expect("other side");
        assert!(!w.buildable_units[me as usize].is_empty(), "{human} has no buildable units");
        let ships = |sim: &Sim, f: FactionId| -> u32 { sim.galaxy().fleets.values().filter(|x| x.owner == f).map(|x| x.unit_count()).sum() };
        let planets = |sim: &Sim, f: FactionId| sim.galaxy().owned_planets(f).count();
        let before: Vec<(usize, u32, u32)> = [me, other]
            .iter()
            .map(|&f| (planets(&sim, f), ships(&sim, f), sim.galaxy().factions[f as usize].stats.ships_built))
            .collect();
        let weeks = 20.0 * w.config.week_seconds;
        let mut t = 0.0;
        while t < weeks {
            sim.step(w.config.tick_seconds.max(1.0));
            t += w.config.tick_seconds.max(1.0);
            for b in sim.pending_battles().iter().map(|b| b.id).collect::<Vec<_>>() {
                let _ = sim.resolve_battle(b);
            }
        }
        for (i, &f) in [me, other].iter().enumerate() {
            let fname = &w.factions[f as usize].name;
            let st = &sim.galaxy().factions[f as usize].stats;
            let (p0, s0, b0) = before[i];
            println!(
                "{name} / {fname}: planets {p0} -> {}, ships {s0} -> {}, built {} (was {b0}), buildable {}",
                planets(&sim, f),
                ships(&sim, f),
                st.ships_built,
                w.buildable_units[f as usize].len()
            );
            assert!(st.ships_built > b0, "{name}: {fname} built no ships in 20 weeks");
            assert!(planets(&sim, f) > p0, "{name}: {fname} did not expand ({p0} -> {})", planets(&sim, f));
        }
    }
}

#[test]
fn a_story_lock_blocks_a_unit_the_data_does_not_lock_and_an_unlock_frees_it() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Republic").expect("campaign");
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let f = w.faction_id("Republic").unwrap();
    let home = w.factions[f as usize].home.expect("home");
    let u = w.unit_id("Retaliation").expect("unit");
    assert!(!w.units[u as usize].locked, "the data does not lock it");
    let before = w.check_build_unit(sim.galaxy(), f, home, u);
    assert!(!matches!(&before, Err(CommandError::RequirementsNotMet(m)) if m == "locked"), "{before:?}");
    sim.set_unlocked(f, "Retaliation", false);
    let locked = w.check_build_unit(sim.galaxy(), f, home, u);
    assert!(matches!(&locked, Err(CommandError::RequirementsNotMet(m)) if m == "locked"), "{locked:?}");
    sim.set_unlocked(f, "Retaliation", true);
    let after = w.check_build_unit(sim.galaxy(), f, home, u);
    assert!(!matches!(&after, Err(CommandError::RequirementsNotMet(m)) if m == "locked"), "{after:?}");
}
