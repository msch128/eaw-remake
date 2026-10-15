//! Rendering and interaction of the galactic conquest map: starfield, planets, owner halos,
//! trade routes, name labels and click selection.

use std::collections::HashMap;

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
use crate::flow::{AppScreen, UiSound, cursor_free, not_paused};
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
            .init_resource::<ReservedBottom>()
            .init_resource::<PanelRect>()
            .init_resource::<Wrecks>()
            .init_resource::<DestroyedPlanets>()
            .init_resource::<MapScale>()
            .init_resource::<MapFilters>()
            .init_gizmo_group::<RingGizmos>()
            .init_gizmo_group::<SelectGizmos>()
            .init_gizmo_group::<TerritoryGizmos>()
            .insert_resource(ClearColor(Color::BLACK))
            .insert_resource(GlobalAmbientLight {
                color: Color::srgb(0.55, 0.62, 0.85),
                brightness: 2.0,
                ..default()
            })
            .init_resource::<CaptureAnimations>()
            .add_systems(Update, apply_filter_spans)
            .add_systems(Update, (update_pip_colors, place_sector_fill, scripted_hover.after(update_hover)).run_if(in_state(AppScreen::Galaxy)))
            .add_systems(Startup, setup_scene)
            .add_systems(OnEnter(AppScreen::Galaxy), spawn_galaxy)
            .add_systems(
                OnTransition { exited: AppScreen::Galaxy, entered: AppScreen::MainMenu },
                despawn_galaxy,
            )
            .add_systems(
                OnTransition { exited: AppScreen::Galaxy, entered: AppScreen::CampaignSelect },
                despawn_galaxy,
            )
            .add_systems(
                Update,
                (follow_camera_skybox, fade_nebula_plane.run_if(resource_exists::<CameraBounds>), fix_star_dome.run_if(resource_exists::<AlamoFs>)),
            )
            .add_systems(
                Update,
                (
                    (update_hover, pick_planet).chain().run_if(not_paused.and_then(cursor_free)),
                    update_labels,
                    draw_rings,
                    update_lanes,
                    draw_trade_route_pulses,
                    update_planet_captures,
                    toggle_map_overlays_hotkey,
                    place_notch_fill,
                    fill_emblems,
                    fill_pip_icons,
                    log_props,
                    sync_wrecks,
                    fade_galaxy_disc,
                )
                    .run_if(in_state(AppScreen::Galaxy)),
            );
    }
}

/// The four galactic information filters next to the minimap (`b_filters0..3` of the command bar).
/// Each is a toggle: credit value of planets (`b_filters0`), space station level and ground slots (`b_filters1`),
/// special ability / weather (`b_filters2`), heroes on top of the unit stack (`b_filters3`).
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct MapFilters {
    pub credits: bool,
    pub base_level: bool,
    pub weather: bool,
    pub heroes: bool,
    pub military: bool,
}

impl Default for MapFilters {
    fn default() -> Self {
        // All filters start off like in the Remake frames (no "+income" next to the planet names until the credit filter is on).
        Self { credits: false, base_level: false, weather: false, heroes: false, military: false }
    }
}

impl MapFilters {
    pub fn cycle_overlays(&mut self) {
        if !self.credits && !self.military {
            self.credits = true;
            self.military = false;
        } else if self.credits {
            self.credits = false;
            self.military = true;
        } else {
            self.credits = false;
            self.military = false;
        }
    }
}

/// Hotkey `I` cycles through galactic information overlays: Normal -> Economic -> Military -> Normal.
fn toggle_map_overlays_hotkey(
    keys: Res<ButtonInput<KeyCode>>,
    mut filters: ResMut<MapFilters>,
) {
    if keys.just_pressed(KeyCode::KeyI) {
        filters.cycle_overlays();
    }
}

/// Which filter shows a text span of a planet label.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum FilterSpan {
    Credits,
    Ability,
}

/// Shows or hides the filtered parts of the planet labels (spans are made transparent).
fn apply_filter_spans(filters: Res<MapFilters>, mut spans: Query<(&FilterSpan, &mut TextColor, &mut TextFont)>) {
    for (kind, mut color, mut font) in &mut spans {
        let (on, c) = match kind {
            FilterSpan::Credits => (filters.credits, Color::srgb(0.6, 0.85, 1.0)),
            FilterSpan::Ability => (filters.weather, Color::srgb(1.0, 0.85, 0.45)),
        };
        color.set_if_neq(TextColor(if on { c } else { Color::NONE }));
        // The label's text shadow would still show a transparent span, so hidden spans shrink to nothing.
        let size = FontSize::Px(if on { 9.0 } else { 0.01 });
        if font.font_size != size {
            font.font_size = size;
        }
    }
}

/// Gizmo group for the owner rings under planets (constant pixel width).
#[derive(Default, Reflect, GizmoConfigGroup)]
struct RingGizmos;
/// Gizmo group for the selection ring.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct SelectGizmos;


/// Bottom strip of the viewport (logical pixels) covered by the command bar; labels avoid it.
#[derive(Resource, Default, Debug)]
pub struct ReservedBottom(pub f32);

/// Screen rectangles (viewport pixels) the name plates avoid: `.0` the HUD panels, `.1` the hero portraits next to fleets.
#[derive(Resource, Default, Debug)]
pub struct PanelRect(pub Vec<(Vec2, Vec2)>, pub Vec<(Vec2, Vec2)>);

/// Currently selected planet (index into [`GalaxyView::planets`]).
#[derive(Resource, Default, Debug)]
pub struct Selection(pub Option<usize>);

/// Planet currently under the mouse cursor.
#[derive(Resource, Default, Debug)]
pub struct Hover(pub Option<usize>);

/// Marks everything spawned for the galaxy map; removed when leaving the screen.
#[derive(Component, Clone)]
pub struct GalaxyEntity;

/// Planets drawn as wrecks (index into the view) with the entity of the wreck model.
#[derive(Resource, Default, Debug)]
pub struct Wrecks(pub std::collections::HashMap<usize, Entity>);

/// Destroyed planets (superweapon) lose their sphere and show the asteroid remnant of the galactic map.
#[allow(clippy::too_many_arguments)]
fn sync_wrecks(
    mut commands: Commands,
    mut wrecks: ResMut<Wrecks>,
    view: Res<GalaxyView>,
    destroyed: Res<DestroyedPlanets>,
    mut visuals: Query<(&crate::planet_visual::PlanetVisual, &mut Visibility)>,
    alamo: Option<Res<AlamoFs>>,
    alive: Query<(), With<GalaxyEntity>>,
) {
    wrecks.0.retain(|_, e| alive.contains(*e));
    for &i in &destroyed.0 {
        if wrecks.0.contains_key(&i) {
            continue;
        }
        let Some(p) = view.planets.get(i) else { continue };
        for (pv, mut vis) in &mut visuals {
            if pv.0 == i {
                *vis = Visibility::Hidden;
            }
        }
        let pos = galaxy_to_world(p.position);
        let e = if alamo.is_some() {
            commands.spawn((AlamoModel::new("W_Planet_Asteroids.alo"), Transform::from_translation(pos).with_scale(Vec3::splat(p.model_scale.max(0.1))), GalaxyEntity)).id()
        } else {
            commands.spawn((Transform::from_translation(pos), Visibility::default(), GalaxyEntity)).id()
        };
        wrecks.0.insert(i, e);
    }
}

