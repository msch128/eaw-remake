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
}

pub(crate) fn setup_assets(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    commands.insert_resource(FleetAssets { cone: meshes.add(Cone::new(0.5, 1.4)), disc: meshes.add(Circle::new(1.0).mesh().resolution(48)) });
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
            commands.spawn((spec, Transform::default(), FleetModel, ChildOf(parent)));
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
    state: Res<SimState>,
    layout: Res<GalaxyLayout>,
    catalog: Res<PlayCatalog>,
    clock: Res<GameClock>,
    time: Res<Time>,
    assets: Res<FleetAssets>,
    alamo: Option<Res<AlamoFs>>,
    cfg: Res<crate::state::PlayConfig>,
    player: Res<PlayerFaction>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut entities: ResMut<FleetEntities>,
    mut fleets: Query<(&mut FleetVisual, &mut Transform, &Children)>,
    models: Query<Entity, With<FleetModel>>,
    labels: Query<(Entity, &FleetLabel)>,
) {
    let world = &state.world;
    let g = state.galaxy();
    let now = time.elapsed_secs_f64();
    let rate = if state.current.human_battles.is_empty() { clock.rate() } else { 0.0 };
    let extra = state.extrapolation(now, rate);
    let t = time.elapsed_secs();

    // Removed fleets.
    entities.0.retain(|id, e| {
        if g.fleets.contains_key(id) {
            true
        } else {
            commands.entity(*e).despawn();
            false
        }
    });
    for (e, l) in &labels {
        if !g.fleets.contains_key(&l.0) {
            commands.entity(e).despawn();
        }
    }

    // Orbit slots: fleets parked at the same planet share a ring.
    let mut parked: BTreeMap<PlanetId, Vec<FleetId>> = BTreeMap::new();
    for f in g.fleets.values() {
        if let Some(p) = f.planet() {
            parked.entry(p).or_default().push(f.id);
        }
    }
    let slot_of = |p: PlanetId, id: FleetId| -> (usize, usize) {
        let v = &parked[&p];
        (v.iter().position(|&x| x == id).unwrap_or(0), v.len())
    };

    for f in g.fleets.values() {
        let flag = flagship(world, f);
        let model = catalog.fleet_model(cfg.fleet_model_style, f.units.keys().copied(), flag).map(str::to_owned);
        let bow = cfg.fleet_model_style == FleetModelStyle::Flagship;
        let team = faction_color(world, f.owner);
        let place = fleet_place(world, f, extra);
        let (pos, heading) = match place {
            FleetPlace::Orbit(p) => {
                let (i, n) = slot_of(p, f.id);
                // Three fixed GUI fleet slots above the planet (no fleet limit in the data); extra fleets share a slot.
                let _ = n;
                let (s, k) = (i % 3, i / 3);
                let a = slot_angle(s);
                let r = layout.orbit_radius(p) + k as f32 * layout.unit * 0.35;
                let c = layout.positions[p as usize];
                let pos = c + Vec3::new(a.cos() * r, layout.unit * 0.25, a.sin() * r);
                let _ = t;
                (pos, Vec3::new(1.0, 0.0, 0.0))
            }
            FleetPlace::Lane { from, to, t, .. } => {
                let (a, b) = (layout.positions[from as usize], layout.positions[to as usize]);
                (a.lerp(b, t) + Vec3::Y * layout.unit * 0.25, (b - a).normalize_or_zero())
            }
        };
        let tf = Transform::from_translation(pos).looking_to(if heading == Vec3::ZERO { Vec3::NEG_Z } else { heading }, Vec3::Y);
        match entities.0.get(&f.id) {
            Some(&e) => {
                if let Ok((mut v, mut t, children)) = fleets.get_mut(e) {
                    t.translation = tf.translation;
                    t.rotation = tf.rotation;
                    v.heading = heading;
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
                    .spawn((tf, Visibility::default(), FleetVisual { id: f.id, owner: f.owner, model: model.clone(), heading }))
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
                    Transform::from_xyz(0.0, -layout.unit * 0.12, 0.0).with_rotation(Quat::from_rotation_x(-FRAC_PI_2)).with_scale(Vec3::splat(layout.unit * 0.6)),
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

/// Scales freshly loaded fleet models to the icon size and centers them.
#[allow(clippy::type_complexity)]
pub fn fit_fleet_models(
    layout: Res<GalaxyLayout>,
    cfg: Res<crate::state::PlayConfig>,
    mut loaded: Query<(&AlamoModelLoaded, &mut Transform), (With<FleetModel>, Added<AlamoModelLoaded>)>,
) {
    for (m, mut tf) in &mut loaded {
        // Clearly larger than the base disc and floating above it, like the original.
        let target = layout.unit * cfg.fleet_icon_scale * 1.9;
        let s = if m.radius > 0.0 { target / m.radius } else { 1.0 };
        tf.scale = Vec3::splat(s);
        tf.translation = -m.center * s + Vec3::Y * layout.unit * 0.9;
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
) {
    let world = &state.world;
    let g = state.galaxy();
    let ring = layout.unit * cfg.fleet_icon_scale * 1.5;
    if let Some(p) = selection.planet {
        // Empty fleet slots of the selected planet stay visible as thin base circles.
        let c = layout.positions[p as usize];
        for s in 0..3 {
            let a = slot_angle(s);
            let r = layout.orbit_radius(p);
            let at = c + Vec3::new(a.cos() * r, 0.0, a.sin() * r);
            gizmos.circle(Isometry3d::new(at, Quat::from_rotation_x(-FRAC_PI_2)), layout.unit * 0.95, Color::srgba(0.4, 0.9, 0.5, 0.35));
        }
    }
    let flat = Quat::from_rotation_x(-FRAC_PI_2);
    let pulse = 1.0 + 0.08 * (time.elapsed_secs() * 4.0).sin();
    for (v, tf) in &fleets {
        let Some(f) = g.fleets.get(&v.id) else { continue };
        let team = map_color(world, player.0, v.owner);
        let base = tf.translation;
        let center = Vec3::new(base.x, base.y - layout.unit * 0.1, base.z);
        gizmos.circle(Isometry3d::new(center, flat), ring, team.with_alpha(1.0));
        if selection.fleets.contains(&v.id) {
            gizmos.circle(Isometry3d::new(center, flat), ring * 1.35 * pulse, Color::WHITE);
            gizmos.circle(Isometry3d::new(center, flat), ring * 1.5 * pulse, Color::srgb(0.6, 0.9, 1.0));
        } else if player.0 == Some(v.owner) {
            gizmos.circle(Isometry3d::new(center, flat), ring * 1.12, team.with_alpha(0.5));
        }
        if let galaxy_sim::FleetLocation::Moving { .. } = f.location {
            // Hyperspace streak trailing behind the ship.
            let tail = base - v.heading * layout.unit * 5.0;
            let linear = team.to_linear();
            let bright = Color::LinearRgba(LinearRgba::new(linear.red * 3.0 + 0.4, linear.green * 3.0 + 0.4, linear.blue * 3.0 + 0.4, 1.0));
            // Chevron markers along the lane behind the ship, as in the original.
            let side = Vec3::Y.cross(v.heading).normalize_or_zero();
            for k in 1..=3 {
                let tip = base - v.heading * layout.unit * (1.2 + 1.3 * k as f32);
                let back = tip - v.heading * layout.unit * 0.8;
                let w = layout.unit * 0.55;
                gizmos.linestrip([back + side * w, tip, back - side * w], team.with_alpha(1.0));
            }
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
