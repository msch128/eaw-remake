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

pub mod audio;
pub mod camera;
pub mod control;
pub mod convert;
pub mod gpu_profile;
pub(crate) mod hud;
mod input_test;
mod effects;
pub mod scene;
pub mod ships;
pub mod sim;
pub mod tactical_ui;
pub mod visuals;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use alamo_audio::PlaySfx;
use alamo_data::GameData;
use alamo_fs::FileSource;
use battle_sim::{Battle, BattleConfig, BattleSetup, Catalog, Command, Event};
use galaxy_sim::{FactionId, TacticalRequest, TacticalResult};
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
    /// Cascaded sun shadows on ships (costs a depth pass per cascade).
    pub shadows: bool,
    /// Multiplier on particle spawn rates (see `alamo_render::ParticleSettings::density`).
    pub particle_density: f32,
    /// Send sound requests ([`audio`]); nothing is audible without `AlamoAudioPlugin`.
    pub audio: bool,
}

/// Graphics quality presets shared with the options menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QualityPreset {
    /// No MSAA, no bloom, no shadows, thinned particles.
    Low,
    /// 2x MSAA, bloom, no shadows.
    Medium,
    /// 4x MSAA, bloom, sun shadows, full particle density.
    #[default]
    High,
}

impl QualityPreset {
    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }

    /// The next preset (wraps around).
    pub fn next(self) -> Self {
        match self {
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Low,
        }
    }

    /// The previous preset (wraps around).
    pub fn prev(self) -> Self {
        self.next().next()
    }

    /// MSAA samples, bloom, shadows and particle density of the preset.
    pub fn values(self) -> (u32, bool, bool, f32) {
        match self {
            Self::Low => (1, false, false, 0.5),
            Self::Medium => (2, true, false, 0.75),
            Self::High => (4, true, true, 1.0),
        }
    }
}

impl BattleViewSettings {
    /// Applies a quality preset to MSAA, bloom, shadows and particle density.
    pub fn apply_quality(&mut self, q: QualityPreset) {
        (self.msaa_samples, self.bloom, self.shadows, self.particle_density) = q.values();
    }
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
            start_distance: 2000.0,
            shadows: false,
            particle_density: 1.0,
            audio: true,
        }
    }
}

/// Inserting this resource starts a battle; removing it ends the battle and despawns its entities.
#[derive(Resource)]
pub struct BattleSession {
    setup: Option<BattleSetup>,
    /// The battle already constructed (by `from_request`, which runs off the main thread) with its catalog.
    built: Option<(Battle, Arc<Catalog>)>,
    visuals: Arc<VisualCatalog>,
    scene: SpaceScene,
    request: Option<Arc<TacticalRequest>>,
    player_side: Option<u8>,
    player_faction: Option<String>,
    /// The model preload for this session was requested (see `start_battle`).
    preload_requested: bool,
}

impl BattleSession {
    /// A battle with its visuals and map.
    pub fn new(setup: BattleSetup, visuals: VisualCatalog, scene: SpaceScene) -> Self {
        Self {
            setup: Some(setup),
            built: None,
            visuals: Arc::new(visuals),
            scene,
            request: None,
            player_side: None,
            player_faction: None,
            preload_requested: false,
        }
    }

