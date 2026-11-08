//! Fleets on the galactic map: the flagship's model parked next to the planet or moving along its
//! hyperspace lane with a streak, owner-coloured rings, selection highlight and count labels.

use std::collections::{BTreeMap, HashMap};
use std::f32::consts::FRAC_PI_2;

use alamo_render::{AlamoFs, AlamoModel, AlamoModelLoaded};
use bevy::prelude::*;
use galaxy_sim::{FactionId, FleetId, PlanetId};

use crate::catalog::{FleetModelStyle, PlayCatalog};
use crate::geometry::{FleetPlace, GalaxyLayout, fleet_place, flagship};
use crate::state::{GameClock, PlayerFaction, PlayerSelection, SimState};

/// The human faction's fog of war, computed once per snapshot (`Snapshot::sight`). Reference video: planets stay visible
/// everywhere with their owner colour and name, but only own/allied fleets and fleets within sight are drawn (no enemy fleet
/// markers far from the player's territory); a fleet that is only `Seen` shows no composition ("?").
#[derive(Resource, Default)]
pub struct Fog {
    seq: Option<u64>,
    /// `None` for a spectator (everything is visible in detail).
    view: Option<crate::SightView>,
}

impl Fog {
    /// What the player sees of a fleet (`Hidden` = not drawn).
    pub fn fleet(&self, id: FleetId) -> galaxy_sim::Sight {
        match &self.view {
            None => galaxy_sim::Sight::Detailed,
            Some(v) => v.fleets.get(&id).copied().unwrap_or(galaxy_sim::Sight::Hidden),
        }
    }

    /// What the player sees of a planet (`Hidden` = owner and name only).
    pub fn planet(&self, p: PlanetId) -> galaxy_sim::Sight {
        match &self.view {
            None => galaxy_sim::Sight::Detailed,
            Some(v) => v.planets.get(p as usize).copied().unwrap_or(galaxy_sim::Sight::Hidden),
        }
    }

    /// What the player's spies reveal of a planet that is not its own (empty for a spectator, own planets and unspied ones).
    pub fn spied(&self, p: PlanetId) -> galaxy_sim::SpyFlags {
        self.view.as_ref().and_then(|v| v.spied.get(p as usize).copied()).unwrap_or_default()
    }

    /// Recomputes the view when the snapshot changed.
    pub fn refresh(&mut self, state: &SimState, player: Option<FactionId>) {
        if self.seq == Some(state.current.seq) {
            return;
        }
        self.seq = Some(state.current.seq);
        self.view = player.map(|me| state.current.sight(&state.world, me));
    }
}

/// Root entity of a fleet on the map.
#[derive(Component, Debug)]
pub struct FleetVisual {
    /// Simulation id.
    pub id: FleetId,
    /// Owning faction.
    pub owner: FactionId,
    /// Model file currently shown.
    pub model: Option<String>,
    /// Where the fleet is drawn, in world space (read by picking).
    pub heading: Vec3,
    /// Radius of the pad the fleet stands on (follows the planet radius, see `GalaxyLayout::pad_radius`).
    pub pad: f32,
}

/// The flat disc or rim of a fleet pad (unit radius, scaled to `FleetVisual::pad`).
#[derive(Component)]
pub struct FleetPad;

/// How a fleet root glides: corrections (a snapshot that disagrees with the extrapolation, the arrival in an orbit slot) are eased
/// out instead of snapping.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct FleetMotion {
    /// Where the latest snapshot put the fleet in the previous frame.
    pub target: Vec3,
    /// Drawn position minus `target`; decays to zero.
    pub offset: Vec3,
    /// Drawn orientation.
    pub rotation: Quat,
}

/// Time constant of the correction glide, real seconds.
const EASE_SECONDS: f32 = 0.12;
/// Jumps longer than this many planet units (a fleet retreating to another system) are not glided.
const SNAP_UNITS: f32 = 4.0;
/// Real seconds the map may run ahead of the latest snapshot on top of one tick (worker latency).
const MAX_LAG_SECONDS: f32 = 0.5;

