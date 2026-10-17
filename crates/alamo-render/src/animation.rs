//! `.ala` animation playback on spawned models, and turret aiming.
//!
//! # How EaW finds animations
//!
//! There is no table: an animation file is named after the model it belongs to,
//! `<MODEL>_<KIND>_<NN>.ALA` (`EMPIRE_X_WING_DEPLOY_00.ALA` for `EMPIRE_X_WING.ALO`). The kind is
//! one of a fixed vocabulary the game code asks for (`IDLE`, `MOVE`, `ATTACK`, `DIE`, `DEPLOY`,
//! `UNDEPLOY`, `LAND`, `TAKEOFF`, `BUILD`, `OPEN`, `CLOSE`, `TURNL_*`, ...) and `NN` numbers
//! variants of the same kind. [`AnimationIndex`] indexes every file once; [`AlamoAnimation`]
//! plays one kind with a speed, looping or once.
//!
//! Rigid animations replace the bone's local transform; skinned meshes follow their joints.

use std::collections::HashMap;
use std::sync::Arc;

use alamo_fs::FileSource;
use alamo_model::Animation;
use bevy::prelude::*;

use crate::model::{AlamoBillboard, AlamoBone, AlamoFs, AlamoModel, AlamoModelLoaded};

/// Plays an animation kind on the model of this entity.
///
/// ```ignore
/// commands.entity(ship).insert(AlamoAnimation::once("DEPLOY"));
/// // later, from game code:
/// anim.play("UNDEPLOY", false);
/// ```
#[derive(Component, Debug, Clone)]
pub struct AlamoAnimation {
    /// Animation kind such as `IDLE`, `DEPLOY` (case-insensitive).
    pub kind: String,
    /// Which numbered variant (`..._00`); `None` uses the first one.
    pub variant: Option<u32>,
    /// Restart at the end.
    pub looping: bool,
    /// Playback speed multiplier (1 = file fps).
    pub speed: f32,
    /// Seconds since the start.
    pub time: f32,
    /// A non-looping animation reached its last frame (the last frame stays applied).
    pub finished: bool,
    /// Paused when false.
    pub playing: bool,
}

impl AlamoAnimation {
    /// A looping animation of `kind`.
    pub fn looped(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            variant: None,
            looping: true,
            speed: 1.0,
            time: 0.0,
            finished: false,
            playing: true,
        }
    }

    /// An animation of `kind` that plays once and holds its last frame.
    pub fn once(kind: impl Into<String>) -> Self {
        Self {
            looping: false,
            ..Self::looped(kind)
        }
    }

    /// The default idle loop.
    pub fn idle() -> Self {
        Self::looped("IDLE")
    }

    /// Switches to another animation kind and restarts it.
    pub fn play(&mut self, kind: impl Into<String>, looping: bool) {
        self.kind = kind.into();
        self.looping = looping;
        self.time = 0.0;
        self.finished = false;
        self.playing = true;
    }

    /// Sets the playback speed.
    pub fn with_speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }
}

/// One animation file belonging to a model.
#[derive(Debug, Clone)]
pub struct AnimEntry {
    /// Kind, e.g. `IDLE` or `TURNL_QUARTER`.
    pub kind: String,
    /// Variant number (`_00` is 0).
    pub variant: u32,
    /// Full game path.
    pub path: String,
}

/// Index of all `.ala` files by model name.
#[derive(Resource, Default)]
pub struct AnimationIndex {
    /// Sorted upper-case file stems with their full paths.
    files: Vec<(String, String)>,
}

impl AnimationIndex {
    /// Builds the index from the file system.
    pub fn build(fs: &alamo_fs::GameFs) -> Self {
        let mut files: Vec<(String, String)> = fs
            .list("Art/Models")
            .into_iter()
            .filter(|f| f.ends_with(".ALA"))
            .map(|f| {
                let stem = f
                    .rsplit('/')
                    .next()
                    .unwrap_or(&f)
                    .trim_end_matches(".ALA")
                    .to_string();
                (stem, f)
            })
            .collect();
        files.sort();
        Self { files }
    }

    /// All animations of the model with file stem `model` (any case, no extension).
    pub fn entries(&self, model: &str) -> Vec<AnimEntry> {
        let prefix = format!("{}_", model.to_ascii_uppercase());
        let start = self
            .files
            .partition_point(|(s, _)| s.as_str() < prefix.as_str());
        let mut out = Vec::new();
        for (stem, path) in &self.files[start..] {
            if !stem.starts_with(&prefix) {
                break;
            }
            let rest = &stem[prefix.len()..];
            let digits = rest.chars().rev().take_while(char::is_ascii_digit).count();
            let (kind, variant) = if digits > 0 && rest.len() > digits {
                let k = rest[..rest.len() - digits].trim_end_matches('_');
                (k, rest[rest.len() - digits..].parse().unwrap_or(0))
            } else {
                (rest, 0)
            };
            out.push(AnimEntry {
                kind: kind.to_string(),
                variant,
                path: path.clone(),
            });
        }
        out
    }

