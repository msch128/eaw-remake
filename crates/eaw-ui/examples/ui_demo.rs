//! Renders the Empire at War command bar offscreen (no window) to a PNG.
//!
//! ```text
//! ui_demo --offscreen <out.png> [--faction Rebel|Empire|Underworld] [--mode galactic|space]
//!         [--size WxH] [--hover button_name] [--tooltip button_name] [--select button_name]
//! ```
//! Without `--offscreen` this opens a normal window with the same content.

use std::sync::Arc;
use std::time::Duration;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use alamo_model::image::{Rgba8, encode_png};
use bevy::app::ScheduleRunnerPlugin;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::time::TimeUpdateStrategy;
use bevy::window::ExitCondition;
use eaw_ui::*;

#[derive(Resource)]
struct Shot {
    out: String,
    target: Handle<Image>,
    frame: u32,
    spec: CommandBarSpec,
    hover: Vec<String>,
    select: Vec<String>,
    data: CommandBarData,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let all = |k: &str| -> Vec<String> {
        args.iter()
            .enumerate()
            .filter(|(_, a)| *a == k)
            .filter_map(|(i, _)| args.get(i + 1).cloned())
            .collect()
    };
    let out = arg("--offscreen");
    let faction = UiFaction::from_name(&arg("--faction").unwrap_or_else(|| "Empire".into()));
    let mode = match arg("--mode").as_deref() {
        Some("space" | "tactical") => CommandBarMode::SpaceTactical,
        _ => CommandBarMode::Galactic,
    };
    let (w, h) = arg("--size")
        .and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().ok(), b.parse().ok())))
        .and_then(|(a, b)| Some((a?, b?)))
        .unwrap_or((1920u32, 1080u32));

    let install = GameInstall::detect().expect("Empire at War install not found");
    let fs = Arc::new(GameFs::for_mod(&install, Some("1770851727")).expect("file system"));
    let (game, _) = load_galaxy(&*fs);
    let game = Arc::new(game);

    let settings = EawUiSettings {
        scale: if out.is_some() {
            UiScaleMode::Fixed(h as f32 / 768.0)
        } else {
            UiScaleMode::Auto
        },
        faceplate_oversample: (h as f32 / 768.0).ceil().max(2.0),
        font_family: arg("--font").map_or(Some("Arial".into()), |f| (f != "none").then_some(f)),
        ..default()
    };
    let data = sample_data(&game, faction);

    let mut app = App::new();
    if out.is_some() {
        app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: None,
            exit_condition: ExitCondition::DontExit,
            ..default()
        }))
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_millis(1)))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(16)));
    } else {
        app.add_plugins(DefaultPlugins);
    }
    app.add_plugins(EawUiPlugin::new(fs).with_game_data(game).with_settings(settings))
        .insert_resource(ClearColor(Color::srgb(0.02, 0.025, 0.04)));

    let spec = CommandBarSpec { mode, faction };
    if let Some(out) = out {
        let mut image = Image::new_uninit(
            Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            TextureFormat::Rgba8UnormSrgb,
            default(),
        );
        image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING
            | TextureUsages::COPY_SRC
            | TextureUsages::RENDER_ATTACHMENT;
        let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        let cam = app
            .world_mut()
            .spawn((Camera2d, RenderTarget::Image(target.clone().into())))
            .id();
        app.world_mut().resource_mut::<EawUiSettings>().camera = Some(cam);
        app.insert_resource(Shot {
            out,
            target,
            frame: 0,
            spec,
            hover: all("--hover"),
            select: all("--select"),
            data,
        });
        if let Some(t) = arg("--tooltip") {
            app.insert_resource(ForcedTooltip(Some(t)));
        }
        app.add_systems(Update, drive);
    } else {
        app.insert_resource(data);
        app.add_systems(Startup, move |mut c: Commands| {
            c.spawn(Camera2d);
            c.spawn_eaw_command_bar(spec);
        });
        app.add_systems(Update, |mut ev: MessageReader<EawButtonClicked>| {
            for e in ev.read() {
                println!("clicked {}", e.name);
            }
        });
    }
    app.run();
}

fn sample_data(game: &alamo_data::GameData, faction: UiFaction) -> CommandBarData {
    let want = faction.name().to_lowercase();
    let mut icons: Vec<String> = game
        .objects
        .iter()
        .filter(|o| {
            ["Squadron", "SpaceUnit", "UniqueUnit"].iter().any(|k| o.kind.eq_ignore_ascii_case(k))
                && !o.name.to_lowercase().contains("template")
        })
        .filter(|o| {
            game.objects
                .resolve(&o.name)
                .and_then(|r| r.get("affiliation").map(|a| a.to_lowercase()))
                .is_some_and(|a| a.contains(&want) || (want == "underworld" && a.contains("underworld")))
        })
        .filter_map(|o| unit_icon(game, &o.name))
        .filter(|i| !i.is_empty())
        .collect();
    icons.dedup();
    println!("{} sample icons, first: {:?}", icons.len(), icons.first());
    let pick = |i: usize| icons.get(i % icons.len().max(1)).cloned().unwrap_or_default();
    CommandBarData {
        credits: 12_450,
        planet_income: 350,
        planet_name: "Coruscant".into(),
        planet_owner: Some(faction),
        day: 37,
        tech_level: 3,
        pop_current: 42,
        pop_cap: 100,
        space_tab: true,
        queue: (0..4)
            .map(|i| QueueEntry {
                icon: pick(i * 3),
                progress: if i == 0 { 0.6 } else { 0.0 },
                count: if i == 1 { 3 } else { 1 },
            })
            .collect(),
        pads: (0..16)
            .map(|i| BuildPadEntry {
                icon: pick(i),
                price: Some(150 + 25 * i as u32),
                disabled: i % 5 == 4,
                building: false,
            })
            .collect(),
        selection: (0..6)
            .map(|i| SelectionEntry {
                icon: pick(i + 1),
                health: 1.0 - i as f32 * 0.15,
                shield: (i % 2 == 0).then_some(0.8 - i as f32 * 0.1),
                count: 1 + i as u32 % 3,
            })
            .collect(),
        abilities: Vec::new(),
    }
}

fn drive(
    mut commands: Commands,
    mut shot: ResMut<Shot>,
    mut buttons: Query<(&EawButton, &mut EawButtonState)>,
    mut exit: MessageWriter<AppExit>,
) {
    shot.frame += 1;
    if shot.frame == 2 {
        let spec = shot.spec;
        commands.insert_resource(shot.data.clone());
        commands.spawn_eaw_command_bar(spec);
    }
    if shot.frame == 6 {
        for (b, mut st) in &mut buttons {
            st.force_hover = shot.hover.iter().any(|h| h.eq_ignore_ascii_case(&b.name));
            st.selected |= shot.select.iter().any(|h| h.eq_ignore_ascii_case(&b.name));
        }
    }
    if shot.frame == 40 {
        let path = shot.out.clone();
        commands
            .spawn(Screenshot::image(shot.target.clone()))
            .observe(move |cap: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                let img = &cap.image;
                let mut px = img.data.clone().unwrap_or_default();
                if img.texture_descriptor.format == TextureFormat::Bgra8UnormSrgb {
                    for p in px.as_chunks_mut::<4>().0.iter_mut() {
                        p.swap(0, 2);
                    }
                }
                let png = encode_png(&Rgba8 {
                    width: img.width() as usize,
                    height: img.height() as usize,
                    pixels: px,
                });
                std::fs::write(&path, png).expect("write png");
                println!("wrote {path}");
                exit.write(AppExit::Success);
            });
    }
    let _ = &mut exit;
}