/// Planets (view indices) that were destroyed; filled from the simulation or by `--force-wreck`.
#[derive(Resource, Default, Debug, PartialEq)]
pub struct DestroyedPlanets(pub Vec<usize>);

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
    added: Query<(), Added<DiscPart>>,
    mut materials: ResMut<Assets<AlamoMaterial>>,
    mut last: Local<f32>,
) {
    // The Remake keeps the galaxy disc (maroon nebula, orbit rings) visible at every zoom level.
    let _ = (&bounds, &camera);
    let k = 1.0;
    if added.is_empty() && (k - *last).abs() < 0.004 && *last != 0.0 {
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
                        // Reference look: orbit lines at full strength, the core glow dimmed to a maroon haze, no grid.
                        strength: match name.as_str() { "grid" => 0.0, "core" => 0.14, _ => 0.9 },
                    });
                }
            }
            if names.get(part).is_ok_and(|n| n.as_str() == "Stars")
                && let Some(tex) = textures.get(&fs.0, &mut images, "w_star_bkgnd.dds")
            {
                let material = materials.add(StandardMaterial {
                    // The texture holds gamma-space colours; this approximates the game's look.
                    base_color: Color::linear_rgb(0.12, 0.12, 0.12),
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
pub(crate) struct Skybox;

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
    scale.unit * size * 0.5
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
        // The Alamo shaders take a unit-less 0..1 sun strength (0.55 left the ships on the fleet pads dark next to the Remake frames; 0.7 is a guess).
        spawn_sun(&mut commands, direction, view.light.level.clamp(0.3, 1.0) * 0.7, Color::WHITE);
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
    gizmo_store.config_mut::<RingGizmos>().0.line.width = 1.6;
    gizmo_store.config_mut::<SelectGizmos>().0.line.width = 1.5;
    gizmo_store.config_mut::<TerritoryGizmos>().0.line.width = 7.0;
}

/// Spawns planets, labels and the selection ring from the [`GalaxyView`] resource.
#[allow(clippy::too_many_arguments)]
fn spawn_galaxy(
    mut commands: Commands,
    view: Res<GalaxyView>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut bounds: ResMut<CameraBounds>,
    mut scale: ResMut<MapScale>,
    alamo: Option<Res<AlamoFs>>,
    existing: Query<(), With<GalaxyEntity>>,
    mut images: ResMut<Assets<Image>>,
) {
    // Coming back from a tactical battle the scene is still there.
    if !existing.is_empty() {
        return;
    }
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
    let pip_images = [images.add(pip_image(true)), images.add(pip_image(false))];
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
        let owner = relation_color(&view, planet.owner);
        // The game prints the name in capitals in the full, saturated faction colour.
        let label_c = if planet.owner.is_some() { owner } else { [0.62, 0.62, 0.62] };
        // Row 0 = orbital space station tiers (up to max tier of this planet); Row 1 = planetary ground slots.
        for (row, count) in [planet.max_space_base.clamp(0, 5), planet.land_slots.clamp(0, 8)].into_iter().enumerate() {
            let count = count as usize;
            if count == 0 {
                continue;
            }
            commands
                .spawn((
                    Node { position_type: PositionType::Absolute, flex_direction: FlexDirection::Row, column_gap: Val::Px(1.0), ..default() },
                    Visibility::Hidden,
                    PipRow { planet: i, row, count },
                    GalaxyEntity,
                ))
                .with_children(|p| {
                    for slot_idx in 0..count {
                        p.spawn((
                            ImageNode::new(pip_images[row].clone()).with_color(rgb(label_c).with_alpha(0.9)),
                            Node { width: Val::Px(8.0), height: Val::Px(8.0), ..default() },
                            PipIcon { space: row == 0, planet: i, slot: slot_idx },
                        ));
                    }
                });
        }
        // Faction emblem beside owned planets (every planet gets one; it follows the owner and hides without one).
        commands.spawn((
            ImageNode::default().with_color(Color::NONE),
            Node { position_type: PositionType::Absolute, width: Val::Px(10.0), height: Val::Px(10.0), ..default() },
            Visibility::Hidden,
            Emblem(i),
            GalaxyEntity,
        ));
        let label = commands.spawn((
            Text::new(planet.display_name.to_uppercase()),
            TextFont {
                font: FontSource::Family("Bahnschrift".into()),
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
        )).id();
        if planet.income > 0 {
            // Small income number after the name, like the original ("+90").
            commands.spawn((
                TextSpan::new(format!(" +{}", planet.income)),
                TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(9.0), ..default() },
                TextColor(Color::srgb(0.6, 0.85, 1.0)),
                FilterSpan::Credits,
                ChildOf(label),
            ));
        }
        if let Some((name, _)) = &planet.ability {
            commands.spawn((
                TextSpan::new(format!("  {name}")),
                TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(9.0), ..default() },
                TextColor(Color::NONE),
                FilterSpan::Ability,
                ChildOf(label),
            ));
        }
    }

    // Decorative models: galaxy disc art, nebulae, faction logos, background planets.
    if use_models {
        spawn_props(&mut commands, &view.props, GalaxyEntity);
    }
    spawn_lanes(&mut commands, &view, &scale, &mut meshes, &mut materials, &mut images);
    // The Remake video shows plain black space outside the galaxy core (maroon only over the core, which the disc art
    // provides), so the extra plane stays off.
    if NEBULA_PLANE {
        spawn_nebula_plane(&mut commands, &view, &mut meshes, &mut materials, &mut images);
    }
    commands.spawn((
        Name::new("sector fill"),
        Mesh3d(meshes.add(Circle::new(1.0).mesh().resolution(96))),
        MeshMaterial3d(materials.add(StandardMaterial { base_color: Color::srgba(0.9, 0.1, 0.1, 0.08), unlit: true, alpha_mode: AlphaMode::Blend, cull_mode: None, depth_bias: -500.0, ..default() })),
        Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        Visibility::Hidden,
        bevy::camera::visibility::NoFrustumCulling,
        SectorFill,
        GalaxyEntity,
    ));
    commands.spawn((
        Name::new("selection ring fill"),
        Mesh3d(meshes.add(notch_mesh())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.35, 1.0, 0.45, 0.38),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        })),
        Transform::default(),
        Visibility::Hidden,
        bevy::camera::visibility::NoFrustumCulling,
        NotchFill,
        GalaxyEntity,
    ));
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
    (hover, filters): (Res<Hover>, Res<MapFilters>),
    bounds: Res<CameraBounds>,
    ui_scale: Res<UiScale>,
    reserved: Res<ReservedBottom>,
    panel: Res<PanelRect>,
    camera: Single<(&Camera, &GlobalTransform, &GalaxyCamera)>,
    mut labels: Query<(&PlanetLabel, &mut Node, &mut Visibility, &ComputedNode, &mut TextFont, &mut TextColor), Without<PipRow>>,
    mut pip_rows: Query<(&PipRow, &mut Node, &mut Visibility), Without<PlanetLabel>>,
    mut emblems: Query<(&Emblem, &mut Node, &mut Visibility), (Without<PlanetLabel>, Without<PipRow>, Without<PipIcon>)>,
    mut pip_icons: Query<&mut Node, (With<PipIcon>, Without<PlanetLabel>, Without<PipRow>)>,
    mut last_font: Local<f32>,
    mut slots: Local<Vec<Option<LabelSlot>>>,
    mut visible: Local<Vec<bool>>,
) {
    let (camera, cam_tf, orbit) = *camera;
    let Some(viewport) = camera.logical_viewport_size() else {
        return;
    };
    // Labels grow when zoomed in and shrink when zoomed out (whole pixel steps only).
    let zoom_out = zoom_fraction(&bounds, orbit.distance);
    // estimate: the Remake labels are about 1.5% of the screen height close up (f0016), smaller than before.
    let font_px = (11.0 + (8.0 - 11.0) * smoothstep(0.0, 0.7, zoom_out)).round();
    // UI coordinates are multiplied by the UI scale; the layout below works in viewport pixels.
    let us = ui_scale.0.max(0.1);
    let font_node_px = (font_px / us).round().max(1.0);
    slots.clear();
    slots.resize(view.planets.len(), None);

    for (label, _, _, computed, mut font, _) in &mut labels {
        if font.font_size != FontSize::Px(font_node_px) {
            font.font_size = FontSize::Px(font_node_px);
        }
        let planet = &view.planets[label.0];
        let radius = radii.0[label.0];
        // Straight under the sphere on the screen at any camera tilt: the sphere's centre plus its pixel radius downwards.
        let centre = galaxy_to_world(planet.position);
        let (Ok(a), Ok(b)) = (camera.world_to_viewport(cam_tf, centre), camera.world_to_viewport(cam_tf, centre + cam_tf.right() * radius)) else {
            continue;
        };
        let p = a + Vec2::new(0.0, a.distance(b) * 0.95);
        let measured = computed.size() * computed.inverse_scale_factor() * us;
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
            // Planets near the screen centre (the camera target) win over far ones, so that names show at the default zoom.
            let centre = (1.0 - ((p - viewport * 0.5).length() / (viewport.length() * 0.5)).min(1.0)).powi(2) * 4000.0;
            (if mine { 1.0e6 } else { 0.0 }) + centre + planet.income as f32 + planet.size
        };
        slots[label.0] = Some(LabelSlot {
            pos: Vec2::new(p.x - size.x * 0.5, p.y),
            // Room for the two rows of structure slot pips under the name.
            size: size + Vec2::new(0.0, font_px * 1.1),
            score,
        });
    }

    // The label colour follows the owner (a captured planet turns green).
    for (label, _, _, _, _, mut color) in &mut labels {
        // Brighter when zoomed in, softer when zoomed out.
        let k = 0.72 + 0.28 * (1.0 - zoom_out.min(1.0));
        let c = rgb(relation_color(&view, view.planets[label.0].owner)).to_srgba();
        color.set_if_neq(TextColor(Color::srgb((c.red * k).min(1.0), (c.green * k).min(1.0), (c.blue * k).min(1.0))));
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
    const MARGIN: f32 = 2.0;
    let close = zoom_out < 0.45;
    for i in order {
        let s = slots[i].expect("filtered above");
        let (lo, hi) = (s.pos - Vec2::splat(MARGIN), s.pos + s.size + Vec2::splat(MARGIN));
        // Close up every planet in view keeps its name: only its anchor has to be on screen, and crowding never hides it.
        let anchor = s.pos + Vec2::new(s.size.x * 0.5, 0.0);
        let on_screen = if close {
            anchor.x >= 0.0 && anchor.y >= 0.0 && anchor.x <= viewport.x && anchor.y <= viewport.y - reserved.0
        } else {
            lo.x >= 0.0 && lo.y >= 0.0 && hi.x <= viewport.x && hi.y <= viewport.y - reserved.0
        };
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
        let behind_panel = panel.0.iter().chain(panel.1.iter()).any(|(plo, phi)| lo.x < phi.x && hi.x > plo.x && lo.y < phi.y && hi.y > plo.y);
        let selected = selection.0 == Some(i);
        if selected || (on_screen && !behind_panel && (close || (!overlaps && (!on_planet || slots[i].is_some_and(|s| s.score >= 1.0e6))))) {
            placed.push((lo, hi));
            visible[i] = true;
        }
    }

    for (label, mut node, mut vis, _, _, _) in &mut labels {
        match slots[label.0] {
            Some(s) if visible[label.0] => {
                node.left = Val::Px(s.pos.x / us);
                node.top = Val::Px(s.pos.y / us);
                *vis = Visibility::Inherited;
            }
            _ => *vis = Visibility::Hidden,
        }
    }

    for (e, mut node, mut vis) in &mut emblems {
        match slots.get(e.planet_index()).copied().flatten() {
            Some(s) if visible[e.planet_index()] && (zoom_out < 0.3 || selection.0 == Some(e.planet_index())) => {
                node.left = Val::Px((s.pos.x - 14.0 * us) / us);
                node.top = Val::Px((s.pos.y - font_px * 0.4) / us);
                *vis = Visibility::Inherited;
            }
            _ => *vis = Visibility::Hidden,
        }
    }

    // Structure slot pips: space row (towers) above the ground row (troopers), centred under the name, as in the game.
    let pip_h = (font_px * 0.95).round().max(6.0);
    let pip_w = pip_h;
    if (*last_font - font_px).abs() > 0.1 {
        *last_font = font_px;
        for mut n in &mut pip_icons {
            n.width = Val::Px(pip_w / us);
            n.height = Val::Px(pip_h / us);
        }
    }
    for (row, mut node, mut vis) in &mut pip_rows {
        match slots.get(row.planet).copied().flatten() {
            Some(s) if visible[row.planet] && (filters.base_level || zoom_out < 0.3 || selection.0 == Some(row.planet) || hover.0 == Some(row.planet)) => {
                let width = row.count as f32 * (pip_w + 1.0);
                node.left = Val::Px((s.pos.x + (s.size.x - width) * 0.5) / us);
                node.top = Val::Px((s.pos.y + font_px * 1.15 + (1 - row.row) as f32 * (pip_h + 2.0)) / us);
                *vis = Visibility::Inherited;
            }
            _ => *vis = Visibility::Hidden,
        }
    }
}

