//! Ship entities: spawning, interpolation between simulation steps, hardpoint damage and death.

use std::collections::{HashMap, HashSet};

use alamo_render::{
    AlamoModel, AlamoModelLoaded, AlamoParticles, LodSelect, SpawnParticleEffect,
};
use battle_sim::{Event, HpKind, ShipId, TargetClass};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::convert::{lerp_angle, ship_rotation, sim_to_bevy};
use crate::flight::{Attitude, Motion, attitude_rotation, weave, weave_room};
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
    /// Bank and pitch from the simulated motion ([`crate::flight`]).
    attitude: Attitude,
    /// Battle time the ship appeared: a small craft's weave grows in from then.
    born: f32,
    /// Radius of the visible geometry in world units (0 until the model has loaded).
    visual_radius: f32,
    /// The model is shown (enemy ships hide in the fog of war).
    shown: bool,
}

#[cfg(test)]
impl ShipView {
    /// The view of simulation ship `id` before its model is loaded, for tests of other modules.
    pub(crate) fn unloaded(id: u32, def: ShipId, side: u8) -> Self {
        Self { id, def, side, hp_applied: vec![], bones: None, hp_parts: vec![], attitude: Attitude::default(), born: 0.0, visual_radius: 0.0, shown: true }
    }
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
    /// Bevy world position where the ship is drawn: its simulated place, plus a small craft's weave (at most
    /// [`crate::flight::weave_reach`], 8.1 units).
    pub pos: Vec3,
    /// Sim heading in radians (interpolated).
    pub yaw: f32,
    /// Side.
    pub side: u8,
    /// Definition.
    pub def: ShipId,
    /// Collision radius from the definition.
    pub radius: f32,
    /// Radius of the model's bounding sphere as it is drawn (world units, small-craft enlargement included, see `small_craft_boost`);
    /// 0 until the model is built and while the ship streams in from hyperspace.
    pub drawn_radius: f32,
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

    /// World position of the model bone called `name`, else of the first proxy whose name starts with it (`Owner_Particle_Bone_Name`
    /// names either).
    fn bone_or_proxy_world(&self, root: Entity, name: &str) -> Option<Vec3> {
        let loaded = self.loaded.get(root).ok()?;
        match loaded.bone_by_name.get(&name.to_ascii_lowercase()) {
            Some(&b) => self.globals.get(b).ok().map(|g| g.translation()),
            None => self.proxy_world(root, &name.to_ascii_lowercase()),
        }
    }

    /// Before a clip that drives the visibility of `tracked` bones (lower-case names): notes their visibility, to be restored by
    /// [`ModelTouch::end_clip`], and the proxies on the `shown` ones (the clip shows them on some frames), whose particle systems
    /// [`ModelTouch::light_clip`] keeps on while the clip runs.
    fn begin_clip(&self, root: Entity, tracked: &[String], shown: &[String]) -> ClipTouch {
        let Ok(loaded) = self.loaded.get(root) else { return ClipTouch::default() };
        let bones = tracked
            .iter()
            .filter_map(|n| loaded.bone_by_name.get(n).copied())
            .map(|b| (b, self.visibility.get(b).copied().unwrap_or_default()))
            .collect();
        let proxies = loaded
            .proxies
            .iter()
            .copied()
            .filter(|&p| self.parents.get(p).is_ok_and(|c| self.names.get(c.parent()).is_ok_and(|n| shown.contains(&n.as_str().to_ascii_lowercase()))))
            .collect();
        ClipTouch { bones, proxies }
    }

    /// Keeps the particle systems of a running clip's proxies on (a proxy gets its particle system a frame after its model is
    /// built; the clip's visibility track decides when they show).
    fn light_clip(&mut self, touched: &ClipTouch) {
        for &p in &touched.proxies {
            if let Ok(mut ps) = self.particles.get_mut(p)
                && !ps.enabled
            {
                ps.enabled = true;
            }
        }
    }

    /// After the clip of [`ModelTouch::begin_clip`]: its proxies are off again and its bones have the visibility they had before.
    fn end_clip(&mut self, touched: &ClipTouch) {
        for &p in &touched.proxies {
            if let Ok(mut ps) = self.particles.get_mut(p) {
                ps.enabled = false;
            }
        }
        for &(bone, vis) in &touched.bones {
            if let Ok(mut v) = self.visibility.get_mut(bone) {
                *v = vis;
            }
        }
    }
}

