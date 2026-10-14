//! User settings: window, graphics, audio and camera options, persisted as a small TOML file
//! under `%APPDATA%\SpacePeace\settings.toml` (never inside the repository). Command line flags
//! override the stored values for the current run.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use alamo_audio::AudioBuses;
use bevy::{
    prelude::*,
    window::{MonitorSelection, PresentMode, PrimaryWindow, WindowMode},
};

use crate::cli::Args;

/// Selectable window resolutions (windowed mode).
pub const RESOLUTIONS: [(u32, u32); 6] =
    [(1280, 720), (1600, 900), (1920, 1080), (2560, 1440), (3200, 1800), (3840, 2160)];
/// Selectable frame rate caps; 0 means unlimited.
pub const FPS_CAPS: [u32; 6] = [0, 30, 60, 120, 144, 240];
/// Selectable UI scales.
pub const UI_SCALES: [f32; 6] = [0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
/// Selectable MSAA sample counts (1 = off).
pub const MSAA_MODES: [u32; 4] = [1, 2, 4, 8];
/// Selectable camera speed multipliers.
pub const CAMERA_SPEEDS: [f32; 5] = [0.5, 0.75, 1.0, 1.5, 2.0];
/// Selectable bloom intensity multipliers.
pub const BLOOM_INTENSITIES: [f32; 4] = [0.5, 1.0, 1.5, 2.0];
/// Selectable key choices for pause (EaW has no default pause key; Space is Movie Mode).
pub const PAUSE_KEYS: [KeyCode; 2] = [KeyCode::KeyP, KeyCode::Pause];
/// Selectable key choices for speed / fast-forward.
pub const SPEED_KEYS: [KeyCode; 4] = [KeyCode::Tab, KeyCode::PageUp, KeyCode::NumpadAdd, KeyCode::BracketRight];
/// Selectable key choices for attack-move.
pub const ATTACK_MOVE_KEYS: [KeyCode; 3] = [KeyCode::KeyA, KeyCode::KeyQ, KeyCode::KeyX];
/// Selectable key choices for stop.
pub const STOP_KEYS: [KeyCode; 3] = [KeyCode::KeyS, KeyCode::KeyH, KeyCode::Backquote];
/// Selectable key choices for retreat / abort.
pub const RETREAT_KEYS: [KeyCode; 3] = [KeyCode::F10, KeyCode::KeyR, KeyCode::Delete];

/// Configurable keybindings for common commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBindings {
    /// Pause toggle.
    pub pause: KeyCode,
    /// Fast-forward / game speed toggle.
    pub speed: KeyCode,
    /// Attack-move command.
    pub attack_move: KeyCode,
    /// Stop current orders.
    pub stop: KeyCode,
    /// Tactical retreat / abort battle.
    pub retreat: KeyCode,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            pause: KeyCode::KeyP,
            speed: KeyCode::Tab,
            attack_move: KeyCode::KeyA,
            stop: KeyCode::KeyS,
            retreat: KeyCode::F10,
        }
    }
}

pub fn key_label(code: KeyCode) -> &'static str {
    match code {
        KeyCode::Space => "Space",
        KeyCode::KeyP => "P",
        KeyCode::Pause => "Pause",
        KeyCode::Tab => "Tab",
        KeyCode::PageUp => "Page Up",
        KeyCode::NumpadAdd => "Num +",
        KeyCode::BracketRight => "]",
        KeyCode::KeyA => "A",
        KeyCode::KeyQ => "Q",
        KeyCode::KeyX => "X",
        KeyCode::KeyS => "S",
        KeyCode::KeyH => "H",
        KeyCode::Backquote => "`",
        KeyCode::F10 => "F10",
        KeyCode::KeyR => "R",
        KeyCode::Delete => "Delete",
        _ => "Custom",
    }
}

pub fn key_code_str(code: KeyCode) -> &'static str {
    match code {
        KeyCode::Space => "space",
        KeyCode::KeyP => "p",
        KeyCode::Pause => "pause",
        KeyCode::Tab => "tab",
        KeyCode::PageUp => "page_up",
        KeyCode::NumpadAdd => "numpad_add",
        KeyCode::BracketRight => "bracket_right",
        KeyCode::KeyA => "a",
        KeyCode::KeyQ => "q",
        KeyCode::KeyX => "x",
        KeyCode::KeyS => "s",
        KeyCode::KeyH => "h",
        KeyCode::Backquote => "backquote",
        KeyCode::F10 => "f10",
        KeyCode::KeyR => "r",
        KeyCode::Delete => "delete",
        _ => "unknown",
    }
}