impl FleetMotion {
    /// The drawn position for this frame. `target` is where the latest snapshot puts the fleet now and `was` where it puts it at
    /// the map time of the previous frame: if that is not where the fleet was drawn from, the difference is a correction that
    /// glides out over `EASE_SECONDS`. Jumps beyond `snap` are taken at once.
    pub fn step(&mut self, target: Vec3, was: Vec3, rotation: Quat, dt: f32, snap: f32) -> (Vec3, Quat) {
        let keep = (-dt.max(0.0) / EASE_SECONDS).exp();
        self.offset = (self.offset + self.target - was) * keep;
        if self.offset.length() > snap {
            self.offset = Vec3::ZERO;
        }
        self.target = target;
        self.rotation = self.rotation.slerp(rotation, 1.0 - keep);
        (target + self.offset, self.rotation)
    }
}

/// The model child of a [`FleetVisual`].
#[derive(Component)]
pub struct FleetModel;

/// Screen-space ship count label of a fleet.
#[derive(Component)]
pub struct FleetLabel(pub FleetId);

/// Fleet id to entity.
#[derive(Resource, Default)]
pub struct FleetEntities(pub HashMap<FleetId, Entity>);

/// Shared placeholder meshes for fleets whose model is unknown.
#[derive(Resource)]
pub struct FleetAssets {
    cone: Handle<Mesh>,
    /// Unit disc (flat in XZ after the child rotation) used as the base under fleets.
    disc: Handle<Mesh>,
    /// Thin coloured rim around the disc.
    rim: Handle<Mesh>,
}

pub(crate) fn setup_assets(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    commands.insert_resource(FleetAssets { cone: meshes.add(Cone::new(0.5, 1.4)), disc: meshes.add(Circle::new(1.0).mesh().resolution(48)), rim: meshes.add(Annulus::new(0.9, 1.0).mesh().resolution(48)) });
}

/// Colour of a faction on the map: the player's own is green like in the game, the others use their data colour.
pub fn map_color(world: &galaxy_sim::World, player: Option<FactionId>, faction: FactionId) -> Color {
    if player == Some(faction) {
        Color::srgb(0.15, 1.0, 0.25)
    } else {
        faction_color(world, faction)
    }
}

/// sRGB triple of a faction.
pub fn faction_color(world: &galaxy_sim::World, faction: FactionId) -> Color {
    let c = world.factions[faction as usize].color;
    Color::srgb_u8(c[0], c[1], c[2])
}

#[allow(clippy::too_many_arguments)]
fn spawn_model(commands: &mut Commands, assets: &FleetAssets, mats: &mut Assets<StandardMaterial>, fs: bool, model: Option<&str>, bow: bool, team: Color, parent: Entity) {
    match (model, fs) {
        (Some(m), true) => {
            let mut spec = AlamoModel::new(m).with_team_color(team);
            spec.bow_forward = bow;
            // Hidden until `fit_fleet_models` has scaled it: alamo-render spawns the parts in PostUpdate, after the fit, so a new fleet
            // showed its flagship at its own size for one frame. A hero Star Destroyer (Thrawn's Strikefast) filled the whole map:
            // the white flash seen in playtesting when recruiting a hero.
            commands.spawn((spec, Transform::default(), Visibility::Hidden, FleetModel, ChildOf(parent)));
        }
        _ => {
            let mat = mats.add(StandardMaterial { base_color: team, emissive: team.to_linear() * 0.6, ..default() });
            // The cone's tip points along +Y; lay it down so the tip faces -Z.
            commands.spawn((
                Mesh3d(assets.cone.clone()),
                MeshMaterial3d(mat),
                Transform::from_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
                FleetModel,
                ChildOf(parent),
            ));
        }
    }
}