/// What a clip touches ([`ModelTouch::begin_clip`]): bones with their visibility before the clip, the proxies it lights.
#[derive(Debug, Default)]
pub struct ClipTouch {
    bones: Vec<(Entity, Visibility)>,
    proxies: Vec<Entity>,
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
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<Camera3d>>,
    mut sfx: MessageWriter<alamo_audio::PlaySfx>,
    mut flash_spots: Local<Vec<Vec3>>,
    // One quad and one glow texture for every arrival and departure flash.
    mut flash_art: Local<Option<FlashArt>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let (Some(cur), alpha) = (rt.cur.clone(), rt.alpha) else {
        return;
    };
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
    // The fighters of one squadron take their size from the squadron's centre, so members never differ in size.
    let centres = squad_centres(cur.ships.iter().filter(|s| s.squad != u32::MAX).map(|s| (s.squad, interpolate(s, next.as_ref().and_then(|n| n.ship(s.id)), alpha).0)));
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
                            attitude: Attitude::new(cur.tick),
                            born: cur.time,
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
                        let art = flash_art.get_or_insert_with(|| FlashArt::new(&mut meshes, &mut images));
                        let flash = ArrivalFlash { at: cur.time + lead + ARRIVAL_SECONDS * 0.85, radius: radius * 0.45 };
                        spawn_flash(&mut commands, art, &mut materials, pos, flash, "hyperspace flash", Visibility::Hidden);
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
        // Where the ship is drawn: its simulated place plus a small craft's weave. Picking, overlays and turrets use it.
        let mut drawn = pos;
        let mut drawn_radius = 0.0;
        if let Ok((mut view, mut tf)) = views.get_mut(entity) {
            if view.visual_radius == 0.0
                && let Ok(l) = touch.loaded.get(entity)
            {
                view.visual_radius = l.radius * vis.scale;
            }
            if view.visual_radius > 0.0 {
                radius = view.visual_radius * 0.8;
            }
            if view.shown != visible {
                view.shown = visible;
                commands.entity(entity).insert(if visible { Visibility::Inherited } else { Visibility::Hidden });
            }

            // Bank into turns and pitch with climbs from the step's motion; small craft also weave around their place (`flight`).
            // A paused battle keeps its lean: the attitude only moves with the simulation steps.
            let look = &vis.flight;
            let motion = nxt.map(|n| Motion::between(s.pos, s.yaw, n.pos, n.yaw, sim_dt));
            view.attitude.update(cur.tick, look, motion, sim_dt);
            let (mut bank, mut pitch) = view.attitude.at(alpha);
            let mut heading = yaw;
            if look.small || s.squad != u32::MAX {
                let t = cur.time + alpha * sim_dt;
                let calm = look.calm(view.attitude.speed());
                let room = eye.map_or(1.0, |(eye, _, _)| weave_room(centres.get(&s.squad).copied().unwrap_or(pos).distance(eye)));
                let w = weave(s.id, t, calm, t - view.born, room);
                drawn = pos + ship_rotation(yaw) * w.offset;
                heading += w.yaw;
                (bank, pitch) = w.lean(look, bank, pitch);
            }
            tf.translation = drawn;
            tf.rotation = attitude_rotation(heading, pitch, bank);
            let mut boost = 1.0;
            // Squadron members of any class count (a Skipray blast boat squadron is no fighter class but reads as a unit of small craft).
            if (matches!(def.class, TargetClass::Fighter | TargetClass::Bomber) || s.squad != u32::MAX)
                && let Some((eye, px_per_unit, height)) = eye
                && view.visual_radius > 0.0
            {
                let at = centres.get(&s.squad).copied().unwrap_or(pos);
                boost = small_craft_boost(view.visual_radius, at.distance(eye), px_per_unit, height);
            }
            tf.scale = Vec3::splat(vis.scale * boost);
            drawn_radius = view.visual_radius * boost;
            if let Ok(a) = arrivals.get(entity) {
                if apply_arrival(a, cur.time, radius, &mut tf) {
                    // Still streaming in from hyperspace: nothing marks its place (no bar) until it is there.
                    drawn_radius = 0.0;
                } else {
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
                pos: drawn,
                yaw,
                side: s.side,
                def: s.def,
                radius,
                drawn_radius,
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
                // A soft glow, not a ball: the opaque white ball that stood here covered the screen on a fleet retreat (QA round 3).
                let art = flash_art.get_or_insert_with(|| FlashArt::new(&mut meshes, &mut images));
                let flash = spawn_flash(&mut commands, art, &mut materials, t.translation, ArrivalFlash { at: cur.time, radius: radius * 0.6 }, "hyperspace departure flash", Visibility::Inherited);
                // The sim clock freezes when the battle ends (DEFEATED, pause); the flash is timed on it, so it also expires on wall time (the user saw it stay forever).
                commands.entity(flash).insert(RemoveAfter(FLASH_WALL_SECONDS));
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
#[allow(clippy::too_many_arguments)]
pub fn ship_events(
    mut commands: Commands,
    mut pulses: ResMut<crate::effects::BloomPulses>,
    defs: Option<Res<alamo_render::XmlDefs>>,
    mut rt: ResMut<BattleRuntime>,
    mut touch: ModelTouch,
    mut views: Query<&mut ShipView>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    fs: Option<Res<alamo_render::AlamoFs>>,
    mut images: ResMut<Assets<Image>>,
    mut beam_art: ResMut<crate::effects::BeamArt>,
    mut shot_sounds: ShotSounds,
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
                let Some(&e) = rt.entities.get(ship) else { continue };
                let now = rt.cur.as_ref().map_or(0.0, |c| c.time);
                let at = rt.cur.as_ref().and_then(|c| c.ship(*ship)).map_or(Vec3::ZERO, |s| sim_to_bevy(s.pos));
                info!("battle: super laser of ship {ship} charging at t={now:.1}s at ({:.0}, {:.0}), shot in {fire_in:.2}s", at.x, at.z);
                let laser = views.get(e).ok().and_then(|v| visuals.ship(v.def).super_laser.as_ref());
                // The shot's sound is read while the laser charges, so that its length is known at the shot.
                if let Some(event) = laser.and_then(|l| l.fire_loop.as_deref()) {
                    shot_sounds.prepare(event);
                }
                if touch.proxy_world(e, SUPER_LASER_SMALL).is_some() {
                    // The Death Star: its `p_planet_deathstar_laser_small` proxies (the converging charge lattice) light up now, the big dish
                    // glow `fire_in` seconds later.
                    touch.set_proxy_particles(e, SUPER_LASER_SMALL, true);
                    commands.entity(e).insert(SuperLaserFx { large_at: now + fire_in, end_at: now + fire_in + SUPER_LASER_GLOW_AFTER, large_on: false });
                } else if let Some(laser) = laser {
                    // Every other super laser plays the shooter's `ATTACK` animation from the order on (`update_super_laser_charge`).
                    commands.entity(e).insert(SuperLaserCharge { start: now, variant: laser.attack_variant, shown: None, length: 0.0 });
                    // Its beam art is decoded now, and an unlit copy of the beam builds its render pipeline before the shot (the battle's
                    // warm-up does both behind the loading screen; this covers scenes without one).
                    if let Some(texture) = laser.beam_texture.as_deref().zip(fs.as_ref()).and_then(|(name, fs)| beam_art.get_or_load(&fs.0, &mut images, name)) {
                        crate::effects::spawn_textured_beam(&mut commands, &mut meshes, &mut materials, texture, at, at + Vec3::X * 10.0, 1.0, Color::NONE, 1, *fire_in);
                    }
                }
            }
            Event::SuperLaserFired { ship, target, at } => {
                info!("battle: super laser of ship {ship} fired at {target}");
                let (Some(&e), Some(cur)) = (rt.entities.get(ship), rt.cur.as_ref()) else { continue };
                let Some(shooter) = cur.ship(*ship) else { continue };
                let to = sim_to_bevy(*at);
                let laser = visuals.ship(shooter.def).super_laser.clone();
                let bone = laser.as_ref().and_then(|l| l.bone.clone()).unwrap_or_else(|| SUPER_LASER_LARGE.to_string());
                let from = touch.bone_or_proxy_world(e, &bone).unwrap_or_else(|| sim_to_bevy(shooter.pos));
                let texture = laser.as_ref().and_then(|l| l.beam_texture.as_deref()).zip(fs.as_ref()).and_then(|(name, fs)| beam_art.get_or_load(&fs.0, &mut images, name));
                let hull = catalog.ship(shooter.def).radius;
                let hold = laser.as_ref().and_then(|l| l.fire_loop.as_deref()).and_then(|event| shot_sounds.hold(event));
                fire_super_laser(&mut commands, &mut meshes, &mut materials, laser.as_ref(), texture, from, to, hull, hold);
                pulses.flash(to, 1.0);
            }
            Event::HardpointDestroyed { .. } => {}
            _ => {}
        }
    }
}

/// The shot of a super laser from `from` to `to`: the data's beam (`Beam_Texture_Name` `Line_Width` wide, tinted with `Line_Color`,
/// for `Fire_Continuation` seconds) when the ability names beam art and `texture` holds it, else the plain green beam line of the
/// Death Star. With `sound_hold` (`None`: the battle's sound is off) `SFXEvent_Special_Ability_Loop` plays at the muzzle while the
/// beam fires and for at least `sound_hold` seconds ([`ShotSounds::hold`]). Returns the beam's seconds.
#[allow(clippy::too_many_arguments)]
pub fn fire_super_laser(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    laser: Option<&crate::visuals::SuperLaserVisual>,
    texture: Option<Handle<Image>>,
    from: Vec3,
    to: Vec3,
    hull_radius: f32,
    sound_hold: Option<f32>,
) -> f32 {
    let seconds = match (laser, texture) {
        (Some(l), Some(texture)) => {
            let seconds = if l.fire_seconds > 0.0 { l.fire_seconds } else { SUPER_LASER_BEAM_SECONDS };
            let width = if l.line_width > 0.0 { l.line_width } else { 2.0 * SUPER_LASER_BEAM_RADIUS };
            crate::effects::spawn_textured_beam(commands, meshes, materials, texture, from, to, width, l.line_color, l.beam_frames, seconds);
            seconds
        }
        _ => {
            let beam_radius = (hull_radius * SUPER_LASER_BEAM_RADIUS_OF_HULL).max(SUPER_LASER_BEAM_RADIUS);
            crate::effects::spawn_beam(commands, meshes, materials, from, to, Color::srgba(0.25, 1.0, 0.4, 0.85), beam_radius, SUPER_LASER_BEAM_SECONDS);
            SUPER_LASER_BEAM_SECONDS
        }
    };
    if let (Some(event), Some(hold)) = (laser.and_then(|l| l.fire_loop.clone()), sound_hold) {
        commands.spawn((Name::new("super laser sound"), BattleEntity, Transform::from_translation(from), alamo_audio::SfxEmitter::new(Some(event)), RemoveAfter(seconds.max(hold))));
    }
    seconds
}

/// Seconds a shot's loop stops short of one pass through its sample: the loop fades out after it is let go, and its voice may
/// start a frame or two after the shot.
const SHOT_SOUND_MARGIN: f32 = 0.1;

/// The sound of super laser shots. The data gives the shot a loop (`SFXEvent_Special_Ability_Loop`: `Unit_Death_Star_2_Fire`,
/// `Preset_INSANE_LOOP`, heard across the whole map), and its sample `GUN_Superlaser_2.wav` is a whole 3.45 s blast: full for
/// 1.5 s, then a 2 s tail. Held only while the beam fires (the Eclipse's `Fire_Continuation` 1 s, the Death Star II's 0.25 s)
/// it would be cut at full volume, so the loop plays one whole pass (estimate: how long the original holds it is not known).
#[derive(SystemParam)]
pub struct ShotSounds<'w> {
    settings: Res<'w, BattleViewSettings>,
    audio: Option<ResMut<'w, alamo_audio::plugin::AlamoAudio>>,
    audio_settings: Option<Res<'w, alamo_audio::AudioSettings>>,
    sources: Option<ResMut<'w, Assets<AudioSource>>>,
}

