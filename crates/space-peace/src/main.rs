//! EAW:Remake - Empire at War galactic conquest shell.

// The installed game (`dist` feature, built by tools/package.ps1) is a windowed app: no console window opens behind it. Normal
// builds stay console apps, the smoke tests read their output.
#![cfg_attr(feature = "dist", windows_subsystem = "windows")]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

mod battle_play;
mod battle_seam;
mod campaign_select;
mod cheat_menu;
mod droid_menu;
mod drag_demo;
mod label_probe;
mod camera;
mod cli;
mod flow;
mod galaxy_map;
mod hero_heads;
mod ui_sounds;
mod planet_fleets;
mod planet_view;
mod galaxy_models;
mod input_test;
mod install_check;
mod galaxy_view;
mod game;
mod loading;
mod logfile;
mod menu;
mod menu_scene;
mod newgame;
mod objectives;
mod offscreen;
mod options;
mod planet_visual;
mod play;
mod play_ui;
mod settings;
mod ui;

use std::{sync::Arc, time::Duration};

use alamo_audio::AlamoAudioPlugin;
use alamo_fs::FileSource;
use alamo_render::AlamoRenderPlugin;
use bevy::{
    app::ScheduleRunnerPlugin,
    audio::AudioPlugin,
    prelude::*,
    render::diagnostic::RenderDiagnosticsPlugin,
    window::{ExitCondition, MonitorSelection, PresentMode, WindowMode},
    winit::WinitPlugin,
};

use battle_play::BattlePlayPlugin;
use battle_seam::BattleSeamPlugin;
use camera::CameraPlugin;
use cli::Args;
use flow::{AppScreen, FlowPlugin};
use galaxy_map::GalaxyMapPlugin;
use galaxy_view::GalaxyView;
use game::{Game, StartupLog};
use loading::{AfterLoading, GameFiles, InitialCampaign, LoadDelay, LoadingPlugin};
use menu::MenuPlugin;
use newgame::NewGamePlugin;
use options::OptionsPlugin;
use play::PlayGluePlugin;
use play_ui::PlayUiPlugin;
use settings::{Settings, SettingsPlugin};
use ui::UiPlugin;