    /// Path of the animation `kind` (variant `variant` or the lowest) of `model`.
    pub fn find(&self, model: &str, kind: &str, variant: Option<u32>) -> Option<String> {
        let mut e: Vec<_> = self
            .entries(model)
            .into_iter()
            .filter(|e| e.kind.eq_ignore_ascii_case(kind))
            .collect();
        e.sort_by_key(|e| e.variant);
        match variant {
            Some(v) => e.into_iter().find(|x| x.variant == v),
            None => e.into_iter().next(),
        }
        .map(|e| e.path)
    }
}

/// File stem of a model path (`Art/Models/X.ALO` -> `X`).
pub fn model_stem(path: &str) -> String {
    let f = path.rsplit(['/', '\\']).next().unwrap_or(path);
    f.rsplit_once('.').map_or(f, |s| s.0).to_string()
}

/// Parsed animation cache.
#[derive(Resource, Default)]
pub struct AnimationCache {
    clips: HashMap<String, Option<Arc<Animation>>>,
}

impl AnimationCache {
    /// Length in seconds of the clip at `path` (`None` if it cannot be read).
    pub fn duration(&mut self, fs: &AlamoFs, path: &str) -> Option<f32> {
        self.get(fs, path).map(|a| a.duration())
    }

    /// The parsed clip at `path` (`None` if it cannot be read), for game code that needs more than the pose, such as which bones a
    /// clip shows ([`alamo_model::ala::BoneTrack::visibility`]).
    pub fn clip(&mut self, fs: &AlamoFs, path: &str) -> Option<Arc<Animation>> {
        self.get(fs, path)
    }

    /// Registers a clip parsed off the main thread (see [`crate::ModelPreloader`]); `None` = unreadable.
    pub(crate) fn insert(&mut self, path: &str, clip: Option<Animation>) {
        self.clips.entry(path.to_ascii_lowercase()).or_insert_with(|| clip.map(Arc::new));
    }

    fn get(&mut self, fs: &AlamoFs, path: &str) -> Option<Arc<Animation>> {
        self.clips
            .entry(path.to_ascii_lowercase())
            .or_insert_with(|| {
                let t0 = std::time::Instant::now();
                let data = fs.0.read(path).ok()?;
                match Animation::parse(&data) {
                    Ok(a) => {
                        crate::frame_trace::note(|| format!("animation {path} parsed on the main thread {:.1?}", t0.elapsed()));
                        Some(Arc::new(a))
                    }
                    Err(e) => {
                        warn!("animation {path}: {e}");
                        None
                    }
                }
            })
            .clone()
    }
}

/// Animation options.
#[derive(Resource, Debug, Clone)]
pub struct AnimationSettings {
    /// Start the `IDLE` animation of every model that has one.
    pub auto_idle: bool,
}

impl Default for AnimationSettings {
    fn default() -> Self {
        Self { auto_idle: true }
    }
}

type NewlyLoaded = (Added<AlamoModelLoaded>, Without<AlamoAnimation>);

/// Inserts [`AlamoAnimation::idle`] on freshly loaded models that have an idle animation.
pub fn auto_idle(
    mut commands: Commands,
    settings: Res<AnimationSettings>,
    index: Res<AnimationIndex>,
    new: Query<(Entity, &AlamoModel), NewlyLoaded>,
) {
    if !settings.auto_idle {
        return;
    }
    for (e, m) in &new {
        if index.find(&model_stem(&m.path), "IDLE", None).is_some() {
            commands.entity(e).insert(AlamoAnimation::idle());
        }
    }
}