/// Spawns, updates and removes fleet entities to match the latest snapshot and moves them.
#[allow(clippy::too_many_arguments)]
pub fn sync_fleets(
    mut commands: Commands,
    mut state: ResMut<SimState>,
    layout: Res<GalaxyLayout>,
    catalog: Res<PlayCatalog>,
    clock: Res<GameClock>,
    time: Res<Time>,
    assets: Res<FleetAssets>,
    alamo: Option<Res<AlamoFs>>,
    (cfg, player): (Res<crate::state::PlayConfig>, Res<PlayerFaction>),
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut entities: ResMut<FleetEntities>,
    mut fog: ResMut<Fog>,
    mut fleets: Query<(&mut FleetVisual, &mut Transform, &mut FleetMotion, &Children)>,
    models: Query<Entity, With<FleetModel>>,
    mut pads: Query<&mut Transform, (With<FleetPad>, Without<FleetVisual>)>,
    labels: Query<(Entity, &FleetLabel)>,
) {
    let dt = time.delta_secs().min(0.25);
    let rate = if state.current.human_battles.is_empty() { clock.rate() } else { 0.0 };
    let shown_before = state.view.time;
    let max_lead = state.world.config.tick_seconds + rate * MAX_LAG_SECONDS;
    let snapshot_time = state.current.time;
    let lead = state.view.advance(snapshot_time, dt, rate, max_lead);
    // Map time of the previous frame, relative to the latest snapshot.
    let lead_before = (shown_before - snapshot_time) as f32;
    let state = &*state;
    let world = &state.world;
    let g = state.galaxy();
    fog.refresh(state, player.0);
    let fog = &*fog;
    let shown = |id: &FleetId| fog.fleet(*id) != galaxy_sim::Sight::Hidden;

    // Removed fleets.
    entities.0.retain(|id, e| {
        if g.fleets.contains_key(id) && shown(id) {
            true
        } else {
            commands.entity(*e).despawn();
            false
        }
    });
    for (e, l) in &labels {
        if !g.fleets.contains_key(&l.0) || !shown(&l.0) {
            commands.entity(e).despawn();
        }
    }

    // Orbit slots: fleets parked at the same planet share a ring.
    let mut parked: BTreeMap<PlanetId, Vec<FleetId>> = BTreeMap::new();
    for f in g.fleets.values().filter(|f| shown(&f.id)) {
        if let Some(p) = f.planet() {
            parked.entry(p).or_default().push(f.id);
        }
    }
    let slot_of = |p: PlanetId, id: FleetId| -> usize { parked[&p].iter().position(|&x| x == id).unwrap_or(0) };
    // Root position, heading and pad radius of a fleet at a place.
    let draw_at = |f: &galaxy_sim::Fleet, place: FleetPlace| -> (Vec3, Vec3, f32) {
        match place {
            FleetPlace::Orbit(p) => {
                // Three fixed GUI fleet slots above the planet (no fleet limit in the data); extra fleets share a slot.
                // The fleet's own slot (the sim keeps three per faction and planet); fleets without one share by index.
                let i = slot_of(p, f.id);
                let (s, k) = if (1..=3).contains(&f.slot) { (f.slot as usize - 1, 0) } else { (i % 3, i / 3) };
                let a = slot_angle(s);
                let r = layout.orbit_radius(p) + k as f32 * layout.pad_radius(p) * 0.9;
                let c = layout.positions[p as usize];
                (c + Vec3::new(a.cos() * r, layout.pad_radius(p) * PAD_LIFT, a.sin() * r), Vec3::new(1.0, 0.0, 0.0), layout.pad_radius(p))
            }
            FleetPlace::Lane { from, to, t, .. } => {
                let (a, b) = (layout.positions[from as usize], layout.positions[to as usize]);
                // The pad grows or shrinks from the size at the planet left to the one at the next planet (no step at a stopover).
                let pad = layout.pad_radius(from) + (layout.pad_radius(to) - layout.pad_radius(from)) * t;
                (a.lerp(b, t) + Vec3::Y * pad * PAD_LIFT, (b - a).normalize_or_zero(), pad)
            }
        }
    };
    let snap = layout.unit * SNAP_UNITS;

    for f in g.fleets.values().filter(|f| shown(&f.id)) {
        let flag = flagship(world, f);
        let model = catalog.fleet_model(cfg.fleet_model_style, f.units.keys().copied(), flag).map(str::to_owned);
        let bow = cfg.fleet_model_style == FleetModelStyle::Flagship;
        let team = faction_color(world, f.owner);
        let (pos, heading, pad) = draw_at(f, fleet_place(world, g, f, lead));
        let tf = Transform::from_translation(pos).looking_to(if heading == Vec3::ZERO { Vec3::NEG_Z } else { heading }, Vec3::Y);
        match entities.0.get(&f.id) {
            Some(&e) => {
                if let Ok((mut v, mut t, mut motion, children)) = fleets.get_mut(e) {
                    let was = draw_at(f, fleet_place(world, g, f, lead_before)).0;
                    (t.translation, t.rotation) = motion.step(tf.translation, was, tf.rotation, dt, snap);
                    v.heading = heading;
                    v.pad = pad;
                    for &c in children {
                        if let Ok(mut pt) = pads.get_mut(c) {
                            pt.scale = Vec3::splat(pad);
                        }
                    }
                    if v.model != model {
                        v.model = model.clone();
                        for &c in children {
                            if models.contains(c) {
                                commands.entity(c).despawn();
                            }
                        }
                        spawn_model(&mut commands, &assets, &mut mats, alamo.is_some(), model.as_deref(), bow, team, e);
                    }
                }
            }
            None => {
                let root = commands
                    .spawn((
                        tf,
                        Visibility::default(),
                        FleetVisual { id: f.id, owner: f.owner, model: model.clone(), heading, pad },
                        FleetMotion { target: tf.translation, offset: Vec3::ZERO, rotation: tf.rotation },
                    ))
                    .id();
                spawn_model(&mut commands, &assets, &mut mats, alamo.is_some(), model.as_deref(), bow, team, root);
                // The flat translucent base disc the fleet stands on (faction coloured, like the original).
                let tl = map_color(world, player.0, f.owner).to_linear();
                commands.spawn((
                    Mesh3d(assets.disc.clone()),
                    MeshMaterial3d(mats.add(StandardMaterial {
                        base_color: Color::LinearRgba(LinearRgba::new(tl.red * 0.01, tl.green * 0.06, tl.blue * 0.02, 0.92)),
                        unlit: true,
                        depth_bias: 400.0,
                        alpha_mode: AlphaMode::Blend,
                        cull_mode: None,
                        ..default()
                    })),
                    Transform::from_xyz(0.0, -pad * (PAD_LIFT - 0.04), 0.0).with_rotation(Quat::from_rotation_x(-FRAC_PI_2)).with_scale(Vec3::splat(pad)),
                    FleetPad,
                    ChildOf(root),
                ));
                commands.spawn((
                    Mesh3d(assets.rim.clone()),
                    MeshMaterial3d(mats.add(StandardMaterial {
                        base_color: map_color(world, player.0, f.owner).with_alpha(0.85),
                        emissive: LinearRgba::new(tl.red, tl.green, tl.blue, 1.0) * 1.5,
                        unlit: true,
                        depth_bias: 402.0,
                        alpha_mode: AlphaMode::Blend,
                        cull_mode: None,
                        ..default()
                    })),
                    Transform::from_xyz(0.0, -pad * (PAD_LIFT - 0.05), 0.0).with_rotation(Quat::from_rotation_x(-FRAC_PI_2)).with_scale(Vec3::splat(pad)),
                    FleetPad,
                    ChildOf(root),
                ));
                entities.0.insert(f.id, root);
                commands.spawn((
                    Text::new(""),
                    TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(13.0), ..default() },
                    TextColor(team),
                    TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.0, 0.9) },
                    Node { position_type: PositionType::Absolute, ..default() },
                    Visibility::Hidden,
                    FleetLabel(f.id),
                ));
            }
        }
    }
}