fn main() {
    let args = Args::from_env();
    if args.check_install {
        // First of all: no saves folder, settings, window or GPU is touched; the installer reads the exit code.
        std::process::exit(install_check::run(&args));
    }
    if args.offscreen.is_some() {
        newgame::isolate_offscreen_saves();
    }

    if args.list_campaigns {
        if let Err(e) = game::list_campaigns(&args) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }

    // Opening the file system is fast; the data itself is parsed in the background while the
    // loading screen is shown. Without an install the demo galaxy is used.
    let mut log = StartupLog::default();
    let mut problem = game::StartupProblem::default();
    let mut content = game::ContentChoice { active: game::Content::from_mod_arg(args.mod_id.as_deref()), ..default() };
    let fs = if args.demo {
        log.info("using the built-in demo galaxy (--demo)".into());
        None
    } else {
        match game::open_fs(&args, &mut log) {
            Ok((install, fs)) => {
                content.clone_wars_available = game::clone_wars_installed(&install);
                Some(fs)
            }
            Err(e) => {
                log.warn(format!("FALLING BACK TO DEMO GALAXY: {e}"));
                problem.0 = Some(game::describe_problem(&e, args.mod_id.as_deref().unwrap_or(game::DEFAULT_MOD_ID)));
                None
            }
        }
    };

    // Screen shown once loading is done: the menu, unless a campaign (or a screen) was requested.
    let after = match args.screen.as_deref() {
        _ if args.via_menu => AppScreen::MainMenu,
        Some("campaigns") => AppScreen::CampaignSelect,
        Some("galaxy" | "pause") => AppScreen::Galaxy,
        Some("menu" | "loading" | "options") => AppScreen::MainMenu,
        _ if args.campaign.is_some() || args.demo || args.offscreen.is_some() => AppScreen::Galaxy,
        _ => AppScreen::MainMenu,
    };
    let mut loading_delay = args.loading_delay;
    if args.screen.as_deref() == Some("loading") && loading_delay <= 0.0 {
        loading_delay = 8.0;
    }

    let settings = Settings::load().with_cli(&args);
    let mode = if settings.borderless {
        WindowMode::BorderlessFullscreen(MonitorSelection::Primary)
    } else {
        WindowMode::Windowed
    };
    let present_mode = if settings.vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync };

    let mut app = App::new();
    // A system whose parameter is missing must never take the whole game down: log it and carry on.
    app.set_error_handler(bevy::ecs::error::warn);
    let offscreen = args.offscreen.clone();
    if let Some(path) = &offscreen {
        // Headless: no window, no winit event loop, no audio device; a schedule runner drives
        // the app.
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .disable::<WinitPlugin>()
                .disable::<AudioPlugin>(),
        )
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::ZERO))
        .add_plugins(RenderDiagnosticsPlugin);
        offscreen::configure(&mut app, &args, path.clone());
    } else {
        let mut plugins = DefaultPlugins.set(bevy::log::LogPlugin { custom_layer: logfile::file_layer, ..default() }).set(WindowPlugin {
            primary_window: Some(Window {
                title: "EAW:Remake".into(),
                visible: !args.hidden,
                present_mode,
                mode,
                resolution: settings.resolution.into(),
                ..default()
            }),
            ..default()
        });
        if args.audio_off() {
            // No output stream is opened (it can crackle under CPU load); the audio logic runs against an empty device.
            plugins = plugins.disable::<AudioPlugin>();
        }
        app.add_plugins(plugins);
        if args.audio_off() {
            app.init_asset::<bevy::audio::AudioSource>().insert_resource(alamo_audio::NoAudioDevice);
            info!("audio: device not opened (--no-audio / --volume 0)");
        }
    }
    if let Some(fs) = &fs {
        app.add_plugins(AlamoRenderPlugin::new(fs.clone()));
        app.add_plugins(eaw_ui::EawUiPlugin::new(fs.clone()));
        app.add_plugins((battle_view::BattleViewPlugin, battle_view::tactical_ui::TacticalUiPlugin));
        // An offscreen run has no audio device; with `SP_AUDIO_LEDGER` / `SP_AUDIO_LOG` it still runs the audio logic against an
        // empty device so every sound event of the scripted run is logged and checked against the file system.
        let audio_audit = std::env::var_os("SP_AUDIO_LEDGER").is_some() || std::env::var_os("SP_AUDIO_LOG").is_some();
        if offscreen.is_some() && audio_audit {
            app.init_asset::<bevy::audio::AudioSource>().insert_resource(alamo_audio::NoAudioDevice);
        }
        if offscreen.is_none() || audio_audit {
            app.add_plugins(AlamoAudioPlugin::new(fs.clone() as Arc<dyn FileSource>));
        }
    }
    if (args.hidden || args.exit_after.is_some()) && offscreen.is_none() {
        let frames = args.exit_after.unwrap_or(args.frames);
        app.add_systems(Last, move |mut n: Local<u32>, mut exit: MessageWriter<bevy::app::AppExit>| {
            *n += 1;
            if *n >= frames {
                info!("exit-after: {frames} frames reached");
                exit.write(bevy::app::AppExit::Success);
            }
        });
    }
    add_game(&mut app, Boot { args, fs, after, loading_delay, settings, content, log, problem });
    app.run();
}

/// What `main` works out before the app exists and the game's own wiring needs.
struct Boot {
    args: Args,
    /// The game's files; `None` without an install (the menu then explains it, see `problem`).
    fs: Option<Arc<alamo_fs::GameFs>>,
    /// Screen shown once loading is done.
    after: AppScreen,
    loading_delay: f32,
    settings: Settings,
    content: game::ContentChoice,
    log: StartupLog,
    problem: game::StartupProblem,
}