/// Draws the owner ring under every planet and the pulsing selection ring. Gizmo lines keep a
/// constant pixel width, so the rings stay thin at any zoom.
/// Tracks planet ownership change animations and conquest celebration crests.
#[derive(Resource, Default)]
pub struct CaptureAnimations {
    /// Planet index -> (old_color, new_color, start_time, duration)
    pub transitions: HashMap<usize, (Color, Color, f32, f32)>,
    /// Last seen owner per planet index
    pub last_owners: HashMap<usize, Option<usize>>,
    /// Planet index -> celebration start time
    pub celebrations: HashMap<usize, f32>,
}

fn update_planet_captures(
    view: Res<GalaxyView>,
    time: Res<Time>,
    mut anim: ResMut<CaptureAnimations>,
    mut select: Gizmos<SelectGizmos>,
    radii: Res<PlanetRadii>,
) {
    let now = time.elapsed_secs();
    let flat = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);

    for (i, p) in view.planets.iter().enumerate() {
        let prev = anim.last_owners.get(&i).copied();
        if let Some(old) = prev {
            if old != p.owner {
                let old_c = Color::from(rgb(relation_color(&view, old)));
                let new_c = Color::from(rgb(relation_color(&view, p.owner)));
                anim.transitions.insert(i, (old_c, new_c, now, 1.0));
                anim.celebrations.insert(i, now);
                anim.last_owners.insert(i, p.owner);
            }
        } else {
            anim.last_owners.insert(i, p.owner);
        }
    }

    anim.transitions.retain(|_, (_, _, start, dur)| now - *start < *dur);

    anim.celebrations.retain(|&i, start| {
        let age = now - *start;
        if age >= 2.0 || i >= view.planets.len() {
            return false;
        }
        let progress = age / 2.0;
        let alpha = (1.0 - progress).powi(2);
        let pos = galaxy_to_world(view.planets[i].position) + Vec3::Y * (0.8 + progress * 0.6);
        let base_r = radii.0[i] * 1.6;
        let r = base_r * (1.0 + progress * 0.8);
        let gold = Color::srgba(1.0, 0.82, 0.18, alpha * 0.95);
        let bright_gold = Color::srgba(1.0, 0.95, 0.5, alpha);

        select.circle(Isometry3d::new(pos, flat), r, gold).resolution(48);
        select.circle(Isometry3d::new(pos, flat), r * 0.7, bright_gold).resolution(36);

        let spike_len = r * 0.35;
        for k in 0..8 {
            let theta = k as f32 / 8.0 * std::f32::consts::TAU + now * 1.5;
            let dir = Vec3::new(theta.cos(), 0.0, theta.sin());
            select.line(pos + dir * (r * 0.7), pos + dir * (r * 0.7 + spike_len), gold);
        }
        true
    });
}

