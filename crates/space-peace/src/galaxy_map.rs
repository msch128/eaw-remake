//! Rendering and interaction of the galactic conquest map: starfield, planets, owner halos,
//! trade routes, name labels and click selection.

use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::NoFrustumCulling,
    gizmos::{
        AppGizmoBuilder,
        config::{GizmoConfigGroup, GizmoConfigStore},
    },
    light::GlobalAmbientLight,
    mesh::{SphereKind, SphereMeshBuilder},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    window::PrimaryWindow,
};
use rand::{Rng, SeedableRng, rngs::SmallRng};

use crate::camera::{CameraBounds, GalaxyCamera};
use crate::flow::{AppScreen, UiSound, not_paused};
use crate::galaxy_view::{GalaxyView, LineStyleView, PropView};
use crate::planet_visual::{
    NEUTRAL, PlanetRadii, PlanetVisualAssets, PlanetVisualPlugin, rgb, spawn_planet_visual,
};
use alamo_render::{
    AlamoFs, AlamoModel, AlamoModelLoaded, material::AlamoMaterial, spawn_sun, texture::TextureCache,
};

/// Draws the [`GalaxyView`] resource and handles planet selection.
pub struct GalaxyMapPlugin;

impl Plugin for GalaxyMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PlanetVisualPlugin)
            .init_resource::<Selection>()
            .init_resource::<Hover>()
            .init_resource::<MapScale>()
            .init_gizmo_group::<RouteWide>()
            .init_gizmo_group::<RouteMid>()
            .init_gizmo_group::<RouteThin>()
            .init_gizmo_group::<RingGizmos>()
            .init_gizmo_group::<SelectGizmos>()
            .insert_resource(ClearColor(Color::BLACK))
            .insert_resource(GlobalAmbientLight {
                color: Color::srgb(0.55, 0.62, 0.85),
                brightness: 80.0,
                ..default()
            })
            .add_systems(Startup, setup_scene)
            .add_systems(OnEnter(AppScreen::Galaxy), spawn_galaxy)
            .add_systems(OnExit(AppScreen::Galaxy), despawn_galaxy)
            .add_systems(
                Update,
                (follow_camera_skybox, fix_star_dome.run_if(resource_exists::<AlamoFs>)),
            )
            .add_systems(
                Update,
                (
                    (update_hover, pick_planet).chain().run_if(not_paused),
                    update_labels,
                    draw_rings,
                    draw_routes,
                    log_props,
                    fade_galaxy_disc,
                )
                    .run_if(in_state(AppScreen::Galaxy)),
            );
    }
}

/// Gizmo group for wide trade routes (`Width` >= 4 in `TRADEROUTELINES.XML`).
#[derive(Default, Reflect, GizmoConfigGroup)]
struct RouteWide;
/// Gizmo group for medium trade routes.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct RouteMid;
/// Gizmo group for thin (capillary) trade routes.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct RouteThin;
/// Gizmo group for the owner rings under planets (constant pixel width).
#[derive(Default, Reflect, GizmoConfigGroup)]
struct RingGizmos;
/// Gizmo group for the selection ring.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct SelectGizmos;

/// The game's route colours are very dark additive teal (around 13, 31, 38) meant to sit on top
/// of the original galaxy backdrop art; this brightens them so they read on plain black space.
const ROUTE_GLOW: f32 = 5.0;

/// Currently selected planet (index into [`GalaxyView::planets`]).
#[derive(Resource, Default, Debug)]
pub struct Selection(pub Option<usize>);

/// Planet currently under the mouse cursor.
#[derive(Resource, Default, Debug)]
pub struct Hover(pub Option<usize>);

/// Marks everything spawned for the galaxy map; removed when leaving the screen.
#[derive(Component, Clone)]
pub struct GalaxyEntity;

