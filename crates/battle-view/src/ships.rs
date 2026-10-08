//! Ship entities: spawning, interpolation between simulation steps, hardpoint damage and death.

use std::collections::{HashMap, HashSet};

use alamo_render::{
    AlamoModel, AlamoModelLoaded, AlamoParticles, LodSelect, SpawnParticleEffect,
};
use battle_sim::{Event, HpKind, ShipId, TargetClass};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::convert::{lerp_angle, ship_rotation, sim_to_bevy};
use crate::sim::ShipSnap;
use crate::visuals::{BoneMap, ship_death_effect};
use crate::{BattleEntity, BattleRuntime, BattleViewSettings};

/// Per-ship render state attached to the ship's root entity.
#[derive(Component, Debug)]
pub struct ShipView {
    /// Simulation id.
    pub id: u32,
    /// Definition.
    pub def: ShipId,
    /// Side.
    pub side: u8,
    /// Hardpoint alive flags that have been applied to the model.
    pub hp_applied: Vec<bool>,
    /// Lower-cased bone names of the loaded model.
    bones: Option<BoneMap>,
    /// Parts hanging below each hardpoint's bone.
    hp_parts: Vec<Vec<Entity>>,
    /// Previous heading for banking.
    last_yaw: f32,
    /// Smoothed bank angle.
    bank: f32,
    /// Radius of the visible geometry in world units (0 until the model has loaded).
    visual_radius: f32,
}

/// A destroyed ship that keeps drifting while it burns and blows up.
#[derive(Component, Debug)]
pub struct Dying {
    age: f32,
    duration: f32,
    velocity: Vec3,
    tumble: Vec3,
    next_boom: f32,
    radius: f32,
    effect: &'static str,
    seed: u32,
}

/// Interpolated state of a living ship, refreshed every frame for picking and overlays.
#[derive(Debug, Clone)]
pub struct ShipRender {
    /// Simulation id.
    pub id: u32,
    /// Root entity.
    pub entity: Entity,
    /// Bevy world position.
    pub pos: Vec3,
    /// Sim heading in radians (interpolated).
    pub yaw: f32,
    /// Side.
    pub side: u8,
    /// Definition.
    pub def: ShipId,
    /// Collision radius from the definition.
    pub radius: f32,
    /// Hull fraction 0..1.
    pub hull: f32,
    /// Shield fraction 0..1 (0 for ships without shields).
    pub shield: f32,
    /// Hardpoint alive flags.
    pub hp_alive: Vec<bool>,
}

/// All living ships by simulation id.
#[derive(Resource, Default)]
pub struct RenderedShips(pub HashMap<u32, ShipRender>);

fn lcg(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*seed >> 8) as f32 / (1u32 << 24) as f32
}

/// Ship transform from two snapshots.
fn interpolate(a: &ShipSnap, b: Option<&ShipSnap>, t: f32) -> (Vec3, f32) {
    match b {
        Some(b) => {
            let pa = sim_to_bevy(a.pos);
            let pb = sim_to_bevy(b.pos);
            (pa.lerp(pb, t), lerp_angle(a.yaw, b.yaw, t))
        }
        None => (sim_to_bevy(a.pos), a.yaw),
    }
}

/// Everything the hardpoint / death code needs to touch the model hierarchy.
#[derive(SystemParam)]
pub struct ModelTouch<'w, 's> {
    loaded: Query<'w, 's, &'static AlamoModelLoaded>,
    names: Query<'w, 's, &'static Name>,
    parents: Query<'w, 's, &'static ChildOf>,
    children: Query<'w, 's, &'static Children>,
    particles: Query<'w, 's, &'static mut AlamoParticles>,
    visibility: Query<'w, 's, &'static mut Visibility>,
    globals: Query<'w, 's, &'static GlobalTransform>,
}

