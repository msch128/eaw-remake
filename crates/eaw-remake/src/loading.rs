//! Loading screen: parses the game data on a worker thread, then warms up the planet models,
//! while the window stays responsive and shows a progress bar.
//!
//! Phases: (1) worker thread parses the XML data and string table (progress 0 to 70%), (2) the
//! campaign list and the first galaxy are built and the galaxy backdrop is spawned,
//! (3) one hidden instance of every planet model is spawned, two per frame, so models and
//! textures are cached before the map needs them (70 to 100%).

use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use alamo_data::GameData;
use alamo_fs::GameFs;
use alamo_render::{AlamoFs, AlamoModel, AlamoModelLoaded, ModelLoadFailed};
use bevy::prelude::*;

use crate::flow::{AppScreen, CurrentCampaign};
use crate::galaxy_view::GalaxyView;
use crate::game::{self, DEFAULT_CAMPAIGN, DEMO_CAMPAIGN, Game, GameAssets, StartupLog};
use crate::menu::{Campaigns, spawn_backdrop_props};

/// Screen to switch to when loading has finished.
#[derive(Resource)]
pub struct AfterLoading(pub AppScreen);

/// The opened game file system (cheap to open), `None` when no game is installed.
#[derive(Resource)]
pub struct GameFiles(pub Option<Arc<GameFs>>);

/// Campaign to build when [`AfterLoading`] is the galaxy map (`None` = the default campaign).
#[derive(Resource, Default)]
pub struct InitialCampaign(pub Option<String>);

/// Extra seconds the data phase is held (screenshots of the loading screen).
#[derive(Resource, Default)]
pub struct LoadDelay(pub f32);

enum Msg {
    Progress(f32, String),
    /// Parsed data, `None` if loading failed.
    Done(Option<Box<GameData>>),
}

#[derive(Resource)]
struct Task(Mutex<Receiver<Msg>>);

#[derive(Resource, Default)]
struct Progress {
    fraction: f32,
    text: String,
}

#[derive(Resource)]
struct Warmup {
    queue: Vec<String>,
    total: usize,
    failed: usize,
    started: Instant,
}

#[derive(Component)]
struct WarmupModel;
#[derive(Component)]
struct LoadingUi;
#[derive(Component)]
struct BarFill;
#[derive(Component)]
struct StatusText;

/// Background loading and the loading screen.
pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Progress>()
            .init_resource::<InitialCampaign>()
            .init_resource::<LoadDelay>()
            .add_observer(on_model_failed)
            .add_systems(OnEnter(AppScreen::Loading), start_loading)
            .add_systems(OnExit(AppScreen::Loading), despawn_ui)
            .add_systems(
                Update,
                (
                    poll_task.run_if(resource_exists::<Task>),
                    warm_up.run_if(resource_exists::<Warmup>),
                    update_ui,
                )
                    .chain()
                    .run_if(in_state(AppScreen::Loading)),
            );
    }
}

fn start_loading(
    mut commands: Commands,
    files: Res<GameFiles>,
    delay: Res<LoadDelay>,
    after: Res<AfterLoading>,
    mut next: ResMut<NextState<AppScreen>>,
    mut progress: ResMut<Progress>,
    mut images: ResMut<Assets<Image>>,
) {
    spawn_ui(&mut commands);
    if let Some(fs) = files.0.as_ref() {
        spawn_art(&mut commands, fs, &mut images);
    }
    let Some(fs) = files.0.clone() else {
        // No game installed: nothing to load, the demo galaxy is used.
        progress.fraction = 1.0;
        commands.insert_resource(Game(None));
        next.set(after.0);
        return;
    };
    progress.text = "Reading game data".into();
    let (tx, rx) = mpsc::channel();
    let delay = delay.0;
    thread::Builder::new()
        .name("game-data-loader".into())
        .spawn(move || {
            let _ = tx.send(Msg::Progress(0.05, "Reading game data".into()));
            if delay > 0.0 {
                thread::sleep(Duration::from_secs_f32(delay));
            }
            let (data, _warnings) = alamo_data::load_galaxy(fs.as_ref());
            let _ = tx.send(Msg::Progress(0.7, "Preparing the galaxy".into()));
            let _ = tx.send(Msg::Done(Some(Box::new(data))));
        })
        .expect("spawn loader thread");
    commands.insert_resource(Task(Mutex::new(rx)));
}

