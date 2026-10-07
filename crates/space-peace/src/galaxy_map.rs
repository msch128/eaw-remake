//! Rendering and interaction of the galactic conquest map: starfield, planets, owner halos,
//! trade routes, name labels and click selection.

use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::NoFrustumCulling,
    gizmos::config::{DefaultGizmoConfigGroup, GizmoConfigStore},
    light::GlobalAmbientLight,
    mesh::{SphereKind, SphereMeshBuilder},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    window::PrimaryWindow,
};
use rand::{Rng, SeedableRng, rngs::SmallRng};

use crate::camera::{CameraBounds, GalaxyCamera};
use crate::galaxy_view::GalaxyView;

/// Draws the [`GalaxyView`] resource and handles planet selection.
pub struct GalaxyMapPlugin;

impl Plugin for GalaxyMapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Selection>()
            .insert_resource(ClearColor(Color::BLACK))
            .insert_resource(GlobalAmbientLight {
                color: Color::srgb(0.55, 0.62, 0.85),
                brightness: 220.0,
                ..default()
            })
            .add_systems(Startup, (setup_scene, spawn_galaxy).chain())
            .add_systems(
                Update,
                (
                    pick_planet,
                    update_labels,
                    update_selection_ring,
                    draw_routes,
                    follow_camera_skybox,
                ),
            );
    }
}

/// Currently selected planet (index into [`GalaxyView::planets`]).
#[derive(Resource, Default, Debug)]
pub struct Selection(pub Option<usize>);

/// Marks a planet's screen-space name label.
#[derive(Component)]
struct PlanetLabel(usize);

/// Marks the shared selection highlight ring.
#[derive(Component)]
struct SelectionRing;

/// Marks the sky sphere that follows the camera.
#[derive(Component)]
struct Skybox;

/// Converts EaW galactic coordinates (x/y plane, z up) to Bevy world space (y up).
pub fn galaxy_to_world(p: Vec3) -> Vec3 {
    Vec3::new(p.x, p.z, -p.y)
}

/// World-space radius of a planet sphere.
fn planet_radius(size: f32) -> f32 {
    4.5 * size
}

/// Converts an sRGB triple to a Bevy colour.
fn rgb(c: [f32; 3]) -> Color {
    Color::srgb(c[0], c[1], c[2])
}

/// Colour used for neutral planets and unowned route ends.
const NEUTRAL: [f32; 3] = [0.55, 0.60, 0.70];