/// Height of a fleet root above the planet plane in pad radii.
const PAD_LIFT: f32 = 0.3;
/// Height of a fleet model's centre above its fleet root, in pad radii of the unscaled root (the ship stands on its pad).
pub const FLEET_MODEL_LIFT: f32 = 0.75;

/// Scales freshly loaded fleet models to the icon size, centers them and shows them (they spawn hidden).
#[allow(clippy::type_complexity)]
pub fn fit_fleet_models(
    layout: Res<GalaxyLayout>,
    cfg: Res<crate::state::PlayConfig>,
    parents: Query<&FleetVisual>,
    mut loaded: Query<(&AlamoModelLoaded, &mut Transform, &mut Visibility, &ChildOf), (With<FleetModel>, Added<AlamoModelLoaded>)>,
) {
    for (m, mut tf, mut vis, parent) in &mut loaded {
        // The ship stands on its pad: about 2 pad radii of model radius (vanilla 05 shows a Star Destroyer about 1.25 pad diameters long;
        // our first render at 1.3 was too small to see). Measured on
        // vanilla 05 only. `fleet_icon_scale` 0.5 is the default and stays the tuning knob.
        let pad = parents.get(parent.parent()).map_or(layout.unit * 0.6, |v| v.pad);
        let target = pad * 2.0 * cfg.fleet_icon_scale * 2.0;
        let s = if m.radius > 0.0 { target / m.radius } else { 1.0 };
        tf.scale = Vec3::splat(s);
        tf.translation = -m.center * s + Vec3::Y * pad * FLEET_MODEL_LIFT;
        *vis = Visibility::Inherited;
    }
}

