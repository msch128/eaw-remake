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
    /// Smoothed angular turn velocity.
    smooth_yaw_rate: f32,
    /// Smoothed bank angle.
    bank: f32,
    /// Radius of the visible geometry in world units (0 until the model has loaded).
    visual_radius: f32,
    /// The model is shown (enemy ships hide in the fog of war).
    shown: bool,
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
    /// Squadron index of a fighter (`u32::MAX` for other ships).
    pub squad: u32,
    /// Setup group index.
    pub group: u32,
    /// The player's side sees this ship (false for enemies in the fog of war).
    pub visible: bool,
    /// Current order (sim space).
    pub order: battle_sim::Order,
    /// Planned path ahead in Bevy space (waypoints, the last one is the destination); empty for fighters and idle ships.
    pub path: Vec<Vec3>,
    /// Hardpoint alive flags.
    pub hp_alive: Vec<bool>,
    /// Hardpoint health fractions 0..1.
    pub hp_health: Vec<f32>,
    /// World position of each hardpoint's bone in the loaded model (`None` until the model exists or when the bone is missing).
    pub hp_pos: Vec<Option<Vec3>>,
    /// Whether the ship has functional hyperdrive engines to retreat.
    pub can_retreat: bool,
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

    /// World position of the first model proxy whose name starts with `prefix`.
    fn proxy_world(&self, root: Entity, prefix: &str) -> Option<Vec3> {
        let loaded = self.loaded.get(root).ok()?;
        let p = loaded.proxies.iter().copied().find(|&p| self.names.get(p).is_ok_and(|n| n.as_str().to_ascii_lowercase().starts_with(prefix)))?;
        self.globals.get(p).ok().map(|g| g.translation())
    }

    /// Switches the particle systems of the model proxies whose name starts with `prefix` on or off.
    fn set_proxy_particles(&mut self, root: Entity, prefix: &str, on: bool) {
        let Ok(loaded) = self.loaded.get(root) else { return };
        let mut n = 0;
        for &p in &loaded.proxies {
            if self.names.get(p).is_ok_and(|n| n.as_str().to_ascii_lowercase().starts_with(prefix))
                && let Ok(mut ps) = self.particles.get_mut(p)
            {
                ps.enabled = on;
                // The bone itself is hidden in the model (`vis 0`; the attack animation shows it), which hides the proxy too.
                if let Ok(parent) = self.parents.get(p).map(|c| c.parent())
                    && let Ok(mut v) = self.visibility.get_mut(parent)
                {
                    *v = if on { Visibility::Visible } else { Visibility::Hidden };
                }
                n += 1;
            }
        }
        debug!("battle: {n} of {} proxies of '{prefix}' switched {on} (super laser)", loaded.proxies.len());
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
    time: Res<Time>,
    mut rt: ResMut<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    mut rendered: ResMut<RenderedShips>,
    mut touch: ModelTouch,
    mut views: Query<(&mut ShipView, &mut Transform), Without<Dying>>,
    arrivals: Query<&HyperspaceArrival>,
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<Camera3d>>,
    mut sfx: MessageWriter<alamo_audio::PlaySfx>,
    mut flash_spots: Local<Vec<Vec3>>,
    // One unit sphere for every arrival and departure flash.
    mut flash_mesh: Local<Option<Handle<Mesh>>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let (Some(cur), alpha) = (rt.cur.clone(), rt.alpha) else {
        return;
    };
    let dt_frame = time.delta_secs().clamp(0.001, 0.1);
    // Camera data for the minimum on-screen size of fighters: (eye, pixels per world unit at distance 1, screen height in pixels).
    let eye = cameras.iter().find(|(c, _, _)| c.is_active).and_then(|(c, gt, p)| match (p, c.physical_target_size()) {
        (Projection::Perspective(p), Some(size)) => Some((gt.translation(), size.y as f32 / (2.0 * (p.fov * 0.5).tan()), size.y as f32)),
        _ => None,
    });
    let sim_dt = rt.dt.max(0.01);
    let next = rt.next.clone();
    let visuals = rt.visuals.clone();
    let catalog = rt.catalog.clone();
    let mut present: HashSet<u32> = HashSet::with_capacity(cur.ships.len());
    flash_spots.clear();
    // Fighters launched from a carrier or station leave a bay: no hyperspace arrival sound for them.
    let launched: HashSet<u32> = rt
        .fresh_events
        .iter()
        .filter_map(|e| if let battle_sim::Event::SquadronLaunched { ships, .. } = e { Some(ships.iter().copied()) } else { None })
        .flatten()
        .collect();
    rendered.0.clear();
    // Debug aid (`SP_DEBUG_KILL_HP=<battle time>`): from then on, the first capital ship of side 0 shows five destroyed hardpoints.
    let debug_kill = std::env::var("SP_DEBUG_KILL_HP").ok().and_then(|v| v.parse::<f32>().ok()).filter(|t| cur.time >= *t);
    let debug_target = debug_kill.and_then(|_| cur.ships.iter().find(|s| s.side == 0 && !s.hps.is_empty() && catalog.ship(s.def).radius > 300.0).map(|s| s.id));
    let mut debug_copy: ShipSnap;
    for s in &cur.ships {
        let def = catalog.ship(s.def);
        if def.is_squadron {
            continue;
        }
        let s = if Some(s.id) == debug_target {
            debug_copy = s.clone();
            debug_copy.hps.iter_mut().take(5).for_each(|h| h.alive = false);
            &debug_copy
        } else {
            s
        };
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
                let e = commands
                    .spawn((
                        Name::new(format!("{} #{}", def.name, s.id)),
                        BattleEntity,
                        ship_model(def, model, visuals.team_color(s.side)),
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
                            smooth_yaw_rate: 0.0,
                            bank: 0.0,
                            visual_radius: 0.0,
                            shown: true,
                        },
                    ))
                    .id();
                // Ships that show up after the start (reinforcements) drop out of hyperspace.
                let lead = if std::env::var_os("SP_FORCE_ARRIVAL").is_some() { 4.0 } else { 0.0 };
                if cur.time > 2.0 || lead > 0.0 {
                    commands.entity(e).insert(HyperspaceArrival { start: cur.time + lead });
                    // `SFXEvent_Arrive_From_Hyperspace` of the factions (the same event for all three).
                    if flash_spots.len() < 3 && !launched.contains(&s.id) {
                        sfx.write(alamo_audio::PlaySfx { event: "Unit_Ship_Hyperspace_Enter".into(), position: Some(pos), source: None });
                    }
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
                        Mesh3d(flash_mesh.get_or_insert_with(|| meshes.add(Sphere::new(1.0).mesh().uv(24, 16))).clone()),
                        MeshMaterial3d(materials.add(flash_material(Color::srgba(0.75, 0.88, 1.0, 0.0)))),
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
        let visible = s.side == settings.player_side || (s.seen_by >> settings.player_side.min(7)) & 1 == 1;
        let hp_alive: Vec<bool> = s.hps.iter().map(|h| h.alive).collect();
        let hp_health: Vec<f32> = s
            .hps
            .iter()
            .enumerate()
            .map(|(i, h)| def.hardpoints.get(i).map_or(1.0, |d| if d.health > 0.0 { (h.health / d.health).clamp(0.0, 1.0) } else { 1.0 }))
            .collect();
        let mut hp_pos: Vec<Option<Vec3>> = vec![None; s.hps.len()];
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
            // Angular velocity from simulation snapshots (noise-free, independent of frame rate)
            let snap_yaw_rate = if let Some(n) = nxt {
                let mut d = n.yaw - s.yaw;
                while d > std::f32::consts::PI {
                    d -= std::f32::consts::TAU;
                }
                while d < -std::f32::consts::PI {
                    d += std::f32::consts::TAU;
                }
                d / sim_dt
            } else {
                0.0
            };
            // Low-pass filter angular velocity over time to avoid snap transitions
            let yaw_filter = 1.0 - (-8.0 * dt_frame).exp();
            if settings.paused || rt.speed == 0.0 {
                view.smooth_yaw_rate = 0.0;
            } else {
                view.smooth_yaw_rate += (snap_yaw_rate - view.smooth_yaw_rate) * yaw_filter;
            }
            view.last_yaw = yaw;

            if view.shown != visible {
                view.shown = visible;
                commands.entity(entity).insert(if visible { Visibility::Inherited } else { Visibility::Hidden });
            }

            // Natural, weighted banking into turns.
            // Capital ships, frigates, structures and super weapons do NOT bank like atmospheric planes.
            // In EaW only fighters and bombers bank into turns (and corvettes very slightly).
            let max_bank = match def.class {
                TargetClass::Super | TargetClass::Structure | TargetClass::Capital | TargetClass::Frigate => 0.0,
                TargetClass::Corvette => 0.04, // ~2 degrees
                TargetClass::Transport | TargetClass::Other => 0.02,
                TargetClass::Fighter | TargetClass::Bomber => {
                    if vis.bank_turn_angle > 0.0 {
                        vis.bank_turn_angle.min(0.55) // max ~31 degrees
                    } else {
                        0.45 // ~26 degrees
                    }
                }
            };
            let turn_fraction = if def.turn_rate > 1e-3 {
                (view.smooth_yaw_rate / def.turn_rate).clamp(-1.0, 1.0)
            } else {
                0.0
            };
            let want = if settings.paused || rt.speed == 0.0 { 0.0 } else { turn_fraction * max_bank };
            let roll_speed = match def.class {
                TargetClass::Super | TargetClass::Structure | TargetClass::Capital | TargetClass::Frigate => 5.0,
                TargetClass::Corvette => 3.2,
                TargetClass::Transport | TargetClass::Other => 2.0,
                TargetClass::Fighter | TargetClass::Bomber => 5.5,
            };
            let roll_step = 1.0 - (-roll_speed * dt_frame).exp();
            view.bank += (want - view.bank) * roll_step;
            tf.translation = pos;
            tf.rotation = ship_rotation(yaw) * Quat::from_rotation_z(view.bank);
            let mut boost = 1.0;
            // Squadron members of any class count (a Skipray blast boat squadron is no fighter class but reads as a unit of small craft).
            if (matches!(def.class, TargetClass::Fighter | TargetClass::Bomber) || s.squad != u32::MAX)
                && let Some((eye, px_per_unit, height)) = eye
                && view.visual_radius > 0.0
            {
                boost = fighter_boost(view.visual_radius, pos.distance(eye), px_per_unit, height);
            }
            tf.scale = Vec3::splat(vis.scale * boost);
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
                    for (i, h) in vis.hardpoints.iter().enumerate().take(hp_pos.len()) {
                        hp_pos[i] = h.bone.as_deref().and_then(|b| touch.bone_world(&view, b));
                    }
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
                squad: s.squad,
                group: s.group,
                visible,
                order: s.order,
                path: if s.squad == u32::MAX { s.path.iter().map(|p| sim_to_bevy(*p)).collect() } else { Vec::new() },
                hp_alive,
                hp_health,
                hp_pos,
                can_retreat: s.can_retreat,
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
            // A ship that leaves without dying retreated: `SFXEvent_Exit_Into_Hyperspace`.
            if let Ok((view, t)) = views.get(e) {
                sfx.write(alamo_audio::PlaySfx { event: "Unit_Ship_Hyperspace_Exit".into(), position: Some(t.translation), source: None });
                let radius = rt.catalog.ship(view.def).radius.max(30.0);
                let fwd = t.rotation * Vec3::Z;
                // Unit sphere: `update_arrival_flashes` scales it to world size (a sphere of `radius` times a scale of `radius` was the giant ball).
                // Additive like the arrival flash: the opaque white ball that stood here covered the screen on a fleet retreat (QA round 3).
                let flash_mat = materials.add(flash_material(Color::srgba(0.8, 0.95, 1.0, 0.0)));
                commands.spawn((
                    Mesh3d(flash_mesh.get_or_insert_with(|| meshes.add(Sphere::new(1.0).mesh().uv(24, 16))).clone()),
                    MeshMaterial3d(flash_mat),
                    Transform::from_translation(t.translation).with_scale(Vec3::splat(1.0)),
                    ArrivalFlash { at: cur.time, radius: radius * 0.6 },
                    // The sim clock freezes when the battle ends (DEFEATED, pause); the flash is timed on it, so it also expires on wall time (the user saw it stay forever).
                    RemoveAfter(FLASH_WALL_SECONDS),
                    Visibility::Inherited,
                    Name::new("hyperspace departure flash"),
                ));
                commands.entity(e).insert(HyperspaceDeparture {
                    start: cur.time,
                    origin: t.translation,
                    heading: fwd,
                    radius,
                });
                commands.entity(e).insert(RemoveAfter(FLASH_WALL_SECONDS));
            } else {
                commands.entity(e).try_despawn();
            }
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
    mut pulses: ResMut<crate::effects::BloomPulses>,
    defs: Option<Res<alamo_render::XmlDefs>>,
    mut rt: ResMut<BattleRuntime>,
    mut touch: ModelTouch,
    mut views: Query<&mut ShipView>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
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
                    commands.entity(e).try_despawn();
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
                let big = matches!(class, TargetClass::Capital | TargetClass::Super | TargetClass::Structure);
                let duration = match class {
                    TargetClass::Structure => 10.0,
                    TargetClass::Super => 9.0,
                    TargetClass::Capital => 6.0,
                    TargetClass::Frigate => 3.5,
                    _ => 2.0,
                };
                if big {
                    pulses.flash(sim_to_bevy(*pos), 0.35);
                }
                let dir = Vec3::new(lcg(&mut s) - 0.5, lcg(&mut s) - 0.5, lcg(&mut s) - 0.5);
                commands.entity(e).insert(Dying {
                    age: 0.0,
                    duration,
                    velocity: Vec3::ZERO,
                    tumble: dir.normalize_or_zero() * if big { 0.03 } else { 0.25 },
                    next_boom: 0.0,
                    radius: def.radius,
                    effect,
                    seed,
                });
                rt.dying.insert(*ship);
            }
            Event::SuperLaserCharging { ship, fire_in, .. } => {
                // The model's `p_planet_deathstar_laser_small` proxies (the converging charge lattice) light up now, the big dish glow `fire_in` seconds later.
                if let Some(&e) = rt.entities.get(ship) {
                    let now = rt.cur.as_ref().map_or(0.0, |c| c.time);
                    debug!("battle: super laser of ship {ship} charging at t={now:.1}s, shot in {fire_in:.2}s");
                    touch.set_proxy_particles(e, SUPER_LASER_SMALL, true);
                    commands.entity(e).insert(SuperLaserFx { large_at: now + fire_in, end_at: now + fire_in + SUPER_LASER_GLOW_AFTER, large_on: false });
                }
            }
            Event::SuperLaserFired { ship, target, at } => {
                debug!("battle: super laser of ship {ship} fired at {target}");
                let (Some(&e), Some(cur)) = (rt.entities.get(ship), rt.cur.as_ref()) else { continue };
                let to = Some(sim_to_bevy(*at));
                let from = touch.proxy_world(e, SUPER_LASER_LARGE).or_else(|| cur.ship(*ship).map(|s| sim_to_bevy(s.pos)));
                let beam_radius = (cur.ship(*ship).map_or(0.0, |s| catalog.ship(s.def).radius) * SUPER_LASER_BEAM_RADIUS_OF_HULL).max(SUPER_LASER_BEAM_RADIUS);
                if let (Some(from), Some(to)) = (from, to) {
                    crate::effects::spawn_beam(&mut commands, &mut meshes, &mut materials, from, to, Color::srgba(0.25, 1.0, 0.4, 0.85), beam_radius, SUPER_LASER_BEAM_SECONDS);
                    pulses.flash(to, 1.0);
                }
            }
            Event::HardpointDestroyed { .. } => {}
            _ => {}
        }
    }
}