fn faction_color(view: &GalaxyView, owner: Option<usize>) -> [f32; 3] {
    owner
        .and_then(|o| view.factions.get(o))
        .map_or(NEUTRAL, |f| f.color)
}

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut gizmo_store: ResMut<GizmoConfigStore>,
) {
    // Sun-like key light from the upper left, as in the original galaxy map.
    commands.spawn((
        DirectionalLight { illuminance: 14_000.0, color: Color::srgb(1.0, 0.96, 0.9), ..default() },
        Transform::from_xyz(-1.0, 1.4, 0.8).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // Starfield: a large inside-out sphere with a procedural equirectangular texture.
    let sky_tex = images.add(make_starfield(6144, 3072));
    let sky_mat = materials.add(StandardMaterial {
        base_color: Color::linear_rgb(2.2, 2.2, 2.2),
        base_color_texture: Some(sky_tex),
        unlit: true,
        cull_mode: Some(bevy::render::render_resource::Face::Front),
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(SphereMeshBuilder::new(8000.0, SphereKind::Uv { sectors: 64, stacks: 32 }).build())),
        MeshMaterial3d(sky_mat),
        Skybox,
        NoFrustumCulling,
        Transform::default(),
    ));

    // Glowing route lines.
    let (cfg, _) = gizmo_store.config_mut::<DefaultGizmoConfigGroup>();
    cfg.line.width = 2.0;
}

/// Spawns planets, halos, labels and the selection ring from the [`GalaxyView`] resource.
fn spawn_galaxy(
    mut commands: Commands,
    view: Res<GalaxyView>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut bounds: ResMut<CameraBounds>,
) {
    let planet_mesh = meshes.add(SphereMeshBuilder::new(1.0, SphereKind::Uv { sectors: 48, stacks: 24 }).build());
    // Flat ring lying in the galaxy plane (Annulus faces +Z, rotate onto XZ).
    let ring_mesh = meshes.add(Annulus::new(1.22, 1.42));
    let thin_ring_mesh = meshes.add(Annulus::new(1.62, 1.72));
    let ring_rot = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);

    // One shared halo material per faction (+ neutral).
    let halo_mat = |c: [f32; 3], strength: f32, alpha: f32, materials: &mut Assets<StandardMaterial>| {
        let l = rgb(c).to_linear();
        materials.add(StandardMaterial {
            base_color: Color::LinearRgba(LinearRgba::new(l.red * strength, l.green * strength, l.blue * strength, alpha)),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        })
    };
    let faction_halos: Vec<_> = view.factions.iter().map(|f| halo_mat(f.color, 3.0, 0.9, &mut materials)).collect();
    let neutral_halo = halo_mat(NEUTRAL, 0.9, 0.35, &mut materials);

    for (i, planet) in view.planets.iter().enumerate() {
        let radius = planet_radius(planet.size);
        let pos = galaxy_to_world(planet.position);
        let owner_c = rgb(faction_color(&view, planet.owner)).to_linear();
        let s = planet.surface_color;
        // Surface colour tinted towards the owner; faint owner-coloured emissive keeps it readable.
        let tint = if planet.owner.is_some() { 0.28 } else { 0.0 };
        let base = Color::linear_rgb(
            s[0].powf(2.2) * (1.0 - tint) + owner_c.red * tint,
            s[1].powf(2.2) * (1.0 - tint) + owner_c.green * tint,
            s[2].powf(2.2) * (1.0 - tint) + owner_c.blue * tint,
        );
        let material = materials.add(StandardMaterial {
            base_color: base,
            perceptual_roughness: 0.75,
            emissive: if planet.owner.is_some() {
                LinearRgba::new(owner_c.red * 0.05, owner_c.green * 0.05, owner_c.blue * 0.05, 1.0)
            } else {
                LinearRgba::BLACK
            },
            ..default()
        });
        let halo = planet.owner.map_or(neutral_halo.clone(), |o| faction_halos[o].clone());

        commands
            .spawn((
                Mesh3d(planet_mesh.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(pos).with_scale(Vec3::splat(radius)),
            ))
            .with_children(|p| {
                // Child scale inherits the planet radius, so ring radii are relative to it.
                p.spawn((Mesh3d(ring_mesh.clone()), MeshMaterial3d(halo.clone()), Transform::from_rotation(ring_rot)));
                if planet.owner.is_some() {
                    p.spawn((Mesh3d(thin_ring_mesh.clone()), MeshMaterial3d(halo), Transform::from_rotation(ring_rot)));
                }
            });

        // Name label (positioned every frame in screen space).
        commands.spawn((
            Text::new(planet.display_name.clone()),
            TextFont { font_size: FontSize::Px(14.0), ..default() },
            TextColor(Color::srgba(0.85, 0.92, 1.0, 0.95)),
            Node { position_type: PositionType::Absolute, ..default() },
            Visibility::Hidden,
            PlanetLabel(i),
        ));
    }

    // Selection ring, moved onto the selected planet.
    let sel_mat = materials.add(StandardMaterial {
        base_color: Color::LinearRgba(LinearRgba::new(4.0, 4.0, 4.0, 0.95)),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Annulus::new(1.9, 2.05))),
        MeshMaterial3d(sel_mat),
        Transform::from_rotation(ring_rot),
        Visibility::Hidden,
        SelectionRing,
    ));

    // Camera bounds: galaxy extent plus a margin.
    if !view.planets.is_empty() {
        let (mut min, mut max) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for p in &view.planets {
            let w = galaxy_to_world(p.position);
            min = min.min(Vec2::new(w.x, w.z));
            max = max.max(Vec2::new(w.x, w.z));
        }
        bounds.min = min - Vec2::splat(20.0);
        bounds.max = max + Vec2::splat(20.0);
        bounds.dirty = true;
    }
}