pub fn parse_key(s: &str) -> Option<KeyCode> {
    match s.trim().to_lowercase().as_str() {
        "space" => Some(KeyCode::Space),
        "p" => Some(KeyCode::KeyP),
        "pause" => Some(KeyCode::Pause),
        "tab" => Some(KeyCode::Tab),
        "page_up" | "pageup" => Some(KeyCode::PageUp),
        "numpad_add" | "num_plus" | "+" => Some(KeyCode::NumpadAdd),
        "bracket_right" | "]" => Some(KeyCode::BracketRight),
        "a" => Some(KeyCode::KeyA),
        "q" => Some(KeyCode::KeyQ),
        "x" => Some(KeyCode::KeyX),
        "s" => Some(KeyCode::KeyS),
        "h" => Some(KeyCode::KeyH),
        "backquote" | "`" => Some(KeyCode::Backquote),
        "f10" => Some(KeyCode::F10),
        "r" => Some(KeyCode::KeyR),
        "delete" | "del" => Some(KeyCode::Delete),
        _ => None,
    }
}

/// Stored master volume and the `--volume` value that replaced it for this run: the override must never be written back
/// into the user's settings file (a `--volume 0` test run once left the real file silent).
static CLI_MASTER: std::sync::OnceLock<(f32, f32)> = std::sync::OnceLock::new();

/// All user-configurable options.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct Settings {
    /// Borderless fullscreen instead of a window.
    pub borderless: bool,
    /// Window size in windowed mode.
    pub resolution: (u32, u32),
    /// Vertical sync.
    pub vsync: bool,
    /// Frame rate cap, 0 = unlimited.
    pub fps_cap: u32,
    /// Show the FPS counter.
    pub show_fps: bool,
    /// UI scale factor.
    pub ui_scale: f32,
    /// Master volume 0..1.
    pub master: f32,
    /// Music volume 0..1.
    pub music: f32,
    /// Sound effect volume 0..1.
    pub sfx: f32,
    /// Speech volume 0..1.
    pub speech: f32,
    /// Scroll the map when the cursor touches the screen edge.
    pub edge_scroll: bool,
    /// Camera pan speed multiplier.
    pub camera_speed: f32,
    /// MSAA samples of the 3D views (1, 2, 4 or 8).
    pub msaa: u32,
    /// Bloom around bright lights.
    pub bloom: bool,
    /// Bloom intensity multiplier.
    pub bloom_intensity: f32,
    /// Cascaded sun shadows in space battles.
    pub shadows: bool,
    /// Particle spawn rate multiplier of the battle view.
    pub particle_density: f32,
    /// Configurable keybindings for common commands.
    pub bindings: KeyBindings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            borderless: false,
            resolution: (1600, 900),
            vsync: true,
            fps_cap: 0,
            show_fps: false,
            ui_scale: 1.0,
            master: 0.5,
            music: 0.6,
            sfx: 1.0,
            speech: 1.0,
            edge_scroll: true,
            camera_speed: 1.0,
            msaa: 4,
            bloom: true,
            bloom_intensity: 1.0,
            shadows: true,
            particle_density: 1.0,
            bindings: KeyBindings::default(),
        }
    }
}

/// One row of the options menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKey {
    /// Window or borderless fullscreen.
    WindowMode,
    /// Window resolution.
    Resolution,
    /// Vertical sync.
    Vsync,
    /// Frame rate cap.
    FpsCap,
    /// FPS counter overlay.
    ShowFps,
    /// UI scale.
    UiScale,
    /// Master volume.
    Master,
    /// Music volume.
    Music,
    /// Effects volume.
    Sfx,
    /// Speech volume.
    Speech,
    /// Edge scrolling.
    EdgeScroll,
    /// Camera speed.
    CameraSpeed,
    /// Anti-aliasing (MSAA).
    Msaa,
    /// Bloom.
    Bloom,
    /// Bloom intensity multiplier.
    BloomIntensity,
    /// Graphics quality preset (sets anti-aliasing, bloom, shadows and particle density).
    Quality,
    /// Sun shadows in battles.
    Shadows,
    /// Keybinding for pause.
    KeyPause,
    /// Keybinding for game speed / fast-forward.
    KeySpeed,
    /// Keybinding for attack-move.
    KeyAttackMove,
    /// Keybinding for stop order.
    KeyStop,
    /// Keybinding for tactical retreat / abort.
    KeyRetreat,
}

