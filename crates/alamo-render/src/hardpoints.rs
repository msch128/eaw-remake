//! Ship hardpoints: aiming the attached turrets and destroying hardpoints.
//!
//! Hardpoints come from the XML (`<HardPoints>` of a unit, `<HardPoint>` objects with
//! `Model_To_Attach`, `Attachment_Bone`, `Damage_Particles`, `Death_Explosion_Particles`); they are
//! spawned by [`AlamoModel::with_unit`](crate::AlamoModel::with_unit).
//!
//! The XML has no "destroyed" model: a destroyed hardpoint hides its attached model, plays its
//! `Death_Explosion_Particles` object and switches on the fire/smoke proxies on the bone named by
//! `Damage_Particles` (for example `TL_01_Fire`, which carries `P_Ship_Fire_*` proxies).

use bevy::prelude::*;

use crate::animation::TurretAim;
use crate::model::{AlamoHardpoint, AlamoModelLoaded};
use crate::particles::AlamoParticles;
use crate::projectiles::SpawnParticleEffect;
use crate::xmldata::XmlDefs;

/// Aims every living turret hardpoint of a ship at a world position.
#[derive(Event, Debug, Clone)]
pub struct AimTurrets {
    /// Ship model root.
    pub model: Entity,
    /// World-space target.
    pub target: Vec3,
}

/// Destroys a hardpoint of a ship (by XML name, case-insensitive).
#[derive(Event, Debug, Clone)]
pub struct HardpointDestroyed {
    /// Ship model root.
    pub model: Entity,
    /// Hardpoint name, e.g. `HP_Allegiance_TL_01`.
    pub hardpoint: String,
}

/// Observer: [`AimTurrets`].
pub fn on_aim_turrets(
    ev: On<AimTurrets>,
    loaded: Query<&AlamoModelLoaded>,
    hardpoints: Query<&AlamoHardpoint>,
    mut turrets: Query<&mut TurretAim>,
) {
    let Ok(l) = loaded.get(ev.model) else { return };
    for &hp in &l.hardpoints {
        let Ok(h) = hardpoints.get(hp) else { continue };
        if h.destroyed {
            continue;
        }
        if let Some(t) = h.turret
            && let Ok(mut aim) = turrets.get_mut(t)
        {
            aim.active = true;
            aim.target = ev.target;
        }
    }
}

/// Observer: [`HardpointDestroyed`].
pub fn on_hardpoint_destroyed(
    ev: On<HardpointDestroyed>,
    mut commands: Commands,
    defs: Res<XmlDefs>,
    loaded: Query<&AlamoModelLoaded>,
    mut hardpoints: Query<(&mut AlamoHardpoint, &GlobalTransform, &mut Visibility)>,
    mut proxies: Query<(&ChildOf, &mut AlamoParticles)>,
) {
    let Ok(l) = loaded.get(ev.model) else { return };
    for &hp in &l.hardpoints {
        let Ok((mut h, gt, mut vis)) = hardpoints.get_mut(hp) else {
            continue;
        };
        if h.destroyed || !h.name.eq_ignore_ascii_case(&ev.hardpoint) {
            continue;
        }
        h.destroyed = true;
        *vis = Visibility::Hidden;
        let Some(def) = defs.hardpoint(&h.name) else {
            continue;
        };
        let position = gt.translation();
        if let Some(p) = def.death_particles.as_deref() {
            commands.trigger(SpawnParticleEffect {
                particle: p.to_string(),
                position,
                normal: Vec3::Y,
            });
        }
        // Damage fire: either the bone with fire proxies, or a particle object.
        if let Some(damage) = def.damage_particles.as_deref() {
            if let Some(&bone) = l.bone_by_name.get(&damage.to_ascii_lowercase()) {
                for (parent, mut p) in &mut proxies {
                    if parent.parent() == bone {
                        p.enabled = true;
                    }
                }
            } else if defs.particle(damage).is_some() {
                commands.trigger(SpawnParticleEffect {
                    particle: damage.to_string(),
                    position,
                    normal: Vec3::Y,
                });
            }
        }
    }
}

/// Registers the hardpoint observers.
pub fn build(app: &mut App) {
    app.add_observer(on_aim_turrets)
        .add_observer(on_hardpoint_destroyed);
}