impl ModelTouch<'_, '_> {
    /// Builds the bone lookup and the per-hardpoint part lists once the model exists.
    fn prepare(&self, view: &mut ShipView, root: Entity, bones_of_hp: &[Option<String>]) {
        if view.bones.is_some() {
            return;
        }
        let Ok(loaded) = self.loaded.get(root) else {
            return;
        };
        let mut map = BoneMap::new();
        for &b in &loaded.bones {
            if let Ok(n) = self.names.get(b) {
                map.entry(n.as_str().to_ascii_lowercase()).or_insert(b);
            }
        }
        let root_bone = loaded.bones.first().copied();
        view.hp_parts = bones_of_hp
            .iter()
            .map(|bone| {
                let Some(be) = bone
                    .as_ref()
                    .and_then(|n| map.get(&n.to_ascii_lowercase()).copied())
                else {
                    return Vec::new();
                };
                if Some(be) == root_bone {
                    return Vec::new();
                }
                loaded
                    .parts
                    .iter()
                    .copied()
                    .filter(|&p| {
                        let mut cur = p;
                        for _ in 0..8 {
                            if cur == be {
                                return true;
                            }
                            match self.parents.get(cur) {
                                Ok(c) => cur = c.parent(),
                                Err(_) => return false,
                            }
                        }
                        false
                    })
                    .collect()
            })
            .collect();
        view.bones = Some(map);
    }

    fn ready(&self, view: &ShipView) -> bool {
        view.bones.is_some()
    }

    /// Lights the damage fires below `bone`.
    fn enable_fires(&mut self, view: &ShipView, bone: &str) {
        let Some(&be) = view
            .bones
            .as_ref()
            .and_then(|m| m.get(&bone.to_ascii_lowercase()))
        else {
            return;
        };
        let mut stack = vec![be];
        while let Some(e) = stack.pop() {
            if let Ok(mut p) = self.particles.get_mut(e)
                && !p.enabled
            {
                p.enabled = true;
            }
            if let Ok(ch) = self.children.get(e) {
                stack.extend(ch.iter());
            }
        }
    }

    fn bone_world(&self, view: &ShipView, bone: &str) -> Option<Vec3> {
        let e = *view.bones.as_ref()?.get(&bone.to_ascii_lowercase())?;
        self.globals.get(e).ok().map(|g| g.translation())
    }
}