impl OptionKey {
    /// All rows in display order.
    pub const ALL: [OptionKey; 22] = [
        OptionKey::WindowMode,
        OptionKey::Resolution,
        OptionKey::Vsync,
        OptionKey::FpsCap,
        OptionKey::ShowFps,
        OptionKey::UiScale,
        OptionKey::Master,
        OptionKey::Music,
        OptionKey::Sfx,
        OptionKey::Speech,
        OptionKey::EdgeScroll,
        OptionKey::CameraSpeed,
        OptionKey::Msaa,
        OptionKey::Bloom,
        OptionKey::BloomIntensity,
        OptionKey::Quality,
        OptionKey::Shadows,
        OptionKey::KeyPause,
        OptionKey::KeySpeed,
        OptionKey::KeyAttackMove,
        OptionKey::KeyStop,
        OptionKey::KeyRetreat,
    ];

    /// Row label.
    pub fn label(self) -> &'static str {
        match self {
            OptionKey::WindowMode => "Window mode",
            OptionKey::Resolution => "Resolution",
            OptionKey::Vsync => "Vertical sync",
            OptionKey::FpsCap => "FPS limit",
            OptionKey::ShowFps => "Show FPS",
            OptionKey::UiScale => "UI scale",
            OptionKey::Master => "Master volume",
            OptionKey::Music => "Music volume",
            OptionKey::Sfx => "Effects volume",
            OptionKey::Speech => "Speech volume",
            OptionKey::EdgeScroll => "Edge scrolling",
            OptionKey::CameraSpeed => "Camera speed",
            OptionKey::Msaa => "Anti-aliasing",
            OptionKey::Bloom => "Bloom",
            OptionKey::BloomIntensity => "Bloom intensity",
            OptionKey::Quality => "Graphics quality",
            OptionKey::Shadows => "Shadows",
            OptionKey::KeyPause => "Key: Pause",
            OptionKey::KeySpeed => "Key: Fast-forward",
            OptionKey::KeyAttackMove => "Key: Attack-move",
            OptionKey::KeyStop => "Key: Stop order",
            OptionKey::KeyRetreat => "Key: Retreat / abort",
        }
    }
}

/// Index of the entry of `list` nearest to `value`.
fn nearest<T: Copy>(list: &[T], dist: impl Fn(T) -> f32) -> usize {
    (0..list.len())
        .min_by(|&a, &b| dist(list[a]).total_cmp(&dist(list[b])))
        .unwrap_or(0)
}

/// Moves `index` by `dir` inside a list of `len` entries, wrapping around.
fn cycle(index: usize, dir: i32, len: usize) -> usize {
    (index as i32 + dir).rem_euclid(len as i32) as usize
}

fn step_volume(v: f32, dir: i32) -> f32 {
    (((v * 10.0).round() + dir as f32).clamp(0.0, 10.0)) / 10.0
}

impl Settings {
    /// Text shown for the current value of `key`.
    pub fn value_text(&self, key: OptionKey) -> String {
        let pct = |v: f32| format!("{:.0}%", v * 100.0);
        let on = |b: bool| if b { "On" } else { "Off" }.to_owned();
        match key {
            OptionKey::WindowMode => if self.borderless { "Borderless" } else { "Windowed" }.into(),
            OptionKey::Resolution => format!("{} x {}", self.resolution.0, self.resolution.1),
            OptionKey::Vsync => on(self.vsync),
            OptionKey::ShowFps => on(self.show_fps),
            OptionKey::FpsCap => {
                if self.fps_cap == 0 { "Unlimited".into() } else { self.fps_cap.to_string() }
            }
            OptionKey::UiScale => format!("{:.0}%", self.ui_scale * 100.0),
            OptionKey::Master => pct(self.master),
            OptionKey::Music => pct(self.music),
            OptionKey::Sfx => pct(self.sfx),
            OptionKey::Speech => pct(self.speech),
            OptionKey::EdgeScroll => on(self.edge_scroll),
            OptionKey::CameraSpeed => format!("{:.2}x", self.camera_speed),
            OptionKey::Msaa => if self.msaa <= 1 { "Off".into() } else { format!("{}x", self.msaa) },
            OptionKey::Bloom => on(self.bloom),
            OptionKey::BloomIntensity => format!("{:.1}x", self.bloom_intensity),
            OptionKey::Quality => self.quality().map_or("Custom", |q| q.label()).into(),
            OptionKey::Shadows => on(self.shadows),
            OptionKey::KeyPause => key_label(self.bindings.pause).into(),
            OptionKey::KeySpeed => key_label(self.bindings.speed).into(),
            OptionKey::KeyAttackMove => key_label(self.bindings.attack_move).into(),
            OptionKey::KeyStop => key_label(self.bindings.stop).into(),
            OptionKey::KeyRetreat => key_label(self.bindings.retreat).into(),
        }
    }