/// Removes the galaxy scene and clears selection state.
fn despawn_galaxy(
    mut commands: Commands,
    entities: Query<Entity, With<GalaxyEntity>>,
    mut selection: ResMut<Selection>,
    mut hover: ResMut<Hover>,
) {
    for e in &entities {
        commands.entity(e).despawn();
    }
    selection.0 = None;
    hover.0 = None;
}

/// Spawns the decorative models (galaxy disc art with its star dome, nebulae, logos), each
/// tagged with `marker`.
pub fn spawn_props<M: Component + Clone>(commands: &mut Commands, props: &[PropView], marker: M) {
    for prop in props {
        commands.spawn((
            marker.clone(),
            PropModel(prop.id.clone()),
            AlamoModel::new(prop.model.clone()),
            Transform::from_translation(galaxy_to_world(prop.position))
                .with_scale(Vec3::splat(prop.scale)),
        ));
    }
}

/// Marks a decorative model entity (named after its data object).
#[derive(Component)]
struct PropModel(String);

/// Zoom as a fraction: 0 at the closest camera distance, 1 at the farthest.
fn zoom_fraction(bounds: &CameraBounds, distance: f32) -> f32 {
    ((distance - bounds.min_distance) / (bounds.max_distance - bounds.min_distance).max(1.0))
        .clamp(0.0, 1.0)
}

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A part of the galaxy disc art (core glow, region lines, grid) whose brightness fades with zoom.
#[derive(Component)]
struct DiscPart {
    material: Handle<AlamoMaterial>,
    /// Diffuse/colour of the material at full brightness.
    base: Vec4,
    /// Extra brightness factor (the grid is kept fainter than the glow).
    strength: f32,
}

/// Fades the galaxy disc out when zooming in: in EaW the disc and its grid belong to the
/// zoomed-out view, close up the map shows dark space with the planets.
fn fade_galaxy_disc(
    bounds: Res<CameraBounds>,
    camera: Single<&GalaxyCamera>,
    parts: Query<&DiscPart>,
    mut materials: ResMut<Assets<AlamoMaterial>>,
    mut last: Local<f32>,
) {
    let k = smoothstep(0.08, 0.5, zoom_fraction(&bounds, camera.distance));
    if (k - *last).abs() < 0.004 && *last != 0.0 {
        return;
    }
    *last = k.max(1e-4);
    for part in &parts {
        if let Some(mut m) = materials.get_mut(&part.material) {
            let f = k * part.strength;
            m.params.diffuse = Vec4::new(part.base.x * f, part.base.y * f, part.base.z * f, part.base.w);
        }
    }
}

/// Replaces the flat grey `Stars` dome of the galaxy art model by an unlit star texture.
///
/// The model's `Stars` mesh only has an emissive colour and a base texture (a star field with
/// nebulae); the Alamo shader would draw it as a uniform grey, so it gets its own material. Only
/// the far inside of the dome is drawn, which gives the galaxy its starry backdrop.
#[allow(clippy::too_many_arguments)]
fn fix_star_dome(
    mut commands: Commands,
    fs: Res<AlamoFs>,
    mut textures: ResMut<TextureCache>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    props: Query<&AlamoModelLoaded, (With<PropModel>, Added<AlamoModelLoaded>)>,
    names: Query<&Name>,
    handles: Query<&MeshMaterial3d<AlamoMaterial>>,
    alamo_materials: Res<Assets<AlamoMaterial>>,
) {
    for model in &props {
        for &part in &model.parts {
            let name = names.get(part).map(|n| n.as_str().to_ascii_lowercase()).unwrap_or_default();
            if matches!(name.as_str(), "core" | "region_lines" | "grid")
                && let Ok(h) = handles.get(part)
                && let Some(m) = alamo_materials.get(&h.0)
                && m.params.diffuse.w < 1.5
            {
                // Only the big disc (several thousand units wide) fades; the small galaxies keep
                // their look.
                if model.radius > 500.0 {
                    commands.entity(part).insert(DiscPart {
                        material: h.0.clone(),
                        base: m.params.diffuse,
                        strength: if name == "grid" { 0.6 } else { 1.0 },
                    });
                }
            }
            if names.get(part).is_ok_and(|n| n.as_str() == "Stars")
                && let Some(tex) = textures.get(&fs.0, &mut images, "w_star_bkgnd.dds")
            {
                let material = materials.add(StandardMaterial {
                    // The texture holds gamma-space colours; this approximates the game's look.
                    base_color: Color::linear_rgb(0.55, 0.55, 0.55),
                    base_color_texture: Some(tex),
                    unlit: true,
                    cull_mode: Some(bevy::render::render_resource::Face::Back),
                    ..default()
                });
                commands
                    .entity(part)
                    .remove::<MeshMaterial3d<AlamoMaterial>>()
                    .insert(MeshMaterial3d(material));
            }
        }
    }
}

