//! Renders and drives a [`battle_sim`] space battle in Bevy with the Empire at War look and controls.
//!
//! ```ignore
//! app.add_plugins(AlamoRenderPlugin::new(fs.clone()))   // required, provides models and particles
//!    .add_plugins(BattleViewPlugin);
//! // later, to start a battle (inserting the resource spawns the scene, camera and worker thread):
//! commands.insert_resource(BattleSession::new(setup, visuals, scene));
//! ```
//!
//! * [`SpaceScene`] holds the static map (sky, nebula, lights, props, bounds) read from a `.ted`.
//! * [`VisualCatalog`] holds what the simulation catalog does not know (scale, hardpoint bones).
//! * The simulation runs on a worker thread ([`sim::SimHandle`]); the render side interpolates
//!   between its per-step snapshots, so the picture is smooth at any frame rate.
//! * Input: [`camera::BattleCamera`] (wheel zoom, WASD / edge pan, Q E / middle drag rotate) and
//!   [`control`] (click and box select, right click move / attack / attack hardpoint).

pub mod camera;
pub mod control;
pub mod convert;
mod effects;
pub mod scene;
pub mod ships;
pub mod sim;
pub mod visuals;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use battle_sim::{Battle, BattleSetup, Catalog, Command, Event};
use bevy::camera::{Hdr, ImageRenderTarget, RenderTarget};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::gizmos::config::{DefaultGizmoConfigGroup, GizmoConfigStore};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;

pub use camera::BattleCamera;
pub use control::{ControlGroups, Hover, OrderMarkers, Pointer, Selection};
pub use scene::{SceneProp, SpaceScene};
pub use ships::{RenderedShips, ShipRender, ShipView};
pub use sim::{SimHandle, Snapshot};
pub use visuals::VisualCatalog;

/// Marks every entity the battle view spawned, for cleanup.
#[derive(Component, Debug, Default)]
pub struct BattleEntity;

/// Tunables of the battle view.
#[derive(Resource, Clone)]
pub struct BattleViewSettings {
    /// Side controlled by the player.
    pub player_side: u8,
    /// Battle speed multiplier.
    pub speed: f32,
    /// Freezes the battle clock.
    pub paused: bool,
    /// Render into this image instead of a window (offscreen runs).
    pub render_target: Option<Handle<Image>>,
    /// Scale factor of the render target.
    pub target_scale: f32,
    /// Wait for the simulation when it is behind the clock instead of stalling the picture.
    /// Offscreen runs set this so that the output is reproducible.
    pub blocking: bool,
    /// MSAA samples (1, 2, 4 or 8).
    pub msaa_samples: u32,
    /// Bloom around bright engine glows and bolts.
    pub bloom: bool,
    /// Particle effects alive above this number suppress new impact effects.
    pub max_effects: usize,
    /// Impact effects spawned per frame at most.
    pub max_impacts_per_frame: usize,
    /// Pointer state is written by the caller instead of read from the window.
    pub synthetic_pointer: bool,
    /// UI scale for overlay sizes (pixel sizes are multiplied by it).
    pub ui_scale: f32,
    /// Radius of the sky sphere around the camera; the camera far plane is larger.
    pub sky_radius: f32,
    /// Vertical field of view in degrees.
    pub fov_degrees: f32,
    /// Camera distance when the battle starts.
    pub start_distance: f32,
}

impl Default for BattleViewSettings {
    fn default() -> Self {
        Self {
            player_side: 0,
            speed: 1.0,
            paused: false,
            render_target: None,
            target_scale: 1.0,
            blocking: false,
            msaa_samples: 4,
            bloom: true,
            max_effects: 260,
            max_impacts_per_frame: 24,
            synthetic_pointer: false,
            ui_scale: 1.0,
            sky_radius: 90_000.0,
            fov_degrees: 45.0,
            start_distance: 4500.0,
        }
    }
}

/// Inserting this resource starts a battle; removing it ends the battle and despawns its entities.
#[derive(Resource)]
pub struct BattleSession {
    setup: Option<BattleSetup>,
    visuals: Arc<VisualCatalog>,
    scene: SpaceScene,
}