    /// Changes `key` one step forward (`dir > 0`) or backward.
    pub fn step(&mut self, key: OptionKey, dir: i32) {
        match key {
            OptionKey::WindowMode => self.borderless = !self.borderless,
            OptionKey::Resolution => {
                let i = nearest(&RESOLUTIONS, |r| {
                    (r.0 as f32 - self.resolution.0 as f32).abs() + (r.1 as f32 - self.resolution.1 as f32).abs()
                });
                self.resolution = RESOLUTIONS[cycle(i, dir, RESOLUTIONS.len())];
            }
            OptionKey::Vsync => self.vsync = !self.vsync,
            OptionKey::ShowFps => self.show_fps = !self.show_fps,
            OptionKey::FpsCap => {
                let i = nearest(&FPS_CAPS, |c| (c as f32 - self.fps_cap as f32).abs());
                self.fps_cap = FPS_CAPS[cycle(i, dir, FPS_CAPS.len())];
            }
            OptionKey::UiScale => {
                let i = nearest(&UI_SCALES, |s| (s - self.ui_scale).abs());
                self.ui_scale = UI_SCALES[cycle(i, dir, UI_SCALES.len())];
            }
            OptionKey::Master => self.master = step_volume(self.master, dir),
            OptionKey::Music => self.music = step_volume(self.music, dir),
            OptionKey::Sfx => self.sfx = step_volume(self.sfx, dir),
            OptionKey::Speech => self.speech = step_volume(self.speech, dir),
            OptionKey::EdgeScroll => self.edge_scroll = !self.edge_scroll,
            OptionKey::CameraSpeed => {
                let i = nearest(&CAMERA_SPEEDS, |s| (s - self.camera_speed).abs());
                self.camera_speed = CAMERA_SPEEDS[cycle(i, dir, CAMERA_SPEEDS.len())];
            }
            OptionKey::Msaa => {
                let i = nearest(&MSAA_MODES, |m| (m as f32 - self.msaa as f32).abs());
                self.msaa = MSAA_MODES[cycle(i, dir, MSAA_MODES.len())];
            }
            OptionKey::Bloom => self.bloom = !self.bloom,
            OptionKey::BloomIntensity => {
                let i = nearest(&BLOOM_INTENSITIES, |b| (b - self.bloom_intensity).abs());
                self.bloom_intensity = BLOOM_INTENSITIES[cycle(i, dir, BLOOM_INTENSITIES.len())];
            }
            OptionKey::Quality => {
                let next = match self.quality() {
                    Some(q) if dir >= 0 => q.next(),
                    Some(q) => q.prev(),
                    None => battle_view::QualityPreset::High,
                };
                self.apply_quality(next);
            }
            OptionKey::Shadows => self.shadows = !self.shadows,
            OptionKey::KeyPause => {
                let i = PAUSE_KEYS.iter().position(|&k| k == self.bindings.pause).unwrap_or(0);
                self.bindings.pause = PAUSE_KEYS[cycle(i, dir, PAUSE_KEYS.len())];
            }
            OptionKey::KeySpeed => {
                let i = SPEED_KEYS.iter().position(|&k| k == self.bindings.speed).unwrap_or(0);
                self.bindings.speed = SPEED_KEYS[cycle(i, dir, SPEED_KEYS.len())];
            }
            OptionKey::KeyAttackMove => {
                let i = ATTACK_MOVE_KEYS.iter().position(|&k| k == self.bindings.attack_move).unwrap_or(0);
                self.bindings.attack_move = ATTACK_MOVE_KEYS[cycle(i, dir, ATTACK_MOVE_KEYS.len())];
            }
            OptionKey::KeyStop => {
                let i = STOP_KEYS.iter().position(|&k| k == self.bindings.stop).unwrap_or(0);
                self.bindings.stop = STOP_KEYS[cycle(i, dir, STOP_KEYS.len())];
            }
            OptionKey::KeyRetreat => {
                let i = RETREAT_KEYS.iter().position(|&k| k == self.bindings.retreat).unwrap_or(0);
                self.bindings.retreat = RETREAT_KEYS[cycle(i, dir, RETREAT_KEYS.len())];
            }
        }
    }