impl ShotSounds<'_> {
    /// Starts reading the samples of the loop `event` on the audio worker threads.
    fn prepare(&mut self, event: &str) {
        let (true, Some(audio), Some(settings), Some(sources)) = (self.settings.audio, self.audio.as_mut(), self.audio_settings.as_ref(), self.sources.as_mut()) else { return };
        if settings.load_on_worker {
            audio.prepare_event(event, settings.cache_budget_bytes, sources);
        }
    }

    /// `None` with the battle's sound off, else the least seconds the shot holds its loop `event`: one pass through its sample less
    /// the loop's fade-out (0 while the sample is not read).
    pub fn hold(&self, event: &str) -> Option<f32> {
        if !self.settings.audio {
            return None;
        }
        let (Some(audio), Some(sources)) = (self.audio.as_deref(), self.sources.as_deref()) else { return Some(0.0) };
        let fade_out = audio.sfx.get(event).map_or(0.0, |e| e.loop_fade.1.max(0.05));
        Some(audio.loaded_pass_seconds(event, sources).map_or(0.0, |pass| pass - fade_out - SHOT_SOUND_MARGIN))
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

/// Charge of a super laser other than the Death Star's, on the ship entity: the model's `ATTACK` animation runs on the battle clock
/// from the order on, so that its `Fire_Frame` (the shot, `fire_in` = `Fire_Frame` / 30) falls on the sim's shot.
#[derive(Component, Debug)]
pub struct SuperLaserCharge {
    /// Battle time of the order.
    start: f32,
    /// `Attack_Anim_Sub_Index`.
    variant: u32,
    /// What the clip changed on the model; `None` until the clip was read.
    shown: Option<ClipTouch>,
    /// Clip length in seconds.
    length: f32,
}

/// Plays the `ATTACK` clip of a [`SuperLaserCharge`]: the clip's time is the battle time since the order (it follows a paused or sped
/// up battle), the particle proxies on the bones the clip shows are switched on for it (the Eclipse's `P_Eclipse_Charge_L` / `_S`
/// lattice is visible from frame 51, the `Ambient_02` / `_03` glow meshes from 226 and 230, the `P_Eclipse_Star_Dreadnaught_Muzzle`
/// flare from 230, one past its `Fire_Frame` 229). At the end of the clip the proxies are off, the bones it drove have their
/// visibility from before again and the model returns to its `IDLE` animation.
#[allow(clippy::type_complexity)]
pub fn update_super_laser_charge(
    mut commands: Commands,
    rt: Res<BattleRuntime>,
    fs: Option<Res<alamo_render::AlamoFs>>,
    index: Option<Res<alamo_render::AnimationIndex>>,
    mut clips: Option<ResMut<alamo_render::animation::AnimationCache>>,
    mut touch: ModelTouch,
    mut q: Query<(Entity, &AlamoModel, &mut SuperLaserCharge, Option<&mut alamo_render::AlamoAnimation>)>,
) {
    let (Some(fs), Some(index), Some(clips)) = (fs, index, clips.as_mut()) else { return };
    let now = rt.cur.as_ref().map_or(0.0, |c| c.time);
    for (e, model, mut charge, anim) in &mut q {
        let stem = alamo_render::animation::model_stem(&model.path);
        if charge.shown.is_none() {
            // The model may still be loading (an order in the first moments of a battle): the clip then starts late, still on the shot.
            if touch.loaded.get(e).is_err() {
                continue;
            }
            let clip = index.find(&stem, "ATTACK", Some(charge.variant)).and_then(|path| clips.clip(&fs, &path));
            let Some(clip) = clip else {
                commands.entity(e).remove::<SuperLaserCharge>();
                continue;
            };
            charge.length = clip.duration();
            let touched = touch.begin_clip(e, &tracked_bones(&clip), &shown_bones(&clip));
            info!("battle: super laser clip of {stem}: {:.1} s, {} bones driven, {} particle proxies", charge.length, touched.bones.len(), touched.proxies.len());
            charge.shown = Some(touched);
            let mut attack = alamo_render::AlamoAnimation::once("ATTACK");
            attack.variant = Some(charge.variant);
            attack.playing = false;
            commands.entity(e).insert(attack);
            continue;
        }
        let t = (now - charge.start).max(0.0);
        if t >= charge.length {
            if let Some(touched) = &charge.shown {
                touch.end_clip(touched);
            }
            let mut ec = commands.entity(e);
            ec.remove::<SuperLaserCharge>();
            if index.find(&stem, "IDLE", None).is_some() {
                ec.insert(alamo_render::AlamoAnimation::idle());
            } else {
                ec.remove::<alamo_render::AlamoAnimation>();
            }
        } else {
            if let Some(touched) = &charge.shown {
                touch.light_clip(touched);
            }
            if let Some(mut a) = anim
                && a.kind.eq_ignore_ascii_case("ATTACK")
            {
                a.time = t;
            }
        }
    }
}

/// Lower-case names of the bones a clip shows on some frame (bones with a visibility track that is not all off), sorted, once each.
pub fn shown_bones(clip: &alamo_model::Animation) -> Vec<String> {
    bone_names(clip, |v| v.iter().any(|&on| on))
}

/// Lower-case names of the bones whose visibility a clip drives, sorted, once each.
pub fn tracked_bones(clip: &alamo_model::Animation) -> Vec<String> {
    bone_names(clip, |_| true)
}

fn bone_names(clip: &alamo_model::Animation, keep: impl Fn(&[bool]) -> bool) -> Vec<String> {
    let mut names: Vec<String> = clip.bones.iter().filter(|b| b.visibility.as_deref().is_some_and(&keep)).map(|b| b.name.to_ascii_lowercase()).collect();
    names.sort();
    names.dedup();
    names
}

/// Share of the screen height the bounding sphere of a fighter, bomber or squadron member keeps at every zoom of the original camera.
/// The Remake draws small craft at one screen size whatever the zoom (the user's 1080p video, battle1s: TIE fighters 22-25 px across at
/// the default camera b0703s/b0732s and still 24-25 px far out in b0760s, where a Star Destroyer is 54 px), far above true scale (the
/// data gives a TIE 4 units next to a 760 unit Star Destroyer). The value is set so that our TIE fighter measures the same, wings
/// included, in a 1080p render at the same camera (its bounding sphere is then 35 px across, its silhouette about 23 x 25 px).
const SMALL_CRAFT_SCREEN_FRACTION: f32 = 0.032;
/// Camera distance (at our fov of 60 degrees) beyond which small craft stop keeping their screen size: the Remake's widest view,
/// `Tactical_Overview2` 6000 at fov 70 in TacticalCameras.xml, sees as much as 7277 at fov 60. Farther out the enlargement stays at
/// its value there and small craft shrink with the world.
const SMALL_CRAFT_FREEZE_DISTANCE: f32 = 7277.0;

/// Centre of every squadron from its members' (squadron, position) pairs; the members take their size from it.
fn squad_centres(members: impl Iterator<Item = (u32, Vec3)>) -> HashMap<u32, Vec3> {
    let mut sum: HashMap<u32, (Vec3, f32)> = HashMap::new();
    for (squad, pos) in members {
        let c = sum.entry(squad).or_insert((Vec3::ZERO, 0.0));
        c.0 += pos;
        c.1 += 1.0;
    }
    sum.into_iter().map(|(squad, (pos, n))| (squad, pos / n)).collect()
}

/// Scale factor that draws a small craft of bounding radius `radius` whose squadron is `dist` from the eye at the screen size of the
/// original ([`SMALL_CRAFT_SCREEN_FRACTION`]): never below its true size (1 close in), proportional to the distance up to
/// [`SMALL_CRAFT_FREEZE_DISTANCE`] and constant beyond, so it is continuous and grows with the distance without jumps.
fn small_craft_boost(radius: f32, dist: f32, px_per_unit: f32, screen_height: f32) -> f32 {
    let px = 2.0 * radius * px_per_unit / dist.clamp(1.0, SMALL_CRAFT_FREEZE_DISTANCE);
    (SMALL_CRAFT_SCREEN_FRACTION * screen_height / px.max(1e-3)).max(1.0)
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

/// Brightest additive alpha of a flash, at the middle of its glow.
const FLASH_PEAK: f32 = 1.0;
/// Seconds a flash lasts.
const FLASH_SECONDS: f32 = 0.4;
/// Half the side of the flash quad in [`ArrivalFlash::radius`]. The glow ([`flash_glow`]) is zero at the edge of the quad and its core is
/// tight, so what shows of a flash is a bloom a fraction of that size.
const FLASH_REACH: f32 = 2.0;

/// Brightness (0..1) of the glow of a flash at `(u, v)` on its square image (-1..1 on both axes): a tight core with thin rays along the
/// screen axes and the diagonals, falling smoothly to nothing at radius 1. A flash is a patch of light with no rim anywhere. (It used to be
/// an additive sphere: a ball of one even brightness and a hard edge, which stood round the arriving ship as a translucent white sphere.)
fn flash_glow(u: f32, v: f32) -> f32 {
    use std::f32::consts::FRAC_PI_4;
    let r = (u * u + v * v).sqrt().min(1.0);
    let angle = v.atan2(u);
    let axes = angle.cos().abs().powi(14) + angle.sin().abs().powi(14);
    let diagonals = (angle - FRAC_PI_4).cos().abs().powi(24) + (angle + FRAC_PI_4).cos().abs().powi(24);
    let core = (1.0 - r).powi(3);
    (core + (0.7 * axes + 0.35 * diagonals) * (1.0 - r).powi(2)).min(1.0) * (1.0 - r)
}

/// The image of [`flash_glow`]: white, the glow in the alpha channel.
pub(crate) fn flash_glow_image() -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    const SIDE: u32 = 128;
    let mut px = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let (u, v) = (((x as f32 + 0.5) / SIDE as f32) * 2.0 - 1.0, ((y as f32 + 0.5) / SIDE as f32) * 2.0 - 1.0);
            px.extend_from_slice(&[255, 255, 255, (flash_glow(u, v) * 255.0).round() as u8]);
        }
    }
    Image::new(Extent3d { width: SIDE, height: SIDE, depth_or_array_layers: 1 }, TextureDimension::D2, px, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default())
}