/// Drives the explosion sequence of destroyed capital ships, and spawns drifting debris chunks (U68).
pub fn update_dying(
    mut commands: Commands,
    mut pulses: ResMut<crate::effects::BloomPulses>,
    time: Res<Time>,
    mut rt: ResMut<BattleRuntime>,
    mut q: Query<(Entity, &ShipView, &mut Dying, &mut Transform)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let dt = time.delta_secs().min(0.1) * rt.speed_factor();
    for (e, view, mut d, mut tf) in &mut q {
        d.age += dt;
        let v = d.velocity;
        tf.translation += v * dt;
        let tumble = d.tumble * dt;
        tf.rotation = Quat::from_euler(EulerRot::XYZ, tumble.x, tumble.y, tumble.z) * tf.rotation;
        d.next_boom -= dt;
        if d.next_boom <= 0.0 && d.age < d.duration {
            // Ships without a `Death_Clone` have no keyed sequence in the data: one `Death_Explosions`
            // effect (which already runs ~30 s with its own loops) every ~1.5 s while the hull burns
            // (estimate); stacking a new one every 0.12-0.6 s made clouds 1.5x the hull.
            d.next_boom = 1.5;
            let mut s = d.seed;
            d.seed = d.seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
            s ^= d.seed;
            let offset = Vec3::new(lcg(&mut s) - 0.5, (lcg(&mut s) - 0.5) * 0.4, lcg(&mut s) - 0.5)
                * d.radius
                * 1.4;
            let local = tf.rotation * offset;
            commands.trigger(SpawnParticleEffect {
                particle: d.effect.into(),
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
            if d.radius >= 40.0 {
                pulses.flash(tf.translation, if d.radius >= 150.0 { 1.0 } else { 0.6 });
                info!("battle: final explosion of a radius {:.0} ship at t={:.1}s, wreck stays {WRECK_SECONDS:.0}s", d.radius, rt.cur.as_ref().map_or(0.0, |c| c.time));
            }
            // U68: Break capital ships and frigates into drifting debris chunks
            if d.radius >= 40.0 {
                let num_chunks = if d.radius > 200.0 { 4 } else { 2 };
                let chunk_size = d.radius * if num_chunks == 4 { 0.35 } else { 0.45 };
                let offsets = [
                    Vec3::new(chunk_size * 0.8, chunk_size * 0.2, 0.0),
                    Vec3::new(-chunk_size * 0.8, -chunk_size * 0.2, 0.0),
                    Vec3::new(0.0, chunk_size * 0.3, chunk_size * 0.6),
                    Vec3::new(0.0, -chunk_size * 0.3, -chunk_size * 0.6),
                ];
                let drift_dirs = [
                    Vec3::new(0.8, 0.2, 0.3).normalize(),
                    Vec3::new(-0.7, -0.3, -0.4).normalize(),
                    Vec3::new(0.1, 0.9, -0.5).normalize(),
                    Vec3::new(-0.2, -0.8, 0.6).normalize(),
                ];
                for i in 0..num_chunks {
                    // Irregular dark hull chunk (a squashed low-poly sphere), not a glowing box.
                    let mesh = meshes.add(Sphere::new(chunk_size * 0.6).mesh().ico(1).unwrap());
                    let mat = materials.add(StandardMaterial {
                        base_color: Color::srgb(0.1, 0.1, 0.12),
                        emissive: LinearRgba::new(CHUNK_EMBER.0, CHUNK_EMBER.1, CHUNK_EMBER.2, 1.0),
                        perceptual_roughness: 0.8,
                        ..default()
                    });
                    let world_offset = tf.rotation * offsets[i];
                    let drift = tf.rotation * drift_dirs[i] * (15.0 + (i as f32) * 5.0);
                    let ang_vel = Vec3::new(
                        (0.03 + (i as f32) * 0.01) * if i % 2 == 0 { 1.0 } else { -1.0 },
                        (0.02 + (i as f32) * 0.015) * if i / 2 == 0 { 1.0 } else { -1.0 },
                        0.025,
                    );
                    commands.spawn((
                        Name::new("ship_debris_chunk"),
                        BattleEntity,
                        Mesh3d(mesh),
                        MeshMaterial3d(mat),
                        Transform::from_translation(tf.translation + world_offset)
                            .with_rotation(tf.rotation)
                            .with_scale(Vec3::new(1.5, 0.7, 1.0 + 0.2 * i as f32)),
                        DebrisChunk {
                            timer: 30.0,
                            max_time: 30.0,
                            velocity: d.velocity + drift,
                            angular_velocity: ang_vel,
                        },
                    ));
                }
            }
            if d.radius >= 40.0 {
                // The hull stays behind as a burning wreck (its own damage fires keep playing), drifting and tumbling slowly.
                commands.entity(e).remove::<Dying>().insert((Wreck { velocity: d.velocity * 0.5, tumble: d.tumble }, RemoveAfter(WRECK_SECONDS)));
            } else {
                commands.entity(e).try_despawn();
            }
            rt.dying.remove(&view.id);
        }
    }
}

/// Proxy (bone) names of the Death Star model: the six converging charge lasers and the dish glow (`Owner_Particle_Bone_Name` of the ability).
const SUPER_LASER_SMALL: &str = "p_planet_deathstar_laser_small";
const SUPER_LASER_LARGE: &str = "p_planet_deathstar_laser_large";
/// Seconds the dish glow and the lattice stay on after the shot (estimate, `Fire_Continuation` is 0.25 s in the data).
const SUPER_LASER_GLOW_AFTER: f32 = 1.5;
/// Look of the beam line from the dish to the target (estimates; the data has no beam art, only the particles).
/// The beam is as wide as this share of the shooter's radius (at least `SUPER_LASER_BEAM_RADIUS`), so the Death Star's reads at battle camera distances.
const SUPER_LASER_BEAM_RADIUS_OF_HULL: f32 = 0.012;
const SUPER_LASER_BEAM_RADIUS: f32 = 60.0;
const SUPER_LASER_BEAM_SECONDS: f32 = 0.8;

/// Charge and glow timeline of a Death Star shot, on the ship entity.
#[derive(Component, Debug)]
pub struct SuperLaserFx {
    large_at: f32,
    end_at: f32,
    large_on: bool,
}

/// Lights the dish glow at the shot time and switches the charge and glow particles off afterwards.
pub fn update_super_laser_fx(mut commands: Commands, rt: Res<BattleRuntime>, mut touch: ModelTouch, mut q: Query<(Entity, &mut SuperLaserFx)>) {
    let now = rt.cur.as_ref().map_or(0.0, |c| c.time);
    for (e, mut fx) in &mut q {
        if now >= fx.end_at {
            touch.set_proxy_particles(e, SUPER_LASER_SMALL, false);
            touch.set_proxy_particles(e, SUPER_LASER_LARGE, false);
            commands.entity(e).remove::<SuperLaserFx>();
        } else if now >= fx.large_at && !fx.large_on {
            fx.large_on = true;
            touch.set_proxy_particles(e, SUPER_LASER_LARGE, true);
        }
    }
}

/// Fighters, bombers and squadron members keep at least this share of the screen height as their size. Estimate: the Remake frames
/// n93 f0011/f0013 show a fighter at about 2.6% (15 px of 576). 3.6% (kept larger than the 24 design px icon box) read far too big
/// next to a Star Destroyer in play, so the Remake measure is used. The data gives a TIE 4 units next to a 760 unit Star
/// Destroyer: the original draws small craft far above true scale.
const FIGHTER_MIN_SCREEN_FRACTION: f32 = 0.026;
/// Largest enlargement of a fighter model (estimate; caps the effect when the camera is far out).
const FIGHTER_MAX_BOOST: f32 = 40.0;

/// Scale factor that lifts a fighter of bounding radius `radius` at `dist` from the eye to the minimum screen size (1 when already larger).
fn fighter_boost(radius: f32, dist: f32, px_per_unit: f32, screen_height: f32) -> f32 {
    let px = 2.0 * radius * px_per_unit / dist.max(1.0);
    (FIGHTER_MIN_SCREEN_FRACTION * screen_height / px.max(1e-3)).clamp(1.0, FIGHTER_MAX_BOOST)
}

/// Seconds a burning wreck stays on the battlefield.
const WRECK_SECONDS: f32 = 45.0;

/// The hull of a destroyed capital ship, burning on after the final explosion.
#[derive(Component, Debug)]
pub struct Wreck {
    velocity: Vec3,
    tumble: Vec3,
}

/// Lets wrecks drift and tumble.
pub fn update_wrecks(time: Res<Time>, rt: Res<BattleRuntime>, mut q: Query<(&Wreck, &mut Transform)>) {
    let dt = time.delta_secs().min(0.1) * rt.speed_factor();
    for (w, mut tf) in &mut q {
        tf.translation += w.velocity * dt;
        let t = w.tumble * dt;
        tf.rotation = Quat::from_euler(EulerRot::XYZ, t.x, t.y, t.z) * tf.rotation;
    }
}

/// Glow of a fresh debris chunk (linear RGB, a faint ember; it cools to black over the chunk lifetime). Look estimate.
const CHUNK_EMBER: (f32, f32, f32) = (0.18, 0.05, 0.01);

/// A drifting debris chunk from a destroyed ship hull (U68).
#[derive(Component, Debug)]
pub struct DebrisChunk {
    pub timer: f32,
    pub max_time: f32,
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
}

/// Updates debris chunks: drifting, tumbling, and cooling emissive glow over 30s.
pub fn update_debris_chunks(
    mut commands: Commands,
    time: Res<Time>,
    rt: Res<BattleRuntime>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(Entity, &mut DebrisChunk, &mut Transform, &MeshMaterial3d<StandardMaterial>)>,
) {
    let dt = time.delta_secs().min(0.1) * rt.speed_factor();
    for (entity, mut chunk, mut tf, mat_handle) in &mut q {
        chunk.timer -= dt;
        if chunk.timer <= 0.0 {
            commands.entity(entity).try_despawn();
        } else {
            tf.translation += chunk.velocity * dt;
            let ang = chunk.angular_velocity * dt;
            tf.rotation = Quat::from_euler(EulerRot::XYZ, ang.x, ang.y, ang.z) * tf.rotation;
            if let Some(mut mat) = materials.get_mut(&mat_handle.0) {
                let cool = (chunk.timer / chunk.max_time).clamp(0.0, 1.0);
                mat.emissive = LinearRgba::new(CHUNK_EMBER.0 * cool, CHUNK_EMBER.1 * cool, CHUNK_EMBER.2 * cool, 1.0);
            }
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

/// The model spec of a battle ship: turrets and art pieces of the unit's hardpoints hang on the hull's bones (see `turrets.rs`), small craft
/// show their most detailed meshes. Shared with the warm-up so that both build the same meshes and materials.
pub(crate) fn ship_model(def: &battle_sim::ShipDef, model: String, team: Color) -> AlamoModel {
    let spec = AlamoModel::new(model).with_team_color(team);
    if matches!(def.class, TargetClass::Fighter | TargetClass::Bomber) {
        spec.with_lod(LodSelect::MostDetailed)
    } else {
        spec.with_unit(def.name.clone())
    }
}

/// Material of the arrival and departure flashes (the alpha is animated per flash by `update_arrival_flashes`).
pub(crate) fn flash_material(color: Color) -> StandardMaterial {
    StandardMaterial { base_color: color, unlit: true, alpha_mode: AlphaMode::Add, ..default() }
}

/// Flash where a reinforcement drops out of hyperspace: an additive sphere that blooms and fades.
#[derive(Component)]
pub struct ArrivalFlash {
    /// Battle time the flash starts.
    at: f32,
    radius: f32,
}

/// Applies the streak to ships that are still arriving (U71).
fn apply_arrival(arrival: &HyperspaceArrival, now: f32, radius: f32, tf: &mut Transform) -> bool {
    let p = ((now - arrival.start) / ARRIVAL_SECONDS).clamp(0.0, 1.0);
    if p >= 1.0 {
        return false;
    }
    let fwd = tf.rotation * Vec3::Z;
    let arrival_dist = (800.0f32).max(radius * 3.5);
    let t = (p / 0.8).clamp(0.0, 1.0);
    let approach = (1.0 - t).powi(3);
    let settle = if p >= 0.8 {
        let st = (p - 0.8) / 0.2;
        (st * std::f32::consts::PI).sin() * (radius * 0.08)
    } else {
        0.0
    };
    tf.translation -= fwd * (arrival_dist * approach - settle);
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
            commands.entity(e).try_despawn();
            continue;
        }
        *vis = Visibility::Inherited;
        tf.scale = Vec3::splat(f.radius * (0.4 + 1.6 * t.sqrt()));
        if let Some(mut m) = materials.get_mut(&mat.0) {
            m.base_color = Color::srgba(0.85, 0.95, 1.0, (1.0 - t) * (1.0 - t) * 0.75);
        }
    }
}

/// A ship that jumped into hyperspace: it shoots forward along its heading, stretched,
/// and despawns after a short streak animation.
#[derive(Component)]
pub struct HyperspaceDeparture {
    pub start: f32,
    pub origin: Vec3,
    pub heading: Vec3,
    pub radius: f32,
}

/// Updates departing ships: rapid acceleration forward into hyperspace with stretching.
pub fn update_departures(
    mut commands: Commands,
    rt: Res<BattleRuntime>,
    mut departures: Query<(Entity, &HyperspaceDeparture, &mut Transform)>,
) {
    let Some(cur) = rt.cur.as_ref() else { return };
    for (e, dep, mut tf) in &mut departures {
        let p = ((cur.time - dep.start) / 0.6).clamp(0.0, 1.0);
        if p >= 1.0 {
            commands.entity(e).try_despawn();
            continue;
        }
        let surge = dep.radius * (12.0 + 120.0 * p * p * p);
        tf.translation = dep.origin + dep.heading * surge;
        tf.scale = Vec3::new(
            (1.0 - p * 0.4).max(0.1),
            (1.0 - p * 0.4).max(0.1),
            1.0 + 16.0 * p * p,
        );
    }
}

/// Wall-clock cap (seconds) on the hyperspace departure effects, which are timed on the sim clock and would stand forever once it stops.
const FLASH_WALL_SECONDS: f32 = 3.0;

/// Despawns an entity after a short delay (the root of a ship that was replaced by its death clone).
#[derive(Component)]
pub struct RemoveAfter(f32);

/// Counts down [`RemoveAfter`] and despawns.
pub fn remove_after(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut RemoveAfter)>) {
    for (e, mut r) in &mut q {
        r.0 -= time.delta_secs();
        if r.0 <= 0.0 {
            commands.entity(e).try_despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fighter_boost_lifts_small_craft_to_the_minimum_screen_size() {
        // 1080 px high, 60 degree fov: 935 px per unit at distance 1. A TIE of radius 2 at 930 units is about 4 px: boosted. A big ship is left alone.
        let px_per_unit = 1080.0 / (2.0 * (30f32.to_radians()).tan());
        let b = fighter_boost(2.0, 930.0, px_per_unit, 1080.0);
        assert!(b > 5.0 && b < FIGHTER_MAX_BOOST, "{b}");
        assert_eq!(fighter_boost(400.0, 930.0, px_per_unit, 1080.0), 1.0);
        assert_eq!(fighter_boost(0.01, 930.0, px_per_unit, 1080.0), FIGHTER_MAX_BOOST);
    }

    #[test]
    fn remove_after_despawns_on_wall_time_when_the_sim_clock_stands() {
        let mut app = App::new();
        app.init_resource::<Time>().add_systems(Update, remove_after);
        let e = app.world_mut().spawn(RemoveAfter(FLASH_WALL_SECONDS)).id();
        app.world_mut().resource_mut::<Time>().advance_by(std::time::Duration::from_secs_f32(FLASH_WALL_SECONDS * 0.5));
        app.update();
        assert!(app.world().get_entity(e).is_ok());
        app.world_mut().resource_mut::<Time>().advance_by(std::time::Duration::from_secs_f32(FLASH_WALL_SECONDS));
        app.update();
        assert!(app.world().get_entity(e).is_err());
    }

    #[test]
    fn test_debris_chunk_cooling_and_drift() {
        let chunk = DebrisChunk {
            timer: 30.0,
            max_time: 30.0,
            velocity: Vec3::new(10.0, 5.0, -2.0),
            angular_velocity: Vec3::new(0.04, 0.02, 0.01),
        };
        assert_eq!(chunk.timer, chunk.max_time);
        let cool_initial = (chunk.timer / chunk.max_time).clamp(0.0, 1.0);
        assert!((cool_initial - 1.0).abs() < 1e-5);
        let cool_halfway = (15.0 / chunk.max_time).clamp(0.0, 1.0);
        assert!((cool_halfway - 0.5).abs() < 1e-5);
    }

    #[test]
    fn test_hyperspace_arrival_displacement_and_stretch() {
        let arrival = HyperspaceArrival { start: 0.0 };
        let mut tf = Transform::IDENTITY;
        let radius = 100.0;
        assert!(apply_arrival(&arrival, 0.0, radius, &mut tf));
        assert!(tf.translation.z <= -799.0, "initial translation offset is at least 800m behind: {:?}", tf.translation);
        assert_eq!(tf.scale, Vec3::ONE, "mesh scale remains undistorted: {:?}", tf.scale);

        let mut tf_done = Transform::IDENTITY;
        assert!(!apply_arrival(&arrival, ARRIVAL_SECONDS, radius, &mut tf_done));
    }
}