fn draw_rings(
    time: Res<Time>,
    view: Res<GalaxyView>,
    radii: Res<PlanetRadii>,
    selection: Res<Selection>,
    hover: Res<Hover>,
    anim: Res<CaptureAnimations>,
    filters: Res<MapFilters>,
    sim: Option<Res<galaxy_play::SimState>>,
    mut rings: Gizmos<RingGizmos>,
    mut select: Gizmos<SelectGizmos>,
    mut border: Gizmos<TerritoryGizmos>,
    mut territory: Local<(u64, Vec<(Vec3, Vec3, Color)>)>,
) {
    let flat = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    // The Remake draws no grid around the selected planet (CS9VdxglewY f0019), only the notched ring and the dashed territory.
    let owners_hash = view.planets.iter().enumerate().fold(selection.0.map_or(0, |s| s as u64 + 7), |h, (i, p)| h.wrapping_mul(1_000_003).wrapping_add(p.owner.map_or(0, |o| o as u64 + 1) + i as u64));
    if territory.0 != owners_hash {
        territory.1 = territory_segments(&view, &radii, selection.0);
        territory.0 = owners_hash;
    }
    for (a, b, c) in territory.1.iter() {
        border.line(*a, *b, *c);
    }
    for (i, p) in view.planets.iter().enumerate() {
        let pos = galaxy_to_world(p.position);
        let hovered = hover.0 == Some(i);
        let in_transition = anim.transitions.get(&i).map(|(c1, c2, start, dur)| {
            let frac = ((time.elapsed_secs() - start) / dur).clamp(0.0, 1.0);
            c1.mix(c2, frac)
        });
        if let Some(color) = in_transition {
            rings
                .circle(Isometry3d::new(pos, flat), radii.0[i] * 1.35, color.with_alpha(0.9))
                .resolution(48);
        } else {
            let (c, alpha) = match p.owner {
                Some(_) if hovered => (relation_color(&view, p.owner), 0.8),
                Some(_) => continue,
                None if hovered => (NEUTRAL, 0.7),
                None => continue,
            };
            let l = rgb(c).to_linear();
            let color = Color::LinearRgba(LinearRgba::new(l.red, l.green, l.blue, alpha));
            rings
                .circle(Isometry3d::new(pos, flat), radii.0[i] * 1.3, color)
                .resolution(48);
        }

        // Economic overlay: glow reflecting planet credit value
        if filters.credits {
            let income = p.income;
            let econ_color = if income >= 100 {
                Color::srgb(0.2, 0.95, 0.3) // rich (green)
            } else if income >= 40 {
                Color::srgb(0.95, 0.85, 0.2) // moderate (yellow)
            } else {
                Color::srgb(0.5, 0.5, 0.55) // poor (gray)
            };
            rings
                .circle(Isometry3d::new(pos, flat), radii.0[i] * 1.55, econ_color.with_alpha(0.85))
                .resolution(48);
        }

        // Military overlay: strength bars over systems showing friendly vs enemy fleet presence
        if filters.military {
            if let Some(sim) = sim.as_ref() {
                let me = view.player_faction.unwrap_or(0) as u32;
                let g = sim.galaxy();
                let p_id = i as u32;
                let own_ships: u32 = g.fleets.values().filter(|f| f.planet() == Some(p_id) && f.owner == me).map(|f| f.unit_count()).sum();
                let enemy_ships: u32 = g.fleets.values().filter(|f| f.planet() == Some(p_id) && f.owner != me).map(|f| f.unit_count()).sum();
                if own_ships > 0 || enemy_ships > 0 {
                    let bar_pos = pos + Vec3::Y * (radii.0[i] * 1.8 + 0.8);
                    let max_width = radii.0[i] * 2.2;
                    let total = (own_ships + enemy_ships).max(1) as f32;
                    let own_w = max_width * (own_ships as f32 / total);
                    let enemy_w = max_width * (enemy_ships as f32 / total);

                    let left = bar_pos - Vec3::X * (max_width * 0.5);
                    if own_ships > 0 {
                        border.line(left, left + Vec3::X * own_w, Color::srgb(0.2, 0.95, 0.3));
                    }
                    if enemy_ships > 0 {
                        border.line(left + Vec3::X * own_w, left + Vec3::X * (own_w + enemy_w), Color::srgb(0.95, 0.25, 0.2));
                    }
                }
            }
        }
    }
    if let Some(i) = selection.0.filter(|&i| i < view.planets.len()) {
        // The notched selection ring: a translucent band with raised teeth, bright inner and outer outline.
        let pulse = 1.0 + 0.012 * (time.elapsed_secs() * 3.0).sin();
        let pos = galaxy_to_world(view.planets[i].position);
        let r = radii.0[i] * NOTCH_RADIUS * pulse;
        // Own planets get the green ring, everything else the cool cyan of the original.
        let owner = view.planets[i].owner;
        let l = rgb(select_tint(&view, owner)).to_linear();
        let bright = Color::LinearRgba(LinearRgba::new(l.red, l.green, l.blue, 0.85));
        select.circle(Isometry3d::new(pos, flat), r, bright).resolution(96);
        let outer = (0..=NOTCH_SEGMENTS).map(|k| {
            let th = k as f32 / NOTCH_SEGMENTS as f32 * std::f32::consts::TAU;
            let ro = r * notch_outer(th);
            pos + Vec3::new(th.cos() * ro, 0.0, th.sin() * ro)
        });
        select.linestrip(outer, bright);
    }
}

/// Radius of the notched selection ring in planet radii. Planets shrank by 1/1.5 (`PLANET_VISUAL_BOOST`) while the ring keeps its
/// size, so this is about 5.7 x 1.1 of the model radius (the visible globe is about 0.54 of it): about 12 visible globe radii, the Remake ring is about 13 (yt CS9VdxglewY f0019), vanilla 05 about 10.5.
const NOTCH_RADIUS: f32 = 6.3;
const NOTCH_SEGMENTS: usize = 360;

/// Outer contour of the notched ring in units of the inner radius.
fn notch_outer(theta: f32) -> f32 {
    let deg = theta.to_degrees().rem_euclid(360.0);
    let slot = (deg / 10.0) as u32;
    let within = deg % 10.0;
    let tooth = if slot.is_multiple_of(2) && within > 2.0 && within < 8.0 { 1.0 } else { 0.0 };
    let big = if slot.is_multiple_of(6) { 1.7 } else { 1.0 };
    1.05 + 0.02 * tooth * big
}