/// Samples the active animations into bone transforms.
pub fn animate_models(
    time: Res<Time>,
    fs: Res<AlamoFs>,
    index: Res<AnimationIndex>,
    mut cache: ResMut<AnimationCache>,
    mut models: Query<(&AlamoModel, &AlamoModelLoaded, &mut AlamoAnimation)>,
    mut bones: Query<(&mut Transform, Option<&AlamoBillboard>), With<AlamoBone>>,
    mut visibility: Query<&mut Visibility>,
) {
    let dt = time.delta_secs();
    for (model, loaded, mut anim) in &mut models {
        let Some(path) = index.find(&model_stem(&model.path), &anim.kind, anim.variant) else {
            continue;
        };
        let Some(clip) = cache.get(&fs, &path) else {
            continue;
        };
        if anim.playing && !anim.finished {
            anim.time += dt * anim.speed;
        }
        let n = clip.frames as usize;
        let period = n.saturating_sub(1).max(1) as f32;
        let mut f = anim.time * clip.fps;
        if anim.looping {
            f = f.rem_euclid(period);
        } else if f >= period {
            f = period;
            anim.finished = true;
        }
        let i0 = (f.floor() as usize).min(n - 1);
        let i1 = (i0 + 1).min(n - 1);
        let a = f - i0 as f32;
        for bt in &clip.bones {
            let Some(&e) = loaded.bone_by_name.get(&bt.name.to_ascii_lowercase()) else {
                continue;
            };
            if let Ok((mut tf, billboard)) = bones.get_mut(e) {
                let (t0, t1) = (bt.translation.at(i0), bt.translation.at(i1));
                tf.translation = Vec3::from(t0).lerp(Vec3::from(t1), a);
                let (s0, s1) = (bt.scale.at(i0), bt.scale.at(i1));
                tf.scale = Vec3::from(s0).lerp(Vec3::from(s1), a);
                if billboard.is_none() {
                    let q0 = Quat::from_array(bt.rotation.at(i0)).normalize();
                    let q1 = Quat::from_array(bt.rotation.at(i1)).normalize();
                    tf.rotation = q0.slerp(q1, a);
                }
            }
            if let Some(v) = &bt.visibility
                && let Ok(mut vis) = visibility.get_mut(e)
            {
                *vis = if v[i0] {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
        }
    }
}

/// Aims the barrel bones of a turret model at a world position.
///
/// The yaw bone turns about its local Z axis (Alamo up), the pitch bone, a child of the yaw
/// bone, about its local Y axis; barrels point along +X in the rest pose. Both turn at no more
/// than `turn_rate` radians per second and stay inside the limits.
#[derive(Component, Debug, Clone)]
pub struct TurretAim {
    /// Bone that yaws (`Turret`).
    pub yaw_bone: String,
    /// Bone that pitches (`Barrel`).
    pub pitch_bone: String,
    /// World-space point to aim at.
    pub target: Vec3,
    /// Yaw limits in radians relative to the rest pose; `None` allows a full circle.
    pub yaw_limits: Option<(f32, f32)>,
    /// Pitch limits in radians (negative looks down).
    pub pitch_limits: (f32, f32),
    /// Maximum turn speed in radians per second.
    pub turn_rate: f32,
    /// Current yaw (radians from rest).
    pub yaw: f32,
    /// Current pitch (radians from rest).
    pub pitch: f32,
    /// Only active turrets move; [`TurretAim::idle`] turrets stay in their rest pose until
    /// `active` is set.
    pub active: bool,
}

impl TurretAim {
    /// A turret with the usual `Turret` / `Barrel` bones, a full yaw circle and pitch from -10
    /// to 85 degrees.
    pub fn new(target: Vec3) -> Self {
        Self {
            yaw_bone: "Turret".into(),
            pitch_bone: "Barrel".into(),
            target,
            yaw_limits: None,
            pitch_limits: (-0.17, 1.48),
            turn_rate: 1.2,
            yaw: 0.0,
            pitch: 0.0,
            active: true,
        }
    }

    /// An idle turret configured from its hardpoint: bones, yaw/pitch extents and speed.
    ///
    /// The yaw range is `Turret_Rotate_Extent_Degrees` to each side of the rest pose (360 = free),
    /// the pitch range goes from slightly below the horizontal up to `Turret_Elevate_Extent_Degrees`.
    pub fn from_hardpoint(def: &crate::xmldata::HardpointDef) -> Self {
        let mut aim = Self::idle();
        if let Some(b) = &def.turret_bone {
            aim.yaw_bone = b.trim().to_string();
        }
        if let Some(b) = &def.barrel_bone {
            aim.pitch_bone = b.trim().to_string();
        }
        if def.rotate_extent_deg < 359.0 {
            // The extent is the reach to each side of the rest pose, as the sim reads it (`battle-sim` fire arc): with half of it the barrel stopped
            // short of targets the turret was already firing at.
            let half = def.rotate_extent_deg.min(180.0).to_radians();
            aim.yaw_limits = Some((-half, half));
        }
        aim.pitch_limits = (-0.09, def.elevate_extent_deg.to_radians().clamp(0.02, 3.1));
        if def.rotate_speed > 0.0 {
            aim.turn_rate = def.rotate_speed;
        }
        aim
    }

    /// A turret that holds its rest pose until activated (set `active` and `target`).
    pub fn idle() -> Self {
        Self {
            active: false,
            ..Self::new(Vec3::ZERO)
        }
    }
}

/// Yaw and pitch (radians from the rest pose) that make a turret face `v`, a point in the yaw bone's rest frame relative to its pivot (barrels point along +X,
/// Z is up), clamped to the turret's limits. The pitch is measured after yawing.
pub fn desired_angles(v: Vec3, yaw_limits: Option<(f32, f32)>, pitch_limits: (f32, f32)) -> (f32, f32) {
    let yaw = v.y.atan2(v.x);
    let yaw = match yaw_limits {
        Some((lo, hi)) => yaw.clamp(lo, hi),
        None => yaw,
    };
    let pitch = v.z.atan2(v.x.hypot(v.y)).clamp(pitch_limits.0, pitch_limits.1);
    (yaw, pitch)
}

fn step_angle(current: f32, desired: f32, max: f32) -> f32 {
    let d = (desired - current + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI;
    current + d.clamp(-max, max)
}

/// Turns [`TurretAim`] bones towards their targets.
pub fn aim_turrets(
    time: Res<Time>,
    mut turrets: Query<(&mut TurretAim, &AlamoModelLoaded)>,
    mut bones: Query<(&mut Transform, &AlamoBone, &ChildOf)>,
    globals: Query<&GlobalTransform>,
) {
    let dt = time.delta_secs();
    for (mut aim, loaded) in &mut turrets {
        if !aim.active {
            continue;
        }
        let yaw_key = aim.yaw_bone.trim().to_ascii_lowercase();
        let pitch_key = aim.pitch_bone.trim().to_ascii_lowercase();
        let ye = loaded.bone_by_name.get(&yaw_key).copied().or_else(|| {
            ["turret", "b_turret", "root_turret", "turret_00", "turret_01"]
                .iter()
                .find_map(|k| loaded.bone_by_name.get(*k).copied())
        });
        let pe = loaded.bone_by_name.get(&pitch_key).copied().or_else(|| {
            ["barrel", "b_barrel", "root_barrel", "barrel_00", "barrel_01"]
                .iter()
                .find_map(|k| loaded.bone_by_name.get(*k).copied())
        });
        let Some(ye) = ye else {
            continue;
        };
        let Ok((_, yaw_bone, parent)) = bones.get(ye) else {
            continue;
        };
        let (rest_y, parent_entity) = (yaw_bone.rest, parent.parent());
        let Ok(parent_gt) = globals.get(parent_entity) else {
            continue;
        };
        let pivot = rest_y.translation;
        // Target in the yaw bone's rest frame.
        let local = parent_gt.affine().inverse().transform_point3(aim.target) - pivot;
        let v = rest_y.rotation.inverse() * local;
        let (desired_yaw, desired_pitch) = desired_angles(v, aim.yaw_limits, aim.pitch_limits);
        let step = aim.turn_rate * dt;
        aim.yaw = step_angle(aim.yaw, desired_yaw, step);
        aim.pitch += (desired_pitch - aim.pitch).clamp(-step, step);
        let (yaw, pitch) = (aim.yaw, aim.pitch);
        if let Ok((mut t, _, _)) = bones.get_mut(ye) {
            t.rotation = rest_y.rotation * Quat::from_rotation_z(yaw);
        }
        if let Some(pe) = pe {
            if let Ok((mut t, b, _)) = bones.get_mut(pe) {
                t.rotation = b.rest.rotation * Quat::from_rotation_y(-pitch);
            }
        }
    }
}

/// Registers animation resources and systems.
pub fn build(app: &mut App, fs: &Arc<alamo_fs::GameFs>) {
    app.insert_resource(AnimationIndex::build(fs))
        .init_resource::<AnimationCache>()
        .init_resource::<AnimationSettings>()
        .add_systems(
            PostUpdate,
            (auto_idle, animate_models, aim_turrets)
                .chain()
                .after(crate::model::spawn_models)
                .before(bevy::transform::TransformSystems::Propagate),
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turret_yaw_limit_is_the_extent_to_each_side() {
        let def = crate::xmldata::HardpointDef { rotate_extent_deg: 45.0, elevate_extent_deg: 60.0, ..Default::default() };
        let aim = TurretAim::from_hardpoint(&def);
        let (lo, hi) = aim.yaw_limits.unwrap();
        assert!((hi - 45f32.to_radians()).abs() < 1e-5 && (lo + hi).abs() < 1e-5);
        assert!(TurretAim::from_hardpoint(&crate::xmldata::HardpointDef { rotate_extent_deg: 360.0, ..Default::default() }).yaw_limits.is_none());
    }

    fn index() -> AnimationIndex {
        let mut files: Vec<(String, String)> = [
            "EMPIRE_X_WING_DEPLOY_00",
            "EMPIRE_X_WING_UNDEPLOY_00",
            "EMPIRE_X_WING_IDLE_01",
            "EMPIRE_X_WING_IDLE_00",
            "EMPIRE_X_WING_TURNL_QUARTER_00",
            "EMPIRE_X_WINGS_IDLE_00",
            "OTHER_IDLE_00",
        ]
        .iter()
        .map(|s| (s.to_string(), format!("Art/Models/{s}.ALA")))
        .collect();
        files.sort();
        AnimationIndex { files }
    }

    #[test]
    fn finds_kinds_and_variants() {
        let i = index();
        let e = i.entries("empire_x_wing");
        assert_eq!(e.len(), 5, "other models must not match: {e:?}");
        assert!(e.iter().any(|x| x.kind == "TURNL_QUARTER"));
        assert_eq!(
            i.find("EMPIRE_X_WING", "idle", None).unwrap(),
            "Art/Models/EMPIRE_X_WING_IDLE_00.ALA"
        );
        assert_eq!(
            i.find("EMPIRE_X_WING", "IDLE", Some(1)).unwrap(),
            "Art/Models/EMPIRE_X_WING_IDLE_01.ALA"
        );
        assert!(i.find("EMPIRE_X_WING", "DIE", None).is_none());
    }

    #[test]
    fn stems_and_angles() {
        assert_eq!(model_stem("Art/Models/X_Y.ALO"), "X_Y");
        assert_eq!(model_stem("X.alo"), "X");
        let a = step_angle(3.0, -3.0, 1.0);
        assert!(a > 3.0, "takes the short way around: {a}");
        assert_eq!(step_angle(0.0, 0.5, 1.0), 0.5);
    }
}

#[cfg(test)]
mod aim_tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

    const FREE: Option<(f32, f32)> = None;
    const PITCH: (f32, f32) = (-0.09, 1.48);

    #[test]
    fn faces_the_target_in_yaw_and_pitch() {
        let (y, p) = desired_angles(Vec3::new(10.0, 0.0, 0.0), FREE, PITCH);
        assert!(y.abs() < 1e-6 && p.abs() < 1e-6, "straight ahead");
        let (y, _) = desired_angles(Vec3::new(0.0, 5.0, 0.0), FREE, PITCH);
        assert!((y - FRAC_PI_2).abs() < 1e-6, "left");
        let (y, _) = desired_angles(Vec3::new(-3.0, 0.0, 0.0), FREE, PITCH);
        assert!((y.abs() - PI).abs() < 1e-6, "behind");
        let (_, p) = desired_angles(Vec3::new(4.0, 0.0, 4.0), FREE, PITCH);
        assert!((p - FRAC_PI_4).abs() < 1e-6, "45 degrees up");
        // Two different targets give two different aims.
        let a = desired_angles(Vec3::new(10.0, 10.0, 0.0), FREE, PITCH);
        let b = desired_angles(Vec3::new(10.0, -10.0, 3.0), FREE, PITCH);
        assert!(a.0 > 0.0 && b.0 < 0.0 && a != b);
    }

    #[test]
    fn limits_clamp_the_aim() {
        let limits = Some((-0.5, 0.5));
        let (y, _) = desired_angles(Vec3::new(0.0, 5.0, 0.0), limits, PITCH);
        assert!((y - 0.5).abs() < 1e-6, "a target to the side stops at the yaw limit");
        let (_, p) = desired_angles(Vec3::new(0.1, 0.0, 9.0), FREE, PITCH);
        assert!((p - 1.48).abs() < 1e-6, "pitch stops at the elevate limit");
        let (_, p) = desired_angles(Vec3::new(5.0, 0.0, -9.0), FREE, PITCH);
        assert!((p + 0.09).abs() < 1e-6, "and barely below the horizon");
    }

    #[test]
    fn turning_is_rate_limited_and_takes_the_short_way() {
        assert!((step_angle(0.0, 1.0, 0.2) - 0.2).abs() < 1e-6, "no more than the rate allows");
        assert!((step_angle(0.0, 0.1, 0.2) - 0.1).abs() < 1e-6, "arrives exactly");
        // From +170 degrees to -170 degrees: 20 degrees through 180, not 340 the other way.
        let from = 170f32.to_radians();
        let to = -170f32.to_radians();
        let next = step_angle(from, to, 0.1);
        assert!(next > from, "keeps turning the same way round: {next}");
    }
}