/// Owner rings, selection highlight and hyperspace streaks.
#[allow(clippy::too_many_arguments)]
pub fn draw_fleet_markers(
    mut gizmos: Gizmos,
    state: Res<SimState>,
    layout: Res<GalaxyLayout>,
    selection: Res<PlayerSelection>,
    player: Res<PlayerFaction>,
    cfg: Res<crate::state::PlayConfig>,
    time: Res<Time>,
    fleets: Query<(&FleetVisual, &Transform)>,
    drag: Res<crate::input::DragState>,
) {
    let world = &state.world;
    let g = state.galaxy();
    let _ = &cfg;
    let flat = Quat::from_rotation_x(-FRAC_PI_2);
    let me = player.0;
    // Thin orbit ellipse through the pads of every planet with a parked fleet of ours, and of the selected planet; its
    // empty pads are outlined (vanilla 05: dim green ellipse through the pads, empty pads only on the player's planet).
    let mut orbits: Vec<PlanetId> = g.fleets.values().filter(|f| Some(f.owner) == me).filter_map(|f| f.planet()).collect();
    if let Some(p) = selection.planet.filter(|&p| g.planets.get(p as usize).is_some_and(|pl| pl.owner == me)) {
        orbits.push(p);
        let (c, r, pad) = (layout.positions[p as usize], layout.orbit_radius(p), layout.pad_radius(p));
        for s in 0..3 {
            let a = slot_angle(s);
            let at = c + Vec3::new(a.cos() * r, 0.0, a.sin() * r);
            gizmos.circle(Isometry3d::new(at, Quat::from_rotation_x(-FRAC_PI_2)), pad, Color::srgba(0.3, 0.85, 0.4, 0.45));
        }
    }
    orbits.sort_unstable();
    orbits.dedup();
    for p in orbits {
        let c = layout.positions[p as usize];
        let col = map_color(world, me, me.unwrap_or(0)).with_alpha(0.3);
        gizmos.circle(Isometry3d::new(c, flat), layout.orbit_radius(p), col).resolution(64);
    }
    let pulse = 1.0 + 0.08 * (time.elapsed_secs() * 4.0).sin();
    for (v, tf) in &fleets {
        let Some(f) = g.fleets.get(&v.id) else { continue };
        if drag.dragging && drag.carry.contains(&v.id) {
            continue;
        }
        let team = map_color(world, player.0, v.owner);
        let base = tf.translation;
        let ring = v.pad;
        let center = Vec3::new(base.x, base.y - ring * (PAD_LIFT - 0.06), base.z);
        // The pad itself is the `FleetPad` mesh pair; a selected fleet gets a second, pulsing outer rim.
        if selection.fleets.contains(&v.id) {
            gizmos.circle(Isometry3d::new(center, flat), ring * 1.18 * pulse, Color::WHITE);
            gizmos.circle(Isometry3d::new(center, flat), ring * 1.3 * pulse, team);
        }
        // Heroes are drawn by the app as portraits on their own discs (hero_heads), no extra marker here.
        if let galaxy_sim::FleetLocation::Moving { .. } = f.location {
            // Hyperspace streak trailing behind the ship.
            let tail = base - v.heading * layout.unit * 5.0;
            let linear = team.to_linear();
            let bright = Color::LinearRgba(LinearRgba::new(linear.red * 3.0 + 0.4, linear.green * 3.0 + 0.4, linear.blue * 3.0 + 0.4, 1.0));
            gizmos.line_gradient(tail, base, Color::BLACK, bright);
            gizmos.line_gradient(tail + Vec3::Y * 0.01, base + Vec3::Y * 0.01, Color::BLACK, Color::WHITE.with_alpha(0.8));
        }
    }
}