/// The game on top of the engine plugins: states, resources and plugins. The same for the window, the offscreen and the test app.
fn add_game(app: &mut App, boot: Boot) {
    let Boot { args, fs, after, loading_delay, settings, content, log, problem } = boot;
    app.insert_state(AppScreen::Loading)
        .insert_resource(AfterLoading(after))
        .insert_resource(GameFiles(fs))
        .insert_resource(InitialCampaign(args.campaign.clone()))
        .insert_resource(LoadDelay(loading_delay))
        .insert_resource(flow::Paused(args.screen.as_deref() == Some("pause")))
        .insert_resource(options::OptionsOpen(args.screen.as_deref() == Some("options")))
        .insert_resource(settings)
        .insert_resource(content)
        .insert_resource(newgame::NewGameOptions::from_args(&args))
        .insert_resource(args)
        .insert_resource(log)
        .insert_resource(Game(None))
        .insert_resource(problem)
        .insert_resource(GalaxyView::default())
        .add_systems(Startup, game::replay_startup_log)
        .add_plugins(input_test::plugin)
        .add_plugins(hero_heads::HeroHeadsPlugin)
        .add_plugins(ui_sounds::UiSoundsPlugin)
        .add_plugins(planet_view::PlanetViewPlugin)
        .add_plugins(planet_fleets::plugin)
        .add_plugins(cheat_menu::plugin)
        .add_plugins(droid_menu::plugin)
        .add_plugins(galaxy_models::GalaxyModelsPlugin)
        .add_plugins(drag_demo::DragDemoPlugin)
        .add_plugins(label_probe::LabelProbePlugin)
        .add_plugins((
            FlowPlugin,
            LoadingPlugin,
            MenuPlugin,
            campaign_select::CampaignSelectPlugin,
            menu_scene::MenuScenePlugin,
            NewGamePlugin,
            OptionsPlugin,
            SettingsPlugin,
            BattleSeamPlugin,
            BattlePlayPlugin,
            CameraPlugin,
            GalaxyMapPlugin,
            UiPlugin,
            PlayGluePlugin,
            PlayUiPlugin,
        ));
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use bevy::{
        ecs::error::ErrorContext,
        render::{
            RenderPlugin,
            settings::{RenderCreation, WgpuSettings},
        },
    };

    use super::*;

    /// Every error a system of the test app returned.
    static FAILURES: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn record(error: BevyError, ctx: ErrorContext) {
        FAILURES.lock().unwrap().push(format!("{} `{}`: {error}", ctx.kind(), ctx.name()));
    }

    /// The app the way `main` builds it without an install of the game, on engine plugins without window, audio or renderer.
    fn app_without_game_files() -> App {
        let args = Args::parse(["--screen", "menu", "--volume", "0"].map(String::from)).unwrap();
        let mut app = App::new();
        app.set_error_handler(record);
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin { primary_window: None, exit_condition: bevy::window::ExitCondition::DontExit, ..default() })
                .set(RenderPlugin { render_creation: RenderCreation::Automatic(Box::new(WgpuSettings { backends: None, ..default() })), ..default() })
                .disable::<WinitPlugin>()
                .disable::<AudioPlugin>()
                .disable::<bevy::log::LogPlugin>(),
        );
        let problem = game::StartupProblem(Some(game::describe_problem("no Empire at War install found", game::DEFAULT_MOD_ID)));
        let boot = Boot {
            settings: Settings::default().with_cli(&args),
            args,
            fs: None,
            after: AppScreen::MainMenu,
            loading_delay: 0.0,
            content: default(),
            log: StartupLog::default(),
            problem,
        };
        add_game(&mut app, boot);
        app.finish();
        app.cleanup();
        app
    }

    /// 2026-10-15 `advisor_keep_out` took the tooltip keep-out of `eaw-ui`, which only exists with the game's files: without them it failed its
    /// parameter validation every frame (1,196 warnings in a 3 s run, the smoke row `missing-game` red). Any system that needs something only the
    /// game's files bring must wait for it with a run condition. If this fails, run it with `--features debug` to see which system it is.
    #[test]
    fn the_menu_without_game_files_runs_without_a_failing_system() {
        FAILURES.lock().unwrap().clear();
        let mut app = app_without_game_files();
        // The loading screen hands over on its own once it sees there is nothing to load.
        let started = std::time::Instant::now();
        while *app.world().resource::<State<AppScreen>>().get() != AppScreen::MainMenu && started.elapsed().as_secs() < 20 {
            app.update();
        }
        assert_eq!(*app.world().resource::<State<AppScreen>>().get(), AppScreen::MainMenu, "the menu never came up");
        for _ in 0..60 {
            app.update();
        }
        let failures = FAILURES.lock().unwrap();
        let mut distinct: Vec<&String> = failures.iter().collect();
        distinct.sort();
        distinct.dedup();
        assert!(failures.is_empty(), "{} errors from {} distinct places in 60 frames of the menu: {distinct:#?}", failures.len(), distinct.len());
    }
}