/// Logs the size of decorative models once they have loaded.
fn log_props(
    props: Query<(&PropModel, &AlamoModelLoaded, &Transform), Added<AlamoModelLoaded>>,
) {
    for (prop, model, tf) in &props {
        debug!(
            "prop {}: radius {:.1} x scale {:.2} = {:.1}, center {:?}",
            prop.0,
            model.radius,
            tf.scale.x,
            model.radius * tf.scale.x,
            model.center
        );
    }
}

/// Marks a planet's screen-space name label.
#[derive(Component)]
struct PlanetLabel(usize);

/// Marks the sky sphere that follows the camera.
#[derive(Component)]
struct Skybox;

/// Converts EaW galactic coordinates (x/y plane, z up) to Bevy world space (y up).
pub fn galaxy_to_world(p: Vec3) -> Vec3 {
    Vec3::new(p.x, p.z, -p.y)
}

/// Scale of the loaded galaxy, derived from the data so any map size works.
#[derive(Resource, Debug, Clone, Copy)]
pub struct MapScale {
    /// World-space radius of a planet with `size == 1.0`.
    pub unit: f32,
}

impl Default for MapScale {
    fn default() -> Self {
        Self { unit: 4.5 }
    }
}

/// World-space radius of a planet sphere.
pub fn planet_radius(scale: &MapScale, size: f32) -> f32 {
    scale.unit * size
}

/// Median distance from a planet to its nearest neighbour (galaxy plane), a robust density measure.
fn median_nearest_neighbour(points: &[Vec2]) -> f32 {
    let mut nn: Vec<f32> = points
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            points
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, q)| p.distance(*q))
                .min_by(f32::total_cmp)
        })
        .collect();
    nn.sort_by(f32::total_cmp);
    nn.get(nn.len() / 2).copied().unwrap_or(30.0)
}

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut gizmo_store: ResMut<GizmoConfigStore>,
    view: Res<GalaxyView>,
    alamo: Option<Res<AlamoFs>>,
) {
    // One sun for the whole galaxy, from the game's galactic light settings.
    let a = view.light.angle_deg;
    let direction = Quat::from_euler(
        EulerRot::YXZ,
        a.z.to_radians(),
        -a.y.to_radians(),
        0.0,
    ) * Vec3::NEG_Z;
    if alamo.is_some() {
        // The Alamo shaders take a unit-less 0..1 sun strength.
        spawn_sun(&mut commands, direction, view.light.level.max(1.0), Color::WHITE);
    } else {
        commands.spawn((
            DirectionalLight {
                illuminance: 14_000.0,
                color: Color::srgb(1.0, 0.96, 0.9),
                ..default()
            },
            Transform::IDENTITY.looking_to(direction, Vec3::Y),
        ));
    }

    // Starfield: a large inside-out sphere with a procedural equirectangular texture.
    let sky_tex = images.add(make_starfield(6144, 3072));
    let sky_mat = materials.add(StandardMaterial {
        base_color: Color::linear_rgb(1.5, 1.5, 1.5),
        base_color_texture: Some(sky_tex),
        unlit: true,
        cull_mode: Some(bevy::render::render_resource::Face::Back),
        ..default()
    });
    commands.spawn((
        Mesh3d(
            meshes.add(
                SphereMeshBuilder::new(
                    8000.0,
                    SphereKind::Uv {
                        sectors: 64,
                        stacks: 32,
                    },
                )
                .build(),
            ),
        ),
        MeshMaterial3d(sky_mat),
        Skybox,
        NoFrustumCulling,
        Transform::default(),
    ));

    // Route line widths in pixels per width tier.
    gizmo_store.config_mut::<RouteWide>().0.line.width = 3.2;
    gizmo_store.config_mut::<RouteMid>().0.line.width = 2.0;
    gizmo_store.config_mut::<RouteThin>().0.line.width = 1.2;
    gizmo_store.config_mut::<RingGizmos>().0.line.width = 1.6;
    gizmo_store.config_mut::<SelectGizmos>().0.line.width = 2.6;
}