/// The original shows no ship numbers on the map; flip to draw them again.
const SHOW_COUNTS: bool = false;

/// Positions the ship count labels in screen space.
pub fn update_fleet_labels(
    state: Res<SimState>,
    camera: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    entities: Res<FleetEntities>,
    transforms: Query<&Transform, With<FleetVisual>>,
    layout: Res<GalaxyLayout>,
    mut labels: Query<(&FleetLabel, &mut Text, &mut Node, &mut Visibility)>,
) {
    let Some((camera, cam_tf)) = camera.iter().next() else { return };
    let g = state.galaxy();
    for (label, mut text, mut node, mut vis) in &mut labels {
        let (Some(f), Some(&e)) = (g.fleets.get(&label.0), entities.0.get(&label.0)) else {
            *vis = Visibility::Hidden;
            continue;
        };
        let Ok(tf) = transforms.get(e) else { continue };
        let n = f.unit_count();
        let s = if n > 1 { format!("{n}") } else { String::new() };
        if text.0 != s {
            text.0 = s;
        }
        match camera.world_to_viewport(cam_tf, tf.translation - Vec3::new(0.0, 0.0, -layout.unit * 0.9)) {
            // The count lives in the tooltip; the map shows no numbers on the fleet discs.
            Ok(p) if n > 1 && SHOW_COUNTS => {
                node.left = Val::Px(p.x + 6.0);
                node.top = Val::Px(p.y);
                *vis = Visibility::Inherited;
            }
            _ => *vis = Visibility::Hidden,
        }
    }
}


/// Angle (in the XZ plane, -Z is screen up) of fleet slot `s` (0..3) around a planet: three bases in an arc above it.
pub fn slot_angle(s: usize) -> f32 {
    -FRAC_PI_2 + (s as f32 - 1.0) * 0.95
}

/// Gizmo group of the route lines: a soft wide glow under a bright core.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct RouteGizmos;

/// Gizmo group of the wide, faint halo under the route line.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct RouteGlowGizmos;

/// Line widths of the route gizmos (pixels).
pub fn configure_route_gizmos(mut store: ResMut<GizmoConfigStore>) {
    store.config_mut::<RouteGizmos>().0.line.width = 3.0;
    store.config_mut::<RouteGlowGizmos>().0.line.width = 9.0;
}

