use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

struct Ctx {
    data: GameData,
}

fn load() -> Option<Ctx> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    let data = load_galaxy(&fs).0;
    Some(Ctx { data })
}

fn new_sim(ctx: &Ctx) -> Sim {
    let campaign = ctx.data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    Sim::new(&ctx.data, campaign, SimConfig { auto_resolve_battles: false, victory_enabled: false, ..SimConfig::default() }).expect("sim")
}

fn empty_planet(sim: &Sim) -> PlanetId {
    let (w, g) = (sim.world(), sim.galaxy());
    (0..w.planets.len() as u32)
        .find(|&p| w.real[p as usize] && g.planets[p as usize].owner.is_none() && g.fleets_at(p).next().is_none())
        .expect("empty planet")
}

fn add_fleet_with_units(sim: &mut Sim, owner: FactionId, units: &[(UnitTypeId, u32)], at: PlanetId) -> FleetId {
    let g = sim.galaxy_mut();
    let id = g.fleets.keys().max().copied().unwrap_or(0) + 1;
    g.fleets.insert(id, Fleet {
        id,
        owner,
        units: units.iter().copied().collect(),
        location: FleetLocation::AtPlanet(at),
        previous_planet: at,
        slot: 0,
    });
    id
}

#[test]
fn zaarin_fleet_with_aura_wins_more_often_and_loses_less() {
    let Some(ctx) = load() else { return };
    let sim = new_sim(&ctx);
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").expect("Empire faction");
    let rebel = w.faction_id("Rebel").expect("Rebel faction");

    let zaarin = w.unit_id("Vindicator_Cruiser_Kas").expect("Zaarin unit");
    let plain_vindi = w.unit_id("Vindicator_Cruiser").expect("Plain Vindicator unit");
    let acc = w.unit_id("Acclamator").expect("Acclamator");
    let neb = w.unit_id("Nebulon_B_Frigate").expect("Nebulon-B");
    let target = empty_planet(&sim);

    println!("Zaarin bonuses count: {}", w.units[zaarin as usize].bonuses.len());
    println!("Zaarin bonuses: {:?}", w.units[zaarin as usize].bonuses);

    let mc80 = w.unit_id("MC-80_Star_Cruiser").expect("MC80");

    // Run 50 seeds with plain Vindicator + 3 Acclamator vs Rebel (1 MC-80 + 4 Nebulon-B).
    let mut plain_sim = new_sim(&ctx);
    add_fleet_with_units(&mut plain_sim, rebel, &[(mc80, 1), (neb, 4)], target);
    add_fleet_with_units(&mut plain_sim, empire, &[(plain_vindi, 1), (acc, 3)], target);
    let plain_battle = plain_sim.step(1.0).into_iter().find_map(|e| if let Event::BattlePending(b) = e { Some(b) } else { None }).expect("battle");

    let (mut plain_wins, mut plain_survivors) = (0u32, 0u32);
    for seed in 1..=50 {
        let p = plain_sim.preview_auto_resolve(plain_battle.id, seed).unwrap();
        if p.winner == Some(empire) {
            plain_wins += 1;
        }
        plain_survivors += p.sides.iter().find(|s| s.0 == empire).unwrap().2;
    }

    // Run same 50 seeds with Zaarin (Vindicator_Cruiser_Kas) + 3 Acclamator vs Rebel.
    let mut zaarin_sim = new_sim(&ctx);
    add_fleet_with_units(&mut zaarin_sim, rebel, &[(mc80, 1), (neb, 4)], target);
    add_fleet_with_units(&mut zaarin_sim, empire, &[(zaarin, 1), (acc, 3)], target);
    let zaarin_battle = zaarin_sim.step(1.0).into_iter().find_map(|e| if let Event::BattlePending(b) = e { Some(b) } else { None }).expect("battle");

    let (mut zaarin_wins, mut zaarin_survivors) = (0u32, 0u32);
    for seed in 1..=50 {
        let p = zaarin_sim.preview_auto_resolve(zaarin_battle.id, seed).unwrap();
        if p.winner == Some(empire) {
            zaarin_wins += 1;
        }
        zaarin_survivors += p.sides.iter().find(|s| s.0 == empire).unwrap().2;
    }

    println!("Plain Vindicator: wins {plain_wins}/50, survivors {plain_survivors}");
    println!("Zaarin with aura: wins {zaarin_wins}/50, survivors {zaarin_survivors}");

    assert!(
        zaarin_wins > plain_wins || (zaarin_wins == plain_wins && zaarin_survivors > plain_survivors),
        "Zaarin with aura must win more often or keep more survivors than plain Vindicator: zaarin={zaarin_wins} ({zaarin_survivors} surv) vs plain={plain_wins} ({plain_survivors} surv)"
    );
}

#[test]
fn bonus_limited_to_unit_types_does_not_apply_to_other_types() {
    let Some(ctx) = load() else { return };
    let sim = new_sim(&ctx);
    let w = sim.world();

    // Verify in catalog that a type-restricted bonus applies only to that type.
    let isd_type = w.unit_id("Star_Destroyer_II").expect("ISD-II");
    let isd_name = &w.units[isd_type as usize].name;
    let isd_cat = battle_sim::TargetClass::from_category(w.units[isd_type as usize].category);

    let cr90_type = w.unit_id("CR-90_Corvette").expect("CR90");
    let cr90_name = &w.units[cr90_type as usize].name;
    let cr90_cat = battle_sim::TargetClass::from_category(w.units[cr90_type as usize].category);

    let type_restricted_bonus = battle_sim::BonusDef {
        health: 0.25,
        unit_types: vec!["Star_Destroyer_II".to_string()],
        ..battle_sim::BonusDef::default()
    };

    assert!(type_restricted_bonus.applies(isd_name, isd_cat), "Must apply to Star_Destroyer_II");
    assert!(!type_restricted_bonus.applies(cr90_name, cr90_cat), "Must NOT apply to CR-90");

    let isd_bonus = battle_sim::Bonus::compute(isd_name, isd_cat, [&type_restricted_bonus]);
    let cr90_bonus = battle_sim::Bonus::compute(cr90_name, cr90_cat, [&type_restricted_bonus]);

    assert!(isd_bonus.taken_hull < 1.0, "ISD-II must receive hull damage reduction");
    assert_eq!(cr90_bonus.taken_hull, 1.0, "CR-90 must NOT receive hull damage reduction");

    // Also verify exclusion list.
    let excluded_bonus = battle_sim::BonusDef {
        health: 0.25,
        excluded_types: vec!["CR-90_Corvette".to_string()],
        ..battle_sim::BonusDef::default()
    };
    assert!(!excluded_bonus.applies(cr90_name, cr90_cat), "Excluded unit must not match");
    assert!(excluded_bonus.applies(isd_name, isd_cat), "Non-excluded unit must match");
}