/// Spawns planets, labels and the selection ring from the [`GalaxyView`] resource.
fn spawn_galaxy(
    mut commands: Commands,
    view: Res<GalaxyView>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut bounds: ResMut<CameraBounds>,
    mut scale: ResMut<MapScale>,
    alamo: Option<Res<AlamoFs>>,
) {
    // Derive world scale from the planet density: sphere radius, camera range, label range.
    let flat: Vec<Vec2> = view
        .planets
        .iter()
        .map(|p| {
            let w = galaxy_to_world(p.position);
            Vec2::new(w.x, w.z)
        })
        .collect();
    if flat.len() > 1 {
        scale.unit = (0.15 * median_nearest_neighbour(&flat)).clamp(0.5, 200.0);
    }
    let scale = *scale;

    let assets = PlanetVisualAssets::new(&mut meshes, &mut materials);
    let use_models = alamo.is_some();
    // Radius estimates; models replace them with their real radius once loaded.
    let radii: Vec<f32> = view.planets.iter().map(|p| planet_radius(&scale, p.size)).collect();
    for (i, planet) in view.planets.iter().enumerate() {
        let radius = radii[i];
        let pos = galaxy_to_world(planet.position);
        let root =
            spawn_planet_visual(&mut commands, &assets, &mut materials, &view, i, pos, radius, use_models);
        commands.entity(root).insert(GalaxyEntity);

        // Name label in the owner's colour (positioned and decluttered every frame).
        let owner = planet
            .owner
            .and_then(|o| view.factions.get(o))
            .map_or(NEUTRAL, |f| f.color);
        let label_c = [
            owner[0] * 0.65 + 0.35,
            owner[1] * 0.65 + 0.35,
            owner[2] * 0.65 + 0.35,
        ];
        commands.spawn((
            Text::new(planet.display_name.clone()),
            TextFont {
                font_size: FontSize::Px(14.0),
                ..default()
            },
            TextColor(rgb(label_c)),
            TextShadow {
                offset: Vec2::splat(1.5),
                color: Color::srgba(0.0, 0.0, 0.0, 0.9),
            },
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Visibility::Hidden,
            PlanetLabel(i),
            GalaxyEntity,
        ));
    }

    // Decorative models: galaxy disc art, nebulae, faction logos, background planets.
    if use_models {
        spawn_props(&mut commands, &view.props, GalaxyEntity);
    }
    commands.insert_resource(PlanetRadii(radii));
    commands.insert_resource(assets);

    // Camera bounds: galaxy extent plus a margin.
    if !view.planets.is_empty() {
        let (mut min, mut max) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for p in &view.planets {
            let w = galaxy_to_world(p.position);
            min = min.min(Vec2::new(w.x, w.z));
            max = max.max(Vec2::new(w.x, w.z));
        }
        let margin = Vec2::splat(scale.unit * 4.0);
        bounds.min = min - margin;
        bounds.max = max + margin;
        let extent = (max - min).max_element().max(scale.unit * 10.0);
        bounds.min_distance = scale.unit * 9.0;
        bounds.max_distance = extent * 1.6;
        bounds.initial_distance = extent * 1.55;
        bounds.dirty = true;
    }
}