/// Translucent fill of the notched ring (unit radius, flat in the XZ plane).
#[derive(Component)]
struct NotchFill;

fn notch_mesh() -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let mut pos = Vec::new();
    let mut idx = Vec::new();
    for k in 0..NOTCH_SEGMENTS {
        let th = k as f32 / NOTCH_SEGMENTS as f32 * std::f32::consts::TAU;
        let (c, s) = (th.cos(), th.sin());
        let ro = notch_outer(th);
        pos.push([c, 0.0, s]);
        pos.push([c * ro, 0.0, s * ro]);
        let (a, b) = ((2 * k) as u32, (2 * k + 1) as u32);
        let (na, nb) = (((2 * ((k + 1) % NOTCH_SEGMENTS)) as u32), ((2 * ((k + 1) % NOTCH_SEGMENTS) + 1) as u32));
        idx.extend([a, b, na, na, b, nb]);
    }
    let n = pos.len();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// Puts the ring fill under the selected planet and tints it with the player's colour.
fn place_notch_fill(
    view: Res<GalaxyView>,
    radii: Res<PlanetRadii>,
    selection: Res<Selection>,
    mut fills: Query<(&mut Transform, &mut Visibility, &MeshMaterial3d<StandardMaterial>), With<NotchFill>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last: Local<Option<usize>>,
) {
    for (mut tf, mut vis, mat) in &mut fills {
        match selection.0.filter(|&i| i < view.planets.len()) {
            Some(i) => {
                let pos = galaxy_to_world(view.planets[i].position);
                tf.translation = pos;
                tf.scale = Vec3::splat(radii.0[i] * NOTCH_RADIUS);
                *vis = Visibility::Inherited;
                if *last != Some(i)
                    && let Some(mut m) = materials.get_mut(&mat.0)
                {
                    let owner = view.planets[i].owner;
                    m.base_color = rgb(select_tint(&view, owner)).with_alpha(0.13);
                    *last = Some(i);
                }
            }
            None => {
                *vis = Visibility::Hidden;
                *last = None;
            }
        }
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

/// A lane class: one combined ribbon mesh per width class (main web ways, inter lanes, capillaries).
#[derive(Component)]
struct LaneMesh {
    mesh: Handle<Mesh>,
    /// Route indices of the ribbon quads in vertex order (4 vertices each).
    routes: Vec<usize>,
}

/// Lane colour/zoom state of the last colour update (vertex colours are only rewritten when it changes).
#[derive(Default)]
struct LaneState {
    zoom_bucket: i32,
    owners: u64,
    built: bool,
}

/// Half widths of the ribbons in planet radii (`MapScale::unit`), by data width class. The reference
/// shows wide soft light-blue beams of about 0.7 planet radii, thinner inter lanes and hair lines.
const LANE_HALF_WIDTH: [f32; 3] = [0.2, 0.08, 0.03];
/// Peak brightness (linear) of the beams.
const LANE_PEAK: [f32; 3] = [0.75, 0.5, 0.3];

fn lane_class(width: f32) -> usize {
    if width >= 4.0 {
        0
    } else if width >= 1.5 {
        1
    } else {
        2
    }
}

/// Soft beam profile across the ribbon (u) and a gentle fade towards both ends (v).
fn lane_texture() -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let (w, h) = (64u32, 16u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        let along = 0.12 + 0.88 * (std::f32::consts::PI * v).sin().powf(0.6);
        for x in 0..w {
            let u = ((x as f32 + 0.5) / w as f32 - 0.5) * 2.0;
            // Bright core with soft shoulders.
            let profile = (1.0 - u * u).max(0.0).powf(1.6);
            let a = (profile * along).clamp(0.0, 1.0);
            let c = 200 + (55.0 * (1.0 - u.abs())) as u8;
            px.extend_from_slice(&[c, c, 255, (a * 255.0) as u8]);
        }
    }
    let mut img = Image::new(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = bevy::image::ImageSampler::linear();
    img
}

/// Spawns the three lane ribbon meshes of the galaxy.
fn spawn_lanes(
    commands: &mut Commands,
    view: &GalaxyView,
    scale: &MapScale,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let tex = images.add(lane_texture());
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(tex),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        cull_mode: None,
        // Transparent objects sort by the distance of their centre: this one is huge, so push it to the very back.
        depth_bias: -1.0e6,
        ..default()
    });
    let mut classes: [Vec<usize>; 3] = Default::default();
    for (i, r) in view.routes.iter().enumerate() {
        let (Some(_), Some(_), Some(style)) = (view.planets.get(r.a), view.planets.get(r.b), view.route_styles.get(r.style)) else {
            continue;
        };
        classes[lane_class(style.neutral.width.max(style.own.width))].push(i);
    }
    for (class, routes) in classes.into_iter().enumerate() {
        if routes.is_empty() {
            continue;
        }
        let half = LANE_HALF_WIDTH[class] * scale.unit;
        let (mut pos, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new());
        for (n, &ri) in routes.iter().enumerate() {
            let r = &view.routes[ri];
            let (pa, pb) = (galaxy_to_world(view.planets[r.a].position), galaxy_to_world(view.planets[r.b].position));
            let dir = (pb - pa).normalize_or_zero();
            let mut side = Vec3::Y.cross(dir);
            if side.length_squared() < 1e-6 {
                side = Vec3::X;
            }
            let side = side.normalize() * half;
            pos.extend([pa - side, pa + side, pb - side, pb + side].map(|p| p.to_array()));
            uv.extend([[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]);
            let b = (n * 4) as u32;
            idx.extend([b, b + 1, b + 2, b + 2, b + 1, b + 3]);
        }
        let n = pos.len();
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; n]);
        mesh.insert_indices(Indices::U32(idx));
        let handle = meshes.add(mesh);
        commands.spawn((
            Name::new("lanes"),
            Mesh3d(handle.clone()),
            MeshMaterial3d(material.clone()),
            Transform::IDENTITY,
            bevy::camera::visibility::NoFrustumCulling,
            LaneMesh { mesh: handle, routes },
            GalaxyEntity,
        ));
    }
}

/// Linear RGB of a lane for the relation of its ends at zoom `t`; `None` when the style hides it.
/// The game's colours are dark additive teal (13, 31, 38) meant to be added on the backdrop; they give the
/// hue, the peak brightness per width class gives the beam.
fn lane_color(relation: RouteRelation, style: &LineStyleView, t: f32, class: usize) -> [f32; 4] {
    let mix = |i: usize| style.color_near[i] + (style.color_far[i] - style.color_near[i]) * t;
    let alpha = mix(3);
    if style.width <= 0.0 || alpha <= 0.01 {
        return [0.0; 4];
    }
    let base_rgb = match relation {
        // The Remake draws every hyperspace lane as a thin cyan beam (f0007/f0019); only the hue shifts a little with the relation.
        RouteRelation::Own => Vec3::new(0.30, 0.95, 0.95),
        RouteRelation::Contested => Vec3::new(0.35, 0.78, 1.0),
        RouteRelation::Enemy => Vec3::new(0.32, 0.78, 1.0),
        RouteRelation::Neutral => {
            let rgb = Vec3::new(mix(0), mix(1), mix(2));
            let rgb = rgb / rgb.max_element().max(1e-3);
            rgb * 0.3 + Vec3::new(0.18, 0.62, 0.72)
        }
    };
    let l = Color::srgb(base_rgb.x, base_rgb.y, base_rgb.z).to_linear();
    let k = LANE_PEAK[class] * alpha;
    [l.red * k, l.green * k, l.blue * k, 1.0]
}