/// The quad and the glow shared by every arrival and departure flash.
#[derive(Clone)]
pub struct FlashArt {
    /// A square of half side 1 in the XY plane.
    pub(crate) mesh: Handle<Mesh>,
    /// [`flash_glow_image`].
    pub(crate) glow: Handle<Image>,
}

impl FlashArt {
    pub(crate) fn new(meshes: &mut Assets<Mesh>, images: &mut Assets<Image>) -> Self {
        Self { mesh: meshes.add(Rectangle::new(2.0, 2.0)), glow: images.add(flash_glow_image()) }
    }
}

/// Material of the arrival and departure flashes: the glow, added to what is behind it (the alpha is animated per flash by
/// `update_arrival_flashes`).
pub(crate) fn flash_material(color: Color, glow: Handle<Image>) -> StandardMaterial {
    StandardMaterial { base_color: color, base_color_texture: Some(glow), unlit: true, alpha_mode: AlphaMode::Add, cull_mode: None, ..default() }
}

/// Flash where a reinforcement drops out of hyperspace (or a ship jumps away): a soft glow that faces the camera, blooms and fades.
#[derive(Component)]
pub struct ArrivalFlash {
    /// Battle time the flash starts.
    at: f32,
    radius: f32,
}

/// Spawns a flash at `pos` (starting at `flash.at`; `update_arrival_flashes` sizes, fades and removes it).
pub(crate) fn spawn_flash(commands: &mut Commands, art: &FlashArt, materials: &mut Assets<StandardMaterial>, pos: Vec3, flash: ArrivalFlash, name: &'static str, visibility: Visibility) -> Entity {
    commands
        .spawn((
            Name::new(name),
            BattleEntity,
            Mesh3d(art.mesh.clone()),
            MeshMaterial3d(materials.add(flash_material(Color::srgba(0.85, 0.95, 1.0, 0.0), art.glow.clone()))),
            Transform::from_translation(pos).with_scale(Vec3::splat(flash.radius)),
            visibility,
            flash,
        ))
        .id()
}

/// How a flash looks `now`: not yet, gone, or a glow of this half size (world units) and additive alpha.
#[derive(Debug, PartialEq)]
enum FlashLook {
    Waiting,
    Gone,
    Glow { half_size: f32, alpha: f32 },
}