/// One label candidate for the screen-space declutter pass.
#[derive(Clone, Copy)]
struct LabelSlot {
    /// Top-left of the label rectangle in logical viewport pixels.
    pos: Vec2,
    size: Vec2,
    score: f32,
}

#[allow(clippy::too_many_arguments)]
/// Positions the label of each planet below its sphere and hides labels that would overlap a
/// more important one (selected, hovered, player-owned, then by income).
fn update_labels(
    view: Res<GalaxyView>,
    radii: Res<PlanetRadii>,
    selection: Res<Selection>,
    hover: Res<Hover>,
    bounds: Res<CameraBounds>,
    camera: Single<(&Camera, &GlobalTransform, &GalaxyCamera)>,
    mut labels: Query<(&PlanetLabel, &mut Node, &mut Visibility, &ComputedNode, &mut TextFont)>,
    mut slots: Local<Vec<Option<LabelSlot>>>,
    mut visible: Local<Vec<bool>>,
) {
    let (camera, cam_tf, orbit) = *camera;
    let Some(viewport) = camera.logical_viewport_size() else {
        return;
    };
    // Labels grow when zoomed in and shrink when zoomed out (whole pixel steps only).
    let zoom_out = zoom_fraction(&bounds, orbit.distance);
    let font_px = (22.0 + (13.0 - 22.0) * smoothstep(0.0, 0.6, zoom_out)).round();
    slots.clear();
    slots.resize(view.planets.len(), None);

    for (label, _, _, computed, mut font) in &mut labels {
        if font.font_size != FontSize::Px(font_px) {
            font.font_size = FontSize::Px(font_px);
        }
        let planet = &view.planets[label.0];
        let radius = radii.0[label.0];
        let below = galaxy_to_world(planet.position) + Vec3::new(0.0, 0.0, radius * 2.3);
        let Ok(p) = camera.world_to_viewport(cam_tf, below) else {
            continue;
        };
        let measured = computed.size() * computed.inverse_scale_factor();
        let size = if measured.x > 1.0 {
            measured
        } else {
            Vec2::new(planet.display_name.len() as f32 * font_px * 0.6, font_px * 1.3)
        };
        let score = if selection.0 == Some(label.0) {
            1.0e9
        } else if hover.0 == Some(label.0) {
            1.0e8
        } else {
            let mine = planet.owner.is_some() && planet.owner == view.player_faction;
            (if mine { 1.0e6 } else { 0.0 }) + planet.income as f32 + planet.size
        };
        slots[label.0] = Some(LabelSlot {
            pos: Vec2::new(p.x - size.x * 0.5, p.y),
            size,
            score,
        });
    }

    // Greedy placement in priority order.
    let mut order: Vec<usize> = (0..slots.len()).filter(|&i| slots[i].is_some()).collect();
    order.sort_by(|&a, &b| {
        slots[b]
            .map_or(0.0, |s| s.score)
            .total_cmp(&slots[a].map_or(0.0, |s| s.score))
    });
    visible.clear();
    visible.resize(slots.len(), false);
    // Planet discs are obstacles too, so labels do not sit on top of other planets.
    let right = cam_tf.right();
    let discs: Vec<Option<(Vec2, f32)>> = view
        .planets
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let c = galaxy_to_world(p.position);
            let r = radii.0[i] * 1.45;
            let a = camera.world_to_viewport(cam_tf, c).ok()?;
            let b = camera.world_to_viewport(cam_tf, c + right * r).ok()?;
            Some((a, a.distance(b)))
        })
        .collect();
    let mut placed: Vec<(Vec2, Vec2)> = Vec::with_capacity(order.len());
    const MARGIN: f32 = 4.0;
    for i in order {
        let s = slots[i].expect("filtered above");
        let (lo, hi) = (s.pos - Vec2::splat(MARGIN), s.pos + s.size + Vec2::splat(MARGIN));
        let on_screen = lo.x >= 0.0 && lo.y >= 0.0 && hi.x <= viewport.x && hi.y <= viewport.y;
        let overlaps = placed
            .iter()
            .any(|&(plo, phi)| lo.x < phi.x && hi.x > plo.x && lo.y < phi.y && hi.y > plo.y);
        let on_planet = discs.iter().enumerate().any(|(j, d)| {
            j != i
                && d.is_some_and(|(c, r)| {
                    let nearest = c.clamp(lo, hi);
                    nearest.distance_squared(c) < r * r
                })
        });
        // The selected planet always keeps its label.
        let selected = selection.0 == Some(i);
        if selected || (on_screen && !overlaps && !on_planet) {
            placed.push((lo, hi));
            visible[i] = true;
        }
    }

    for (label, mut node, mut vis, _, _) in &mut labels {
        match slots[label.0] {
            Some(s) if visible[label.0] => {
                node.left = Val::Px(s.pos.x);
                node.top = Val::Px(s.pos.y);
                *vis = Visibility::Inherited;
            }
            _ => *vis = Visibility::Hidden,
        }
    }
}