/// Keeps the ribbon colours in sync with ownership (relation of the ends) and zoom.
fn update_lanes(
    view: Res<GalaxyView>,
    bounds: Res<CameraBounds>,
    camera: Single<&GalaxyCamera>,
    lanes: Query<&LaneMesh>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut state: Local<LaneState>,
) {
    let t = zoom_fraction(&bounds, camera.distance);
    let bucket = (t * 24.0) as i32;
    let owners = view.planets.iter().fold(1469598103934665603u64, |h, p| (h ^ p.owner.map_or(0xffff, |o| o as u64)).wrapping_mul(1099511628211));
    if state.built && state.zoom_bucket == bucket && state.owners == owners {
        return;
    }
    for lane in &lanes {
        let mut col: Vec<[f32; 4]> = Vec::with_capacity(lane.routes.len() * 4);
        for &ri in &lane.routes {
            let r = &view.routes[ri];
            let (a, b) = (&view.planets[r.a], &view.planets[r.b]);
            let style = &view.route_styles[r.style];
            let class = lane_class(style.neutral.width.max(style.own.width));
            let relation = route_relation(a.owner, b.owner, view.player_faction);
            let line = match relation {
                RouteRelation::Own => &style.own,
                RouteRelation::Enemy => &style.enemy,
                RouteRelation::Contested | RouteRelation::Neutral => &style.neutral,
            };
            let c = lane_color(relation, line, t, class);
            col.extend([c; 4]);
        }
        if let Some(mut m) = meshes.get_mut(&lane.mesh) {
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
        }
    }
    *state = LaneState { zoom_bucket: bucket, owners, built: true };
}

/// Animates light pulses along major trade routes.
fn draw_trade_route_pulses(
    mut gizmos: Gizmos,
    view: Res<GalaxyView>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs();
    let speed = 2.0;
    for r in &view.routes {
        let (Some(pa_info), Some(pb_info), Some(style)) = (view.planets.get(r.a), view.planets.get(r.b), view.route_styles.get(r.style)) else {
            continue;
        };
        let class = lane_class(style.neutral.width.max(style.own.width));
        if class > 1 {
            continue;
        }
        let pa = galaxy_to_world(pa_info.position);
        let pb = galaxy_to_world(pb_info.position);
        let len = pa.distance(pb);
        if len < 1.0 {
            continue;
        }
        let dir = (pb - pa) / len;
        let relation = route_relation(pa_info.owner, pb_info.owner, view.player_faction);
        let color = match relation {
            RouteRelation::Own => Color::srgba(0.25, 0.95, 0.65, 0.85),
            RouteRelation::Contested => Color::srgba(0.95, 0.3, 0.2, 0.85),
            RouteRelation::Enemy => Color::srgba(0.8, 0.2, 0.35, 0.75),
            RouteRelation::Neutral => Color::srgba(0.4, 0.75, 1.0, 0.75),
        };
        let count = ((len / 14.0) as usize).clamp(1, 4);
        let streak_len = (len * 0.08).clamp(0.4, 1.2);
        for i in 0..count {
            let offset = i as f32 / count as f32;
            let phase = ((now * speed / len + offset) % 1.0 + 1.0) % 1.0;
            let center = pa.lerp(pb, phase);
            let p_head = center + dir * streak_len * 0.5;
            let p_tail = center - dir * streak_len * 0.5;
            gizmos.line_gradient(p_tail, p_head, color.with_alpha(0.0), color);
        }
    }
}

/// Finds the planet under the mouse cursor (ray vs. a generous pick sphere).
fn update_hover(
    args: Res<crate::cli::Args>,
    selection: Res<Selection>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform)>,
    view: Res<GalaxyView>,
    radii: Res<PlanetRadii>,
    blocked: Option<Res<galaxy_play::PlayCursorBlocked>>,
    cheat_panel: Query<(&ComputedNode, &UiGlobalTransform), With<crate::cheat_menu::CheatRoot>>,
    mut hover: ResMut<Hover>,
    mut sounds: MessageWriter<UiSound>,
) {
    if blocked.is_some_and(|b| b.0) {
        hover.set_if_neq_none();
        return;
    }
    let log_cur = window.cursor_position();
    let phys_cur = window.physical_cursor_position();
    let in_cheat = cheat_panel.iter().any(|(n, t)| {
        log_cur.is_some_and(|c| n.contains_point(*t, c)) || phys_cur.is_some_and(|p| n.contains_point(*t, p))
    });
    if in_cheat {
        hover.set_if_neq_none();
        return;
    }
    let Some(cursor) = window.cursor_position() else {
        if args.hover_selected {
            hover.0 = selection.0;
        } else {
            hover.set_if_neq_none();
        }
        return;
    };
    let (camera, cam_tf) = *camera;
    let Ok(ray) = camera.viewport_to_world(cam_tf, cursor) else {
        return;
    };
    let cam_pos = cam_tf.translation();

    let mut best: Option<(usize, f32, f32)> = None; // (index, normalized_perp, dist_along)
    for (i, p) in view.planets.iter().enumerate() {
        let center = galaxy_to_world(p.position);
        let oc = center - ray.origin;
        let dist_along = oc.dot(*ray.direction);
        if dist_along <= 0.0 {
            continue;
        }
        let perp_sq = oc.length_squared() - dist_along * dist_along;
        let d_perp = if perp_sq > 0.0 { perp_sq.sqrt() } else { 0.0 };
        // Generous pick radius that never gets smaller than a few pixels on screen.
        let r = (radii.0[i] * 1.35).max(center.distance(cam_pos) * 0.015);
        if d_perp <= r {
            let norm_perp = d_perp / r;
            let is_better = match best {
                None => true,
                Some((_, best_norm, best_dist)) => {
                    if (norm_perp - best_norm).abs() > 0.15 {
                        norm_perp < best_norm
                    } else {
                        dist_along < best_dist
                    }
                }
            };
            if is_better {
                best = Some((i, norm_perp, dist_along));
            }
        }
    }
    let new = best.map(|(i, _, _)| i);
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
    fleet_hover: Option<Res<galaxy_play::PlayHover>>,
    blocked: Option<Res<galaxy_play::PlayCursorBlocked>>,
    cheat_panel: Query<(&ComputedNode, &UiGlobalTransform), With<crate::cheat_menu::CheatRoot>>,
    mut selection: ResMut<Selection>,
    mut press: Local<Option<Vec2>>,
    mut sounds: MessageWriter<UiSound>,
) {
    if blocked.is_some_and(|b| b.0) {
        press.take();
        return;
    }
    let log_cur = window.cursor_position();
    let phys_cur = window.physical_cursor_position();
    let in_cheat = cheat_panel.iter().any(|(n, t)| {
        log_cur.is_some_and(|c| n.contains_point(*t, c)) || phys_cur.is_some_and(|p| n.contains_point(*t, p))
    });
    if in_cheat {
        press.take();
        return;
    }
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
    // A fleet under the cursor wins over the planet beneath it: the click selects the fleet only.
    if fleet_hover.is_some_and(|h| h.fleet.is_some()) {
        return;
    }
    if hover.0.is_some() && hover.0 != selection.0 {
        sounds.write(UiSound(crate::menu::SFX_PLANET_SELECT));
    }
    selection.0 = hover.0;
}