impl BattleSession {
    /// A battle with its visuals and map.
    pub fn new(setup: BattleSetup, visuals: VisualCatalog, scene: SpaceScene) -> Self {
        Self {
            setup: Some(setup),
            visuals: Arc::new(visuals),
            scene,
        }
    }
}

/// Live numbers of the running battle.
#[derive(Resource, Debug, Default, Clone)]
pub struct BattleStatus {
    /// Battle time in seconds of the displayed step.
    pub time: f32,
    /// Displayed step.
    pub tick: u64,
    /// Living ships per side.
    pub alive: Vec<u32>,
    /// True once the battle ended.
    pub over: bool,
    /// Winner, if any.
    pub winner: Option<u8>,
    /// Wall-clock milliseconds of the last simulation step on the worker.
    pub step_ms: f32,
}

/// Runtime of the active battle (simulation handle, clock, entity map).
#[derive(Resource)]
pub struct BattleRuntime {
    /// Simulation worker.
    pub sim: SimHandle,
    /// Definitions.
    pub catalog: Arc<Catalog>,
    /// Visual data.
    pub visuals: Arc<VisualCatalog>,
    /// Seconds per simulation step.
    pub dt: f32,
    /// Virtual battle clock in seconds.
    pub clock: f64,
    /// Snapshot of the displayed step.
    pub cur: Option<Arc<Snapshot>>,
    /// Snapshot of the following step, if available.
    pub next: Option<Arc<Snapshot>>,
    /// Fraction between `cur` and `next`.
    pub alpha: f32,
    /// Events of steps reached since the last frame.
    pub fresh_events: Vec<Event>,
    /// Ship entities by simulation id.
    pub entities: HashMap<u32, Entity>,
    /// Ships that are exploding.
    pub dying: HashSet<u32>,
    /// Current speed multiplier.
    speed: f32,
    framed: bool,
}

impl BattleRuntime {
    /// Speed multiplier of the battle clock (0 while paused).
    pub fn speed_factor(&self) -> f32 {
        self.speed
    }

    /// Sends a command as `side`.
    pub fn command(&self, side: u8, cmd: Command) {
        self.sim.command(side, cmd);
    }
}

/// Adds the battle view. Requires [`alamo_render::AlamoRenderPlugin`] to be added as well.
pub struct BattleViewPlugin;

impl Plugin for BattleViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BattleViewSettings>()
            .init_resource::<BattleStatus>()
            .init_resource::<RenderedShips>()
            .init_resource::<Selection>()
            .init_resource::<ControlGroups>()
            .init_resource::<Pointer>()
            .init_resource::<Hover>()
            .init_resource::<OrderMarkers>()
            .init_resource::<control::Drag>()
            .init_resource::<effects::BoltPools>()
            .add_systems(
                Update,
                (
                    start_battle.run_if(resource_added::<BattleSession>),
                    stop_battle.run_if(resource_removed::<BattleSession>),
                    (
                        control::read_pointer,
                        advance_clock,
                        (effects::impact_effects, ships::ship_events),
                        ships::update_ships,
                        ships::update_dying,
                        effects::update_projectiles,
                        camera::camera_input,
                        camera::apply_camera,
                        camera::sky_follow,
                        control::selection,
                        control::orders,
                        control::overlays,
                    )
                        .chain()
                        .run_if(resource_exists::<BattleRuntime>),
                )
                    .chain(),
            );
    }
}