    /// The graphics preset that matches the current MSAA / bloom / shadow / particle values, if any.
    pub fn quality(&self) -> Option<battle_view::QualityPreset> {
        use battle_view::QualityPreset::*;
        [Low, Medium, High].into_iter().find(|q| {
            let (msaa, bloom, shadows, density) = q.values();
            self.msaa == msaa && self.bloom == bloom && self.shadows == shadows && (self.particle_density - density).abs() < 0.01
        })
    }

    /// Applies a graphics preset.
    pub fn apply_quality(&mut self, q: battle_view::QualityPreset) {
        (self.msaa, self.bloom, self.shadows, self.particle_density) = q.values();
    }

    /// Location of the settings file, `None` when `%APPDATA%` is not set.
    pub fn path() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("SpacePeace").join("settings.toml"))
    }

    /// Loads the stored settings, falling back to defaults for a missing or unreadable file.
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| Self::from_toml(&s))
            .unwrap_or_default()
    }

    /// The settings as they go into the file: without the `--volume` override of this run.
    fn stored(&self) -> Settings {
        let mut s = self.clone();
        if let Some(&(stored, over)) = CLI_MASTER.get()
            && s.master == over
        {
            s.master = stored;
        }
        s
    }

    /// Writes the settings file (errors are logged, not fatal).
    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        let result = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, self.stored().to_toml()));
        if let Err(e) = result {
            warn!("cannot save settings to {}: {e}", path.display());
        }
    }

    /// Serialises to a flat TOML document.
    pub fn to_toml(&self) -> String {
        format!(
            "# Space Peace settings\n\
             window_mode = \"{}\"\n\
             resolution = [{}, {}]\n\
             vsync = {}\n\
             fps_cap = {}\n\
             show_fps = {}\n\
             ui_scale = {}\n\
             master_volume = {}\n\
             music_volume = {}\n\
             sfx_volume = {}\n\
             speech_volume = {}\n\
             edge_scroll = {}\n\
             camera_speed = {}\n\
             msaa = {}\n\
             bloom = {}\n\
             bloom_intensity = {}\n\
             shadows = {}\n\
             particle_density = {}\n\
             key_pause = \"{}\"\n\
             key_speed = \"{}\"\n\
             key_attack_move = \"{}\"\n\
             key_stop = \"{}\"\n\
             key_retreat = \"{}\"\n",
            if self.borderless { "borderless" } else { "windowed" },
            self.resolution.0,
            self.resolution.1,
            self.vsync,
            self.fps_cap,
            self.show_fps,
            self.ui_scale,
            self.master,
            self.music,
            self.sfx,
            self.speech,
            self.edge_scroll,
            self.camera_speed,
            self.msaa,
            self.bloom,
            self.bloom_intensity,
            self.shadows,
            self.particle_density,
            key_code_str(self.bindings.pause),
            key_code_str(self.bindings.speed),
            key_code_str(self.bindings.attack_move),
            key_code_str(self.bindings.stop),
            key_code_str(self.bindings.retreat),
        )
    }

    /// Parses a document written by [`Settings::to_toml`]; unknown or invalid entries keep the
    /// default value.
    pub fn from_toml(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            let float = || value.parse::<f32>().ok();
            let boolean = || value.parse::<bool>().ok();
            match key {
                "window_mode" => s.borderless = value.trim_matches('"') == "borderless",
                "resolution" => {
                    let nums: Vec<u32> = value
                        .trim_matches(|c| c == '[' || c == ']')
                        .split(',')
                        .filter_map(|n| n.trim().parse().ok())
                        .collect();
                    if let [w, h] = nums[..]
                        && w >= 640
                        && h >= 480
                    {
                        s.resolution = (w, h);
                    }
                }
                "vsync" => s.vsync = boolean().unwrap_or(s.vsync),
                "fps_cap" => s.fps_cap = value.parse().unwrap_or(s.fps_cap),
                "show_fps" => s.show_fps = boolean().unwrap_or(s.show_fps),
                "ui_scale" => s.ui_scale = float().unwrap_or(s.ui_scale).clamp(0.5, 3.0),
                "master_volume" => s.master = float().unwrap_or(s.master).clamp(0.0, 1.0),
                "music_volume" => s.music = float().unwrap_or(s.music).clamp(0.0, 1.0),
                "sfx_volume" => s.sfx = float().unwrap_or(s.sfx).clamp(0.0, 1.0),
                "speech_volume" => s.speech = float().unwrap_or(s.speech).clamp(0.0, 1.0),
                "edge_scroll" => s.edge_scroll = boolean().unwrap_or(s.edge_scroll),
                "camera_speed" => s.camera_speed = float().unwrap_or(s.camera_speed).clamp(0.1, 4.0),
                "msaa" => s.msaa = value.parse().ok().filter(|m| MSAA_MODES.contains(m)).unwrap_or(s.msaa),
                "bloom" => s.bloom = boolean().unwrap_or(s.bloom),
                "bloom_intensity" => s.bloom_intensity = float().unwrap_or(s.bloom_intensity).clamp(0.25, 3.0),
                "shadows" => s.shadows = boolean().unwrap_or(s.shadows),
                "particle_density" => s.particle_density = float().unwrap_or(s.particle_density).clamp(0.25, 1.0),
                "key_pause" => if let Some(k) = parse_key(value.trim_matches('"')) { s.bindings.pause = if k == KeyCode::Space { KeyCode::KeyP } else { k }; },
                "key_speed" => if let Some(k) = parse_key(value.trim_matches('"')) { s.bindings.speed = k; },
                "key_attack_move" => if let Some(k) = parse_key(value.trim_matches('"')) { s.bindings.attack_move = k; },
                "key_stop" => if let Some(k) = parse_key(value.trim_matches('"')) { s.bindings.stop = k; },
                "key_retreat" => if let Some(k) = parse_key(value.trim_matches('"')) { s.bindings.retreat = k; },
                _ => {}
            }
        }
        s
    }

    /// Applies the command line flags on top of the stored settings.
    pub fn with_cli(mut self, args: &Args) -> Self {
        if args.no_vsync {
            self.vsync = false;
        }
        if args.fullscreen {
            self.borderless = true;
        }
        if let Some(v) = args.volume {
            let _ = CLI_MASTER.set((self.master, v));
            self.master = v;
        }
        self
    }
}