/// Hides/positions the label of each planet below its sphere.
fn update_labels(
    view: Res<GalaxyView>,
    camera: Single<(&Camera, &GlobalTransform, &GalaxyCamera)>,
    mut labels: Query<(&PlanetLabel, &mut Node, &mut Visibility, &ComputedNode)>,
) {
    let (camera, cam_tf, orbit) = *camera;
    // Labels fade out of existence when zoomed far out to avoid clutter.
    let show = orbit.distance < 560.0;
    for (label, mut node, mut vis, computed) in &mut labels {
        let planet = &view.planets[label.0];
        let radius = planet_radius(planet.size);
        let below = galaxy_to_world(planet.position) + Vec3::new(0.0, 0.0, radius * 1.9);
        let screen = camera.world_to_viewport(cam_tf, below).ok();
        match (show, screen) {
            (true, Some(p)) => {
                let w = computed.size().x * computed.inverse_scale_factor();
                node.left = Val::Px(p.x - w * 0.5);
                node.top = Val::Px(p.y);
                *vis = Visibility::Inherited;
            }
            _ => *vis = Visibility::Hidden,
        }
    }
}

/// Moves the selection ring onto the selected planet and gently pulses it.
fn update_selection_ring(
    time: Res<Time>,
    view: Res<GalaxyView>,
    selection: Res<Selection>,
    mut ring: Single<(&mut Transform, &mut Visibility), With<SelectionRing>>,
) {
    let (tf, vis) = &mut *ring;
    match selection.0.and_then(|i| view.planets.get(i)) {
        Some(p) => {
            let pulse = 1.0 + 0.06 * (time.elapsed_secs() * 3.0).sin();
            tf.translation = galaxy_to_world(p.position);
            tf.scale = Vec3::splat(planet_radius(p.size) * pulse);
            **vis = Visibility::Visible;
        }
        None => **vis = Visibility::Hidden,
    }
}

/// Draws all trade routes as glowing lines, coloured by the owners of both ends.
fn draw_routes(view: Res<GalaxyView>, mut gizmos: Gizmos) {
    for r in &view.routes {
        let (Some(a), Some(b)) = (view.planets.get(r.a), view.planets.get(r.b)) else { continue };
        let (ca, cb) = (faction_color(&view, a.owner), faction_color(&view, b.owner));
        // Same owner -> faction glow, otherwise a dim neutral blue.
        let c = if a.owner.is_some() && a.owner == b.owner {
            let l = rgb(ca).to_linear();
            Color::LinearRgba(LinearRgba::new(l.red * 2.2, l.green * 2.2, l.blue * 2.2, 0.9))
        } else {
            let l = rgb([(ca[0] + cb[0]) * 0.25, (ca[1] + cb[1]) * 0.25, (ca[2] + cb[2]) * 0.25]).to_linear();
            Color::LinearRgba(LinearRgba::new(l.red * 1.2 + 0.04, l.green * 1.2 + 0.07, l.blue * 1.2 + 0.12, 0.7))
        };
        gizmos.line(galaxy_to_world(a.position), galaxy_to_world(b.position), c);
    }
}

/// Click (without dragging) selects the planet under the cursor; clicking empty space clears it.
#[allow(clippy::too_many_arguments)]
fn pick_planet(
    buttons: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform)>,
    view: Res<GalaxyView>,
    mut selection: ResMut<Selection>,
    mut press: Local<Option<Vec2>>,
) {
    let Some(cursor) = window.cursor_position() else { return };
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
    let (camera, cam_tf) = *camera;
    let Ok(ray) = camera.viewport_to_world(cam_tf, cursor) else { return };
    let cam_pos = cam_tf.translation();

    let mut best: Option<(usize, f32)> = None;
    for (i, p) in view.planets.iter().enumerate() {
        let center = galaxy_to_world(p.position);
        // Generous pick radius that never gets smaller than a few pixels on screen.
        let r = (planet_radius(p.size) * 1.5).max(center.distance(cam_pos) * 0.012);
        if let Some(t) = ray_sphere(ray.origin, *ray.direction, center, r)
            && best.is_none_or(|(_, bt)| t < bt)
        {
            best = Some((i, t));
        }
    }
    selection.0 = best.map(|(i, _)| i);
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
    (t >= 0.0).then_some(t).or_else(|| (-b + disc.sqrt() >= 0.0).then_some(0.0))
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
            let m = ((sample((u * 2.0 + 0.37) % 1.0, 1.0 - v) - 0.5) * 2.0).max(0.0).powf(1.8);
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
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}