/// Spawns, moves and updates all ship entities for the current snapshot pair.
#[allow(clippy::too_many_arguments)]
pub fn update_ships(
    mut commands: Commands,
    mut rt: ResMut<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    mut rendered: ResMut<RenderedShips>,
    mut touch: ModelTouch,
    mut views: Query<(&mut ShipView, &mut Transform), Without<Dying>>,
    arrivals: Query<&HyperspaceArrival>,
    mut flash_spots: Local<Vec<Vec3>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let (Some(cur), alpha) = (rt.cur.clone(), rt.alpha) else {
        return;
    };
    let next = rt.next.clone();
    let visuals = rt.visuals.clone();
    let catalog = rt.catalog.clone();
    let mut present: HashSet<u32> = HashSet::with_capacity(cur.ships.len());
    flash_spots.clear();
    rendered.0.clear();
    for s in &cur.ships {
        let def = catalog.ship(s.def);
        if def.is_squadron {
            continue;
        }
        let vis = visuals.ship(s.def);
        let Some(model) = vis.model.clone() else {
            continue;
        };
        present.insert(s.id);
        let nxt = next.as_ref().and_then(|n| n.ship(s.id));
        let (pos, yaw) = interpolate(s, nxt, alpha);
        let entity = match rt.entities.get(&s.id) {
            Some(&e) => e,
            None => {
                let mut spec = AlamoModel::new(model).with_team_color(visuals.team_color(s.side));
                if matches!(def.class, TargetClass::Fighter | TargetClass::Bomber) {
                    spec = spec.with_lod(LodSelect::MostDetailed);
                }
                let e = commands
                    .spawn((
                        Name::new(format!("{} #{}", def.name, s.id)),
                        BattleEntity,
                        spec,
                        Transform {
                            translation: pos,
                            rotation: ship_rotation(yaw),
                            scale: Vec3::splat(vis.scale),
                        },
                        ShipView {
                            id: s.id,
                            def: s.def,
                            side: s.side,
                            hp_applied: vec![true; s.hps.len()],
                            bones: None,
                            hp_parts: Vec::new(),
                            last_yaw: yaw,
                            bank: 0.0,
                            visual_radius: 0.0,
                        },
                    ))
                    .id();
                // Ships that show up after the start (reinforcements) drop out of hyperspace.
                let lead = if std::env::var_os("SP_FORCE_ARRIVAL").is_some() { 4.0 } else { 0.0 };
                if cur.time > 2.0 || lead > 0.0 {
                    commands.entity(e).insert(HyperspaceArrival { start: cur.time + lead });
                    let fighter = matches!(def.class, TargetClass::Fighter | TargetClass::Bomber);
                    // One flash per squadron: a fighter arriving next to another fighter that already got one shares it.
                    let shared = fighter && flash_spots.iter().any(|p| p.distance(pos) < 900.0);
                    if fighter && !shared {
                        flash_spots.push(pos);
                    }
                    let radius = if fighter { 220.0 } else { (def.radius * vis.scale.max(1.0)).max(40.0) };
                    if !shared {
                    commands.spawn((
                        Name::new("hyperspace flash"),
                        BattleEntity,
                        Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(24, 16))),
                        MeshMaterial3d(materials.add(StandardMaterial {
                            base_color: Color::srgba(0.75, 0.88, 1.0, 0.0),
                            unlit: true,
                            alpha_mode: AlphaMode::Add,
                            ..default()
                        })),
                        Transform::from_translation(pos).with_scale(Vec3::splat(radius)),
                        Visibility::Hidden,
                        ArrivalFlash { at: cur.time + lead + ARRIVAL_SECONDS * 0.85, radius: radius * 0.45 },
                    ));
                    }
                }
                rt.entities.insert(s.id, e);
                e
            }
        };
        let hull_frac = (s.hull / def.hull.max(1.0)).clamp(0.0, 1.0);
        let shield_frac = if def.shield > 0.0 {
            (s.shield / def.shield).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let hp_alive: Vec<bool> = s.hps.iter().map(|h| h.alive).collect();
        let mut radius = def.radius;
        if let Ok((mut view, mut tf)) = views.get_mut(entity) {
            if view.visual_radius == 0.0
                && let Ok(l) = touch.loaded.get(entity)
            {
                view.visual_radius = l.radius * vis.scale;
            }
            if view.visual_radius > 0.0 {
                radius = view.visual_radius * 0.8;
            }
            // Bank into turns a little, like the real ships do.
            let fast = matches!(def.class, TargetClass::Fighter | TargetClass::Bomber);
            let yaw_rate = {
                let mut d = yaw - view.last_yaw;
                while d > std::f32::consts::PI {
                    d -= std::f32::consts::TAU;
                }
                while d < -std::f32::consts::PI {
                    d += std::f32::consts::TAU;
                }
                d / 0.016
            };
            view.last_yaw = yaw;
            let max_bank = if fast { 0.9 } else { 0.25 };
            let want = (yaw_rate * if fast { 0.5 } else { 1.2 }).clamp(-max_bank, max_bank);
            view.bank += (want - view.bank) * 0.08;
            tf.translation = pos;
            tf.rotation = ship_rotation(yaw) * Quat::from_rotation_z(view.bank);
            tf.scale = Vec3::splat(vis.scale);
            if let Ok(a) = arrivals.get(entity) {
                if !apply_arrival(a, cur.time, radius, &mut tf) {
                    commands.entity(entity).remove::<HyperspaceArrival>();
                }
            }

            // Hardpoint damage.
            if !s.hps.is_empty() {
                let bones: Vec<Option<String>> =
                    vis.hardpoints.iter().map(|h| h.bone.clone()).collect();
                touch.prepare(&mut view, entity, &bones);
                if touch.ready(&view) {
                    for (i, h) in s.hps.iter().enumerate() {
                        if i < view.hp_applied.len() && view.hp_applied[i] && !h.alive {
                            view.hp_applied[i] = false;
                            apply_hardpoint_death(&mut commands, &mut touch, &view, i, &vis.hardpoints[i], def.hardpoints[i].kind);
                        }
                    }
                }
            }
        }
        rendered.0.insert(
            s.id,
            ShipRender {
                id: s.id,
                entity,
                pos,
                yaw,
                side: s.side,
                def: s.def,
                radius,
                hull: hull_frac,
                shield: shield_frac,
                hp_alive,
            },
        );
    }

    // Ships that left the snapshot: dead ones get their explosion sequence from the events,
    // retreated ones just vanish.
    let gone: Vec<u32> = rt
        .entities
        .keys()
        .copied()
        .filter(|id| !present.contains(id))
        .collect();
    for id in gone {
        let Some(e) = rt.entities.remove(&id) else {
            continue;
        };
        if !rt.dying.contains(&id) {
            commands.entity(e).despawn();
        }
    }
    let _ = &settings;
}