/// Applies [`Settings`] to the window, UI, audio buses and frame pacing.
pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (apply_window, apply_graphics, apply_ui_scale.run_if(not(resource_exists::<eaw_ui::EawUiSettings>)), apply_audio).run_if(resource_changed::<Settings>),
        )
        .add_systems(Last, limit_fps);
    }
}

/// MSAA and bloom of the galaxy camera (the battle reads them when it starts).
fn apply_graphics(
    settings: Res<Settings>,
    mut commands: Commands,
    mut cameras: Query<(Entity, &mut Msaa, Has<bevy::post_process::bloom::Bloom>, Option<&mut bevy::post_process::bloom::Bloom>), With<crate::camera::GalaxyCamera>>,
) {
    let msaa = match settings.msaa {
        1 => Msaa::Off,
        2 => Msaa::Sample2,
        8 => Msaa::Sample8,
        _ => Msaa::Sample4,
    };
    for (e, mut current, has_bloom, maybe_bloom) in &mut cameras {
        if *current != msaa {
            *current = msaa;
        }
        let intensity = 0.12 * settings.bloom_intensity;
        if settings.bloom && !has_bloom {
            commands.entity(e).insert(bevy::post_process::bloom::Bloom { intensity, ..bevy::post_process::bloom::Bloom::NATURAL });
        } else if settings.bloom && has_bloom {
            if let Some(mut b) = maybe_bloom {
                if (b.intensity - intensity).abs() > 0.001 {
                    b.intensity = intensity;
                }
            }
        } else if !settings.bloom && has_bloom {
            commands.entity(e).remove::<bevy::post_process::bloom::Bloom>();
        }
    }
}

