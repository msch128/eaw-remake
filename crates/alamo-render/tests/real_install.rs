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
