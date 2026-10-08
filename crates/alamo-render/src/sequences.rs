//! Scripted ship sequences: hyperspace arrival and death.
//!
//! # Data references
//!
//! * **Death** (`XML/Units/Space/*.xml`): a unit names its replacement with
//!   `<Death_Clone>Damage_Normal, Death_Clone_<Unit></Death_Clone>`. That `SpaceUnit` has
//!   `Space_Model_Name = <Model>_Deathclone.ALO` (a hull split into pieces) and a
//!   `<Model>_Deathclone_DIE_00.ALA` animation that tears the pieces apart. `Death_Explosions`
//!   lists `<Particle>` objects (`Large_Explosion_Space_Empire_pd_medium_3_300`, ...) whose
//!   particle files exist per ship size (`_150`, `_300`, `_750`: the number is the ship length).
//! * **Hyperspace**: the engine draws the jump itself (the XML only has speed factors and sound
//!   events: `Hyperspace_Speed_Factor_*`, `SFXEvent_Exit_Into_Hyperspace`), so the arrival here
//!   is procedural: a fast approach along the ship's forward axis that decelerates, a stretched
//!   hull, a bright streak behind it and a flash when it drops out.

use alamo_fs::FileSource;
use bevy::prelude::*;

use crate::animation::{AlamoAnimation, AnimationIndex, model_stem};
use crate::model::{AlamoFs, AlamoModel, AlamoModelLoaded};
use crate::particles::{ExtraBatchGeometry, MeshBuf, ParticleView};
use crate::projectiles::SpawnParticleEffect;
use crate::xmldata::XmlDefs;

/// Makes a ship arrive from hyperspace: insert on the model root, at its final transform.
#[derive(Component, Debug, Clone)]
pub struct HyperspaceArrival {
    /// Seconds from the first frame to the moment the ship stops stretching.
    pub duration: f32,
    /// How far behind its final position the ship starts, in model radii.
    pub distance_radii: f32,
    /// Hull stretch along the flight direction at the start (1 = none).
    pub stretch: f32,
    /// Streak and flash color.
    pub color: Color,
}

impl Default for HyperspaceArrival {
    fn default() -> Self {
        Self {
            duration: 2.0,
            distance_radii: 40.0,
            stretch: 14.0,
            color: Color::srgb(0.55, 0.75, 1.0),
        }
    }
}

/// Triggered on the entity when the arrival is complete.
#[derive(EntityEvent, Debug)]
pub struct HyperspaceArrived {
    /// The ship.
    pub entity: Entity,
}

#[derive(Component, Debug)]
pub(crate) struct HyperspaceState {
    start: Vec3,
    end: Vec3,
    dir: Vec3,
    scale: Vec3,
    radius: f32,
    elapsed: f32,
    forward_axis: Vec3,
}

const FLASH_SECS: f32 = 0.35;

/// Sets up newly inserted [`HyperspaceArrival`]s once the model size is known.
pub(crate) fn start_hyperspace(
    mut commands: Commands,
    new: Query<
        (
            Entity,
            &HyperspaceArrival,
            &AlamoModel,
            &AlamoModelLoaded,
            &Transform,
        ),
        Without<HyperspaceState>,
    >,
) {
    for (e, h, m, l, t) in &new {
        // Bow is local -Z with `bow_forward`, +Z otherwise.
        let axis = if m.bow_forward { Vec3::NEG_Z } else { Vec3::Z };
        let dir = (t.rotation * axis).normalize();
        let radius = l.radius.max(1.0) * t.scale.x.max(0.001);
        commands.entity(e).insert(HyperspaceState {
            start: t.translation - dir * radius * h.distance_radii,
            end: t.translation,
            dir,
            scale: t.scale,
            radius,
            elapsed: 0.0,
            forward_axis: axis.abs(),
        });
    }
}

/// Block of `p_particle_light` with the large radial glow (top-left 256 px of the 1280 px atlas).
const SOFT_GLOW: [f32; 4] = [0.0, 0.0, 0.2, 0.2];

fn push_quad(buf: &mut MeshBuf, corners: [Vec3; 4], color: [f32; 4]) {
    let base = buf.pos.len() as u32;
    let [u0, v0, u1, v1] = SOFT_GLOW;
    for (c, uv) in corners.iter().zip([[u0, v1], [u1, v1], [u0, v0], [u1, v0]]) {
        buf.pos.push(c.to_array());
        buf.uv.push(uv);
        buf.col.push(color);
    }
    buf.idx
        .extend([base, base + 1, base + 2, base + 2, base + 1, base + 3]);
}