fn start_battle(
    mut commands: Commands,
    mut session: ResMut<BattleSession>,
    settings: Res<BattleViewSettings>,
    mut gizmo_config: ResMut<GizmoConfigStore>,
) {
    let Some(setup) = session.setup.take() else {
        return;
    };
    let catalog = setup.catalog.clone();
    let battle = Battle::new(setup);
    let dt = battle.config().dt;
    let sim = SimHandle::spawn(battle);
    sim.request(2);
    let sky = settings.sky_radius;
    scene::spawn_scene(&mut commands, &session.scene, sky);

    let (cfg, _) = gizmo_config.config_mut::<DefaultGizmoConfigGroup>();
    cfg.line.width = 2.0 * settings.ui_scale;
    cfg.depth_bias = -1.0;

    let fov = settings.fov_degrees.to_radians();
    let b = &session.scene;
    let cam = BattleCamera {
        bounds_min: Vec2::new(b.bounds_min.x, -b.bounds_max.y),
        bounds_max: Vec2::new(b.bounds_max.x, -b.bounds_min.y),
        ..default()
    };
    let mut ec = commands.spawn((
        Name::new("battle camera"),
        BattleEntity,
        Camera3d::default(),
        Tonemapping::None,
        match settings.msaa_samples {
            1 => Msaa::Off,
            2 => Msaa::Sample2,
            8 => Msaa::Sample8,
            _ => Msaa::Sample4,
        },
        Projection::Perspective(PerspectiveProjection {
            fov,
            near: 5.0,
            far: sky * 1.5,
            ..default()
        }),
        cam,
        Transform::default(),
    ));
    if settings.bloom {
        ec.insert((
            Hdr,
            Bloom {
                intensity: 0.1,
                ..Bloom::NATURAL
            },
        ));
    }
    if let Some(img) = &settings.render_target {
        ec.insert(RenderTarget::Image(ImageRenderTarget {
            handle: img.clone(),
            scale_factor: settings.target_scale,
        }));
    }
    commands.insert_resource(BattleRuntime {
        sim,
        catalog,
        visuals: session.visuals.clone(),
        dt,
        clock: 0.0,
        cur: None,
        next: None,
        alpha: 0.0,
        fresh_events: Vec::new(),
        entities: HashMap::new(),
        dying: HashSet::new(),
        speed: 1.0,
        framed: false,
    });
}

fn stop_battle(
    mut commands: Commands,
    all: Query<Entity, With<BattleEntity>>,
    mut ships: ResMut<RenderedShips>,
    mut pools: ResMut<effects::BoltPools>,
) {
    for e in &all {
        commands.entity(e).despawn();
    }
    ships.0.clear();
    *pools = effects::BoltPools::default();
    commands.remove_resource::<BattleRuntime>();
}

/// Advances the battle clock and fetches the snapshots around it.
fn advance_clock(
    time: Res<Time>,
    settings: Res<BattleViewSettings>,
    mut rt: ResMut<BattleRuntime>,
    mut status: ResMut<BattleStatus>,
    mut cam: Single<&mut BattleCamera>,
) {
    rt.speed = if settings.paused { 0.0 } else { settings.speed };
    let step = rt.dt as f64;
    let dt_real = (time.delta_secs() as f64).min(0.1);
    rt.clock += dt_real * rt.speed as f64;
    let tick = (rt.clock / step).floor() as u64;
    rt.sim.request(tick + 3);
    let (a, b) = rt.sim.fetch(tick, settings.blocking);
    let (cur, next, alpha) = match (a, b) {
        (Some(a), Some(b)) => {
            let alpha = ((rt.clock - a.tick as f64 * step) / step).clamp(0.0, 1.0) as f32;
            (a, Some(b), alpha)
        }
        (Some(a), None) => {
            // Simulation not ready (or battle over): hold the last step.
            rt.clock = rt.clock.min(a.tick as f64 * step);
            (a, None, 0.0)
        }
        (None, _) => {
            let Some(l) = rt.sim.latest() else { return };
            rt.clock = rt.clock.min(l.tick as f64 * step);
            (l, None, 0.0)
        }
    };
    rt.fresh_events = rt.sim.drain_events(cur.tick);
    status.time = cur.time;
    status.tick = cur.tick;
    status.alive = cur.alive.clone();
    status.over = cur.over;
    status.winner = cur.winner;
    status.step_ms = cur.step_ms;
    if !rt.framed && !cur.ships.is_empty() {
        rt.framed = true;
        // Start over the player's fleet, like the original game does.
        let pts: Vec<Vec3> = cur
            .ships
            .iter()
            .filter(|s| s.side == settings.player_side)
            .map(|s| convert::sim_to_bevy(s.pos))
            .collect();
        if !pts.is_empty() {
            let center = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
            cam.focus = center;
            cam.target_focus = center;
            cam.distance = settings.start_distance;
            cam.target_distance = settings.start_distance;
        }
    }
    rt.cur = Some(cur);
    rt.next = next;
    rt.alpha = alpha;
}

