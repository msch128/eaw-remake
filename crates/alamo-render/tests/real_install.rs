//! Tests against a real Empire at War install; skipped when none is found.
use alamo_fs::{GameFs, GameInstall};
use alamo_render::XmlDefs;

fn game_fs() -> Option<GameFs> {
    let install = GameInstall::detect()?;
    GameFs::new(std::slice::from_ref(&install.eaw_data)).ok()
}

#[test]
fn reads_projectile_and_particle_definitions() {
    let Some(fs) = game_fs() else { return };
    let defs = XmlDefs::load(&fs);
    assert!(!defs.projectiles.is_empty());
    assert!(!defs.particles.is_empty());
    // A laser with a bolt model exists in the base game data.
    assert!(
        defs.projectiles
            .values()
            .any(|p| p.model.is_some() && p.max_speed > 0.0)
    );
    // Impact particles resolve to particle objects.
    let resolved = defs
        .projectiles
        .values()
        .filter_map(|p| p.shield_absorb.as_deref())
        .filter(|n| defs.particle(n).is_some())
        .count();
    assert!(resolved > 0);
}

/// The Remake's game files, if installed.
fn remake_fs() -> Option<GameFs> {
    let install = GameInstall::detect().filter(|i| i.mod_data("1770851727").is_dir())?;
    GameFs::for_mod(&install, Some("1770851727")).ok()
}

#[test]
fn a_star_base_is_a_unit_with_its_hardpoints_and_its_death_clone() {
    let Some(fs) = remake_fs() else { return };
    let t0 = std::time::Instant::now();
    let defs = XmlDefs::load(&fs);
    println!("XmlDefs::load {:.0?}, {} units", t0.elapsed(), defs.units.len());
    // `<StarBase Name="Pirate_Starbase_Lv2">` (`Buildings/Space/GC/Pirates/Starbase.xml`): its model, its hardpoints and the clone that
    // replaces it when it dies (`Death_Clone` `Damage_Normal, Death_Clone_Pirate_Starbase_Lv2`).
    let base = defs.unit("Pirate_Starbase_Lv2").expect("a StarBase is a unit");
    assert_eq!(base.model.as_deref(), Some("Pirate_Starbase.ALO"));
    assert!(!base.hardpoints.is_empty() && base.hardpoints.iter().all(|h| defs.hardpoint(h).is_some()), "{:?}", base.hardpoints);
    let clone = base.death_clone.as_deref().and_then(|c| defs.unit(c)).expect("its death clone is a unit");
    assert_eq!(clone.model.as_deref(), Some("Pirate_Starbase_Deathclone.ALO"));
    // Every object kind with a space model is read: each `Death_Clone` a unit names is a unit itself.
    let mut dangling: Vec<String> =
        defs.units.values().filter(|u| u.model.is_some()).filter_map(|u| u.death_clone.as_deref().filter(|c| defs.unit(c).is_none()).map(|c| format!("{} -> {c}", u.name))).collect();
    dangling.sort();
    assert!(dangling.is_empty(), "death clones that are no units: {dangling:?}");
    let with_clone = defs.units.values().filter(|u| u.model.is_some() && u.death_clone.is_some()).count();
    println!("{with_clone} units with a space model and a death clone");
    assert!(with_clone >= 100, "{with_clone}");
}