/// Moves, stretches and draws arriving ships.
pub(crate) fn fly_hyperspace(
    mut commands: Commands,
    time: Res<Time>,
    view: Res<ParticleView>,
    mut extra: ResMut<ExtraBatchGeometry>,
    mut ships: Query<(
        Entity,
        &HyperspaceArrival,
        &mut HyperspaceState,
        &mut Transform,
    )>,
) {
    let right = view.rotation * Vec3::X;
    let up = view.rotation * Vec3::Y;
    let dt = time.delta_secs();
    for (e, h, mut s, mut t) in &mut ships {
        s.elapsed += dt;
        let u = (s.elapsed / h.duration).clamp(0.0, 1.0);
        // Fast at first, easing out into the final position.
        let p = 1.0 - (1.0 - u).powi(4);
        let pos = s.start.lerp(s.end, p);
        let stretch = 1.0 + (h.stretch - 1.0) * (1.0 - u).powi(3);
        t.translation = pos;
        t.scale = s.scale * (Vec3::ONE + (Vec3::splat(stretch) - Vec3::ONE) * s.forward_axis);
        let color = h.color.to_linear();
        let col = |gain: f32| [color.red * gain, color.green * gain, color.blue * gain, 1.0];
        let buf = extra
            .buffers
            .entry(("p_particle_light.dds".to_string(), 2))
            .or_default();
        // Streak: an elongated glow behind the ship that shrinks as it slows down.
        if u < 1.0 {
            let len = s.radius * h.distance_radii * 0.12 * (1.0 - u).powi(2) + s.radius * 1.5;
            let side = s
                .dir
                .cross((view.position - pos).normalize_or_zero())
                .normalize_or_zero();
            let w = s.radius * 0.35;
            let tail = pos - s.dir * len;
            push_quad(
                buf,
                [
                    tail - side * w * 0.2,
                    tail + side * w * 0.2,
                    pos - side * w,
                    pos + side * w,
                ],
                col(0.7 * (1.0 - u * 0.5)),
            );
        }
        // Flash when the ship drops out of hyperspace.
        let since = s.elapsed - h.duration;
        if since > -0.15 && since < FLASH_SECS {
            let k = if since < 0.0 {
                1.0 + since / 0.15
            } else {
                1.0 - since / FLASH_SECS
            };
            let size = s.radius * (0.7 + 1.3 * (1.0 - k));
            let c = s.end;
            push_quad(
                buf,
                [
                    c - (right + up) * size,
                    c + (right - up) * size,
                    c + (up - right) * size,
                    c + (right + up) * size,
                ],
                col(2.0 * k * k),
            );
        }
        if since >= FLASH_SECS {
            t.translation = s.end;
            t.scale = s.scale;
            commands
                .entity(e)
                .remove::<(HyperspaceArrival, HyperspaceState)>();
            commands.trigger(HyperspaceArrived { entity: e });
        }
    }
}

/// Plays the death sequence of a ship: swaps in its death clone model (if it has one) with its
/// `DIE` animation, plays chained explosions over the hull and removes the pieces afterwards.
#[derive(Event, Debug, Clone)]
pub struct PlayDeath {
    /// The ship's model root.
    pub entity: Entity,
    /// The unit's XML name (e.g. `Imperial_Star_Destroyer`), used to look up `Death_Clone` and
    /// `Death_Explosions`. Without it the clone is guessed as `<model>_DEATHCLONE.ALO`.
    pub unit: Option<String>,
}

/// Runs the explosions of a dying ship.
#[derive(Component, Debug)]
pub struct DeathSequence {
    elapsed: f32,
    duration: f32,
    next_boom: f32,
    center: Vec3,
    radius: f32,
    explosions: Vec<String>,
    finale: Option<String>,
    rng: u32,
    final_done: bool,
}

fn rand(s: &mut u32) -> f32 {
    *s ^= *s << 13;
    *s ^= *s >> 17;
    *s ^= *s << 5;
    (*s >> 8) as f32 / (1u32 << 24) as f32
}