/// The planned route of the player's selected or moving fleets, as in the original: a yellow line along the
/// lanes, hop by hop, with marching dashes, plus a preview to the hovered planet while fleets are selected.
#[allow(clippy::too_many_arguments)]
pub fn draw_routes(
    mut core: Gizmos<RouteGizmos>,
    mut glow: Gizmos<RouteGlowGizmos>,
    state: Res<SimState>,
    layout: Res<GalaxyLayout>,
    selection: Res<PlayerSelection>,
    player: Res<PlayerFaction>,
    hover: Res<crate::input::PlayHover>,
    time: Res<Time>,
    fleets: Query<(&FleetVisual, &Transform)>,
) {
    let g = state.galaxy();
    let world = &state.world;
    let lift = Vec3::Y * 0.3;
    let phase = time.elapsed_secs() * 0.8;
    let mut draw = |pts: &[Vec3], alpha: f32| {
        let yellow = Color::srgba(1.0, 0.85, 0.1, alpha);
        let halo = Color::srgba(1.0, 0.75, 0.05, alpha * 0.10);
        glow.linestrip(pts.iter().copied(), halo);
        // Marching dashes: lengths in world units of the planet size.
        let dash = layout.unit * 1.6;
        let gap = layout.unit * 0.9;
        let period = dash + gap;
        let mut travelled = -(phase * period).rem_euclid(period);
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = a.distance(b);
            if len < 1e-3 {
                continue;
            }
            let dir = (b - a) / len;
            let mut s = travelled;
            while s < len {
                let (s0, s1) = (s.max(0.0), (s + dash).min(len));
                if s1 > s0 {
                    core.line(a + dir * s0, a + dir * s1, yellow);
                }
                s += period;
            }
            travelled += len;
        }
    };
    let mut previewed = false;
    for (v, tf) in &fleets {
        let Some(f) = g.fleets.get(&v.id) else { continue };
        if player.0 != Some(v.owner) {
            continue;
        }
        let selected = selection.fleets.contains(&v.id);
        match &f.location {
            galaxy_sim::FleetLocation::Moving { to, rest, .. } => {
                // Always shown for the player's moving fleets, brighter when selected.
                let mut pts = vec![tf.translation + lift, layout.positions[*to as usize] + lift];
                pts.extend(rest.iter().map(|&p| layout.positions[p as usize] + lift));
                draw(&pts, if selected { 1.0 } else { 0.55 });
            }
            galaxy_sim::FleetLocation::AtPlanet(at) if selected && !previewed => {
                if let Some(target) = hover.planet.filter(|t| t != at)
                    && let Some(plan) = crate::geometry::plan_route(world, g, v.owner, world.fleet_speed(f), *at, target)
                {
                    let pts: Vec<Vec3> = plan.path.iter().map(|&p| layout.positions[p as usize] + lift).collect();
                    draw(&pts, 0.9);
                    previewed = true;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use galaxy_sim::Sight;

    #[test]
    fn fog_hides_unlisted_fleets_and_defaults_to_open_for_spectators() {
        let spectator = Fog::default();
        assert_eq!(spectator.fleet(7), Sight::Detailed);
        assert_eq!(spectator.planet(3), Sight::Detailed);
        let view = crate::SightView { planets: vec![Sight::Detailed, Sight::Seen, Sight::Hidden], fleets: [(1, Sight::Detailed), (2, Sight::Seen)].into_iter().collect(), spied: Vec::new() };
        let fog = Fog { seq: Some(0), view: Some(view) };
        assert_eq!((fog.fleet(1), fog.fleet(2), fog.fleet(3)), (Sight::Detailed, Sight::Seen, Sight::Hidden));
        assert_eq!((fog.planet(1), fog.planet(2), fog.planet(9)), (Sight::Seen, Sight::Hidden, Sight::Hidden));
    }

    /// Stand-in for alamo-render, which finishes a model in PostUpdate, after the Update fit: here every model loads in its second
    /// frame, a hull of radius 800 like a Star Destroyer.
    fn finish_models(mut commands: Commands, models: Query<Entity, (With<AlamoModel>, Without<AlamoModelLoaded>)>, mut frame: Local<u32>) {
        *frame += 1;
        if *frame < 2 {
            return;
        }
        for e in &models {
            let axis = commands.spawn((Transform::default(), Visibility::Inherited, ChildOf(e))).id();
            commands.entity(e).insert(AlamoModelLoaded {
                bones: Vec::new(),
                parts: Vec::new(),
                proxies: Vec::new(),
                axis,
                hardpoints: Vec::new(),
                bone_by_name: Default::default(),
                center: Vec3::ZERO,
                radius: 800.0,
            });
        }
    }

    /// (shown, scale) of a new fleet's model in each of `frames` frames; `old` spawns it the way it was before (visible at once).
    fn new_fleet_model(frames: usize, old: bool) -> Vec<(bool, f32)> {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<StandardMaterial>>()
            .insert_resource(GalaxyLayout { positions: vec![Vec3::ZERO], radii: vec![1.0], unit: 1.0, index: Default::default(), pickable: Vec::new() })
            .insert_resource(crate::state::PlayConfig::default())
            .add_systems(Update, fit_fleet_models)
            .add_systems(PostUpdate, finish_models);
        let world = app.world_mut();
        let root = world.spawn((Transform::default(), Visibility::default(), FleetVisual { id: 1, owner: 0, model: None, heading: Vec3::X, pad: 2.0 })).id();
        if old {
            world.spawn((AlamoModel::new("strikefast.alo"), Transform::default(), FleetModel, ChildOf(root)));
        } else {
            let assets = FleetAssets { cone: Handle::default(), disc: Handle::default(), rim: Handle::default() };
            world.resource_scope(|world, mut mats: Mut<Assets<StandardMaterial>>| {
                spawn_model(&mut world.commands(), &assets, &mut mats, true, Some("strikefast.alo"), true, Color::WHITE, root);
            });
            world.flush();
        }
        (0..frames)
            .map(|_| {
                app.update();
                let (vis, tf) = app.world_mut().query_filtered::<(&Visibility, &Transform), With<FleetModel>>().single(app.world()).unwrap();
                (*vis != Visibility::Hidden, tf.scale.x)
            })
            .collect()
    }

    #[test]
    fn a_new_fleet_model_never_shows_at_its_own_size() {
        let frames = new_fleet_model(6, false);
        // Shown only once fitted to the icon size, and shown in the end.
        assert!(frames.iter().all(|&(shown, scale)| !shown || scale < 0.1), "{frames:?}");
        assert!(frames.last().is_some_and(|f| f.0), "{frames:?}");
        // Spawned visible, the model showed at scale 1 in the frame it loaded: the hull filled the map (the white flash).
        let old = new_fleet_model(6, true);
        assert!(old.iter().any(|&(shown, scale)| shown && scale == 1.0), "{old:?}");
    }

    #[test]
    fn corrections_glide_and_steady_motion_is_drawn_as_it_is() {
        let dt = 1.0 / 60.0;
        let mut m = FleetMotion { target: Vec3::ZERO, offset: Vec3::ZERO, rotation: Quat::IDENTITY };
        // Steady flight along x: drawn exactly where the snapshot puts it.
        for i in 1..=30 {
            let x = i as f32 * 0.1;
            let (p, _) = m.step(Vec3::X * x, Vec3::X * (x - 0.1), Quat::IDENTITY, dt, 4.0);
            assert!((p.x - x).abs() < 1e-5, "{p}");
        }
        // The fleet arrives and settles in an orbit slot one unit to the side: the map glides there, no frame step above a third of
        // the way, and it is there (within 1 %) after 0.6 s.
        let slot = Vec3::new(3.0, 0.0, 1.0);
        let mut drawn = vec![Vec3::X * 3.0];
        for _ in 0..36 {
            drawn.push(m.step(slot, slot, Quat::IDENTITY, dt, 4.0).0);
        }
        assert!(drawn.windows(2).all(|w| w[0].distance(w[1]) < 0.34), "{drawn:?}");
        assert!(drawn.last().unwrap().distance(slot) < 0.01, "{drawn:?}");
        // A retreat to another system is no glide across the map.
        let far = Vec3::new(40.0, 0.0, 0.0);
        assert_eq!(m.step(far, far, Quat::IDENTITY, dt, 4.0).0, far);
    }
}
