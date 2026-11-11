//! Missiles and torpedoes as real models with their particle trails.
//!
//! The batched bolt renderer (`ProjectileVisual`) draws a projectile model as static geometry, which is right for laser bolts but loses what makes a
//! missile: the `pp_*` particle proxy on its model (smoke trail, engine glow, launch flash). A missile projectile whose XML names a model
//! (`Space_Model_Name`) is therefore drawn here as a full [`AlamoModel`], one entity per missile in flight, flying nose-first along its velocity
//! (so it turns as the sim's homing turns it), with its proxy particle systems switched on.
//!
//! The simulation publishes anonymous projectiles (position and velocity only), so the entities are matched to the snapshot projectiles from
//! frame to frame by distance ([`match_tracks`]).
//!
//! What a missile leaves behind comes from the same particle system ([`TrailLook`]): a concussion missile's smoke lives 2.25 s and is white,
//! a proton torpedo's glow lives 0.2 s and is pink. The line drawn behind each missile has that length and colour, so a torpedo is a short
//! pink streak and not a rocket on a smoke trail.

use alamo_render::{AlamoModel, AlamoModelLoaded, AlamoParticles, XmlDefs};
use bevy::gizmos::config::{GizmoConfigGroup, GizmoConfigStore};
use bevy::prelude::*;

use crate::convert::{sim_dir_to_bevy, sim_to_bevy};
use crate::{BattleEntity, BattleRuntime};

/// True if the projectile is drawn as a missile model: a missile or torpedo, or a rocket-category projectile whose model is a missile.
pub(crate) fn is_missile_model(category: &str, model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    match category {
        "missile" | "torpedo" => true,
        "rocket" => m.contains("missile") || m.contains("torpedo") || m.contains("rocket"),
        _ => false,
    }
}

/// True if projectile definition `def` is drawn by this module (so the bolt renderer skips it).
pub(crate) fn drawn_as_model(rt: &BattleRuntime, defs: &XmlDefs, def: u32) -> bool {
    let Some(d) = rt.catalog.projectiles.get(def as usize) else { return false };
    defs.projectile(&d.name).and_then(|x| x.model.as_deref()).is_some_and(|m| is_missile_model(&d.category, m))
}