/// Receives worker messages; on completion builds the campaign list and starts the warm-up.
#[allow(clippy::too_many_arguments)]
fn poll_task(
    mut commands: Commands,
    task: Res<Task>,
    files: Res<GameFiles>,
    after: Res<AfterLoading>,
    initial: Res<InitialCampaign>,
    alamo: Option<Res<AlamoFs>>,
    mut progress: ResMut<Progress>,
    mut next: ResMut<NextState<AppScreen>>,
) {
    let mut done = None;
    {
        let rx = task.0.lock().expect("loader channel");
        loop {
            match rx.try_recv() {
                Ok(Msg::Progress(f, text)) => {
                    progress.fraction = f;
                    progress.text = text;
                }
                Ok(Msg::Done(data)) => {
                    done = Some(data);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    done = Some(None);
                    break;
                }
            }
        }
    }
    let Some(data) = done else { return };
    commands.remove_resource::<Task>();

    let assets = match (data, files.0.clone()) {
        (Some(data), Some(fs)) => Some(GameAssets { fs, data: Arc::new(*data) }),
        _ => {
            warn!("game data could not be loaded, using the demo galaxy");
            if files.0.is_some() {
                commands.insert_resource(game::StartupProblem(Some(game::describe_problem("the game data could not be loaded", game::DEFAULT_MOD_ID))));
            }
            None
        }
    };

    let mut entries = assets.as_ref().map(|a| game::campaign_entries(&a.data)).unwrap_or_default();
    if entries.is_empty() {
        if assets.is_some() {
            commands.insert_resource(game::StartupProblem(Some(game::describe_problem("no galactic conquest campaigns found", game::DEFAULT_MOD_ID))));
        }
        entries.push(game::demo_entry());
    }
    commands.insert_resource(Campaigns(entries));

    // The galaxy to show when the map is the first screen.
    if after.0 == AppScreen::Galaxy {
        let name = initial.0.clone().unwrap_or_else(|| {
            if assets.is_some() { DEFAULT_CAMPAIGN.to_owned() } else { DEMO_CAMPAIGN.to_owned() }
        });
        let mut log = StartupLog::default();
        let built = match &assets {
            Some(a) => game::build_campaign(a, &name, &mut log),
            None => Ok((GalaxyView::demo(), Some("Rebel".to_owned()))),
        };
        let (view, faction, name) = match built {
            Ok((v, f)) => (v, f, name),
            Err(e) => {
                log.warn(format!("FALLING BACK TO DEMO GALAXY: {e}"));
                (GalaxyView::demo(), Some("Rebel".to_owned()), DEMO_CAMPAIGN.to_owned())
            }
        };
        log.emit();
        commands.insert_resource(view);
        commands.insert_resource(CurrentCampaign { name: Some(name), faction });
    }

    match (&assets, alamo.is_some()) {
        (Some(a), true) => {
            spawn_backdrop_props(&mut commands, a);
            let queue = warmup_models(&a.data);
            progress.fraction = 0.7;
            progress.text = "Loading models".into();
            commands.insert_resource(Warmup {
                total: queue.len(),
                queue,
                failed: 0,
                started: Instant::now(),
            });
        }
        _ => {
            progress.fraction = 1.0;
            next.set(after.0);
        }
    }
    commands.insert_resource(Game(assets));
}

/// Distinct planet models of the default campaign.
fn warmup_models(data: &GameData) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    if let Some(c) = data.campaign(DEFAULT_CAMPAIGN) {
        for p in data.campaign_planets(c) {
            if let Some(m) = &p.galactic_model
                && !m.to_ascii_lowercase().contains("invisible")
                && seen.insert(m.to_ascii_lowercase())
            {
                out.push(m.clone());
            }
        }
    }
    out
}

/// Spawns two hidden model instances per frame and finishes when all are loaded.
fn warm_up(
    mut commands: Commands,
    mut warmup: ResMut<Warmup>,
    loaded: Query<(), (With<WarmupModel>, With<AlamoModelLoaded>)>,
    models: Query<Entity, With<WarmupModel>>,
    after: Res<AfterLoading>,
    mut progress: ResMut<Progress>,
    mut next: ResMut<NextState<AppScreen>>,
) {
    for _ in 0..2 {
        if let Some(name) = warmup.queue.pop() {
            commands.spawn((AlamoModel::new(name), WarmupModel, Visibility::Hidden));
        }
    }
    let ready = loaded.iter().count() + warmup.failed;
    let total = warmup.total.max(1);
    progress.fraction = 0.7 + 0.3 * (ready as f32 / total as f32).min(1.0);
    progress.text = format!("Loading models ({}/{})", ready.min(warmup.total), warmup.total);
    let timed_out = warmup.started.elapsed() > Duration::from_secs(20);
    if (warmup.queue.is_empty() && ready >= warmup.total) || timed_out {
        for e in &models {
            commands.entity(e).despawn();
        }
        commands.remove_resource::<Warmup>();
        progress.fraction = 1.0;
        next.set(after.0);
    }
}

