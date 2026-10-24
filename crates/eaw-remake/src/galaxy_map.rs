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
    light::{GlobalAmbientLight, NotShadowCaster},
    mesh::{MeshVertexBufferLayoutRef, SphereKind, SphereMeshBuilder},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError, TextureDimension, TextureFormat,
    },
    shader::ShaderRef,
    window::PrimaryWindow,
};
use rand::{Rng, SeedableRng, rngs::SmallRng};

use crate::camera::{CameraBounds, GalaxyCamera};
use crate::flow::{AppScreen, UiSound, cursor_free, not_paused};
use crate::galaxy_view::{GalaxyView, LineStyleView, PropView, RouteStyleView};
use crate::planet_visual::{
    NEUTRAL, PlanetRadii, PlanetVisualAssets, PlanetVisualPlugin, rgb, spawn_planet_visual,
};
use alamo_render::{
    AlamoFs, AlamoModel, AlamoModelLoaded,
    material::{AlamoMaterial, FLAG_UNDERLAY},
    spawn_sun,
    texture::TextureCache,
};

/// Draws the [`GalaxyView`] resource and handles planet selection.
pub struct GalaxyMapPlugin;

impl Plugin for GalaxyMapPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "lane.wgsl");
        app.add_plugins((PlanetVisualPlugin, MaterialPlugin::<LaneMaterial>::default()))
            .init_resource::<Selection>()
            .init_resource::<Hover>()
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
            .add_systems(Update, (update_pip_colors, place_sector_shadow, scripted_hover.after(update_hover)).run_if(in_state(AppScreen::Galaxy)))
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
                    (update_labels, log_label_stats).chain(),
                    draw_rings,
                    update_lanes,
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


/// UI depth of the planet names, their pip rows and emblems: below the command bar and its backing plate (-1). Like in the Remake
/// (b0613s: LORONAR), a name behind the translucent build strip shows faintly through it and one behind the opaque art is covered.
const LABEL_Z: i32 = -2;

/// Screen rectangles (viewport pixels) the name plates avoid: `.0` the HUD panels, `.1` the hero portraits next to fleets. The command
/// bar is not one of them: it is drawn over the names (see [`LABEL_Z`]).
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