/// Draws the owner ring under every planet and the pulsing selection ring. Gizmo lines keep a
/// constant pixel width, so the rings stay thin at any zoom.
fn draw_rings(
    time: Res<Time>,
    view: Res<GalaxyView>,
    radii: Res<PlanetRadii>,
    selection: Res<Selection>,
    mut rings: Gizmos<RingGizmos>,
    mut select: Gizmos<SelectGizmos>,
) {
    let flat = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    for (i, p) in view.planets.iter().enumerate() {
        let pos = galaxy_to_world(p.position);
        let (c, alpha) = match p.owner.and_then(|o| view.factions.get(o)) {
            Some(f) => (f.color, 0.85),
            None => (NEUTRAL, 0.3),
        };
        let l = rgb(c).to_linear();
        let color = Color::LinearRgba(LinearRgba::new(l.red, l.green, l.blue, alpha));
        rings
            .circle(Isometry3d::new(pos, flat), radii.0[i] * 1.3, color)
            .resolution(48);
    }
    if let Some(i) = selection.0.filter(|&i| i < view.planets.len()) {
        let pulse = 1.0 + 0.05 * (time.elapsed_secs() * 3.0).sin();
        let pos = galaxy_to_world(view.planets[i].position);
        select
            .circle(Isometry3d::new(pos, flat), radii.0[i] * 1.65 * pulse, Color::WHITE)
            .resolution(64);
    }
}
/// Relation of a trade route to the human player, derived from the owners of its end points.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RouteRelation {
    /// Both ends belong to the player.
    Own,
    /// Both ends belong to the same other faction.
    Enemy,
    /// The ends belong to different factions (an active front line).
    Contested,
    /// At least one end is unowned.
    Neutral,
}

/// Classifies a route from the owners of its two end points.
pub fn route_relation(a: Option<usize>, b: Option<usize>, player: Option<usize>) -> RouteRelation {
    match (a, b) {
        (Some(x), Some(y)) if x == y && Some(x) == player => RouteRelation::Own,
        (Some(x), Some(y)) if x == y => RouteRelation::Enemy,
        (Some(_), Some(_)) => RouteRelation::Contested,
        _ => RouteRelation::Neutral,
    }
}

/// Linear colour of a line style at zoom `t` (0 = close, 1 = far), as additive-looking RGB.
fn route_color(style: &LineStyleView, t: f32) -> Option<Color> {
    let mix = |i: usize| style.color_near[i] + (style.color_far[i] - style.color_near[i]) * t;
    let alpha = mix(3);
    if style.width <= 0.0 || alpha <= 0.01 {
        return None;
    }
    let l = Color::srgb(mix(0), mix(1), mix(2)).to_linear();
    let k = alpha * ROUTE_GLOW;
    Some(Color::LinearRgba(LinearRgba::new(l.red * k, l.green * k, l.blue * k, 1.0)))
}