fn on_model_failed(
    ev: On<ModelLoadFailed>,
    warm: Query<(), With<WarmupModel>>,
    warmup: Option<ResMut<Warmup>>,
) {
    if warm.contains(ev.entity)
        && let Some(mut w) = warmup
    {
        w.failed += 1;
    }
}

/// A random loading screen of the game (`Art/Textures/LoadScreens`), of `faction` (`Rebel`, `Empire`, `Underworld`) when it has some.
pub fn load_art(fs: &GameFs, faction: Option<&str>, images: &mut Assets<Image>) -> Option<Handle<Image>> {
    decode_art(fs, faction, None).map(|image| images.add(image))
}

/// Decodes a random loading screen to a CPU image (thread-safe: run it off the main thread).
/// `preferred` is a texture path of the game (`Loadscreens/Space_Rebel_17.png`) tried first, e.g. the planet's own loading screen.
pub fn decode_art(fs: &GameFs, faction: Option<&str>, preferred: Option<&str>) -> Option<Image> {
    use alamo_fs::FileSource;
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() as usize);
    let prefix = match faction.map(str::to_ascii_lowercase).as_deref() {
        Some("rebel" | "republic") => Some("SPACE_REBEL"),
        Some("empire") => Some("SPACE_EMPIRE"),
        Some("underworld" | "cis") => Some("SPACE_UNDERWORLD"),
        _ => None,
    };
    let mut candidates: Vec<String> = Vec::new();
    if let Some(p) = prefix {
        candidates.extend((0..30).map(|i| format!("Art/Textures/LoadScreens/{p}_{}.PNG", 1 + (seed + i) % 30)));
    }
    candidates.extend((0..8).map(|i| format!("Art/Textures/LoadScreens/AAA_NEW_SPACE_{}.PNG", 1 + (seed + i) % 8)));
    if let Some(p) = preferred {
        candidates.insert(0, format!("Art/Textures/{p}"));
    }
    let rgba = candidates.iter().find_map(|n| fs.read(n).ok().and_then(|b| alamo_model::image::decode_texture(&b).ok()))?;
    let image = Image::new(
        Extent3d { width: rgba.width as u32, height: rgba.height as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba.pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    Some(image)
}

/// The loading screen art behind the progress bar.
fn spawn_art(commands: &mut Commands, fs: &GameFs, images: &mut Assets<Image>) {
    let Some(handle) = load_art(fs, None, images) else {
        warn!("loading art not available");
        return;
    };
    commands.spawn((
        Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
        ImageNode::new(handle).with_mode(NodeImageMode::Stretch),
        GlobalZIndex(19),
        LoadingUi,
    ));
}

fn spawn_ui(commands: &mut Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::FlexEnd,
                align_items: AlignItems::Center,
                row_gap: Val::Px(16.0),
                padding: UiRect::bottom(Val::Px(90.0)),
                ..default()
            },
            GlobalZIndex(20),
            LoadingUi,
        ))
        .with_children(|root| {
            root.spawn((
                Text::new("EAW:REMAKE"),
                TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(54.0), ..default() },
                TextColor(Color::srgb(0.85, 0.93, 1.0)),
                TextShadow { offset: Vec2::splat(2.0), color: Color::srgba(0.0, 0.1, 0.3, 0.9) },
            ));
            root.spawn((
                Text::new("Loading"),
                TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(22.0), ..default() },
                TextColor(Color::srgb(0.75, 0.86, 1.0)),
                TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.1, 0.95) },
                StatusText,
            ));
            root.spawn((
                Node {
                    width: Val::Px(560.0),
                    height: Val::Px(12.0),
                    border: UiRect::all(Val::Px(1.0)),
                    margin: UiRect::top(Val::Px(10.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.05, 0.10, 0.85)),
                BorderColor::all(Color::srgba(0.35, 0.65, 1.0, 0.8)),
            ))
            .with_children(|bar| {
                bar.spawn((
                    Node { width: Val::Percent(0.0), height: Val::Percent(100.0), ..default() },
                    BackgroundColor(Color::srgb(0.35, 0.7, 1.0)),
                    BarFill,
                ));
            });
        });
}

fn despawn_ui(mut commands: Commands, ui: Query<Entity, With<LoadingUi>>) {
    for e in &ui {
        commands.entity(e).despawn();
    }
}

fn update_ui(
    progress: Res<Progress>,
    mut fill: Single<&mut Node, With<BarFill>>,
    mut text: Single<&mut Text, With<StatusText>>,
) {
    fill.width = Val::Percent((progress.fraction * 100.0).clamp(0.0, 100.0));
    if text.0 != progress.text {
        text.0 = progress.text.clone();
    }
}