fn apply_window(settings: Res<Settings>, window: Option<Single<&mut Window, With<PrimaryWindow>>>) {
    let Some(mut window) = window else { return };
    let mode = if settings.borderless {
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    } else {
        WindowMode::Windowed
    };
    if window.mode != mode {
        window.mode = mode;
    }
    let (w, h) = settings.resolution;
    if !settings.borderless && (window.resolution.width() as u32, window.resolution.height() as u32) != (w, h) {
        window.resolution.set(w as f32, h as f32);
    }
    let present = if settings.vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync };
    if window.present_mode != present {
        window.present_mode = present;
    }
}

fn apply_ui_scale(settings: Res<Settings>, mut scale: ResMut<UiScale>) {
    if scale.0 != settings.ui_scale {
        scale.0 = settings.ui_scale;
    }
}

fn apply_audio(settings: Res<Settings>, buses: Option<ResMut<AudioBuses>>) {
    if let Some(mut b) = buses {
        b.master = settings.master;
        b.music = settings.music;
        b.sfx = settings.sfx;
        b.speech = settings.speech;
    }
}

/// Sleeps at the end of the frame so the frame rate does not exceed the cap.
fn limit_fps(settings: Res<Settings>, mut last: Local<Option<Instant>>) {
    if settings.fps_cap > 0 {
        let target = Duration::from_secs_f64(1.0 / f64::from(settings.fps_cap));
        if let Some(prev) = *last {
            let spent = prev.elapsed();
            if spent < target {
                let remaining = target - spent;
                // Sleep most of the wait, spin for the last bit for accuracy.
                if remaining > Duration::from_millis(2) {
                    std::thread::sleep(remaining - Duration::from_millis(1));
                }
                while prev.elapsed() < target {
                    std::hint::spin_loop();
                }
            }
        }
    }
    *last = Some(Instant::now());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_round_trip() {
        let s = Settings {
            borderless: true,
            resolution: (2560, 1440),
            vsync: false,
            fps_cap: 144,
            show_fps: true,
            ui_scale: 1.5,
            master: 0.3,
            music: 0.8,
            sfx: 0.9,
            speech: 0.1,
            edge_scroll: false,
            camera_speed: 1.5,
            msaa: 2,
            bloom: false,
            bloom_intensity: 1.5,
            shadows: false,
            particle_density: 0.5,
            bindings: KeyBindings {
                pause: KeyCode::KeyP,
                speed: KeyCode::PageUp,
                attack_move: KeyCode::KeyQ,
                stop: KeyCode::KeyH,
                retreat: KeyCode::Delete,
            },
        };
        assert_eq!(Settings::from_toml(&s.to_toml()), s);
    }

    #[test]
    fn invalid_entries_keep_defaults() {
        let s = Settings::from_toml("vsync = maybe\nui_scale = 99\nresolution = [1, 2]\nunknown = 3\n");
        assert!(s.vsync);
        assert_eq!(s.ui_scale, 3.0);
        assert_eq!(s.resolution, Settings::default().resolution);
    }

    #[test]
    fn steps_cycle_and_clamp() {
        let mut s = Settings::default();
        s.step(OptionKey::Master, 1);
        assert_eq!(s.master, 0.6);
        for _ in 0..20 {
            s.step(OptionKey::Master, 1);
        }
        assert_eq!(s.master, 1.0);
        s.step(OptionKey::FpsCap, -1);
        assert_eq!(s.fps_cap, 240);
        s.step(OptionKey::WindowMode, 1);
        assert!(s.borderless);
        s.step(OptionKey::KeyPause, 1);
        assert_eq!(s.bindings.pause, KeyCode::Pause);
        s.step(OptionKey::BloomIntensity, 1);
        assert_eq!(s.bloom_intensity, 1.5);
    }

    #[test]
    fn cli_overrides() {
        let args = Args { no_vsync: true, volume: Some(0.2), ..Args::default() };
        let s = Settings::default().with_cli(&args);
        assert!(!s.vsync);
        assert_eq!(s.master, 0.2);
    }
}