/// Draws all trade routes with the styles from `TRADEROUTELINES.XML`, picked by the route's
/// relation to the player and blended between the zoomed-in and zoomed-out colours.
fn draw_routes(
    view: Res<GalaxyView>,
    bounds: Res<CameraBounds>,
    camera: Single<&GalaxyCamera>,
    mut wide: Gizmos<RouteWide>,
    mut mid: Gizmos<RouteMid>,
    mut thin: Gizmos<RouteThin>,
) {
    let t = ((camera.distance - bounds.min_distance)
        / (bounds.max_distance - bounds.min_distance).max(1.0))
    .clamp(0.0, 1.0);
    for r in &view.routes {
        let (Some(a), Some(b)) = (view.planets.get(r.a), view.planets.get(r.b)) else {
            continue;
        };
        let Some(style) = view.route_styles.get(r.style) else {
            continue;
        };
        let line = match route_relation(a.owner, b.owner, view.player_faction) {
            RouteRelation::Own => &style.own,
            RouteRelation::Enemy => &style.enemy,
            RouteRelation::Contested | RouteRelation::Neutral => &style.neutral,
        };
        let Some(color) = route_color(line, t) else {
            continue;
        };
        let (pa, pb) = (galaxy_to_world(a.position), galaxy_to_world(b.position));
        if line.width >= 4.0 {
            wide.line(pa, pb, color);
        } else if line.width >= 1.5 {
            mid.line(pa, pb, color);
        } else {
            thin.line(pa, pb, color);
        }
    }
}

/// Finds the planet under the mouse cursor (ray vs. a generous pick sphere).
fn update_hover(
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform)>,
    view: Res<GalaxyView>,
    radii: Res<PlanetRadii>,
    mut hover: ResMut<Hover>,
    mut sounds: MessageWriter<UiSound>,
) {
    let Some(cursor) = window.cursor_position() else {
        hover.set_if_neq_none();
        return;
    };
    let (camera, cam_tf) = *camera;
    let Ok(ray) = camera.viewport_to_world(cam_tf, cursor) else {
        return;
    };
    let cam_pos = cam_tf.translation();

    let mut best: Option<(usize, f32)> = None;
    for (i, p) in view.planets.iter().enumerate() {
        let center = galaxy_to_world(p.position);
        // Generous pick radius that never gets smaller than a few pixels on screen.
        let r = (radii.0[i] * 1.5).max(center.distance(cam_pos) * 0.012);
        if let Some(t) = ray_sphere(ray.origin, *ray.direction, center, r)
            && best.is_none_or(|(_, bt)| t < bt)
        {
            best = Some((i, t));
        }
    }
    let new = best.map(|(i, _)| i);
    if hover.0 != new {
        hover.0 = new;
        if new.is_some() {
            sounds.write(UiSound(crate::menu::SFX_PLANET_HOVER));
        }
    }
}

impl Hover {
    /// Clears the hover without touching change detection when already empty.
    fn set_if_neq_none(&mut self) {
        if self.0.is_some() {
            self.0 = None;
        }
    }
}