    /// Builds the battle of a galactic [`TacticalRequest`] (`Sim::tactical_setup`) on the planet's real
    /// space map: unit catalog, arena, scene, visuals. The side of `player` is controlled by the player (its
    /// ships stay idle until ordered); every other side is run by the built-in AI. `None` lets the AI play all
    /// sides (spectate). Reports back through [`BattleRuntime::outcome`] and the [`BattleFinished`] message.
    pub fn from_request(
        data: &GameData,
        fs: &dyn FileSource,
        request: &TacticalRequest,
        player: Option<FactionId>,
    ) -> Result<Self, String> {
        let map = request
            .map
            .as_deref()
            .ok_or_else(|| format!("planet {} has no space map", request.planet_name))?;
        let scene = SpaceScene::load(fs, data, &format!("Art/Maps/{map}"))?;
        let names: Vec<&str> = request.sides.iter().flat_map(|s| s.groups.iter().chain(&s.reserve).map(|g| g.name.as_str())).collect();
        let catalog = Arc::new(Catalog::build(data, Some(fs), &names));
        let arena = battle_sim::Arena::load(data, fs, map).map(Arc::new);
        let player_side = player.and_then(|p| request.sides.iter().position(|s| s.faction == p)).map(|i| i as u8);
        let config = BattleConfig { record_events: true, ..request.battle_config(arena) };
        let setup = request
            .battle_setup(&catalog, config, |i| Some(i as u8) != player_side)
            .ok_or_else(|| "a unit of the battle has no definition".to_string())?;
        let colors = request
            .sides
            .iter()
            .map(|s| {
                data.faction(&s.faction_name)
                    .and_then(|f| f.color)
                    .map_or(Color::WHITE, |c| Color::srgb_u8(c[0], c[1], c[2]))
            })
            .collect();
        let visuals = VisualCatalog::build(data, &catalog, colors);
        // Building the battle (navigation, spawning the ships) is the slow part: do it here, not on the render thread.
        let built = Battle::new(setup);
        Ok(Self {
            setup: None,
            built: Some((built, catalog.clone())),
            visuals: Arc::new(visuals),
            scene,
            request: Some(Arc::new(request.clone())),
            player_side,
            player_faction: player_side.map(|i| request.sides[i as usize].faction_name.clone()),
            preload_requested: false,
        })
    }

    /// Model files of the battle (ships, sky, nebula, props) for [`alamo_render::ModelPreloader::request`].
    pub fn asset_paths(&self) -> Vec<String> {
        let mut v: Vec<String> = self.visuals.ships.iter().filter_map(|s| s.model.clone()).collect();
        v.extend(self.visuals.ships.iter().filter_map(|s| s.death_clone_model.clone()));
        v.extend(self.scene.skydome.clone());
        v.extend(self.scene.nebula.clone());
        v.extend(self.scene.props.iter().map(|p| p.model.clone()));
        v
    }

    /// Particle objects (death effects of ships and hardpoints, the class explosions) and projectile names of
    /// the battle, for [`alamo_render::ModelPreloader::request_particles`].
    pub fn effect_names(&self) -> (Vec<String>, Vec<String>) {
        let mut particles: Vec<String> = Vec::new();
        for s in &self.visuals.ships {
            particles.extend(s.death_effects.iter().cloned());
            particles.extend(s.hardpoints.iter().filter_map(|h| h.death_effect.clone()));
        }
        // Fallback death explosions by ship size and the finales of the death sequence.
        for n in ["Large_Explosion_Space_Empire_pd_medium_3_150", "Large_Explosion_Space_Empire_pd_medium_3_300", "Large_Explosion_Space_Empire_pd_medium_3_750", "Huge_Explosion_Space", "Large_Explosion_Space", "Medium_Explosion_Space"] {
            particles.push(n.to_string());
        }
        use battle_sim::TargetClass::*;
        for class in [Fighter, Transport, Frigate, Capital, Super] {
            particles.push(visuals::ship_death_effect(class).to_string());
        }
        let catalog = self.built.as_ref().map(|b| &b.1).or(self.setup.as_ref().map(|s| &s.catalog));
        let projectiles = catalog.map_or_else(Vec::new, |c| c.projectiles.iter().map(|p| p.name.clone()).collect());
        (particles, projectiles)
    }

    /// The galactic request this battle was built from, if any.
    pub fn request(&self) -> Option<&TacticalRequest> {
        self.request.as_deref()
    }
}

