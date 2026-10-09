//! Options menu: an overlay with one row per setting (`<` value `>`), shown over the main menu
//! or the pause menu. Changes apply live and are saved to the settings file on "Done".

use bevy::{prelude::*, ui::FocusPolicy};

use crate::flow::UiSound;
use crate::menu::SFX_CLICK;
use crate::settings::{OptionKey, Settings};

/// True while the options overlay is open.
#[derive(Resource, Default, Debug)]
pub struct OptionsOpen(pub bool);

/// Root of the options overlay.
#[derive(Component)]
struct OptionsOverlay;

/// The scrollable list of setting rows (the window can be shorter than the list, e.g. 1080p windowed).
#[derive(Component)]
struct OptionsList;

/// Scroll offset of the list, kept across the rebuilds that follow every changed setting.
#[derive(Resource, Default)]
struct OptionsScroll(f32);

/// What a button of the overlay does.
#[derive(Component, Clone, Copy)]
enum OptionButton {
    Step(OptionKey, i32),
    Defaults,
    Done,
}

const PANEL_BG: Color = Color::srgba(0.03, 0.05, 0.12, 1.0);
const PANEL_BORDER: Color = Color::srgb(0.42, 0.58, 0.88);
const BUTTON_IDLE: Color = Color::srgb(0.01, 0.01, 0.03);
const BUTTON_HOVER: Color = Color::srgb(0.1, 0.13, 0.24);
const BUTTON_PRESSED: Color = Color::srgb(0.2, 0.28, 0.5);

/// Options overlay and its input handling.
pub struct OptionsPlugin;

impl Plugin for OptionsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OptionsOpen>().init_resource::<OptionsScroll>().add_systems(
            Update,
            (
                option_buttons,
                scroll_list,
                rebuild_overlay.run_if(resource_changed::<OptionsOpen>.or_eager(resource_changed::<Settings>)),
            )
                .chain(),
        );
    }
}

/// Closes the overlay and stores the settings.
pub fn close(open: &mut OptionsOpen, settings: &Settings) {
    open.0 = false;
    settings.save();
}

fn option_buttons(
    mut buttons: Query<(&Interaction, &OptionButton, &mut BackgroundColor), Changed<Interaction>>,
    mut settings: ResMut<Settings>,
    mut open: ResMut<OptionsOpen>,
    mut sounds: MessageWriter<UiSound>,
) {
    for (interaction, button, mut bg) in &mut buttons {
        match interaction {
            Interaction::Pressed => {
                bg.0 = BUTTON_PRESSED;
                sounds.write(UiSound(SFX_CLICK));
                match *button {
                    OptionButton::Step(key, dir) => settings.step(key, dir),
                    OptionButton::Defaults => *settings = Settings::default(),
                    OptionButton::Done => close(&mut open, &settings),
                }
            }
            Interaction::Hovered => bg.0 = BUTTON_HOVER,
            Interaction::None => bg.0 = BUTTON_IDLE,
        }
    }
}

/// Rebuilds the overlay whenever it is opened or a value changed.
fn rebuild_overlay(
    mut commands: Commands,
    open: Res<OptionsOpen>,
    settings: Res<Settings>,
    scroll: Res<OptionsScroll>,
    existing: Query<Entity, With<OptionsOverlay>>,
) {
    for e in &existing {
        commands.entity(e).despawn();
    }
    if open.0 {
        spawn_overlay(&mut commands, &settings, scroll.0);
    }
}

fn small_button(parent: &mut ChildSpawnerCommands, text: &str, action: OptionButton, width: f32) {
    parent
        .spawn((
            Button,
            Node {
                width: Val::Px(width),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(BUTTON_IDLE),
            BorderColor::all(PANEL_BORDER),
            action,
        ))
        .with_children(|b| {
            crate::menu::small_caps_label(b, text, TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(19.0), ..default() }, Color::WHITE, 0.04);
        });
}

fn spawn_overlay(commands: &mut Commands, settings: &Settings, scroll: f32) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.03, 0.7)),
            FocusPolicy::Block,
            GlobalZIndex(70),
            OptionsOverlay,
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    padding: UiRect::all(Val::Px(22.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    align_items: AlignItems::Center,
                    // Never taller than the window: the rows scroll, title and Done stay visible.
                    max_height: Val::Percent(96.0),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
                BorderColor::all(PANEL_BORDER),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new("Options"),
                    TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(34.0), ..default() },
                    TextColor(Color::srgb(0.85, 0.93, 1.0)),
                ));
                panel.spawn((OptionsList, ScrollPosition(Vec2::new(0.0, scroll)), Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Center, flex_shrink: 1.0, min_height: Val::Px(0.0), overflow: Overflow::scroll_y(), ..default() })).with_children(|panel| {
                for key in OptionKey::ALL {
                    panel
                        .spawn(Node {
                            width: Val::Px(480.0),
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(8.0),
                            margin: UiRect::top(Val::Px(4.0)),
                            ..default()
                        })
                        .with_children(|row| {
                            row.spawn((
                                Node { width: Val::Px(200.0), align_self: AlignSelf::Center, ..default() },
                                Text::new(key.label()),
                                TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(18.0), ..default() },
                                TextColor(Color::srgb(0.75, 0.85, 0.95)),
                            ));
                            small_button(row, "<", OptionButton::Step(key, -1), 34.0);
                            row.spawn((
                                Node {
                                    width: Val::Px(150.0),
                                    align_self: AlignSelf::Center,
                                    justify_content: JustifyContent::Center,
                                    ..default()
                                },
                                Text::new(settings.value_text(key)),
                                TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(18.0), ..default() },
                                TextColor(Color::WHITE),
                            ));
                            small_button(row, ">", OptionButton::Step(key, 1), 34.0);
                        });
                }
                });
                panel
                    .spawn(Node {
                        column_gap: Val::Px(14.0),
                        margin: UiRect::top(Val::Px(14.0)),
                        ..default()
                    })
                    .with_children(|row| {
                        small_button(row, "Defaults", OptionButton::Defaults, 140.0);
                        small_button(row, "Done", OptionButton::Done, 140.0);
                    });
            });
        });
}

/// Mouse wheel over the options scrolls the setting rows.
fn scroll_list(
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    mut list: Query<(&mut ScrollPosition, &ComputedNode), With<OptionsList>>,
    mut keep: ResMut<OptionsScroll>,
) {
    let delta: f32 = wheel.read().map(|w| if matches!(w.unit, bevy::input::mouse::MouseScrollUnit::Line) { w.y * 40.0 } else { w.y }).sum();
    for (mut pos, node) in &mut list {
        let inv = node.inverse_scale_factor();
        let max = ((node.content_size().y - node.size().y) * inv).max(0.0);
        pos.y = (pos.y - delta).clamp(0.0, max);
        if keep.0 != pos.y {
            keep.0 = pos.y;
        }
    }
}