/// Click (without dragging) selects the hovered planet; clicking empty space clears it.
fn pick_planet(
    buttons: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    hover: Res<Hover>,
    mut selection: ResMut<Selection>,
    mut press: Local<Option<Vec2>>,
    mut sounds: MessageWriter<UiSound>,
) {
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    if buttons.just_pressed(MouseButton::Left) {
        *press = Some(cursor);
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    // Ignore drags.
    if press.take().is_none_or(|p| p.distance(cursor) > 6.0) {
        return;
    }
    if hover.0.is_some() && hover.0 != selection.0 {
        sounds.write(UiSound(crate::menu::SFX_PLANET_SELECT));
    }
    selection.0 = hover.0;
}

/// Distance along the ray to the first intersection with the sphere, if any.
fn ray_sphere(origin: Vec3, dir: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let oc = origin - center;
    let b = oc.dot(dir);
    let c = oc.length_squared() - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    (t >= 0.0)
        .then_some(t)
        .or_else(|| (-b + disc.sqrt() >= 0.0).then_some(0.0))
}

/// Keeps the sky sphere centred on the camera so it never parallaxes.
fn follow_camera_skybox(
    camera: Single<&GlobalTransform, (With<GalaxyCamera>, Without<Skybox>)>,
    mut sky: Single<&mut Transform, With<Skybox>>,
) {
    sky.translation = camera.translation();
}

/// Generates an equirectangular starfield with a few faint coloured nebulae.
fn make_starfield(width: u32, height: u32) -> Image {
    let mut rng = SmallRng::seed_from_u64(0x5EED_57A2);
    let (w, h) = (width as usize, height as usize);
    let mut data = vec![0u8; w * h * 4];

    // Low-resolution value noise for nebulae, bilinearly upsampled.
    let (nw, nh) = (48usize, 24usize);
    let noise: Vec<f32> = (0..nw * nh).map(|_| rng.random::<f32>()).collect();
    let sample = |u: f32, v: f32| -> f32 {
        let (x, y) = (u * nw as f32, v * (nh - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (fx, fy) = (x - x.floor(), y - y.floor());
        let g = |xx: usize, yy: usize| noise[(yy.min(nh - 1)) * nw + (xx % nw)];
        let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
        let top = g(x0, y0) * (1.0 - sx) + g(x0 + 1, y0) * sx;
        let bot = g(x0, y0 + 1) * (1.0 - sx) + g(x0 + 1, y0 + 1) * sx;
        top * (1.0 - sy) + bot * sy
    };
    for y in 0..h {
        let v = y as f32 / h as f32;
        for x in 0..w {
            let u = x as f32 / w as f32;
            let n = ((sample(u, v) - 0.5) * 2.0).max(0.0).powf(1.8);
            let m = ((sample((u * 2.0 + 0.37) % 1.0, 1.0 - v) - 0.5) * 2.0)
                .max(0.0)
                .powf(1.8);
            let i = (y * w + x) * 4;
            data[i] = (n * 22.0 + m * 6.0) as u8;
            data[i + 1] = (n * 10.0 + m * 12.0) as u8;
            data[i + 2] = (n * 30.0 + m * 26.0) as u8;
            data[i + 3] = 255;
        }
    }

    // Stars: soft gaussian dots with varying brightness and colour temperature.
    let count = w * h / 900;
    for _ in 0..count {
        let (cx, cy) = (rng.random_range(0..w) as i32, rng.random_range(0..h) as i32);
        let b: f32 = rng.random::<f32>().powf(5.0);
        let peak = 0.25 + 0.75 * b;
        let sigma = 0.8 + 1.0 * b;
        let tint: f32 = rng.random();
        let col = [0.75 + 0.25 * tint, 0.85, 1.0 - 0.3 * tint];
        let r = (sigma * 3.0).ceil() as i32;
        for dy in -r..=r {
            for dx in -r..=r {
                let (px, py) = ((cx + dx).rem_euclid(w as i32) as usize, cy + dy);
                if !(0..h as i32).contains(&py) {
                    continue;
                }
                let f = peak * (-((dx * dx + dy * dy) as f32) / (2.0 * sigma * sigma)).exp();
                let i = (py as usize * w + px) * 4;
                for (k, c) in col.iter().enumerate() {
                    data[i + k] = data[i + k].saturating_add((f * c * 255.0) as u8);
                }
            }
        }
    }

    Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_relations() {
        let p = Some(1);
        assert_eq!(route_relation(Some(1), Some(1), p), RouteRelation::Own);
        assert_eq!(route_relation(Some(2), Some(2), p), RouteRelation::Enemy);
        assert_eq!(route_relation(Some(1), Some(2), p), RouteRelation::Contested);
        assert_eq!(route_relation(None, Some(2), p), RouteRelation::Neutral);
        assert_eq!(route_relation(Some(1), Some(1), None), RouteRelation::Enemy);
    }
}