/// Brightness of the galaxy art's `core` glow (`W_GAL_gradient.tga` on the additive `core` disc: the pink nebula over the middle of
/// the map): [`CORE_GLOW`], or `SP_GALAXY_CORE=<value>` to try another one (comparison renders).
fn core_glow() -> f32 {
    static CORE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *CORE.get_or_init(|| std::env::var("SP_GALAXY_CORE").ok().and_then(|v| v.parse().ok()).unwrap_or(CORE_GLOW))
}
/// Measured on the user's Remake video at the same map points (projected through the planets of each frame, median of 13x13 px,
/// outside any selection ring): v0240 (nothing selected) averages sRGB (103, 69, 65) over 16 points round the core, b0613s (86, 45,
/// 49) over 8; our core at 0.3 / 0.4 / 0.5 gives (78, 49, 48) / (105, 68, 66) / (133, 86, 84) on the first set and (68, 39, 41) /
/// (93, 54, 57) / (118, 69, 74) on the second, so 0.39 matches v0240 and 0.35 b0613s (the gradient scrolls, frames differ). It was
/// 0.14 (0506963, a maroon haze), and 1.0 floods the whole view with pink.
const CORE_GLOW: f32 = 0.37;

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
    mut alamo_materials: ResMut<Assets<AlamoMaterial>>,
) {
    for model in &props {
        for &part in &model.parts {
            let name = names.get(part).map(|n| n.as_str().to_ascii_lowercase()).unwrap_or_default();
            if matches!(name.as_str(), "core" | "region_lines" | "grid")
                && let Ok(h) = handles.get(part)
                && let Some(mut m) = alamo_materials.get_mut(&h.0)
                && m.params.diffuse.w < 1.5
            {
                // Only the big disc (several thousand units wide) fades; the small galaxies keep
                // their look.
                if model.radius > 500.0 {
                    // The lanes add over the core glow and the orbit lines in display values (`LaneMaterial`), so both draw with
                    // the backdrop the lanes read. The grid stays off.
                    if name != "grid" {
                        m.params.flags |= FLAG_UNDERLAY;
                        commands.entity(part).insert(NotShadowCaster);
                    }
                    commands.entity(part).insert(DiscPart {
                        material: h.0.clone(),
                        base: m.params.diffuse,
                        // Reference look: faint orbit lines, the core glow at the measured brightness ([`CORE_GLOW`]), no grid. The
                        // orbit lines (`Region_Lines`, the thin double curves across the map with a light running along them) are faint
                        // in the Remake frames (CS9 f0016: about sRGB 13..18, 23..28, 25..29 on black; v0520 fainter still), well below
                        // the lanes (40, 63, 77). At 0.9 they were the brightest lines on the map (5, 54, 82); 0.4 keeps them visible but
                        // faint (estimate by eye against f0016).
                        strength: match name.as_str() { "grid" => 0.0, "core" => core_glow(), _ => 0.4 },
                    });
                }
            }
            if names.get(part).is_ok_and(|n| n.as_str() == "Stars")
                && let Some(tex) = textures.get(&fs.0, &mut images, "w_star_bkgnd.dds")
            {
                let material = materials.add(StandardMaterial {
                    // The texture holds gamma-space colours; this approximates the game's look.
                    base_color: Color::linear_rgb(0.06, 0.06, 0.06), // estimate: only the brightest stars of the texture stay visible, the Remake sky is sparse
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
pub(crate) struct PlanetLabel(pub(crate) usize);

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
    gizmo_store.config_mut::<TerritoryGizmos>().0.line.width = 2.0; // estimate: the Remake ring is a fine dashed line
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
    mut lane_materials: ResMut<Assets<LaneMaterial>>,
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

        // Name label in the owner's colour (positioned every frame, `update_labels`).
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
                    GlobalZIndex(LABEL_Z),
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
            GlobalZIndex(LABEL_Z),
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
            GlobalZIndex(LABEL_Z),
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
    spawn_lanes(&mut commands, &view, &mut meshes, &mut lane_materials, &mut images);
    // The Remake video shows plain black space outside the galaxy core (maroon only over the core, which the disc art
    // provides), so the extra plane stays off.
    if NEBULA_PLANE {
        spawn_nebula_plane(&mut commands, &view, &mut meshes, &mut materials, &mut images);
    }
    commands.spawn((
        Name::new("sector shadow"),
        Mesh3d(meshes.add(sector_shadow_mesh())),
        // Multiply: the map behind is scaled by the vertex colour (`sector_shadow_mesh`).
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            fog_enabled: false,
            alpha_mode: AlphaMode::Multiply,
            cull_mode: None,
            depth_bias: -500.0,
            ..default()
        })),
        Transform::default(),
        Visibility::Hidden,
        bevy::camera::visibility::NoFrustumCulling,
        SectorShadow,
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

/// Where a planet's label can go on the screen.
#[derive(Clone, Copy)]
struct LabelSlot {
    /// Top-left of the label rectangle in logical viewport pixels (below the globe; after the placement, where it is shown).
    pos: Vec2,
    /// Top-left of the same rectangle above the globe, for a name that would sit off the bottom or under the interface.
    above: Vec2,
    size: Vec2,
}

/// Zoom fraction (0 closest, 1 farthest) from which the map shows no planet names but the selected planet's (the hover card still
/// names the planet under the cursor), and the one below which they come back; the gap is a hysteresis, so a zoom resting at the
/// threshold never makes them blink. The Remake has no far zoom to measure this on (its galaxy camera has a single view, about our
/// zoom 0.15, and b0621s without names is the camera flying back from the planet screen); 0.45 is where the crowding cull used to
/// start thinning the names out (G3c, 2026-10-17).
const NAMES_HIDE_FROM: f32 = 0.47;
const NAMES_SHOW_BELOW: f32 = 0.43;

/// Whether the map is zoomed out too far for planet names this frame, given last frame's answer (see [`NAMES_HIDE_FROM`]).
fn names_far(zoom_out: f32, was_far: bool) -> bool {
    if was_far { zoom_out >= NAMES_SHOW_BELOW } else { zoom_out >= NAMES_HIDE_FROM }
}

/// Room kept between a label and a HUD panel or the screen edge (viewport px).
const LABEL_MARGIN: f32 = 2.0;
/// Hysteresis of a name's place (viewport px): a name under its globe stays there while a HUD panel or the screen edge covers its box
/// by up to `PLACE_HYSTERESIS - LABEL_MARGIN` px (the box has room around the letters; close up its anchor must stay on screen), and a
/// name above its globe comes back once the place under the globe is clear by `PLACE_HYSTERESIS` px more than [`LABEL_MARGIN`].
/// Without it a name grazing the top of the advisor's text line while the zoom moved it (Ithor at zoom 0.21, `--label-probe`: 0.9 px
/// for 5 frames) jumped above its globe and back.
const PLACE_HYSTERESIS: f32 = 4.0;

/// Whether a label rectangle at `pos` (top-left, viewport px) is on screen and clear of the HUD panel rectangles by
/// [`LABEL_MARGIN`]` + extra` px (a negative `extra` allows an overlap). Close up only its anchor (the top centre) has to be on screen
/// (`extra` px inside when positive). The command bar does not count: names are drawn below it ([`LABEL_Z`]).
fn label_clear(pos: Vec2, size: Vec2, viewport: Vec2, close: bool, panel: &PanelRect, extra: f32) -> bool {
    let (lo, hi) = (pos - Vec2::splat(LABEL_MARGIN + extra), pos + size + Vec2::splat(LABEL_MARGIN + extra));
    let anchor = pos + Vec2::new(size.x * 0.5, 0.0);
    let inset = extra.max(0.0);
    let on_screen = if close {
        anchor.x >= inset && anchor.y >= inset && anchor.x <= viewport.x - inset && anchor.y <= viewport.y - inset
    } else {
        lo.x >= 0.0 && lo.y >= 0.0 && hi.x <= viewport.x && hi.y <= viewport.y
    };
    on_screen && !panel.0.iter().chain(panel.1.iter()).any(|(plo, phi)| lo.x < phi.x && hi.x > plo.x && lo.y < phi.y && hi.y > plo.y)
}

/// Whether the place under its globe counts as clear for a name this frame (see [`PLACE_HYSTERESIS`]): `lenient` is the check that
/// lets a name stay there, `with_room` the one a name above its globe needs to come back. With no room above it the name stays under
/// its globe whenever `lenient` holds, so the hysteresis never hides a name.
fn below_clear(lenient: bool, with_room: bool, above: bool, was_above: bool) -> bool {
    if was_above && above { with_room } else { lenient }
}

/// `SP_NO_PLANET_NAMES=1`: the names are laid out as usual but not drawn, so a comparison render of the same view shows exactly what
/// the names add on screen (how well a name shows through the build strip).
fn draw_planet_names() -> bool {
    static DRAW: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DRAW.get_or_init(|| std::env::var_os("SP_NO_PLANET_NAMES").is_none())
}

#[allow(clippy::too_many_arguments)]
/// Positions the label of each planet in view below its sphere (above it where a HUD panel or the screen edge is in the way);
/// zoomed out past [`NAMES_HIDE_FROM`] only the selected planet keeps its label.
fn update_labels(
    view: Res<GalaxyView>,
    radii: Res<PlanetRadii>,
    selection: Res<Selection>,
    (hover, filters): (Res<Hover>, Res<MapFilters>),
    bounds: Res<CameraBounds>,
    ui_scale: Res<UiScale>,
    panel: Res<PanelRect>,
    camera: Single<(&Camera, &GlobalTransform, &GalaxyCamera)>,
    mut labels: Query<(&PlanetLabel, &mut Node, &mut Visibility, &ComputedNode, &mut TextFont, &mut TextColor), Without<PipRow>>,
    mut pip_rows: Query<(&PipRow, &mut Node, &mut Visibility), Without<PlanetLabel>>,
    mut emblems: Query<(&Emblem, &mut Node, &mut Visibility), (Without<PlanetLabel>, Without<PipRow>, Without<PipIcon>)>,
    mut pip_icons: Query<&mut Node, (With<PipIcon>, Without<PlanetLabel>, Without<PipRow>)>,
    mut last_font: Local<f32>,
    mut slots: Local<Vec<Option<LabelSlot>>>,
    (mut visible, mut was_above, mut was_far): (Local<Vec<bool>>, Local<Vec<bool>>, Local<bool>),
) {
    let (camera, cam_tf, orbit) = *camera;
    let Some(viewport) = camera.logical_viewport_size() else {
        return;
    };
    // Labels grow when zoomed in and shrink when zoomed out (whole pixel steps only).
    let zoom_out = zoom_fraction(&bounds, orbit.distance);
    let far = names_far(zoom_out, *was_far);
    *was_far = far;
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
        let r_px = a.distance(b) * 0.95;
        let p = a + Vec2::new(0.0, r_px);
        let measured = computed.size() * computed.inverse_scale_factor() * us;
        let size = if measured.x > 1.0 {
            measured
        } else {
            Vec2::new(planet.display_name.len() as f32 * font_px * 0.6, font_px * 1.3)
        };
        // Room for the two rows of structure slot pips under the name.
        let size = size + Vec2::new(0.0, font_px * 1.1);
        slots[label.0] = Some(LabelSlot { pos: Vec2::new(p.x - size.x * 0.5, p.y), above: Vec2::new(p.x - size.x * 0.5, a.y - r_px - size.y), size });
    }

    // The label colour follows the owner (a captured planet turns green).
    for (label, _, _, _, _, mut color) in &mut labels {
        // Brighter when zoomed in, softer when zoomed out.
        let k = 0.72 + 0.28 * (1.0 - zoom_out.min(1.0));
        let c = rgb(relation_color(&view, view.planets[label.0].owner)).to_srgba();
        color.set_if_neq(TextColor(Color::srgb((c.red * k).min(1.0), (c.green * k).min(1.0), (c.blue * k).min(1.0))));
    }

    visible.clear();
    visible.resize(slots.len(), false);
    // Placement of this frame (`was_above` keeps last frame's for the hysteresis).
    let mut now_above = vec![false; slots.len()];
    // Close up (names shown) only the anchor of a name has to be on screen; zoomed out, for the selected planet's, the whole name.
    let close = !far;
    for (i, slot) in slots.iter_mut().enumerate() {
        let Some(s) = slot.as_mut() else { continue };
        // The selected planet always keeps its label; zoomed out too far it is the only one.
        let selected = selection.0 == Some(i);
        if far && !selected {
            continue;
        }
        // Every planet in view keeps its name, crowding never hides it. A name that would sit off the screen or under a HUD panel (the
        // ticker line) goes above its globe instead; under the command bar it stays where it is and shows through the translucent
        // build strip like in the Remake.
        let above = label_clear(s.above, s.size, viewport, close, &panel, 0.0);
        let was = was_above.get(i).copied().unwrap_or(false);
        let below = below_clear(
            label_clear(s.pos, s.size, viewport, close, &panel, -PLACE_HYSTERESIS),
            was && label_clear(s.pos, s.size, viewport, close, &panel, PLACE_HYSTERESIS),
            above,
            was,
        );
        let moved_up = !below && above;
        if moved_up {
            s.pos = s.above;
        }
        visible[i] = selected || below || above;
        now_above[i] = visible[i] && moved_up;
    }
    *was_above = now_above;

    let draw_names = draw_planet_names();
    for (label, mut node, mut vis, _, _, _) in &mut labels {
        match slots[label.0] {
            Some(s) if visible[label.0] => {
                node.left = Val::Px(s.pos.x / us);
                node.top = Val::Px(s.pos.y / us);
                *vis = if draw_names { Visibility::Inherited } else { Visibility::Hidden };
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

/// Offscreen runs: how many planets on screen (centre in view, not under a HUD panel; the command bar draws over the names) show
/// their name, also for the lower third of the screen, and how often a label appeared or vanished between two frames while the camera
/// stood still (must be 0). One log line every 60 frames: `labels: N of M on screen named, lower third n of m, toggles k, interface
/// rects r`.
fn log_label_stats(
    args: Res<crate::cli::Args>,
    view: Res<GalaxyView>,
    panel: Res<PanelRect>,
    camera: Single<(&Camera, &GlobalTransform), With<GalaxyCamera>>,
    labels: Query<(&PlanetLabel, &Visibility)>,
    mut last: Local<(Option<GlobalTransform>, Vec<bool>, u32, u32)>,
) {
    if args.offscreen.is_none() {
        return;
    }
    let (camera, cam_tf) = *camera;
    let Some(viewport) = camera.logical_viewport_size() else { return };
    let mut shown = vec![false; view.planets.len()];
    for (label, vis) in &labels {
        shown[label.0] = *vis != Visibility::Hidden;
    }
    let (prev_tf, prev, toggles, frames) = &mut *last;
    *frames += 1;
    // The first frames lay the labels out with estimated sizes, then with the measured texts; after that nothing may change.
    if *frames > 10 && prev_tf.is_some_and(|t| t == *cam_tf) && prev.len() == shown.len() {
        *toggles += prev.iter().zip(&shown).filter(|(a, b)| a != b).count() as u32;
    }
    *prev_tf = Some(*cam_tf);
    *prev = shown.clone();
    if *frames % 60 != 0 {
        return;
    }
    let under_ui = |p: Vec2| panel.0.iter().any(|(lo, hi)| p.cmpge(*lo).all() && p.cmple(*hi).all());
    let (mut on, mut named, mut low_on, mut low_named) = (0, 0, 0, 0);
    for (i, p) in view.planets.iter().enumerate() {
        let Ok(c) = camera.world_to_viewport(cam_tf, galaxy_to_world(p.position)) else { continue };
        if c.x < 0.0 || c.y < 0.0 || c.x > viewport.x || c.y > viewport.y || under_ui(c) {
            continue;
        }
        let lower = c.y > viewport.y * 2.0 / 3.0;
        on += 1;
        low_on += u32::from(lower);
        named += u32::from(shown[i]);
        low_named += u32::from(lower && shown[i]);
    }
    info!("labels: {named} of {on} on screen named, lower third {low_named} of {low_on}, toggles {toggles}, interface rects {}", panel.0.len());
    *toggles = 0;
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
/// All lane ribbons of the galaxy in one mesh.
#[derive(Component)]
struct LaneMesh {
    mesh: Handle<Mesh>,
    /// Route indices of the ribbons in vertex order (4 vertices each: two rows of two).
    routes: Vec<usize>,
    /// The shared lane material (its texture scrolls to move the glints).
    material: Handle<LaneMaterial>,
    /// Zoom step the vertex colours were written for: `None` until the first frame paints a new mesh, which starts black. A galaxy that
    /// is spawned again (back to the menu, the same campaign) must be painted whatever the zoom was when the old one went.
    painted: Option<i32>,
}

/// The lane ribbons add in display (sRGB) values like the original engine, which blends in gamma space (`lane.wgsl`). Added in linear
/// light, a lane over the pink core nebula all but vanished: next to Byss the Remake's lane adds about sRGB (+57, +72, +72) to the
/// nebula's (105, 77, 68) (v0240), the linear sum gave (+4, +20, +28). The shader reads what lies under the lane from Bevy's view
/// transmission texture (the opaque pass plus the galaxy disc art, drawn first with `FLAG_UNDERLAY`), so the ribbon is a transmissive
/// "opaque" material that writes the sum itself.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct LaneMaterial {
    /// xy: UV offset of the lane texture (the glints move along v).
    #[uniform(0)]
    scroll: Vec4,
    /// [`lane_texture`].
    #[texture(1)]
    #[sampler(2)]
    texture: Handle<Image>,
}

impl Material for LaneMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://eaw_remake/lane.wgsl".into()
    }

    fn reads_view_transmission_texture(&self) -> bool {
        true
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // A flat ribbon seen from both sides; it hides nothing that is drawn after it.
        descriptor.primitive.cull_mode = None;
        if let Some(d) = descriptor.depth_stencil.as_mut() {
            d.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// What a lane adds over `under` (linear RGB), as `lane.wgsl` computes it: `amount` is in display values (the vertex colour times
/// the lane texture).
#[cfg(test)]
fn lane_over(under: [f32; 3], amount: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| {
        let shown = Color::linear_rgb(under[i], 0.0, 0.0).to_srgba().red + amount[i];
        Color::srgb(shown, 0.0, 0.0).to_linear().red
    })
}

/// World width of a lane per unit of its data `Width` (`TRADEROUTELINES.XML`: main 6.0, inter 2.5, capillary 0.5, hidden 0). Estimate
/// measured on the user's Remake video at full HD: the main lanes Ansion - Bilbringi (v0520, 8.3 px per map unit) and Rendili - Corellia
/// (v0540, 8.6 px per unit) are 0.8 to 1.0 map units wide, the inter lane west of Ansion about 0.3: a flat band of 0.14 units per width unit.
const LANE_WIDTH_PER_DATA: f32 = 0.14;
/// Brightness on the data colour (additive teal 13, 31, 38) in linear light. Estimate from the same frames: on black space a main lane
/// reads about sRGB (32..44, 62..68, 76..80) in its middle, 3.6 times the data colour, and has no glow around it.
const LANE_GAIN: f32 = 3.6;
/// Share of the full band brightness the lane shows outside a glint, in display values (the lanes add in sRGB, see [`LaneMaterial`]):
/// a glint (v0520, on Ansion - Bilbringi) reads about 1.6 times as bright as the lane on screen (sRGB 60, 107, 122 against 40, 63,
/// 77), over about 3 map units of its length, same width, soft ends.
const LANE_BASE: f32 = 0.625;
/// Map units between two glints on a lane, glint length and speed (map units per second, the speed of the old pulses). Estimates: the
/// stills show about one glint per lane on screen, they cannot show the motion.
const GLINT_PERIOD: f32 = 40.0;
const GLINT_LENGTH: f32 = 3.0;
const GLINT_SPEED: f32 = 2.0;

/// Lane texture: a flat band with soft edges across the ribbon (u) and one glint per period along it (v, repeating).
fn lane_texture() -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let (w, h) = (32u32, 128u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        // Distance to the glint centre in map units (the glint sits in the middle of the period).
        let d = ((y as f32 + 0.5) / h as f32 - 0.5).abs() * GLINT_PERIOD;
        let glint = 1.0 - smoothstep(0.0, GLINT_LENGTH * 0.5, d);
        let along = LANE_BASE + (1.0 - LANE_BASE) * glint;
        for x in 0..w {
            let u = ((x as f32 + 0.5) / w as f32 - 0.5).abs() * 2.0;
            let profile = 1.0 - smoothstep(0.7, 1.0, u);
            px.extend_from_slice(&[255, 255, 255, ((profile * along).clamp(0.0, 1.0) * 255.0) as u8]);
        }
    }
    let mut img = Image::new(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    img
}

/// Width of a route's ribbon in world units (0 for a hidden route).
fn lane_width(style: &RouteStyleView) -> f32 {
    style.neutral.width.max(style.own.width).max(style.enemy.width).max(0.0) * LANE_WIDTH_PER_DATA
}

/// The ribbon of one lane: exactly from planet centre to planet centre (the globes cover its ends), `width` wide, flat in the galaxy
/// plane. `v` runs along it in glint periods from `phase`.
fn lane_quad(pa: Vec3, pb: Vec3, width: f32, phase: f32) -> ([[f32; 3]; 4], [[f32; 2]; 4]) {
    let dir = (pb - pa).normalize_or_zero();
    let side = Vec3::Y.cross(dir).try_normalize().unwrap_or(Vec3::X) * width * 0.5;
    let v1 = phase + pa.distance(pb) / GLINT_PERIOD;
    ([(pa - side).to_array(), (pa + side).to_array(), (pb - side).to_array(), (pb + side).to_array()], [[0.0, phase], [1.0, phase], [0.0, v1], [1.0, v1]])
}

/// Spawns the lane ribbon mesh of the galaxy.
fn spawn_lanes(
    commands: &mut Commands,
    view: &GalaxyView,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LaneMaterial>,
    images: &mut Assets<Image>,
) {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let routes = lane_routes(view);
    if routes.is_empty() {
        return;
    }
    let material = materials.add(LaneMaterial { scroll: Vec4::ZERO, texture: images.add(lane_texture()) });
    let (mut pos, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    for &i in &routes {
        let r = &view.routes[i];
        let (a, b) = (&view.planets[r.a], &view.planets[r.b]);
        // Every lane gets its own glint phase, so the glints of neighbouring lanes do not line up.
        let phase = (i as f32 * 0.618_034).fract();
        let (p, t) = lane_quad(galaxy_to_world(a.position), galaxy_to_world(b.position), lane_width(&view.route_styles[r.style]), phase);
        let k = pos.len() as u32;
        pos.extend(p);
        uv.extend(t);
        idx.extend([k, k + 1, k + 2, k + 2, k + 1, k + 3]);
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
        LaneMesh { mesh: handle, routes, material, painted: None },
        GalaxyEntity,
    ));
}

/// What a lane drawn in `style` at zoom `t` (0 close, 1 far) adds at a glint, in display values, as the vertex colour of its ribbon
/// (see [`LaneMaterial`]); black when the style hides it. The data colour (`Color_Zoomed_In` / `Color_Zoomed_Out`, additive teal 13, 31,
/// 38 for every relation in the Remake) is the colour, its alpha fades the lane (the Remake's `Enemy` look fades out when zoomed in; the
/// map does not draw a lane in that look, see [`lane_line`]); `LANE_GAIN` brings it to the frames' brightness.
fn lane_color(style: &LineStyleView, t: f32) -> [f32; 4] {
    let mix = |i: usize| style.color_near[i] + (style.color_far[i] - style.color_near[i]) * t;
    let alpha = mix(3);
    if style.width <= 0.0 || alpha <= 0.01 {
        return [0.0; 4];
    }
    let l = Color::srgb(mix(0), mix(1), mix(2)).to_linear();
    let band = Color::linear_rgb(l.red * LANE_GAIN, l.green * LANE_GAIN, l.blue * LANE_GAIN).to_srgba();
    // The texture carries the band at `LANE_BASE` outside the glints.
    let k = alpha / LANE_BASE;
    [band.red * k, band.green * k, band.blue * k, 1.0]
}

/// The routes that get a ribbon, in ribbon order: every route between two planets of the map whose style has a width (the data's `Hidden`
/// style has none).
fn lane_routes(view: &GalaxyView) -> Vec<usize> {
    let on_map = |p: usize| view.planets.get(p).is_some();
    let wide = |s: usize| view.route_styles.get(s).is_some_and(|s| lane_width(s) > 0.0);
    view.routes.iter().enumerate().filter(|(_, r)| on_map(r.a) && on_map(r.b) && wide(r.style)).map(|(i, _)| i).collect()
}

/// The look every lane of `style` has, whoever holds the planets at its ends. The data gives a route one look per relation to the player
/// (`Neutral`, `Ally`, `Enemy`), and in the Remake's main and inter styles the `Enemy` look fades out when zoomed in
/// (`Color_Zoomed_In` alpha 0). Picked by the owners of a lane's planets, that look hid every lane between two planets of one other
/// faction, so only the lanes at the player's own and unclaimed planets showed: at the Imperial start 54 of the 189 routes were faded
/// at the opening zoom (0.25) and only came back at the farthest zoom, 64 of 189 at the Rebel start, and each conquest lit the lanes at
/// the new planet. The map shows every route at all times (design request, 2026-10-17: the connection between two planets must always be visible),
/// so a lane takes the look of a route nobody holds, which is the same teal as the other looks in the data.
fn lane_line(style: &RouteStyleView) -> &LineStyleView {
    &style.neutral
}

/// Vertex colour of the ribbon of `route` at zoom `t` (0 close, 1 far).
fn route_lane_color(view: &GalaxyView, route: usize, t: f32) -> [f32; 4] {
    lane_color(lane_line(&view.route_styles[view.routes[route].style]), t)
}

/// What the map draws of the campaign's routes at zoom `t`: for the log line and the tests.
#[derive(Debug, PartialEq, Eq)]
struct LaneReport {
    /// Routes between two planets of the map.
    routes: usize,
    /// Routes that have a ribbon (the others use a hidden style).
    drawn: usize,
    /// Ribbons at least as bright at this zoom as the lane between two unowned planets of their style: a ribbon whose colour is black or
    /// faded counts as drawn but not as bright.
    bright: usize,
}

fn lane_report(view: &GalaxyView, t: f32) -> LaneReport {
    let drawn = lane_routes(view);
    let bright = drawn
        .iter()
        .filter(|&&ri| {
            let full = lane_color(&view.route_styles[view.routes[ri].style].neutral, t);
            let col = route_lane_color(view, ri, t);
            full[..3].iter().any(|&c| c > 0.0) && (0..3).all(|i| col[i] >= full[i] * 0.99)
        })
        .count();
    LaneReport { routes: view.routes.len(), drawn: drawn.len(), bright }
}

/// Keeps the ribbon colours in sync with the zoom (never with who holds the planets, see [`lane_line`]) and moves the glints along the
/// lanes. The first frame of a new lane mesh paints it and logs what is drawn.
fn update_lanes(
    view: Res<GalaxyView>,
    bounds: Res<CameraBounds>,
    time: Res<Time>,
    camera: Single<&GalaxyCamera>,
    mut lanes: Query<&mut LaneMesh>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LaneMaterial>>,
) {
    // The glints travel from the first end of every lane to the second.
    let scroll = (time.elapsed_secs() * GLINT_SPEED / GLINT_PERIOD).fract();
    for lane in &lanes {
        if let Some(mut m) = materials.get_mut(&lane.material) {
            m.scroll = Vec4::new(0.0, -scroll, 0.0, 0.0);
        }
    }
    let t = zoom_fraction(&bounds, camera.distance);
    let step = (t * 24.0) as i32;
    for mut lane in &mut lanes {
        if lane.painted == Some(step) {
            continue;
        }
        let col: Vec<[f32; 4]> = lane.routes.iter().flat_map(|&ri| [route_lane_color(&view, ri, t); 4]).collect();
        let Some(mut m) = meshes.get_mut(&lane.mesh) else { continue };
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
        if lane.painted.is_none() {
            let r = lane_report(&view, t);
            info!("lanes: {} routes in the campaign, {} drawn, {} at full brightness at zoom {t:.2} ({} use a hidden style)", r.routes, r.drawn, r.bright, r.routes - r.drawn);
        }
        lane.painted = Some(step);
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
    fn the_sector_shadow_follows_the_remake_profile() {
        // b0613s against v0240s at the same map points south of Coruscant (ring radius 61.5 map units): distance outside the ring in
        // map units and the brightness ratio of the map there (sRGB values).
        let remake = [(1.4, 0.45), (4.3, 0.45), (7.2, 0.48), (12.0, 0.55), (16.8, 0.67), (21.7, 0.77), (26.6, 0.88), (31.5, 0.93), (36.4, 0.95)];
        for (d, ratio) in remake {
            let ours = sector_shadow_srgb(d / 61.5);
            assert!((ours - ratio).abs() < 0.06, "{d} map units out: {ours:.3} against {ratio}");
        }
        // Darker the closer to the ring, back to normal at the outer edge.
        let profile: Vec<f32> = (0..=20).map(|k| sector_shadow_srgb(SECTOR_SHADOW_WIDTH * k as f32 / 20.0)).collect();
        assert!(profile.windows(2).all(|w| w[1] >= w[0]), "{profile:?}");
        assert!((profile[20] - 1.0).abs() < 1e-4);
        // The mesh carries the factor in linear light: darkest at the ring, untouched at the outer edge.
        let mesh = sector_shadow_mesh();
        let Some(bevy::mesh::VertexAttributeValues::Float32x4(col)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!("no vertex colours") };
        assert!((col[0][0] - 0.44f32.powf(SECTOR_SHADOW_GAMMA)).abs() < 1e-3, "{:?}", col[0]);
        assert!((col[col.len() - 1][0] - 1.0).abs() < 1e-4 && col.iter().all(|c| c[3] == 1.0));
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

    #[test]
    fn names_in_the_lower_part_show_and_only_hud_panels_move_them() {
        // 1080p map. The command bar is no obstacle: the names are drawn below it and show through its translucent build strip like
        // in the Remake (b0613s: LORONAR). Its bounding box, the whole bottom from y 629, used to hide every name there.
        let viewport = Vec2::new(1920.0, 1080.0);
        let size = Vec2::new(90.0, 24.0);
        let ticker = PanelRect(vec![(Vec2::new(88.0, 679.0), Vec2::new(680.0, 702.0))], Vec::new());
        let bbox = PanelRect(vec![(Vec2::new(-16.0, 629.0), Vec2::new(1947.0, 1080.0))], Vec::new());
        for close in [true, false] {
            // Above the build strip (y 715), at 70 % of the height (the user's missing Fresia / N'Zoth) and behind the strip (y 950).
            for y in [715.0, 756.0, 950.0] {
                let below = Vec2::new(1228.0 - 45.0, y);
                assert!(label_clear(below, size, viewport, close, &ticker, 0.0), "y {y}");
                assert!(!label_clear(below, size, viewport, close, &bbox, 0.0), "y {y}");
            }
        }
        // A name on the ticker line goes above its globe: below the globe it is covered, above it is clear.
        let (below, above) = (Vec2::new(400.0, 680.0), Vec2::new(400.0, 640.0));
        assert!(!label_clear(below, size, viewport, false, &ticker, 0.0) && label_clear(above, size, viewport, false, &ticker, 0.0));
        // At the bottom edge only the anchor has to be on screen close up; zoomed out the whole name does.
        assert!(label_clear(Vec2::new(900.0, 1070.0), size, viewport, true, &ticker, 0.0));
        assert!(!label_clear(Vec2::new(900.0, 1070.0), size, viewport, false, &ticker, 0.0));
    }

    /// Runs the placement of one name over `frames` frames (below-globe position per frame, above-globe position fixed) and returns
    /// how often it switched between below and above its globe.
    fn name_switches(below_at: impl Fn(u32) -> Vec2, above: Vec2, close: bool, panel: &PanelRect, hysteresis: bool, frames: u32) -> u32 {
        let (viewport, size) = (Vec2::new(1920.0, 1080.0), Vec2::new(28.0, 23.0));
        let (mut was, mut switches) = (false, 0);
        for t in 0..frames {
            let pos = below_at(t);
            let above = label_clear(above, size, viewport, close, panel, 0.0);
            let below = if hysteresis {
                below_clear(
                    label_clear(pos, size, viewport, close, panel, -PLACE_HYSTERESIS),
                    was && label_clear(pos, size, viewport, close, panel, PLACE_HYSTERESIS),
                    above,
                    was,
                )
            } else {
                label_clear(pos, size, viewport, close, panel, 0.0)
            };
            let up = !below && above;
            switches += u32::from(t > 0 && up != was);
            was = up;
        }
        switches
    }

    #[test]
    fn far_zoom_hides_the_names_once_each_way_without_blinking() {
        // 240 frames: a slow zoom out from 0.40 to 0.52 and back in, with a 0.01 jitter on top (a wheel step easing in, an unsteady
        // hand). With one threshold the names blink; with the hysteresis they go once and come back once.
        let zoom = |t: u32| 0.40 + 0.12 * (1.0 - (t as f32 / 120.0 - 1.0).abs()) + 0.01 * (t as f32 * 0.9).sin();
        let (mut far, mut switches) = (false, 0);
        let (mut single, mut single_switches) = (false, 0);
        for t in 0..240 {
            let now = names_far(zoom(t), far);
            switches += u32::from(now != far);
            far = now;
            let now = zoom(t) >= 0.45;
            single_switches += u32::from(now != single);
            single = now;
        }
        assert!(single_switches > 2, "with a single threshold the case blinks ({single_switches} switches)");
        assert_eq!(switches, 2);
        assert!(!far, "back in, the names are back");
        // Resting at the threshold with a 0.015 jitter for 120 frames: nothing changes, whichever way the zoom came from.
        for start in [false, true] {
            let mut far = start;
            for t in 0..120 {
                far = names_far(0.45 + 0.015 * (t as f32 * 0.9).sin(), far);
                assert_eq!(far, start, "came from far {start}, frame {t}");
            }
        }
        // Close up there are names, far out none, whatever the last frame said.
        assert!(!names_far(0.30, false) && !names_far(0.30, true) && names_far(0.60, false) && names_far(0.60, true));
    }

    #[test]
    fn a_name_grazing_a_panel_or_the_screen_edge_does_not_blink() {
        // Ithor at zoom 0.21 (`--label-probe`, Yavin): its name under the globe touched the top of the advisor's text line by 0.9 px
        // for 5 frames and jumped above the globe and back. Here the place under the globe jitters by 0.8 px for 120 frames.
        let ticker = PanelRect(vec![(Vec2::new(88.0, 642.0), Vec2::new(680.0, 702.0))], Vec::new());
        let jitter = |base: Vec2| move |t: u32| base + Vec2::new(0.0, 0.8 * (t as f32 * 0.9).sin());
        let up = Vec2::new(70.0, 560.0);
        for close in [true, false] {
            // Grazing the panel: without hysteresis the name blinks, with it it stays under its globe.
            assert!(name_switches(jitter(Vec2::new(70.0, 617.0)), up, close, &ticker, false, 120) > 10, "close {close}: the case blinks");
            assert_eq!(name_switches(jitter(Vec2::new(70.0, 617.0)), up, close, &ticker, true, 120), 0, "close {close}");
            // Covered by 2 px and more: the name goes above once and stays there.
            assert_eq!(name_switches(jitter(Vec2::new(70.0, 621.0)), up, close, &ticker, true, 120), 1, "close {close}");
            // A slow sweep into the panel and out again (zoom in, then out) with a 1.5 px jitter: once each way.
            let sweep = |t: u32| Vec2::new(70.0, 604.0 + 24.0 * (1.0 - (t as f32 / 60.0 - 1.0).abs()) + 1.5 * (t as f32 * 0.9).sin());
            assert!(name_switches(sweep, up, close, &ticker, false, 120) > 2, "close {close}: the sweep blinks");
            assert_eq!(name_switches(sweep, up, close, &ticker, true, 120), 2, "close {close}");
        }
        // Close up, the anchor of a name jittering across the bottom edge of the screen: the name goes above once (it never sits
        // below the screen edge) and stays there.
        let at_edge = jitter(Vec2::new(900.0, 1080.0));
        assert!(name_switches(at_edge, Vec2::new(900.0, 1000.0), true, &ticker, false, 120) > 10);
        assert_eq!(name_switches(at_edge, Vec2::new(900.0, 1000.0), true, &ticker, true, 120), 1);
        let size = Vec2::new(28.0, 23.0);
        assert!(!label_clear(Vec2::new(900.0, 1081.0), size, Vec2::new(1920.0, 1080.0), true, &ticker, -PLACE_HYSTERESIS));
        // Nowhere to go above: the name stays under its globe whenever the lenient check holds.
        assert!(below_clear(true, false, false, true) && !below_clear(true, false, true, true) && below_clear(true, false, true, false));
    }

    /// The Remake's `Web_Way_Main` style: additive teal (13, 31, 38), width 6; the enemy look fades out when zoomed in.
    fn remake_style(width: f32) -> RouteStyleView {
        let line = |alpha_near: f32| LineStyleView { color_far: [13.0 / 255.0, 31.0 / 255.0, 38.0 / 255.0, 1.0], color_near: [13.0 / 255.0, 31.0 / 255.0, 38.0 / 255.0, alpha_near], width };
        RouteStyleView { name: "Web_Way_Main".into(), own: line(1.0), enemy: line(0.0), neutral: line(1.0) }
    }

    #[test]
    fn a_lane_runs_exactly_from_planet_centre_to_planet_centre() {
        let (a, b) = (Vec3::new(-108.0, 10.0, -267.0), Vec3::new(-62.0, 10.0, -192.0));
        let (pos, uv) = lane_quad(a, b, 0.84, 0.25);
        let p = pos.map(Vec3::from_array);
        // No overshoot: the ends of the ribbon are the two centres, its sides `width` apart.
        assert!(((p[0] + p[1]) * 0.5).distance(a) < 1e-4 && ((p[2] + p[3]) * 0.5).distance(b) < 1e-4);
        assert!((p[0].distance(p[1]) - 0.84).abs() < 1e-4 && (p[2].distance(p[3]) - 0.84).abs() < 1e-4);
        // Flat in the galaxy plane, and `v` counts glint periods along it.
        assert!(p.iter().all(|q| (q.y - 10.0).abs() < 1e-4));
        assert!((uv[2][1] - uv[0][1] - a.distance(b) / GLINT_PERIOD).abs() < 1e-5);
    }

    #[test]
    fn lane_width_follows_the_data_width() {
        // Main 6.0, inter 2.5, capillary 0.5: about 0.84, 0.35 and 0.07 map units (main lanes measured 0.8 to 1.0 on v0520 / v0540).
        assert!((lane_width(&remake_style(6.0)) - 0.84).abs() < 1e-4);
        assert!((lane_width(&remake_style(2.5)) - 0.35).abs() < 1e-4);
        assert!(lane_width(&remake_style(6.0)) > 2.0 * lane_width(&remake_style(2.5)));
        assert_eq!(lane_width(&remake_style(0.0)), 0.0);
    }

    #[test]
    fn lane_colour_is_the_data_colour_at_the_frames_brightness() {
        let style = remake_style(6.0);
        // On black space the band (at `LANE_BASE` of the texture) shows the measured lane colour, about sRGB (32, 62, 76).
        let c = lane_color(&style.neutral, 1.0);
        let over_black = lane_over([0.0; 3], [c[0] * LANE_BASE, c[1] * LANE_BASE, c[2] * LANE_BASE]);
        let shown = Color::linear_rgb(over_black[0], over_black[1], over_black[2]).to_srgba();
        let px = [shown.red, shown.green, shown.blue].map(|v| v * 255.0);
        assert!((px[0] - 32.0).abs() < 6.0 && (px[1] - 62.0).abs() < 6.0 && (px[2] - 76.0).abs() < 6.0, "{px:?}");
        // The same teal for every relation (no invented green or red lanes); the enemy style's alpha fades it out close up.
        assert_eq!(lane_color(&style.own, 0.3), lane_color(&style.neutral, 0.3));
        assert_eq!(lane_color(&style.enemy, 0.0), [0.0; 4]);
        assert!(lane_color(&style.enemy, 1.0)[1] > 0.0);
        // A hidden style draws nothing.
        assert_eq!(lane_color(&remake_style(0.0).neutral, 1.0), [0.0; 4]);
    }

    /// The demo galaxy with the Remake's lane styles spread over its routes: main, inter and hidden. Its owners give every kind of lane:
    /// the player's own, between two planets of one other faction (the Enemy look fades out when zoomed in), between rivals, unowned.
    fn lane_test_view() -> GalaxyView {
        let mut view = GalaxyView::demo();
        view.route_styles = vec![remake_style(6.0), remake_style(2.5), remake_style(0.0)];
        for (i, r) in view.routes.iter_mut().enumerate() {
            r.style = i % 3;
        }
        view
    }

    /// Zooms from the closest view to the farthest, the map's start views (0.15, 0.25) among them.
    const LANE_ZOOMS: [f32; 8] = [0.0, 0.01, 0.06, 0.15, 0.25, 0.5, 0.75, 1.0];

    #[test]
    fn every_drawn_lane_is_as_bright_as_the_next_at_every_zoom_whoever_holds_its_planets() {
        let view = lane_test_view();
        let held_by_another = |r: &crate::galaxy_view::RouteView| {
            let (a, b) = (view.planets[r.a].owner, view.planets[r.b].owner);
            a == b && a.is_some() && a != view.player_faction
        };
        assert!(view.routes.iter().filter(|r| held_by_another(r)).count() >= 5, "the test galaxy needs lanes between planets of another faction");
        let shown = view.routes.iter().filter(|r| r.style != 2).count();
        for t in LANE_ZOOMS {
            let report = lane_report(&view, t);
            assert_eq!(report, LaneReport { routes: view.routes.len(), drawn: shown, bright: shown }, "zoom {t}");
            for ri in lane_routes(&view) {
                let neutral = &view.route_styles[view.routes[ri].style].neutral;
                assert_eq!(route_lane_color(&view, ri, t), lane_color(neutral, t), "route {ri} at zoom {t}");
            }
        }
    }

    #[test]
    fn a_galaxy_spawned_again_gets_its_lane_colours_at_the_same_zoom_and_owners() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<LaneMaterial>>();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<Time>();
        world.insert_resource(lane_test_view());
        world.insert_resource(CameraBounds::default());
        world.spawn(GalaxyCamera { focus: Vec3::ZERO, target_focus: Vec3::ZERO, distance: 200.0, target_distance: 200.0, yaw: 0.0, pitch: 0.9 });
        let spawn = |world: &mut World| {
            world
                .run_system_once(|mut commands: Commands, view: Res<GalaxyView>, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<LaneMaterial>>, mut images: ResMut<Assets<Image>>| {
                    spawn_lanes(&mut commands, &view, &mut meshes, &mut materials, &mut images)
                })
                .expect("spawn the lanes");
        };
        // Ribbons with a colour, as the renderer sees the mesh.
        let painted = |world: &mut World| -> usize {
            let mesh = world.query::<&LaneMesh>().single(world).expect("one lane mesh").mesh.clone();
            let Some(bevy::mesh::VertexAttributeValues::Float32x4(col)) = world.resource::<Assets<Mesh>>().get(&mesh).and_then(|m| m.attribute(Mesh::ATTRIBUTE_COLOR)).cloned() else {
                panic!("no vertex colours")
            };
            col.chunks(4).filter(|ribbon| ribbon[0][..3].iter().any(|&c| c > 0.0)).count()
        };
        let mut frame = Schedule::default();
        frame.add_systems(update_lanes);
        let ribbons = lane_routes(world.resource::<GalaxyView>()).len();
        spawn(&mut world);
        frame.run(&mut world);
        assert_eq!(painted(&mut world), ribbons, "the first galaxy");
        // Back to the menu and the same campaign again: a new mesh, black until painted, with the zoom and the owners of the old one.
        let old = world.query::<(Entity, &LaneMesh)>().single(&world).map(|(e, _)| e).expect("lane mesh");
        world.despawn(old);
        spawn(&mut world);
        frame.run(&mut world);
        assert_eq!(painted(&mut world), ribbons, "the galaxy spawned again");
    }

    #[test]
    fn the_full_web_way_campaigns_draw_every_route_of_their_data_at_full_brightness() {
        use crate::cli::Args;
        use crate::game::{StartupLog, build_view, load_assets};
        let Ok(assets) = load_assets(&Args::default(), &mut StartupLog::default()) else {
            eprintln!("skipped: no game install");
            return;
        };
        let mut short = Vec::new();
        for name in ["GC_Full_Web_Way_Imperial", "GC_Full_Web_Way_Rebel"] {
            let Some(campaign) = assets.data.campaign(name) else {
                eprintln!("skipped: campaign {name} missing");
                continue;
            };
            let (view, notes) = build_view(&assets.data, campaign);
            let (listed, defined) = (campaign.trade_routes.len(), assets.data.campaign_trade_routes(campaign).len());
            for t in LANE_ZOOMS {
                let r = lane_report(&view, t);
                eprintln!(
                    "{name} at zoom {t}: the campaign lists {listed} routes, {defined} are defined, {} lie between planets of the map, {} are drawn, {} at full brightness",
                    r.routes, r.drawn, r.bright
                );
                if (defined, r.routes, r.drawn, r.bright) != (listed, listed, listed, listed) {
                    short.push(format!("{name} at zoom {t}: {listed} listed, {defined} defined, {} on the map, {} drawn, {} bright ({notes:?})", r.routes, r.drawn, r.bright));
                }
            }
        }
        assert!(short.is_empty(), "{short:#?}");
    }

    #[test]
    fn the_lane_band_is_flat_with_one_glint_per_period() {
        let img = lane_texture();
        let (w, h) = (img.width() as usize, img.height() as usize);
        let data = img.data.as_ref().expect("cpu data");
        let alpha = |x: usize, y: usize| data[(y * w + x) * 4 + 3];
        let base = (LANE_BASE * 255.0) as u8;
        // Across: full in the middle and on most of the width, nothing at the very edge.
        assert!(alpha(w / 2, 0).abs_diff(base) <= 1 && alpha(w / 4, 0).abs_diff(base) <= 1 && alpha(0, 0) < base / 4);
        // Along: the band outside the glint, full brightness at the glint centre (middle of the period).
        assert!(alpha(w / 2, h / 2) >= 245);
        assert!(alpha(w / 2, h / 4).abs_diff(base) <= 1);
    }

    #[test]
    fn a_lane_adds_the_same_on_screen_over_the_core_nebula_as_on_black() {
        let c = lane_color(&remake_style(6.0).neutral, 1.0);
        let band = [c[0] * LANE_BASE, c[1] * LANE_BASE, c[2] * LANE_BASE];
        // The nebula next to Byss in v0240, sRGB (105, 77, 68); the Remake's lane adds about (+57, +72, +72) there (the video's
        // chroma subsampling pulls a thin line toward white), on black about its own colour (40, 63, 77).
        let nebula = Color::srgb_u8(105, 77, 68).to_linear();
        let under = [nebula.red, nebula.green, nebula.blue];
        let screen = |lin: [f32; 3]| {
            let s = Color::linear_rgb(lin[0], lin[1], lin[2]).to_srgba();
            [s.red * 255.0 - 105.0, s.green * 255.0 - 77.0, s.blue * 255.0 - 68.0]
        };
        let added = screen(lane_over(under, band));
        for (a, b) in added.iter().zip(band) {
            assert!((a - b * 255.0).abs() < 1.0, "{added:?} {band:?}");
        }
        // The same band added in linear light: about (+5, +21, +32), the lane all but vanished.
        let over_black = lane_over([0.0; 3], band);
        let linear = screen(std::array::from_fn(|i| under[i] + over_black[i]));
        assert!(linear[1] < 25.0 && added[1] > 2.5 * linear[1], "{linear:?} {added:?}");
    }

    #[test]
    fn the_lane_material_draws_after_the_copy_of_the_backdrop() {
        // Bevy draws a blended material in the transparent phase whatever it reads; only an opaque or masked one that reads the view
        // transmission texture goes to the transmissive phase, after the copy of the opaque pass and the disc art.
        let m = LaneMaterial { scroll: Vec4::ZERO, texture: Handle::default() };
        assert!(m.reads_view_transmission_texture());
        assert!(matches!(m.alpha_mode(), AlphaMode::Opaque | AlphaMode::Mask(_)));
    }

    #[test]
    fn the_core_glow_stays_in_the_measured_window() {
        // Measured against two Remake frames (0.39 on v0240, 0.35 on b0613s); 0.14 left the core a maroon haze. A new value needs
        // new measurements (see CORE_GLOW).
        assert!((0.35..=0.39).contains(&CORE_GLOW), "{CORE_GLOW}");
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
    const SAMPLES: usize = 720;
    // Remake f0007/f0019: about 63 dashes round the circle, dash about twice the gap: 4 samples on, 2 off.
    const DASH: usize = 3;
    let Some(i) = selected.filter(|&i| i < view.planets.len()) else { return Vec::new() };
    let p = &view.planets[i];
    let c = galaxy_to_world(p.position);
    let r = (radii.0[i] * TERRITORY_RADIUS).clamp(30.0, 85.0);
    let l = rgb(select_tint(view, p.owner)).to_linear();
    let color = Color::LinearRgba(LinearRgba::new(l.red, l.green, l.blue, 0.8));
    let a = |k: usize| k as f32 / SAMPLES as f32 * std::f32::consts::TAU;
    let pt = |k: usize| c + Vec3::new(a(k).cos() * r, 0.0, a(k).sin() * r);
    (0..SAMPLES).filter(|k| k % (DASH + 3) < DASH).map(|k| (pt(k), pt(k + 1), color)).collect()
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

/// The shadow the Remake lays round the selected planet's dashed sector ring: nothing changes inside the ring, right outside it the
/// map shows at 0.44 of its brightness (sRGB values) and it is back to normal 0.62 ring radii out. Measured on the user's video at the
/// same map points with and without a selection (homography through the planets of each frame, 11x11 px medians): b0613s against
/// v0240s on a line south of Coruscant gives 0.97-1.08 inside, 0.45 / 0.45 / 0.50 at 1.4 / 4.3 / 7.2 map units out (ring radius
/// 61.5), 0.55 / 0.67 / 0.77 / 0.88 / 0.93 at 12 / 17 / 22 / 27 / 32 and 0.95-0.98 beyond 36; v0220s south of Fresia (orange ring)
/// 0.87-0.93 inside and 0.42-0.48 right outside. The same for every owner and colour-neutral (red and green ratios agree).
#[derive(Component)]
struct SectorShadow;

/// Width of the shadow outside the ring, in ring radii.
const SECTOR_SHADOW_WIDTH: f32 = 0.62;

/// Brightness factor (sRGB values) of the map `d` ring radii outside the selected planet's sector ring (see [`SectorShadow`]).
fn sector_shadow_srgb(d: f32) -> f32 {
    1.0 - 0.56 * (1.0 - smoothstep(0.065, SECTOR_SHADOW_WIDTH, d))
}

/// The Remake scales 8-bit sRGB values; the camera multiplies in linear light, where the same darkening is a power of the factor that
/// depends on the brightness behind it: 1.85 for the map's typical 70/255 inside the shadow (our --zoom 0.15 render: luma p10 33, median
/// 70, p90 95), about 1.5 on the darkest space (a bit darker than the Remake there) and 2.0 near the core. 2.2 left 0.28 instead of 0.45
/// right outside the ring on dark space.
const SECTOR_SHADOW_GAMMA: f32 = 1.85;

/// Unit ring (inner radius 1, in the map plane) out to `1 + SECTOR_SHADOW_WIDTH`, with the brightness factor of
/// [`sector_shadow_srgb`] in linear light (power [`SECTOR_SHADOW_GAMMA`]) as vertex colour.
fn sector_shadow_mesh() -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    const SEGMENTS: usize = 128;
    const STEPS: usize = 16;
    let (mut pos, mut col, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    for s in 0..=STEPS {
        let d = SECTOR_SHADOW_WIDTH * s as f32 / STEPS as f32;
        let f = sector_shadow_srgb(d).powf(SECTOR_SHADOW_GAMMA);
        for k in 0..SEGMENTS {
            let th = k as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            pos.push([(1.0 + d) * th.cos(), 0.0, (1.0 + d) * th.sin()]);
            col.push([f, f, f, 1.0]);
        }
    }
    for s in 0..STEPS {
        for k in 0..SEGMENTS {
            let (a, b) = ((s * SEGMENTS + k) as u32, (s * SEGMENTS + (k + 1) % SEGMENTS) as u32);
            let (c, d) = (a + SEGMENTS as u32, b + SEGMENTS as u32);
            idx.extend([a, c, b, b, c, d]);
        }
    }
    let n = pos.len();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// Lays the sector shadow round the selected planet's dashed ring (same radius as `territory_segments`), just under the map plane.
/// `SP_NO_SECTOR_SHADOW=1` leaves it out, for comparison renders of the same view.
fn place_sector_shadow(
    view: Res<GalaxyView>,
    radii: Res<PlanetRadii>,
    selection: Res<Selection>,
    mut shadow: Single<(&mut Transform, &mut Visibility), With<SectorShadow>>,
) {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let (tf, vis) = &mut *shadow;
    let off = *OFF.get_or_init(|| std::env::var_os("SP_NO_SECTOR_SHADOW").is_some());
    match selection.0.filter(|&i| i < view.planets.len() && !off) {
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
