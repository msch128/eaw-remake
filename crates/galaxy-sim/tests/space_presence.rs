//! Which structures stand in a space battle: the ones the data gives a space side (`SpaceBehavior`). The hardpoints of a ground building
//! (watchtower, fortress, turret tower: `DUMMY_GROUND_STRUCTURE` with a `LandBehavior` only) are for the land map. Real-data test.

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn load() -> Option<GameData> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    Some(load_galaxy(&fs).0)
}

#[test]
fn only_structures_with_a_space_behaviour_fight_in_space() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let sim = Sim::new(&data, campaign, SimConfig::default()).unwrap();
    let w = sim.world();
    let space_side = |name: &str| data.objects.structure(name).is_some_and(|d| d.fields.get("spacebehavior").is_some_and(|v| !v.trim().is_empty()));
    let (mut armed, mut ground_armed, mut special) = (0, 0, 0);
    for st in &w.structures {
        let def = data.objects.structure(&st.name).expect("the structure of the catalog is in the data");
        if st.combat.is_some() || st.space_reach {
            // Its own space side, or the one of the armed structure that stands in orbit for it (Golan platforms, Uplink Command).
            let form = st.battle_form.map(|f| w.structures[f as usize].name.clone());
            assert!(space_side(&st.name) || form.as_deref().is_some_and(space_side), "{} fights in space without a SpaceBehavior", st.name);
        }
        armed += u32::from(st.combat.is_some());
        special += u32::from(st.special_combat.is_some());
        ground_armed += u32::from(!def.hardpoints.is_empty() && !space_side(&st.name));
    }
    // 198 structures had combat numbers while every hardpoint counted: 137 with a space side and these 61 ground buildings.
    assert_eq!((armed, special, ground_armed), (137, 6, 61));
    for ground in ["Empire_Watchtower", "Hapan_Watchtower", "Empire_Fortress", "Rocket_Tower", "Flak_Tower", "Command_Center_U"] {
        let st = &w.structures[w.structure_id(ground).expect(ground) as usize];
        assert!(st.combat.is_none() && !st.space_reach && st.special_combat.is_none() && !st.orbital, "{ground} is a ground building");
    }
    // The build menus do not change with it: an armed ground building stays on offer (docs/RULES.md, "Production").
    assert!(w.structures[w.structure_id("Empire_Watchtower").unwrap() as usize].galactic);
    for gun in ["Hypervelocity_Gun", "Ground_Ion_Cannon"] {
        let st = &w.structures[w.structure_id(gun).expect(gun) as usize];
        assert!(st.space_reach && st.special_combat.is_some() && st.combat.is_none(), "{gun} reaches ships in orbit and cannot be hit");
    }
    let golan = &w.structures[w.structure_id("E_Golan_3").expect("a Golan platform") as usize];
    assert!(golan.combat.is_some() && golan.orbital);
}