/// Greedy one-to-one assignment of snapshot positions to tracked positions by distance (closest pairs first, none farther than `max_dist`).
/// Returns, for every entry of `snaps`, the index into `tracked` it continues, or `None` for a new missile.
pub(crate) fn match_tracks(tracked: &[Vec3], snaps: &[Vec3], max_dist: f32) -> Vec<Option<usize>> {
    let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
    for (s, sp) in snaps.iter().enumerate() {
        for (t, tp) in tracked.iter().enumerate() {
            let d = sp.distance(*tp);
            if d <= max_dist {
                pairs.push((d, s, t));
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = vec![None; snaps.len()];
    let mut taken = vec![false; tracked.len()];
    for (_, s, t) in pairs {
        if out[s].is_none() && !taken[t] {
            out[s] = Some(t);
            taken[t] = true;
        }
    }
    out
}

/// Rotation that points a missile model's nose (+Y in Bevy space, the model's +Z up axis) along `velocity`.
pub(crate) fn nose_rotation(velocity: Vec3) -> Quat {
    let dir = velocity.normalize_or_zero();
    if dir == Vec3::ZERO { Quat::IDENTITY } else { Quat::from_rotation_arc(Vec3::Y, dir) }
}

/// What a missile model leaves behind, read from the particle systems on its proxies (`pp_*`): the life of the longest-lived emitter's
/// particles in seconds and their colour at birth. Data; the line that shows it is the view's stand-in for those sprites, which are a few
/// units across and vanish at battle camera distances.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TrailLook {
    pub secs: f32,
    pub color: [f32; 3],
}

/// Trails shorter than this are not drawn (a system with only one-frame glows).
const TRAIL_MIN_SECONDS: f32 = 0.05;
/// A trail keeps about this many points over its length, at least `TRAIL_STEP_MIN` units apart (keeps the polyline short).
const TRAIL_POINTS: f32 = 12.0;
const TRAIL_STEP_MIN: f32 = 4.0;

/// The trail the emitters of a missile's particle systems describe, each given as (particle life in seconds, colour at birth): that of
/// the emitter whose particles live longest, the first of them in the file (the launch flashes that live as long come last).
pub(crate) fn trail_look(emitters: impl IntoIterator<Item = (f32, [f32; 3])>) -> Option<TrailLook> {
    let mut best: Option<TrailLook> = None;
    for (secs, color) in emitters {
        if secs >= TRAIL_MIN_SECONDS && best.is_none_or(|b| secs > b.secs) {
            best = Some(TrailLook { secs, color });
        }
    }
    best
}

/// Reads the trail of projectile model `model` from the game files: the particle systems its proxies name.
fn load_trail_look(fs: &dyn alamo_fs::FileSource, model: &str) -> Option<TrailLook> {
    let m = alamo_model::Model::parse(&fs.read(&alamo_render::model::model_path(model)).ok()?).ok()?;
    let systems: Vec<alamo_model::ParticleSystem> = m.proxies.iter().filter_map(|p| fs.read(&alamo_render::particles::particle_path(&p.name)).ok()).filter_map(|d| alamo_model::ParticleSystem::parse(&d).ok()).collect();
    trail_look(systems.iter().flat_map(|s| &s.emitters).map(|e| (e.lifetime, [e.color[0].start, e.color[1].start, e.color[2].start])))
}

/// Gizmo group of the smoke trails (thicker lines than the overlays).
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct MissileTrailGizmos;

/// The trail of every missile: a polyline of past positions with the battle time they were left at, in the look of its model's particles;
/// it fades out after the missile is gone.
#[derive(Resource, Default)]
pub struct MissileTrails {
    lines: std::collections::HashMap<u64, (TrailLook, std::collections::VecDeque<(Vec3, f64)>)>,
    next: u64,
    /// Trail per projectile model (lower case); `None`: the model has no particles that last.
    looks: std::collections::HashMap<String, Option<TrailLook>>,
}

/// Thick smoke lines.
fn configure_trail_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (cfg, _) = store.config_mut::<MissileTrailGizmos>();
    cfg.line.width = 2.5;
    cfg.depth_bias = -0.5;
}

/// Draws the trails in the colour of their particles, fading with age over the particles' life.
fn draw_trails(rt: Res<BattleRuntime>, mut trails: ResMut<MissileTrails>, mut gizmos: Gizmos<MissileTrailGizmos>) {
    let now = rt.clock;
    // A new battle restarts the clock: trails of the old one are gone.
    if trails.lines.values().any(|(_, p)| p.back().is_some_and(|b| b.1 > now + 1.0)) {
        trails.lines.clear();
    }
    trails.lines.retain(|_, (look, pts)| {
        while pts.front().is_some_and(|p| now - p.1 > f64::from(look.secs)) {
            pts.pop_front();
        }
        !pts.is_empty()
    });
    if std::env::var_os("SP_MISSILE_LOG").is_some() && now.fract() < 0.03 {
        info!("missile-log: {} trails, longest {} points, now {now:.1}", trails.lines.len(), trails.lines.values().map(|(_, p)| p.len()).max().unwrap_or(0));
    }
    for (look, pts) in trails.lines.values() {
        let [r, g, b] = look.color;
        for pair in pts.iter().zip(pts.iter().skip(1)) {
            let age = ((now - pair.1.1) / f64::from(look.secs)).clamp(0.0, 1.0) as f32;
            let a = (1.0 - age) * (1.0 - age) * 0.8;
            gizmos.line(pair.0.0, pair.1.0, Color::srgba(r, g, b, a));
        }
    }
}

struct Track {
    entity: Entity,
    def: u32,
    pos: Vec3,
    /// `Scale_Factor` of the projectile model; its particles are not shrunk by it.
    scale: f32,
    particles_on: bool,
    /// Key of this missile's trail in [`MissileTrails`], if its particles leave one.
    trail: Option<u64>,
    /// Distance between two points of the trail.
    trail_step: f32,
}

/// Missiles in flight.
#[derive(Resource, Default)]
pub struct MissileTracks {
    items: Vec<Track>,
}

/// A missile that hit or ran out: its trail fades for a moment before the entity goes.
#[derive(Component)]
struct Burnout(Timer);

/// Flies one model entity per missile and keeps its trail on.
fn update_missiles(
    mut commands: Commands,
    rt: Res<BattleRuntime>,
    defs: Option<Res<XmlDefs>>,
    fs: Option<Res<alamo_render::AlamoFs>>,
    mut tracks: ResMut<MissileTracks>,
    mut trails: ResMut<MissileTrails>,
    mut transforms: Query<&mut Transform>,
    loaded: Query<&AlamoModelLoaded>,
    mut particles: Query<&mut AlamoParticles>,
) {
    let (Some(defs), Some(cur)) = (defs, rt.cur.clone()) else { return };
    if std::env::var_os("SP_MISSILE_LOG").is_some() && rt.clock.fract() < 0.03 {
        let mut seen: Vec<String> = Vec::new();
        for p in &cur.projectiles {
            if let Some(d) = rt.catalog.projectiles.get(p.def as usize) {
                let s = format!("{} [{}] model {:?}", d.name, d.category, defs.projectile(&d.name).and_then(|x| x.model.clone()));
                if !seen.contains(&s) {
                    seen.push(s);
                }
            }
        }
        info!("missile-log t={:.1} projectile kinds: {:?}", rt.clock, seen);
    }
    let t = rt.alpha * rt.dt;
    // The missiles of this frame: snapshot index, definition, Bevy position and velocity.
    let mut now: Vec<(u32, Vec3, Vec3, String, f32)> = Vec::new();
    for p in &cur.projectiles {
        let Some(def) = rt.catalog.projectiles.get(p.def as usize) else { continue };
        let Some(xml) = defs.projectile(&def.name) else { continue };
        let Some(model) = xml.model.as_deref() else { continue };
        if !is_missile_model(&def.category, model) {
            continue;
        }
        now.push((p.def, sim_to_bevy(p.pos + p.vel * t), sim_dir_to_bevy(p.vel), model.to_owned(), xml.scale));
    }
    if std::env::var_os("SP_MISSILE_LOG").is_some() && rt.clock.fract() < 0.03 && !now.is_empty() {
        info!("missile-log t={:.1}: {} missiles in flight, first at {:?} heading {:?}", rt.clock, now.len(), now[0].1, now[0].2.normalize_or_zero());
    }
    let tracked: Vec<Vec3> = tracks.items.iter().map(|t| t.pos).collect();
    let speed = now.iter().map(|m| m.2.length()).fold(0.0f32, f32::max);
    let matched = match_tracks(&tracked, &now.iter().map(|m| m.1).collect::<Vec<_>>(), speed * rt.dt * 2.0 + 5.0);
    let mut used = vec![false; tracks.items.len()];
    for (m, hit) in now.iter().zip(&matched) {
        let rotation = nose_rotation(m.2);
        match hit.filter(|&i| tracks.items[i].def == m.0) {
            Some(i) => {
                used[i] = true;
                let track = &mut tracks.items[i];
                track.pos = m.1;
                if let Some((_, pts)) = track.trail.and_then(|k| trails.lines.get_mut(&k))
                    && pts.back().is_none_or(|p| p.0.distance(m.1) >= track.trail_step)
                {
                    pts.push_back((m.1, rt.clock));
                }
                if let Ok(mut tf) = transforms.get_mut(track.entity) {
                    tf.translation = m.1;
                    tf.rotation = rotation;
                }
                // The trail and engine glow sit on the model's proxies, which exist once the model has loaded.
                if !track.particles_on
                    && let Ok(l) = loaded.get(track.entity)
                {
                    let mut any = false;
                    for &proxy in &l.proxies {
                        if let Ok(mut ps) = particles.get_mut(proxy) {
                            ps.enabled = true;
                            ps.world_trail = true;
                            ps.size_mul = 1.0 / track.scale.max(1e-3);
                            any = true;
                        }
                    }
                    track.particles_on = any;
                    if any && std::env::var_os("SP_MISSILE_LOG").is_some() {
                        info!("missile-log: trail particles switched on for {:?} ({} proxies)", track.entity, l.proxies.len());
                    }
                }
            }
            None => {
                let entity = commands
                    .spawn((
                        BattleEntity,
                        Name::new("missile"),
                        AlamoModel::new(m.3.clone()),
                        Transform { translation: m.1, rotation, scale: Vec3::splat(m.4) },
                    ))
                    .id();
                let look = *trails.looks.entry(m.3.to_ascii_lowercase()).or_insert_with(|| fs.as_ref().and_then(|fs| load_trail_look(&*fs.0, &m.3)));
                let trail = look.map(|look| {
                    let key = trails.next;
                    trails.next += 1;
                    trails.lines.insert(key, (look, std::collections::VecDeque::from([(m.1, rt.clock)])));
                    key
                });
                let trail_step = look.map_or(0.0, |l| (m.2.length() * l.secs / TRAIL_POINTS).max(TRAIL_STEP_MIN));
                tracks.items.push(Track { entity, def: m.0, pos: m.1, scale: m.4, particles_on: false, trail, trail_step });
                used.push(true);
            }
        }
    }
    // Missiles that are gone: stop the emitters, let the smoke thin out, then remove.
    let mut keep = Vec::new();
    for (track, u) in tracks.items.drain(..).zip(used) {
        if u {
            keep.push(track);
            continue;
        }
        if let Ok(l) = loaded.get(track.entity) {
            for &proxy in &l.proxies {
                if let Ok(mut ps) = particles.get_mut(proxy) {
                    ps.enabled = false;
                }
            }
        }
        commands.entity(track.entity).insert(Burnout(Timer::from_seconds(2.5, TimerMode::Once)));
    }
    tracks.items = keep;
}

/// Removes burnt-out missiles once their smoke is gone; the model itself is hidden at once.
fn burnout(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Burnout, &mut Visibility)>) {
    for (e, mut b, mut vis) in &mut q {
        *vis = Visibility::Hidden;
        if b.0.tick(time.delta()).is_finished() {
            commands.entity(e).despawn();
        }
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<MissileTracks>().init_resource::<MissileTrails>().init_gizmo_group::<MissileTrailGizmos>().add_systems(Startup, configure_trail_gizmos).add_systems(Update, draw_trails.run_if(resource_exists::<BattleRuntime>)).add_systems(Update, (update_missiles, burnout).run_if(resource_exists::<BattleRuntime>));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torpedo_sphere_uses_the_proton_torpedo_model() {
        // In flight the view draws `Space_Model_Name` when the category is missile or torpedo. The Torpedo Sphere variant inherits the proton torpedo model; a concussion missile does not.
        let Some(install) = alamo_fs::GameInstall::detect() else { return };
        let Ok(fs) = alamo_fs::GameFs::for_mod(&install, Some("1770851727")) else { return };
        let defs = XmlDefs::load(&fs);
        let xml = |n: &str| defs.projectile(n).unwrap_or_else(|| panic!("{n}"));
        let ts = xml("Proj_Proton_Torpedo_TS");
        let proton = xml("Proj_Proton_Torpedo");
        let conc = xml("Proj_Concussion_Missile");
        let model = |p: &alamo_render::ProjectileDef| p.model.as_deref().unwrap_or("").trim().to_ascii_lowercase();
        assert_eq!(model(ts), model(proton), "the variant keeps the proton torpedo model");
        assert!(model(ts).contains("torpedo_proton"), "{}", model(ts));
        assert_ne!(model(ts), model(conc), "a concussion missile is a different model");
        assert!(is_missile_model(&ts.category.to_ascii_lowercase(), &model(ts)));
        let stage = xml("Proj_Proton_Torpedo_TS_Damage");
        assert_eq!(stage.object_detonation.as_deref().map(str::trim), Some("Damage_proton_torpedo"));
        assert!(defs.particle("Damage_proton_torpedo").is_some(), "the warhead's detonation is a particle");
    }

    #[test]
    fn a_proton_torpedo_leaves_a_short_pink_trail_and_a_concussion_missile_white_smoke() {
        // The data's particles: `pp_projectile_torpedo_proton` lives 0.2 s at most and is pink, `pp_concussion_missile_b` smokes white for
        // 2.25 s. Every missile used to get the same 2 s white line, so a Torpedo Sphere volley looked like a salvo of rockets.
        let Some(install) = alamo_fs::GameInstall::detect() else { return };
        let Ok(fs) = alamo_fs::GameFs::for_mod(&install, Some("1770851727")) else { return };
        let defs = XmlDefs::load(&fs);
        let look = |n: &str| load_trail_look(&fs, defs.projectile(n).unwrap_or_else(|| panic!("{n}")).model.as_deref().expect("model")).unwrap_or_else(|| panic!("{n}: no trail"));
        let (ts, proton, conc) = (look("Proj_Proton_Torpedo_TS"), look("Proj_Proton_Torpedo"), look("Proj_Concussion_Missile"));
        assert_eq!(ts, proton, "the Torpedo Sphere fires the proton torpedo's model");
        assert!((proton.secs - 0.2).abs() < 1e-3, "{proton:?}");
        assert!(proton.color[0] > 0.7 && proton.color[1] < 0.4 && proton.color[2] > 0.3 && proton.color[2] < 0.6, "pink: {proton:?}");
        assert!((conc.secs - 2.25).abs() < 1e-3, "{conc:?}");
        assert!(conc.color.iter().all(|&c| c > 0.9), "white: {conc:?}");
    }

    #[test]
    fn the_trail_is_the_longest_lived_emitter() {
        let (glow, smoke, flash) = ((0.1, [1.0, 0.0, 0.0]), (2.0, [0.0, 0.0, 0.5]), (0.01, [1.0; 3]));
        assert_eq!(trail_look([glow, smoke]), Some(TrailLook { secs: 2.0, color: [0.0, 0.0, 0.5] }));
        assert_eq!(trail_look([smoke, (2.0, [1.0; 3])]), Some(TrailLook { secs: 2.0, color: [0.0, 0.0, 0.5] }), "the first of two that live as long");
        assert_eq!(trail_look([flash]), None);
        assert_eq!(trail_look([]), None);
    }

    #[test]
    fn missiles_and_torpedoes_are_models_bolts_are_not() {
        assert!(is_missile_model("missile", "w_projectile_concussion_missile_b.alo"));
        assert!(is_missile_model("torpedo", "w_projectile_proton_torpedo_large_b.alo"));
        assert!(is_missile_model("rocket", "w_projectile_concussion_missile_b.alo"), "a trajectory missile is a rocket with a missile model");
        assert!(!is_missile_model("rocket", "w_turbolaser_g22.alo"), "rocket-category turbolaser barrages stay bolts");
        assert!(!is_missile_model("laser", "w_laser_r.alo"));
    }

    #[test]
    fn tracks_follow_the_closest_missile_and_new_ones_start_fresh() {
        let tracked = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(100.0, 0.0, 0.0)];
        // Both moved a little, one new missile appeared far away.
        let snaps = [Vec3::new(101.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(500.0, 0.0, 0.0)];
        assert_eq!(match_tracks(&tracked, &snaps, 10.0), vec![Some(1), Some(0), None]);
        // Two snapshots near one track: the closer one continues it, the other is new.
        let snaps = [Vec3::new(2.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)];
        assert_eq!(match_tracks(&tracked[..1], &snaps, 10.0), vec![None, Some(0)]);
        assert!(match_tracks(&[], &snaps, 10.0).iter().all(Option::is_none));
    }

    #[test]
    fn the_nose_points_along_the_velocity() {
        for v in [Vec3::X, Vec3::NEG_Z, Vec3::new(1.0, 2.0, -3.0), Vec3::NEG_Y] {
            let nose = nose_rotation(v) * Vec3::Y;
            assert!(nose.distance(v.normalize()) < 1e-5, "{v:?} -> {nose:?}");
        }
        assert_eq!(nose_rotation(Vec3::ZERO), Quat::IDENTITY);
    }
}