fn apply_hardpoint_death(
    commands: &mut Commands,
    touch: &mut ModelTouch,
    view: &ShipView,
    index: usize,
    hv: &crate::visuals::HpVisual,
    kind: HpKind,
) {
    // Hide the destroyed turret / dish meshes.
    if let Some(parts) = view.hp_parts.get(index) {
        for &p in parts {
            if let Ok(mut v) = touch.visibility.get_mut(p) {
                *v = Visibility::Hidden;
            }
        }
    }
    if let Some(bone) = &hv.damage_bone {
        touch.enable_fires(view, bone);
    }
    if let Some(effect) = &hv.death_effect {
        let at = hv
            .bone
            .as_deref()
            .and_then(|b| touch.bone_world(view, b))
            .unwrap_or(Vec3::ZERO);
        if at != Vec3::ZERO {
            commands.trigger(SpawnParticleEffect {
                particle: effect.clone(),
                position: at,
                normal: Vec3::Y,
            });
        }
    }
    let _ = kind;
}

/// Reacts to events of newly reached steps: ship deaths and hardpoint explosions.
pub fn ship_events(
    mut commands: Commands,
    defs: Option<Res<alamo_render::XmlDefs>>,
    mut rt: ResMut<BattleRuntime>,
    mut touch: ModelTouch,
    mut views: Query<&mut ShipView>,
) {
    let events = rt.fresh_events.clone();
    let catalog = rt.catalog.clone();
    let visuals = rt.visuals.clone();
    for ev in &events {
        match ev {
            Event::ShipDestroyed { ship, pos } => {
                let Some(&e) = rt.entities.get(ship) else {
                    continue;
                };
                let Ok(mut view) = views.get_mut(e) else {
                    continue;
                };
                let def = catalog.ship(view.def);
                let class = def.class;
                let seed = *ship ^ 0x9E37_79B9;
                let mut s = seed;
                let fighter = matches!(class, TargetClass::Fighter | TargetClass::Bomber);
                let effect: &'static str = ship_death_effect(class);
                if fighter {
                    commands.trigger(SpawnParticleEffect {
                        particle: effect.into(),
                        position: sim_to_bevy(*pos),
                        normal: Vec3::Y,
                    });
                    commands.entity(e).despawn();
                    rt.entities.remove(ship);
                    continue;
                }
                // Ships with a `Death_Clone` in the data (the hull split into pieces with a `DIE` animation)
                // play the original break-up: the clone replaces the ship, with the data's death explosions.
                let has_clone = defs.as_ref().is_some_and(|d| {
                    d.unit(&def.name)
                        .and_then(|u| u.death_clone.as_deref())
                        .and_then(|c| d.unit(c))
                        .is_some_and(|c| c.model.is_some())
                });
                if has_clone {
                    commands.trigger(alamo_render::PlayDeath { entity: e, unit: Some(def.name.clone()) });
                    commands.entity(e).insert(RemoveAfter(0.2));
                    rt.entities.remove(ship);
                    continue;
                }
                // Capital ship: burn for a few seconds with fires on every hardpoint.
                let n_hp = view.hp_applied.len();
                for i in 0..n_hp {
                    if let Some(hv) = visuals.ship(view.def).hardpoints.get(i)
                        && let Some(b) = &hv.damage_bone
                    {
                        touch.enable_fires(&view, b);
                    }
                }
                view.hp_applied.iter_mut().for_each(|a| *a = false);
                let big = matches!(class, TargetClass::Capital | TargetClass::Super);
                let duration = match class {
                    TargetClass::Super => 9.0,
                    TargetClass::Capital => 6.0,
                    TargetClass::Frigate => 3.5,
                    _ => 2.0,
                };
                let dir = Vec3::new(lcg(&mut s) - 0.5, lcg(&mut s) - 0.5, lcg(&mut s) - 0.5);
                commands.entity(e).insert(Dying {
                    age: 0.0,
                    duration,
                    velocity: Vec3::ZERO,
                    tumble: dir.normalize_or_zero() * if big { 0.06 } else { 0.25 },
                    next_boom: 0.0,
                    radius: def.radius,
                    effect,
                    seed,
                });
                rt.dying.insert(*ship);
            }
            Event::HardpointDestroyed { .. } => {}
            _ => {}
        }
    }
}