/// The battle is over and the displayed picture reached its end. Written once per battle when the session
/// came from [`BattleSession::from_request`]; apply `result` with `Sim::apply_tactical_result(battle_id, &result)`.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct BattleFinished {
    /// Pending battle id of the galaxy sim.
    pub battle_id: u32,
    /// Survivors and winner.
    pub result: TacticalResult,
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
    /// Galactic request of the battle, if it came from [`BattleSession::from_request`].
    pub request: Option<Arc<TacticalRequest>>,
    /// Faction name of the player's side, if any.
    pub player_faction: Option<String>,
    /// Living ships per side at the start.
    pub start_alive: Vec<u32>,
    /// Seconds before the retreat command becomes available.
    pub retreat_countdown: f32,
    /// Population cap per side.
    pub unit_cap: i32,
    /// A reinforcement position must be within this distance of an own ship or structure and farther from every enemy.
    pub reinforce_zone: f32,
    /// Free space needed at the arrival point.
    pub reinforce_clearance: f32,
    /// Half the side of the battle area (sim units).
    pub arena_half: f32,
    /// Fog of war is on.
    pub fog_of_war: bool,
    /// Current speed multiplier.
    speed: f32,
    framed: bool,
    reported: bool,
    /// Map objects for the minimap: position (map x, -z), size and whether it is a backdrop object.
    pub(crate) minimap_props: Vec<(Vec2, f32)>,
    /// Dark tint of the tactical minimap derived from the map lights.
    pub(crate) minimap_tint: Color,
}

impl BattleRuntime {
    /// The result for the galaxy once the battle is over (all of one side destroyed or retreated, or the time
    /// limit hit): survivors per request group, retreated units, winner. `None` while it is still running or
    /// when the battle did not come from a [`TacticalRequest`].
    pub fn outcome(&self) -> Option<TacticalResult> {
        let request = self.request.as_ref()?;
        let snap = self.sim.latest()?;
        request.result_from(&self.catalog, snap.result.as_ref()?)
    }