/// Distance along the ray to the first intersection with the sphere, if any.
#[allow(dead_code)]
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
    let count = w * h / 3600; // estimate: the Remake sky is sparse
    for _ in 0..count {
        let (cx, cy) = (rng.random_range(0..w) as i32, rng.random_range(0..h) as i32);
        let b: f32 = rng.random::<f32>().powf(5.0);
        let peak = 0.15 + 0.7 * b;
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

    #[test]
    fn map_filters_cycle_overlays() {
        let mut f = MapFilters::default();
        assert!(!f.credits && !f.military);
        f.cycle_overlays();
        assert!(f.credits && !f.military);
        f.cycle_overlays();
        assert!(!f.credits && f.military);
        f.cycle_overlays();
        assert!(!f.credits && !f.military);
    }
}
/// A row of structure slot pips under a planet name: `row` 0 = ground slots, 1 = space slots.
#[derive(Component)]
struct PipRow {
    planet: usize,
    row: usize,
    count: usize,
}

/// One pip of a [`PipRow`].
#[derive(Component)]
struct PipIcon {
    space: bool,
    planet: usize,
    slot: usize,
}

/// sRGB colour a planet owner is drawn in on the map. Like the game the player's own planets are green; the other
/// factions keep their `Color` from `FACTIONS.XML` (Rebel red, Underworld orange, ...), brightened to read on the nebula.
fn relation_color(view: &GalaxyView, owner: Option<usize>) -> [f32; 3] {
    match owner {
        None => [0.62, 0.62, 0.62],
        Some(o) if Some(o) == view.player_faction => [0.15, 1.0, 0.25],
        Some(o) => view.factions.get(o).map_or(NEUTRAL, |f| {
            let m = f.color[0].max(f.color[1]).max(f.color[2]).max(1e-3);
            [(f.color[0] / m).powf(1.4), (f.color[1] / m).powf(1.4), (f.color[2] / m).powf(1.4)]
        }),
    }
}

/// Tint of the selection ring and influence circle: the owner's map colour (green own, the data colour of any other faction; the
/// Remake frames show orange and red rings for different owners, CS9VdxglewY f0018/f0019), orange for an unowned planet.
fn select_tint(view: &GalaxyView, owner: Option<usize>) -> [f32; 3] {
    match owner {
        Some(o) => relation_color(view, Some(o)),
        None => [1.0, 0.62, 0.15],
    }
}

/// Faction emblem icon next to an owned planet (screen space, positioned with the label).
#[derive(Component)]
struct Emblem(usize);

impl Emblem {
    fn planet_index(&self) -> usize {
        self.0
    }
}

/// Gives each emblem its texture once the UI texture library can provide it (the mega texture
/// decodes in the background).
fn fill_emblems(
    view: Res<GalaxyView>,
    textures: Option<ResMut<eaw_ui::textures::UiTextures>>,
    mut images: ResMut<Assets<Image>>,
    mut emblems: Query<(&Emblem, &mut ImageNode)>,
) {
    let Some(mut textures) = textures else { return };
    for (e, mut node) in &mut emblems {
        let Some(planet) = view.planets.get(e.0) else { continue };
        let icon = planet.owner.and_then(|o| view.factions.get(o)).and_then(|f| f.icon.clone());
        let Some(name) = icon else {
            if node.color != Color::NONE { node.color = Color::NONE; }
            continue;
        };
        // The emblem follows ownership changes (capture): texture and colour are refreshed every frame.
        if let Some(t) = textures.get(&mut images, &name) {
            if node.image != t.handle {
                node.image = t.handle;
            }
            let want = rgb(relation_color(&view, planet.owner));
            if node.color != want { node.color = want; }
        }
    }
}

/// Swaps the hand-drawn pip silhouettes for the game's own row icons once the UI textures are decoded: `i_bar_icons_space` (the
/// pod tower, space station tiers) and `i_bar_icons_land` (the turret, ground slots), both 14x14 in the command bar mega texture.
/// Which texture the galaxy map rows use in the engine is not documented; both match vanilla 05 (pod row above turret row).
fn fill_pip_icons(textures: Option<ResMut<eaw_ui::textures::UiTextures>>, mut images: ResMut<Assets<Image>>, mut pips: Query<(&PipIcon, &mut ImageNode)>) {
    let Some(mut textures) = textures else { return };
    let space = textures.get(&mut images, "i_bar_icons_space.tga");
    let land = textures.get(&mut images, "i_bar_icons_land.tga");
    for (pip, mut node) in &mut pips {
        if let Some(t) = if pip.space { space.as_ref() } else { land.as_ref() }
            && node.image != t.handle
        {
            node.image = t.handle.clone();
        }
    }
}

/// Territory outline: the union boundary of discs around the planets of each faction, sampled as
/// dashes (only the parts of a circle that no other disc of the same faction covers).
fn territory_segments(view: &GalaxyView, radii: &PlanetRadii, selected: Option<usize>) -> Vec<(Vec3, Vec3, Color)> {
    // Like the original: one dashed ring, around the selected planet's sector, in the colour of its relation.
    const SAMPLES: usize = 360;
    // Remake f0007/f0019: about 63 dashes round the circle, dash about twice the gap: 4 samples on, 2 off.
    const DASH: usize = 4;
    let Some(i) = selected.filter(|&i| i < view.planets.len()) else { return Vec::new() };
    let p = &view.planets[i];
    let c = galaxy_to_world(p.position);
    let r = (radii.0[i] * TERRITORY_RADIUS).clamp(30.0, 85.0);
    let l = rgb(select_tint(view, p.owner)).to_linear();
    let color = Color::LinearRgba(LinearRgba::new(l.red, l.green, l.blue, 0.8));
    let a = |k: usize| k as f32 / SAMPLES as f32 * std::f32::consts::TAU;
    let pt = |k: usize| c + Vec3::new(a(k).cos() * r, 0.0, a(k).sin() * r);
    (0..SAMPLES).filter(|k| k % (DASH + 2) < DASH).map(|k| (pt(k), pt(k + 1), color)).collect()
}

/// Radius of a planet's territory disc in planet radii.
const TERRITORY_RADIUS: f32 = 22.5;

/// Gizmo group of the thick dashed faction territory outline.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct TerritoryGizmos;