/// Drives the explosion sequence of destroyed capital ships.
pub fn update_dying(
    mut commands: Commands,
    time: Res<Time>,
    mut rt: ResMut<BattleRuntime>,
    mut q: Query<(Entity, &ShipView, &mut Dying, &mut Transform)>,
) {
    let dt = time.delta_secs().min(0.1) * rt.speed_factor();
    for (e, view, mut d, mut tf) in &mut q {
        d.age += dt;
        let v = d.velocity;
        tf.translation += v * dt;
        let tumble = d.tumble * dt;
        tf.rotation = Quat::from_euler(EulerRot::XYZ, tumble.x, tumble.y, tumble.z) * tf.rotation;
        d.next_boom -= dt;
        let progress = d.age / d.duration;
        if d.next_boom <= 0.0 && d.age < d.duration {
            d.next_boom = 0.12 + 0.5 * (1.0 - progress);
            let mut s = d.seed;
            d.seed = d.seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
            s ^= d.seed;
            let offset = Vec3::new(lcg(&mut s) - 0.5, (lcg(&mut s) - 0.5) * 0.4, lcg(&mut s) - 0.5)
                * d.radius
                * 1.4;
            let local = tf.rotation * offset;
            commands.trigger(SpawnParticleEffect {
                particle: if progress < 0.7 {
                    "Large_Explosion_Space".into()
                } else {
                    d.effect.into()
                },
                position: tf.translation + local,
                normal: Vec3::Y,
            });
        }
        if d.age >= d.duration {
            commands.trigger(SpawnParticleEffect {
                particle: d.effect.into(),
                position: tf.translation,
                normal: Vec3::Y,
            });
            commands.entity(e).despawn();
            rt.dying.remove(&view.id);
        }
    }
}

/// The ship definitions' death effect when the XML names none.
pub fn default_effect(class: TargetClass) -> &'static str {
    ship_death_effect(class)
}





/// Seconds a reinforcement takes to drop out of hyperspace (stretched in from far away).
const ARRIVAL_SECONDS: f32 = 1.5;

/// A ship that just arrived from hyperspace: it streaks in along its heading, stretched, and
/// settles at its sim position. EaW has no data-driven effect for this (only the enter/exit sounds
/// in `FACTIONS.XML`), so the look is built here.
#[derive(Component)]
pub struct HyperspaceArrival {
    /// Battle time the ship appeared.
    start: f32,
}

/// Flash where a reinforcement drops out of hyperspace: an additive sphere that blooms and fades.
#[derive(Component)]
pub struct ArrivalFlash {
    /// Battle time the flash starts.
    at: f32,
    radius: f32,
}

/// Applies the streak to ships that are still arriving.
fn apply_arrival(arrival: &HyperspaceArrival, now: f32, radius: f32, tf: &mut Transform) -> bool {
    let p = ((now - arrival.start) / ARRIVAL_SECONDS).clamp(0.0, 1.0);
    if p >= 1.0 {
        return false;
    }
    let fwd = tf.rotation * Vec3::Z;
    let left = (1.0 - p).powi(3);
    tf.translation -= fwd * (radius * 40.0 * left);
    // Stretch along the heading (the model's local Z axis) while far away.
    tf.scale *= Vec3::new(1.0, 1.0, 1.0 + 7.0 * left);
    true
}

/// Grows, fades and removes arrival flashes.
pub fn update_arrival_flashes(
    mut commands: Commands,
    rt: Res<BattleRuntime>,
    mut flashes: Query<(Entity, &ArrivalFlash, &mut Transform, &MeshMaterial3d<StandardMaterial>, &mut Visibility)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(now) = rt.cur.as_ref().map(|c| c.time) else { return };
    for (e, f, mut tf, mat, mut vis) in &mut flashes {
        if now < f.at {
            continue;
        }
        let t = (now - f.at) / 0.6;
        if t >= 1.0 {
            commands.entity(e).despawn();
            continue;
        }
        *vis = Visibility::Inherited;
        tf.scale = Vec3::splat(f.radius * (0.4 + 1.6 * t.sqrt()));
        if let Some(mut m) = materials.get_mut(&mat.0) {
            m.base_color = Color::srgba(0.75, 0.88, 1.0, (1.0 - t) * (1.0 - t) * 0.55);
        }
    }
}

/// Despawns an entity after a short delay (the root of a ship that was replaced by its death clone).
#[derive(Component)]
pub struct RemoveAfter(f32);

/// Counts down [`RemoveAfter`] and despawns.
pub fn remove_after(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut RemoveAfter)>) {
    for (e, mut r) in &mut q {
        r.0 -= time.delta_secs();
        if r.0 <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}