    /// The state of a battle that is still running, shaped like a finished result: used to abandon the fight half way so that
    /// the galaxy keeps the losses so far. `None` without a request or before the first battle second.
    pub fn partial_outcome(&self) -> Option<TacticalResult> {
        let request = self.request.as_ref()?;
        request.result_from(&self.catalog, &self.sim.progress()?)
    }

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
            .init_resource::<control::RecentOrders>()
            .init_resource::<control::MoveLines>()
            .init_resource::<control::Drag>()
            .init_resource::<control::UiBlock>()
            .init_resource::<control::ArmedOrder>()
            .init_resource::<camera::ActionCam>()
            .add_message::<control::FocusCamera>()
            .init_resource::<effects::BoltPools>()
            .init_resource::<audio::BattleAudioStats>()
            .add_message::<control::OrderIssued>()
            .add_message::<BattleFinished>();
        // Sound requests need a message queue even when no audio plugin consumes them.
        if !app.world().contains_resource::<Messages<PlaySfx>>() {
            app.add_message::<PlaySfx>();
        }
        app.add_systems(
                Update,
                (
                    start_battle.run_if(resource_exists::<BattleSession>),
                    stop_battle.run_if(resource_removed::<BattleSession>),
                    (
                        control::read_pointer,
                        advance_clock,
                        (effects::impact_effects, ships::ship_events),
                        ships::update_ships,
                        ships::update_arrival_flashes,
                        ships::update_dying,
                        ships::remove_after,
                        effects::update_projectiles,
                        camera::camera_input,
                        camera::focus_camera,
                        camera::action_cam,
                        camera::apply_camera,
                        camera::sky_follow,
                        control::selection,
                        control::orders,
                        control::overlays,
                        audio::battle_audio,
                        audio::battle_music,
                        report_outcome,
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
    mut settings: ResMut<BattleViewSettings>,
    mut gizmo_config: ResMut<GizmoConfigStore>,
    mut render_settings: ResMut<alamo_render::AlamoRenderSettings>,
    particles: Option<ResMut<alamo_render::ParticleSettings>>,
    preloader: Option<Res<alamo_render::ModelPreloader>>,
    alamo_fs: Option<Res<alamo_render::AlamoFs>>,
    model_cache: Option<Res<alamo_render::ModelCache>>,
    xml_defs: Option<Res<alamo_render::XmlDefs>>,
) {
    if session.built.is_none() && session.setup.is_none() {
        return;
    }
    // Load the battle's models and textures on worker threads first (a few frames) instead of stalling
    // the render thread for the length of the whole load on the frame the ships spawn.
    // `SP_NO_PRELOAD=1` disables the preload (A/B measurements).
    if std::env::var_os("SP_NO_PRELOAD").is_none()
        && let (Some(pre), Some(fs), Some(cache)) = (&preloader, &alamo_fs, &model_cache)
    {
        if !session.preload_requested {
            session.preload_requested = true;
            pre.request(&fs.0, cache, session.asset_paths());
            if let Some(defs) = &xml_defs {
                let (particles, projectiles) = session.effect_names();
                pre.request_particles(&fs.0, defs, particles, projectiles);
            }
        }
        if !pre.is_idle() {
            return;
        }
    }
    let Some((battle, catalog)) = session.built.take().or_else(|| {
        let setup = session.setup.take()?;
        let catalog = setup.catalog.clone();
        Some((Battle::new(setup), catalog))
    }) else {
        return;
    };
    if let Some(side) = session.player_side {
        settings.player_side = side;
    }
    let start_alive: Vec<u32> = (0..battle.side_count()).map(|s| battle.alive_count(s as u8) as u32).collect();
    let (retreat_countdown, unit_cap) = (battle.config().retreat_countdown, battle.config().unit_cap);
    let dt = battle.config().dt;
    let (reinforce_zone, reinforce_clearance, arena_half, fog_of_war) =
        (battle.config().reinforce_zone, battle.config().reinforce_clearance, battle.config().half_size, battle.config().fog_of_war);
    let sim = SimHandle::spawn(battle);
    sim.request(2);
    let l = &session.scene.lights;
    info!("battle scene lights: sun dir {:?} color {:?}, fill colors {:?} {:?}, ambient {:?}", l[0].direction, l[0].color, l[1].color, l[2].color, session.scene.ambient);
    let sky = settings.sky_radius;
    // Models read the shadow flag when their materials are created, so set it before they spawn.
    render_settings.shadows = settings.shadows;
    // Spawn the ships over several frames (the app keeps its loading screen until all are there).
    render_settings.spawn_budget = Some(std::time::Duration::from_millis(8));
    if let Some(mut p) = particles {
        p.density = settings.particle_density;
    }
    scene::spawn_scene(&mut commands, &session.scene, sky, settings.shadows);

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
                intensity: 0.03,
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
        request: session.request.clone(),
        player_faction: session.player_faction.clone(),
        start_alive,
        retreat_countdown,
        unit_cap,
        reinforce_zone,
        reinforce_clearance,
        arena_half,
        fog_of_war,
        speed: 1.0,
        framed: false,
        reported: false,
        minimap_props: session.scene.props.iter().filter(|p| !p.backdrop).map(|p| (Vec2::new(p.position.x, -p.position.z), p.scale)).collect(),
        minimap_tint: {
            let t = session.scene.ambient * 3.0 + session.scene.lights[0].color * 0.06;
            Color::srgb(t.x.clamp(0.02, 0.35), t.y.clamp(0.02, 0.35), t.z.clamp(0.03, 0.4))
        },
    });
}

/// Writes [`BattleFinished`] once the displayed picture reached the end of a battle that came from a request.
fn report_outcome(mut rt: ResMut<BattleRuntime>, status: Res<BattleStatus>, mut out: MessageWriter<BattleFinished>) {
    if rt.reported || !status.over {
        return;
    }
    let (Some(request), Some(result)) = (rt.request.clone(), rt.outcome()) else {
        return;
    };
    rt.reported = true;
    out.write(BattleFinished { battle_id: request.battle_id, result });
}

fn stop_battle(
    mut commands: Commands,
    all: Query<Entity, With<BattleEntity>>,
    mut ships: ResMut<RenderedShips>,
    mut pools: ResMut<effects::BoltPools>,
    render_settings: Option<ResMut<alamo_render::AlamoRenderSettings>>,
) {
    if let Some(mut s) = render_settings {
        s.spawn_budget = None;
    }
    for e in &all {
        commands.entity(e).try_despawn();
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