/// Small white silhouettes for the slot pips under a planet name: an orbital starbase (space slots)
/// and a planetary bunker / garrison (ground slots), 16x16.
pub(crate) fn pip_image(space: bool) -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    const N: i32 = 16;
    let inside = |x: i32, y: i32| -> bool {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        if space {
            // Authentic EaW space station icon: communication mast, cupola dome, wide primary saucer, docking stalk
            ((1.0..3.5).contains(&fy) && (fx - 8.0).abs() <= 1.0)
                || ((3.5..6.0).contains(&fy) && (fx - 8.0).abs() <= 3.5)
                || ((6.0..9.5).contains(&fy) && (fx - 8.0).abs() <= 6.5)
                || ((9.5..13.5).contains(&fy) && (fx - 8.0).abs() <= 2.5)
                || ((13.5..15.5).contains(&fy) && (fx - 8.0).abs() <= 1.2)
        } else {
            // Authentic EaW ground base bunker: rooftop antenna, fortified roof, central bunker walls, foundation base
            ((1.0..3.5).contains(&fy) && (fx - 8.0).abs() <= 1.0)
                || ((3.5..6.5).contains(&fy) && (fx - 8.0).abs() <= 5.5 - (6.5 - fy) * 0.7)
                || ((6.5..12.5).contains(&fy) && (fx - 8.0).abs() <= 5.5 && !((8.5..10.5).contains(&fy) && (fx - 8.0).abs() >= 1.2 && (fx - 8.0).abs() <= 3.5))
                || ((12.5..15.5).contains(&fy) && (fx - 8.0).abs() <= 6.5)
        }
    };
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            px.extend_from_slice(&[255, 255, 255, if inside(x, y) { 255 } else { 0 }]);
        }
    }
    let mut img = Image::new(
        Extent3d { width: N as u32, height: N as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = bevy::image::ImageSampler::linear();
    img
}

/// Off: the Remake galaxy is black deep space (yt CS9VdxglewY f0007/f0018/f0019); the maroon wash is vanilla FoC.
const NEBULA_PLANE: bool = false;

/// The original lays a dense maroon nebula over the whole picture (not only over the galaxy disc art): a huge flat,
/// unlit, additive plane under the map with a soft radial falloff and cloudy variation.
fn spawn_nebula_plane(commands: &mut Commands, view: &GalaxyView, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>, images: &mut Assets<Image>) {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for p in &view.planets {
        let w = galaxy_to_world(p.position);
        lo = lo.min(Vec2::new(w.x, w.z));
        hi = hi.max(Vec2::new(w.x, w.z));
    }
    if lo.x > hi.x {
        return;
    }
    let centre = (lo + hi) * 0.5;
    let size = ((hi - lo).max_element() * 1.9).max(2000.0);
    const N: usize = 256;
    let mut rng = SmallRng::seed_from_u64(0xA11CE);
    let grid: Vec<f32> = (0..17 * 17).map(|_| rng.random::<f32>()).collect();
    let noise = |x: f32, y: f32| {
        let (fx, fy) = (x * 16.0, y * 16.0);
        let (ix, iy) = (fx.floor() as usize, fy.floor() as usize);
        let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
        let g = |a: usize, b: usize| grid[b.min(16) * 17 + a.min(16)];
        let top = g(ix, iy) * (1.0 - tx) + g(ix + 1, iy) * tx;
        let bot = g(ix, iy + 1) * (1.0 - tx) + g(ix + 1, iy + 1) * tx;
        top * (1.0 - ty) + bot * ty
    };
    let mut px = Vec::with_capacity(N * N * 4);
    for y in 0..N {
        for x in 0..N {
            let (u, v) = (x as f32 / (N - 1) as f32, y as f32 / (N - 1) as f32);
            let r = ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt() * 2.0;
            let falloff = (1.0 - r).clamp(0.0, 1.0);
            let falloff = falloff * falloff * (3.0 - 2.0 * falloff);
            let cloud = 0.55 + 0.45 * noise(u, v);
            let a = (falloff * cloud).clamp(0.0, 1.0);
            px.extend_from_slice(&[(a * 255.0) as u8, (a * 255.0) as u8, (a * 255.0) as u8, 255]);
        }
    }
    let mut img = Image::new(Extent3d { width: N as u32, height: N as u32, depth_or_array_layers: 1 }, TextureDimension::D2, px, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    img.sampler = bevy::image::ImageSampler::linear();
    let tex = images.add(img);
    let material = materials.add(StandardMaterial {
        base_color: Color::linear_rgb(0.2, 0.05, 0.06),
        base_color_texture: Some(tex),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        cull_mode: None,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(size, size))),
        MeshMaterial3d(material),
        Transform::from_xyz(centre.x, -8.0, centre.y),
        NebulaPlane,
        GalaxyEntity,
    ));
}

/// The maroon nebula plane.
#[derive(Component)]
struct NebulaPlane;

/// Fades the nebula with the zoom: dense when close (reference 04), black space when zoomed out (v0520).
fn fade_nebula_plane(
    bounds: Res<CameraBounds>,
    camera: Option<Single<&GalaxyCamera>>,
    plane: Query<&MeshMaterial3d<StandardMaterial>, With<NebulaPlane>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(camera) = camera else { return };
    let z = zoom_fraction(&bounds, camera.distance);
    let k = 1.0 - smoothstep(0.12, 0.45, z);
    for m in &plane {
        if let Some(mut mat) = materials.get_mut(&m.0) {
            let c = Color::linear_rgb(0.17 * k, 0.07 * k, 0.075 * k);
            if mat.base_color != c {
                mat.base_color = c;
            }
        }
    }
}

/// Slot pips follow the built status and owner colour of their planet.
/// Active built tiers/structures are brightly colored; empty unbuilt tiers/slots are dimmed/greyed out.
fn update_pip_colors(
    view: Res<GalaxyView>,
    sim: Option<Res<galaxy_play::SimState>>,
    bridge: Option<Res<crate::play::PlayBridge>>,
    mut pips: Query<(&PipIcon, &mut ImageNode)>,
) {
    for (pip, mut img) in &mut pips {
        let planet_view = view.planets.get(pip.planet);
        let owner = planet_view.and_then(|p| p.owner);
        let c = if owner.is_some() { relation_color(&view, owner) } else { [0.62, 0.62, 0.62] };
        let active_color = Color::srgba(c[0], c[1], c[2], 0.95);
        // Empty slots are not drawn: the Remake shows icons only for what is built.
        let dimmed_color = Color::NONE;

        let is_active = if let (Some(sim), Some(bridge)) = (sim.as_ref(), bridge.as_ref()) {
            if let Some(Some(sim_p)) = bridge.planet_to_sim.get(pip.planet) {
                if let Some(ps) = sim.galaxy().planets.get(*sim_p as usize) {
                    if pip.space {
                        let level = ps.starbase.map_or(0, |sb| sim.world.starbases.get(sb as usize).map_or(0, |b| b.level as usize));
                        pip.slot < level
                    } else {
                        pip.slot < ps.structures.len()
                    }
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            // Outside of an active campaign nothing is built.
            false
        };

        let want = if is_active { active_color } else { dimmed_color };
        if img.color != want {
            img.color = want;
        }
    }
}

/// Offscreen `--hover`: hovers the selected planet (the real hover needs a cursor and a window).
fn scripted_hover(args: Res<crate::cli::Args>, selection: Res<Selection>, mut hover: ResMut<Hover>) {
    if args.hover_selected && hover.0 != selection.0 {
        hover.0 = selection.0;
    }
}

/// Translucent red fill over the sector of a selected enemy planet (v0540).
#[derive(Component)]
struct SectorFill;

fn place_sector_fill(
    view: Res<GalaxyView>,
    radii: Res<PlanetRadii>,
    selection: Res<Selection>,
    mut fill: Single<(&mut Transform, &mut Visibility), With<SectorFill>>,
) {
    let (tf, vis) = &mut *fill;
    let enemy = selection.0.filter(|&i| i < view.planets.len()).filter(|_| false); // the Remake draws no red wash over an enemy sector, only the dashed circle
    match enemy {
        Some(i) => {
            tf.translation = galaxy_to_world(view.planets[i].position) - Vec3::Y * 0.5;
            tf.scale = Vec3::splat((radii.0[i] * TERRITORY_RADIUS).clamp(30.0, 85.0));
            vis.set_if_neq(Visibility::Inherited);
        }
        None => {
            vis.set_if_neq(Visibility::Hidden);
        }
    }
}