/// Observer: [`PlayDeath`].
pub fn on_play_death(
    ev: On<PlayDeath>,
    mut commands: Commands,
    fs: Res<AlamoFs>,
    defs: Res<XmlDefs>,
    index: Res<AnimationIndex>,
    roots: Query<(&AlamoModel, &AlamoModelLoaded, &GlobalTransform)>,
) {
    let Ok((model, loaded, gt)) = roots.get(ev.entity) else {
        return;
    };
    let unit = ev.unit.as_deref().and_then(|u| defs.unit(u));
    // Clone model: XML first, then the naming convention.
    let from_xml = unit
        .and_then(|u| u.death_clone.as_deref())
        .and_then(|c| defs.unit(c))
        .and_then(|c| c.model.clone());
    let stem = model_stem(&model.path);
    let guessed = format!("{stem}_Deathclone.ALO");
    let clone_model = from_xml.or_else(|| {
        fs.0.exists(&format!("Art/Models/{guessed}"))
            .then_some(guessed)
    });
    // Ship size selects the explosion variant (`..._150` / `_300` / `_750` = ship length).
    let length = loaded.radius * 2.0;
    let variant = if length < 250.0 {
        150
    } else if length < 500.0 {
        300
    } else {
        750
    };
    let mut explosions: Vec<String> = unit.map(|u| u.death_explosions.clone()).unwrap_or_default();
    explosions.retain(|n| defs.particle(n).is_some());
    let fallback = format!("Large_Explosion_Space_Empire_pd_medium_3_{variant}");
    if explosions.is_empty() && defs.particle(&fallback).is_some() {
        explosions.push(fallback);
    }
    info!("death sequence: unit {:?} length {length:.0} explosions {explosions:?}", ev.unit);
    let finale_names: [&str; 2] = if length > 3000.0 {
        ["Huge_Explosion_Space", "Large_Explosion_Space"]
    } else {
        ["Large_Explosion_Space", "Medium_Explosion_Space"]
    };
    let finale = finale_names
        .iter()
        .find(|n| defs.particle(n).is_some())
        .map(|s| s.to_string());

    let mut seq = DeathSequence {
        elapsed: 0.0,
        duration: 6.0,
        next_boom: 0.2,
        center: loaded.center,
        radius: loaded.radius,
        explosions,
        finale,
        rng: 0x9e37_79b9 ^ (ev.entity.to_bits() as u32),
        final_done: false,
    };
    let transform = gt.compute_transform();
    if let Some(clone) = clone_model {
        let clone_stem = model_stem(&clone);
        let mut anim = AlamoAnimation::once("DIE");
        if let Some(path) = index.find(&clone_stem, "DIE", None) {
            let _ = path;
            seq.duration = 7.0;
        } else {
            anim.playing = false;
        }
        let mut spec = AlamoModel::new(clone)
            .with_team_color(model.team_color)
            .with_lod(model.lod);
        spec.bow_forward = model.bow_forward;
        commands.spawn((spec, transform, anim, seq, Name::new("death clone")));
        commands.entity(ev.entity).insert(Visibility::Hidden);
    } else {
        // No clone: explode in place.
        commands.entity(ev.entity).insert(seq);
    }
}

/// Spawns the chained explosions and cleans up when the sequence ends.
pub fn run_death(
    mut commands: Commands,
    time: Res<Time>,
    mut seqs: Query<(Entity, &mut DeathSequence, &GlobalTransform)>,
) {
    let dt = time.delta_secs();
    for (e, mut s, gt) in &mut seqs {
        s.elapsed += dt;
        while s.elapsed >= s.next_boom && s.elapsed < s.duration && !s.explosions.is_empty() {
            let mut rng = s.rng;
            let dir = Vec3::new(
                rand(&mut rng) - 0.5,
                (rand(&mut rng) - 0.5) * 0.3,
                rand(&mut rng) - 0.5,
            )
            .normalize_or_zero();
            let local = s.center + dir * s.radius * (0.2 + 0.6 * rand(&mut rng));
            let which = (rand(&mut rng) * s.explosions.len() as f32) as usize % s.explosions.len();
            let name = s.explosions[which].clone();
            commands.trigger(SpawnParticleEffect {
                particle: name,
                position: gt.transform_point(local),
                normal: Vec3::Y,
            });
            // Explosions get denser as the hull breaks up.
            let k = (s.elapsed / s.duration).clamp(0.0, 1.0);
            s.next_boom += 0.6 - 0.4 * k + rand(&mut rng) * 0.3;
            s.rng = rng;
        }
        if s.elapsed >= s.duration && !s.final_done {
            s.final_done = true;
            if let Some(f) = s.finale.clone() {
                commands.trigger(SpawnParticleEffect {
                    particle: f,
                    position: gt.transform_point(s.center),
                    normal: Vec3::Y,
                });
            }
        }
        if s.elapsed >= s.duration + 1.5 {
            commands.entity(e).despawn();
        }
    }
}

/// Registers the sequence systems and observers.
pub fn build(app: &mut App) {
    app.add_observer(on_play_death)
        .add_systems(
            PostUpdate,
            (start_hyperspace, run_death)
                .after(crate::model::spawn_models)
                .before(bevy::transform::TransformSystems::Propagate),
        )
        .add_systems(
            PostUpdate,
            fly_hyperspace
                .after(crate::particles::update_particle_view)
                .before(crate::particles::update_particle_batches),
        );
}