/// The look of `flash` at battle time `now`: its quad blooms from 0.8 to 4 times its radius over [`FLASH_SECONDS`] while its middle fades
/// from [`FLASH_PEAK`] to nothing.
fn flash_look(flash: &ArrivalFlash, now: f32) -> FlashLook {
    if now < flash.at {
        return FlashLook::Waiting;
    }
    let t = (now - flash.at) / FLASH_SECONDS;
    if t >= 1.0 {
        return FlashLook::Gone;
    }
    FlashLook::Glow { half_size: flash.radius * FLASH_REACH * (0.4 + 1.6 * t.sqrt()), alpha: (1.0 - t) * (1.0 - t) * FLASH_PEAK }
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

/// Grows, fades and removes arrival flashes, and turns their glow to the camera.
pub fn update_arrival_flashes(
    mut commands: Commands,
    rt: Res<BattleRuntime>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut flashes: Query<(Entity, &ArrivalFlash, &mut Transform, &MeshMaterial3d<StandardMaterial>, &mut Visibility)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(now) = rt.cur.as_ref().map(|c| c.time) else { return };
    let facing = cameras.iter().find(|(c, _)| c.is_active).map(|(_, gt)| gt.rotation());
    for (e, f, mut tf, mat, mut vis) in &mut flashes {
        if !apply_flash(f, now, facing, &mut tf, &mut vis, materials.get_mut(&mat.0).as_deref_mut()) {
            commands.entity(e).try_despawn();
        }
    }
}

/// Sizes, fades and aims `flash` for battle time `now` (`facing`: the camera's rotation); false once it is gone.
fn apply_flash(flash: &ArrivalFlash, now: f32, facing: Option<Quat>, tf: &mut Transform, vis: &mut Visibility, material: Option<&mut StandardMaterial>) -> bool {
    match flash_look(flash, now) {
        FlashLook::Waiting => true,
        FlashLook::Gone => false,
        FlashLook::Glow { half_size, alpha } => {
            *vis = Visibility::Inherited;
            tf.scale = Vec3::splat(half_size);
            if let Some(rotation) = facing {
                tf.rotation = rotation;
            }
            if let Some(m) = material {
                m.base_color = Color::srgba(0.85, 0.95, 1.0, alpha);
            }
            true
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
    fn small_craft_keep_one_screen_size_up_to_the_widest_original_view() {
        // 1080 px high, 60 degree fov: 935 px per unit at distance 1. A TIE of bounding radius 2.6.
        let px_per_unit = 1080.0 / (2.0 * (30f32.to_radians()).tan());
        let r = 2.6;
        let screen_px = |d: f32| 2.0 * r * small_craft_boost(r, d, px_per_unit, 1080.0) * px_per_unit / d;
        // The Remake's cameras at our fov: closest 384, default 2000, overview 6064, widest 7277. Inside that range one size.
        for d in [384.0, 1000.0, 2000.0, 6064.0, 7277.0] {
            assert!((screen_px(d) - SMALL_CRAFT_SCREEN_FRACTION * 1080.0).abs() < 1e-2, "{d}: {} px", screen_px(d));
        }
        // Beyond it the enlargement holds and the craft shrinks with the world (our widest zoom is 16000).
        let frozen = small_craft_boost(r, SMALL_CRAFT_FREEZE_DISTANCE, px_per_unit, 1080.0);
        assert_eq!(small_craft_boost(r, 16000.0, px_per_unit, 1080.0), frozen);
        assert!((screen_px(16000.0) - screen_px(7277.0) * 7277.0 / 16000.0).abs() < 1e-2);
        // Close in it is never drawn below its true size, and a big ship is left alone.
        assert_eq!(small_craft_boost(r, 20.0, px_per_unit, 1080.0), 1.0);
        assert_eq!(small_craft_boost(400.0, 2000.0, px_per_unit, 1080.0), 1.0);
        // Continuous and never shrinking as the camera moves out: no jumps between zoom steps.
        let mut last = small_craft_boost(r, 10.0, px_per_unit, 1080.0);
        for k in 0..=20_000 {
            let d = 10.0 + k as f32;
            let b = small_craft_boost(r, d, px_per_unit, 1080.0);
            assert!(b >= last && b - last < 0.01, "{d}: {last} -> {b}");
            last = b;
        }
    }

    #[test]
    fn the_members_of_a_squadron_are_drawn_the_same_size() {
        let px_per_unit = 1080.0 / (2.0 * (30f32.to_radians()).tan());
        let eye = Vec3::new(0.0, 2000.0, 0.0);
        // Squadron 7 spread 400 units along the line of sight: by their own distances the members would differ by a fifth.
        let members = [(7, Vec3::new(0.0, -200.0, 0.0)), (7, Vec3::new(100.0, 0.0, 0.0)), (7, Vec3::new(-100.0, 200.0, 0.0)), (9, Vec3::new(0.0, 500.0, 0.0))];
        let own: Vec<f32> = members[..3].iter().map(|(_, p)| small_craft_boost(2.6, p.distance(eye), px_per_unit, 1080.0)).collect();
        assert!(own[2] < own[0] * 0.85, "{own:?}");
        let centres = squad_centres(members.into_iter());
        assert_eq!(centres[&7], Vec3::ZERO);
        assert_eq!(centres[&9], Vec3::new(0.0, 500.0, 0.0));
        let sizes: Vec<f32> = members[..3].iter().map(|(q, _)| small_craft_boost(2.6, centres[q].distance(eye), px_per_unit, 1080.0)).collect();
        assert!(sizes.iter().all(|b| *b == sizes[0]), "{sizes:?}");
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

    /// The arrival flash was an additive UV sphere: a ball of one even brightness with a hard edge, which stood round the arriving ship as
    /// a translucent white sphere (a Star Destroyer inside it, 1.3 s after it appeared). The glow it has now falls off smoothly to nothing
    /// before the edge of its image, so there is no rim anywhere.
    #[test]
    fn a_flash_is_a_soft_glow_without_a_rim() {
        // Along the screen axes, the diagonals and between them: brightest in the middle, falling steadily, no step anywhere (the sphere had
        // one of 0.75 at its edge, here the steepest fall is 0.025 per hundredth of the radius), nothing at the edge.
        for degrees in [0.0f32, 12.0, 22.5, 45.0, 60.0, 90.0, 135.0, 200.0, 315.0] {
            let (sin, cos) = degrees.to_radians().sin_cos();
            let along: Vec<f32> = (0..=100).map(|k| flash_glow(cos * k as f32 / 100.0, sin * k as f32 / 100.0)).collect();
            assert!(along[0] > 0.99, "{degrees} degrees: {:?}", along[0]);
            assert!(along.windows(2).all(|p| p[1] <= p[0]), "{degrees} degrees: {along:?}");
            assert!(along.windows(2).all(|p| p[0] - p[1] <= 0.05), "{degrees} degrees: {along:?}");
            assert!(along[99] < 0.01 && along[100] < 1e-4, "{degrees} degrees: {along:?}");
        }
        // The image carries it: opaque in the middle, nothing at the edges and in the corners.
        let img = flash_glow_image();
        let (w, h) = (img.width() as usize, img.height() as usize);
        let data = img.data.as_ref().expect("cpu data");
        let alpha = |x: usize, y: usize| data[(y * w + x) * 4 + 3];
        assert!(alpha(w / 2, h / 2) >= 250 && alpha(w / 2 - 1, h / 2 - 1) >= 250);
        assert!([(0, h / 2), (w - 1, h / 2), (w / 2, 0), (w / 2, h - 1), (0, 0), (w - 1, h - 1)].iter().all(|&(x, y)| alpha(x, y) <= 1));
    }

    #[test]
    fn a_flash_is_one_flat_quad_not_a_ball() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<Image>>();
        let art = world.run_system_once(|mut meshes: ResMut<Assets<Mesh>>, mut images: ResMut<Assets<Image>>| FlashArt::new(&mut meshes, &mut images)).expect("flash art");
        let mesh = world.resource::<Assets<Mesh>>().get(&art.mesh).expect("flash mesh");
        assert_eq!(mesh.count_vertices(), 4, "the sphere had 425");
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(points)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!("no positions") };
        assert!(points.iter().all(|p| p[2] == 0.0 && p[0].abs() == 1.0 && p[1].abs() == 1.0), "{points:?}");
    }

    #[test]
    fn a_spawned_flash_waits_hidden_and_adds_its_glow_to_what_is_behind_it() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<Assets<StandardMaterial>>();
        let flash = world
            .run_system_once(|mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut images: ResMut<Assets<Image>>, mut materials: ResMut<Assets<StandardMaterial>>| {
                let art = FlashArt::new(&mut meshes, &mut images);
                (spawn_flash(&mut commands, &art, &mut materials, Vec3::new(1.0, 2.0, 3.0), ArrivalFlash { at: 9.0, radius: 300.0 }, "test flash", Visibility::Hidden), art)
            })
            .expect("spawn");
        let (e, art) = flash;
        assert_eq!(world.get::<Visibility>(e), Some(&Visibility::Hidden));
        assert_eq!(world.get::<Transform>(e).map(|t| t.translation), Some(Vec3::new(1.0, 2.0, 3.0)));
        assert!(world.get::<BattleEntity>(e).is_some(), "it goes with the battle");
        assert_eq!(world.get::<Mesh3d>(e).map(|m| m.0.clone()), Some(art.mesh.clone()));
        let material = world.get::<MeshMaterial3d<StandardMaterial>>(e).expect("material").0.clone();
        let m = world.resource::<Assets<StandardMaterial>>().get(&material).expect("material asset");
        assert!(m.unlit && matches!(m.alpha_mode, AlphaMode::Add) && m.base_color_texture.as_ref() == Some(&art.glow));
        assert_eq!(m.base_color.alpha(), 0.0, "nothing shows before the flash starts");
    }

    #[test]
    fn a_flash_blooms_and_fades_without_outgrowing_its_ship() {
        // A Star Destroyer's flash: radius 0.45 of the ship's radius, as `update_ships` spawns it.
        let ship = 700.0;
        let flash = ArrivalFlash { at: 10.0, radius: ship * 0.45 };
        assert_eq!(flash_look(&flash, 9.99), FlashLook::Waiting);
        assert_eq!(flash_look(&flash, 10.0 + FLASH_SECONDS + 0.01), FlashLook::Gone);
        let looks: Vec<(f32, f32)> = (0..40)
            .map(|k| match flash_look(&flash, 10.0 + k as f32 * 0.01) {
                FlashLook::Glow { half_size, alpha } => (half_size, alpha),
                other => panic!("{other:?}"),
            })
            .collect();
        assert!(looks.windows(2).all(|p| p[1].0 >= p[0].0 && p[1].1 <= p[0].1), "it grows and fades");
        // The quad stays within two ship radii (its glow is much tighter), and the middle of the glow is never brighter than the peak.
        assert!(looks.iter().all(|&(size, alpha)| size <= 2.0 * ship && alpha <= FLASH_PEAK + 1e-6), "{looks:?}");
        assert!((looks[0].1 - FLASH_PEAK).abs() < 1e-6);
        // Where the old sphere had its rim, the ship's own radius, the glow adds next to nothing at any moment (along the brightest ray).
        let at_the_hull = looks.iter().map(|&(size, alpha)| alpha * flash_glow((ship / size).min(1.0), 0.0)).fold(0.0f32, f32::max);
        assert!(at_the_hull < 0.05, "{at_the_hull}");
    }

    #[test]
    fn a_flash_turns_its_glow_to_the_camera_and_leaves_when_over() {
        let flash = ArrivalFlash { at: 10.0, radius: 100.0 };
        let (mut tf, mut vis) = (Transform::from_scale(Vec3::splat(100.0)), Visibility::Hidden);
        let mut material = StandardMaterial::default();
        let facing = Some(Quat::from_rotation_y(0.7));
        // Waiting: still hidden, nothing touched.
        assert!(apply_flash(&flash, 9.0, facing, &mut tf, &mut vis, Some(&mut material)));
        assert_eq!((vis, tf.rotation), (Visibility::Hidden, Quat::IDENTITY));
        // Under way: shown, sized, faced to the camera, faded.
        assert!(apply_flash(&flash, 10.2, facing, &mut tf, &mut vis, Some(&mut material)));
        let FlashLook::Glow { half_size, alpha } = flash_look(&flash, 10.2) else { panic!("glow") };
        assert_eq!((vis, tf.rotation, tf.scale), (Visibility::Inherited, Quat::from_rotation_y(0.7), Vec3::splat(half_size)));
        assert!((material.base_color.alpha() - alpha).abs() < 1e-6);
        // Over: gone.
        assert!(!apply_flash(&flash, 10.0 + FLASH_SECONDS + 0.01, facing, &mut tf, &mut vis, Some(&mut material)));
    }

    fn eclipse_laser() -> crate::visuals::SuperLaserVisual {
        crate::visuals::SuperLaserVisual {
            bone: Some("MuzzleA_00".into()),
            beam_texture: Some("tractor_beam01.tga".into()),
            beam_frames: 4,
            line_width: 250.0,
            line_color: Color::srgba_u8(255, 255, 255, 255),
            fire_seconds: 1.0,
            attack_variant: 0,
            fire_loop: Some("Unit_Death_Star_2_Fire".into()),
        }
    }

    /// Fires `laser` from `from` to `to` in an empty world (with a stand-in beam texture when `textured`, the loop held at least
    /// `sound_hold` seconds) and returns the world.
    fn shot(laser: crate::visuals::SuperLaserVisual, textured: bool, from: Vec3, to: Vec3, sound_hold: Option<f32>) -> (World, f32) {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<Image>>();
        let seconds = world
            .run_system_once(move |mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>, mut images: ResMut<Assets<Image>>| {
                let texture = textured.then(|| images.add(Image::default()));
                fire_super_laser(&mut commands, &mut meshes, &mut materials, Some(&laser), texture, from, to, 4000.0, sound_hold)
            })
            .expect("shot");
        (world, seconds)
    }

    /// The shot sounds of `world`: loop event, seconds it is held, position.
    fn shot_sounds(world: &mut World) -> Vec<(Option<String>, f32, Vec3)> {
        let mut q = world.query::<(&alamo_audio::SfxEmitter, &RemoveAfter, &Transform)>();
        q.iter(world).map(|(e, after, tf)| (e.event.clone(), after.0, tf.translation)).collect()
    }

    #[test]
    fn the_eclipse_shot_draws_its_data_beam_from_the_muzzle_and_plays_its_blast() {
        let (from, to) = (Vec3::new(100.0, 0.0, 0.0), Vec3::new(100.0, 0.0, -5000.0));
        let (mut world, seconds) = shot(eclipse_laser(), true, from, to, Some(3.3));
        assert_eq!(seconds, 1.0, "Fire_Continuation");
        let mut beams = world.query::<(&crate::effects::SpecialBeam, &crate::effects::BeamFrames, &Mesh3d, &MeshMaterial3d<StandardMaterial>, &Transform)>();
        let (beam, frames, mesh, material, tf) = beams.single(&world).expect("one textured beam");
        assert_eq!((beam.max_time, frames.0), (1.0, 4));
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(points)) = world.resource::<Assets<Mesh>>().get(&mesh.0).and_then(|m| m.attribute(Mesh::ATTRIBUTE_POSITION)).cloned() else {
            panic!("beam mesh")
        };
        let extent = |axis: usize| points.iter().map(|p| p[axis]).fold(f32::MIN, f32::max) - points.iter().map(|p| p[axis]).fold(f32::MAX, f32::min);
        assert!((extent(0) - 250.0).abs() < 1e-2 && (extent(2) - 250.0).abs() < 1e-2, "Line_Width");
        assert!((extent(1) - 5000.0).abs() < 1e-2, "muzzle to target");
        assert!((tf.translation - from).length() < 1e-3 && (tf.rotation * Vec3::Y).dot((to - from).normalize()) > 0.999);
        let m = world.resource::<Assets<StandardMaterial>>().get(&material.0).expect("beam material");
        assert!(m.base_color_texture.is_some() && m.alpha_mode == AlphaMode::Add && m.unlit);
        // The loop plays at the muzzle for one pass of its blast, longer than the 1 s beam.
        assert_eq!(shot_sounds(&mut world), [(Some("Unit_Death_Star_2_Fire".to_string()), 3.3, from)]);
    }

    #[test]
    fn a_shot_loop_plays_at_least_while_the_beam_fires_and_not_with_the_sound_off() {
        // Its sample not read yet: the loop is held while the beam fires.
        let (from, to) = (Vec3::ZERO, Vec3::new(0.0, 0.0, -9000.0));
        let (mut world, _) = shot(eclipse_laser(), true, from, to, Some(0.0));
        assert_eq!(shot_sounds(&mut world), [(Some("Unit_Death_Star_2_Fire".to_string()), 1.0, from)]);
        let (mut world, _) = shot(eclipse_laser(), true, from, to, None);
        assert!(shot_sounds(&mut world).is_empty());
    }

    #[test]
    fn a_super_laser_without_beam_art_keeps_the_plain_beam_and_gets_its_loop() {
        // The Death Star's data names no beam texture: the green beam line, now with `SFXEvent_Special_Ability_Loop`.
        let ds = crate::visuals::SuperLaserVisual { bone: Some("p_planet_deathstar_laser_large".into()), beam_texture: None, line_width: 0.0, fire_seconds: 0.25, ..eclipse_laser() };
        let (mut world, seconds) = shot(ds, false, Vec3::ZERO, Vec3::new(0.0, 0.0, -9000.0), Some(3.3));
        assert_eq!(seconds, SUPER_LASER_BEAM_SECONDS);
        let mut plain = world.query_filtered::<&crate::effects::SpecialBeam, Without<crate::effects::BeamFrames>>();
        assert_eq!(plain.iter(&world).count(), 2, "outer glow and core");
        assert_eq!(shot_sounds(&mut world), [(Some("Unit_Death_Star_2_Fire".to_string()), 3.3, Vec3::ZERO)]);
    }

    /// The Remake's data (skipped without it): what a super laser looks like comes from the unit, and the shooter's ATTACK clip runs
    /// the charge so that its `Fire_Frame` is the sim's shot.
    #[test]
    fn the_eclipse_charge_and_beam_come_from_the_remake_data() {
        use alamo_fs::FileSource;
        let Some(install) = alamo_fs::GameInstall::detect().filter(|i| i.mod_data("1770851727").is_dir()) else { return };
        let fs = alamo_fs::GameFs::for_mod(&install, Some("1770851727")).expect("game files");
        let (data, _) = alamo_data::load_galaxy(&fs);
        let catalog = battle_sim::Catalog::build(&data, Some(&fs), &["Eclipse_Star_Dreadnought"]);
        let id = catalog.ship_id("Eclipse_Star_Dreadnought").expect("Eclipse");
        let visuals = crate::visuals::VisualCatalog::build(&data, &catalog, vec![Color::WHITE]);
        assert_eq!(visuals.ship(id).super_laser, Some(eclipse_laser()));
        let fire_in = catalog.ship(id).abilities.iter().find_map(|a| a.super_laser.as_ref()).expect("SUPER_LASER").delay;
        let clip = alamo_model::Animation::parse(&fs.read("Art/Models/Empire_Eclipse_Super_Star_Destroyer_ATTACK_00.ALA").expect("ATTACK clip")).expect("clip");
        let shown = shown_bones(&clip);
        assert_eq!(shown, ["ambient_02", "ambient_03", "p_eclipse_charge_l", "p_eclipse_charge_s", "p_eclipse_star_dreadnaught_muzzle"]);
        let first_visible = |bone: &str| clip.track(bone).and_then(|t| t.visibility.as_ref()).and_then(|v| v.iter().position(|&on| on));
        // Frame 229 (`Fire_Frame`) of the clip is the sim's shot; the charge lattice shows from frame 51, the muzzle flare right after the shot.
        assert!((fire_in * clip.fps - 229.0).abs() < 1e-3, "fire_in {fire_in}");
        assert_eq!((first_visible("P_Eclipse_Charge_L"), first_visible("P_Eclipse_Star_Dreadnaught_Muzzle")), (Some(51), Some(230)));
    }
    #[test]
    fn a_charge_clip_lights_its_proxies_and_leaves_the_model_as_it_was() {
        use bevy::ecs::system::RunSystemOnce;
        // A charge bone with a particle proxy and a glow-mesh bone, both hidden at rest (`vis 0`), and a damage-fire bone the clip
        // keeps hidden.
        let mut world = World::new();
        let axis = world.spawn_empty().id();
        let charge = world.spawn((Name::new("P_Eclipse_Charge_L"), Visibility::Hidden)).id();
        let glow = world.spawn((Name::new("Ambient_03"), Visibility::Hidden)).id();
        let fire = world.spawn((Name::new("P_Ship_Fire_Huge"), Visibility::Hidden)).id();
        let mut ps = AlamoParticles::new("P_Eclipse_Charge_L");
        ps.enabled = false;
        let proxy = world.spawn((Name::new("P_Eclipse_Charge_L"), ps, ChildOf(charge))).id();
        let bone_by_name = [("p_eclipse_charge_l", charge), ("ambient_03", glow), ("p_ship_fire_huge", fire)].map(|(n, e)| (n.to_string(), e)).into();
        let root = world
            .spawn(AlamoModelLoaded { bones: vec![charge, glow, fire], parts: vec![], proxies: vec![proxy], axis, hardpoints: vec![], bone_by_name, center: Vec3::ZERO, radius: 1.0 })
            .id();
        let tracked: Vec<String> = ["ambient_03", "p_eclipse_charge_l", "p_ship_fire_huge"].map(String::from).to_vec();
        let shown: Vec<String> = ["ambient_03", "p_eclipse_charge_l"].map(String::from).to_vec();
        let touched = world
            .run_system_once(move |mut touch: ModelTouch| {
                let touched = touch.begin_clip(root, &tracked, &shown);
                touch.light_clip(&touched);
                touched
            })
            .expect("begin");
        assert!(world.get::<AlamoParticles>(proxy).is_some_and(|p| p.enabled), "the charge particles are on for the clip");
        // The clip shows its bones (`animate_models`), then ends.
        for b in [charge, glow] {
            *world.get_mut::<Visibility>(b).expect("bone") = Visibility::Inherited;
        }
        world.run_system_once(move |mut touch: ModelTouch| touch.end_clip(&touched)).expect("end");
        assert!(world.get::<AlamoParticles>(proxy).is_some_and(|p| !p.enabled));
        for b in [charge, glow, fire] {
            assert_eq!(world.get::<Visibility>(b), Some(&Visibility::Hidden), "no glow mesh stays lit after the shot");
        }
    }
    /// The view's half of a super laser shot, fed with the sim's events: the order (`SuperLaserCharging`) starts the shooter's charge
    /// clip, the shot (`SuperLaserFired`) draws the beam from the shooter and plays the loop there. No game files here: the plain beam,
    /// and the loop held while the beam fires (its sample is not read).
    #[test]
    fn the_super_laser_events_start_the_charge_then_draw_the_beam_and_play_its_loop() {
        use battle_sim::{Battle, BattleConfig, BattleSetup, Catalog, ProjDef, ShipDef, SideSetup, UnitSetup};
        use bevy::ecs::system::RunSystemOnce;
        let ship = |name: &str, class| ShipDef { name: name.into(), class, hull: 1.0e6, radius: 500.0, population: 1, ..ShipDef::default() };
        let catalog = std::sync::Arc::new(Catalog::from_defs(vec![ship("eclipse", TargetClass::Super), ship("target", TargetClass::Capital)], vec![ProjDef { name: "laser".into(), ..ProjDef::default() }]));
        let side = |n: &str| SideSetup { name: n.into(), units: vec![UnitSetup { ship: catalog.ship_id(n).unwrap(), count: 1, tag: 1 }], structures: vec![], ai: false };
        let sim = crate::sim::SimHandle::spawn(Battle::new(BattleSetup { sides: vec![side("eclipse"), side("target")], catalog: catalog.clone(), config: BattleConfig::default() }));
        sim.request(1);
        let cur = sim.fetch(0, true).0.expect("snapshot");
        let (shooter, target) = (cur.ships.iter().find(|s| s.side == 0).expect("shooter").clone(), cur.ships.iter().find(|s| s.side == 1).expect("target").clone());
        let mut ships = vec![crate::visuals::ShipVisual::default(); 2];
        ships[shooter.def as usize].super_laser = Some(eclipse_laser());
        let visuals = crate::visuals::VisualCatalog { ships, team_colors: vec![Color::WHITE; 2], proj_detonate: vec![None], proj_detonate_armor: vec![None], proj_detonate_shield: vec![None] };

        let mut world = World::new();
        world.init_resource::<crate::effects::BloomPulses>();
        world.init_resource::<crate::effects::BeamArt>();
        world.init_resource::<BattleViewSettings>();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<Image>>();
        let view = ShipView { id: shooter.id, def: shooter.def, side: 0, hp_applied: vec![], bones: None, hp_parts: vec![], attitude: Attitude::default(), born: 0.0, visual_radius: 0.0, shown: true };
        let e = world.spawn(view).id();
        world.insert_resource(BattleRuntime {
            sim,
            catalog,
            visuals: std::sync::Arc::new(visuals),
            dt: 1.0 / 30.0,
            clock: 0.0,
            cur: Some(cur.clone()),
            next: None,
            alpha: 0.0,
            fresh_events: vec![Event::SuperLaserCharging { ship: shooter.id, target: target.id, fire_in: 229.0 / 30.0 }],
            entities: HashMap::from([(shooter.id, e)]),
            dying: HashSet::new(),
            request: None,
            player_faction: None,
            start_alive: vec![1, 1],
            retreat_countdown: 0.0,
            unit_cap: 0,
            no_spawn_radius: 0.0,
            reinforce_clearance: 0.0,
            arena_half: 10_000.0,
            fog_of_war: false,
            speed: 1.0,
            framed: false,
            reported: false,
            minimap_planets: vec![],
            minimap_props: vec![],
            minimap_tint: Color::BLACK,
        });
        world.run_system_once(ship_events).expect("order");
        let charge = world.get::<SuperLaserCharge>(e).expect("the order starts the charge clip");
        assert_eq!((charge.start, charge.variant), (cur.time, 0), "the ATTACK clip `Attack_Anim_Sub_Index` 0 runs from the order");
        assert!(world.query::<&crate::effects::SpecialBeam>().iter(&world).next().is_none(), "no beam before the shot");

        world.resource_mut::<BattleRuntime>().fresh_events = vec![Event::SuperLaserFired { ship: shooter.id, target: target.id, at: target.pos }];
        world.run_system_once(ship_events).expect("shot");
        let mut beams = world.query::<&crate::effects::SpecialBeam>();
        assert_eq!(beams.iter(&world).count(), 2, "the beam: outer glow and core");
        let muzzle = sim_to_bevy(shooter.pos);
        assert_eq!(shot_sounds(&mut world), [(Some("Unit_Death_Star_2_Fire".to_string()), SUPER_LASER_BEAM_SECONDS, muzzle)]);
        // With the battle's sound off the beam still fires, silently.
        world.resource_mut::<BattleViewSettings>().audio = false;
        world.run_system_once(ship_events).expect("second shot");
        assert_eq!((beams.iter(&world).count(), shot_sounds(&mut world).len()), (4, 1));
    }
    /// The Remake's sound data (skipped without it): the Eclipse's shot loop is `Preset_INSANE_LOOP` with the 3.45 s
    /// `GUN_Superlaser_2.wav` blast, and the shot holds it one pass less its 0.05 s fade-out and the margin.
    #[test]
    fn the_eclipse_shot_plays_one_pass_of_its_blast_from_the_remake_data() {
        use bevy::ecs::system::RunSystemOnce;
        let Some(install) = alamo_fs::GameInstall::detect().filter(|i| i.mod_data("1770851727").is_dir()) else { return };
        let fs = std::sync::Arc::new(alamo_fs::GameFs::for_mod(&install, Some("1770851727")).expect("game files"));
        let mut audio = alamo_audio::plugin::AlamoAudio::load(fs);
        let event = audio.sfx.get("Unit_Death_Star_2_Fire").expect("the loop of the Eclipse's SUPER_LASER").clone();
        assert!(event.looping && !event.is_2d && event.loop_fade.1 == 0.05, "{event:?}");
        let mut sources = Assets::<AudioSource>::default();
        for name in &event.samples {
            assert!(audio.sample(name, 256 << 20, &mut sources).is_some(), "{name}");
        }
        let mut world = World::new();
        world.init_resource::<BattleViewSettings>();
        world.init_resource::<alamo_audio::AudioSettings>();
        world.insert_resource(audio);
        world.insert_resource(sources);
        let hold = world.run_system_once(|sounds: ShotSounds| sounds.hold("Unit_Death_Star_2_Fire")).expect("hold").expect("sound on");
        assert!((hold - (3.4535 - 0.05 - SHOT_SOUND_MARGIN)).abs() < 0.01, "{hold}");
    }
}
