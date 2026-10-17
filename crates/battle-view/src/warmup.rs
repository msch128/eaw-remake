//! Warm-up behind the loading screen: one instance of every unit type a side can field (reserve units called in later,
//! fighters launched from hangars, missiles) is spawned in front of the battle camera while the opaque loading screen
//! covers it, together with the arrival flash and marker. Their meshes, team-colour materials, turrets, proxy particle
//! systems and render pipelines are built then, so a reinforcement that drops out of hyperspace mid-battle only spawns
//! entities. The battle clock stands until the warm-up is done; the instances are removed before the battle shows.
//!
//! Opt-in through [`BattleViewSettings::warm_up`]: the instances are visible to the battle camera, so only a front end
//! that covers the view while it waits for [`BattleWarmup::done`] may switch it on.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use alamo_render::model::AlamoLoadFailed;
use alamo_render::{AlamoModel, AlamoModelLoaded};
use battle_sim::ShipId;
use bevy::prelude::*;
use bevy::render::render_resource::PipelineCache;
use bevy::render::{Render, RenderApp, RenderSystems};

use crate::camera::BattleCamera;
use crate::{BattleEntity, BattleRuntime, BattleViewSettings};

/// Longest the warm-up waits for render pipelines once every model is built (a pipeline that never compiles must not hold the battle).
const PIPELINE_WAIT: Duration = Duration::from_secs(3);
/// Longest the whole warm-up may take.
const MAX_WARMUP: Duration = Duration::from_secs(10);
/// Frames every model has to be built before the pipeline count is trusted (specialization runs in the render world a frame or two later).
const SETTLE_FRAMES: u32 = 4;

/// State of the warm-up of the running battle; inserted with [`BattleRuntime`].
#[derive(Resource, Default)]
pub struct BattleWarmup {
    started: Option<Instant>,
    entities: Vec<Entity>,
    /// Frames in a row with every model built.
    settled: u32,
    /// When every model was built.
    built_at: Option<Instant>,
    done: bool,
}

impl BattleWarmup {
    /// True once the warm-up instances are gone (or there was nothing to warm).
    pub fn done(&self) -> bool {
        self.done
    }
}

/// Render pipelines queued or compiling in the render world, published every frame.
#[derive(Resource, Clone, Default)]
struct PipelinesWaiting(Arc<AtomicUsize>);

/// Marks a warm-up instance.
#[derive(Component)]
struct WarmupInstance;

/// Models still waiting for their meshes (or for a failed load to be noted).
type Unbuilt = (With<AlamoModel>, Without<AlamoModelLoaded>, Without<AlamoLoadFailed>);

pub(crate) fn plugin(app: &mut App) {
    let waiting = PipelinesWaiting::default();
    app.insert_resource(waiting.clone()).add_systems(
        Update,
        run_warmup.after(crate::start_battle).run_if(resource_exists::<BattleRuntime>).run_if(resource_exists::<BattleWarmup>),
    );
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.insert_resource(waiting).add_systems(Render, publish_waiting.in_set(RenderSystems::Cleanup));
    }
}

fn publish_waiting(cache: Res<PipelineCache>, waiting: Res<PipelinesWaiting>) {
    waiting.0.store(cache.waiting_pipelines().count(), Ordering::Relaxed);
}

/// Every `(ship type, side)` the battle can show: the deployed and reserve groups of each side of the request, their
/// squadron members and the fighters their hangars launch.
fn unit_types(rt: &BattleRuntime) -> Vec<(ShipId, u8)> {
    let Some(request) = rt.request.as_ref() else { return Vec::new() };
    let roots: Vec<Vec<ShipId>> =
        request.sides.iter().map(|s| s.groups.iter().chain(&s.reserve).filter_map(|g| rt.catalog.ship_id(&g.name)).collect()).collect();
    reachable(&roots, |id| {
        let d = rt.catalog.ship(id);
        let launched = d.spawned_by_tech.iter().flat_map(|t| t.1.iter().chain(&t.2));
        d.members.iter().chain(&d.carried).chain(&d.reserve).chain(launched).map(|m| m.0).collect()
    })
}

/// Every type reachable from each side's `roots` through `children`, once per side.
fn reachable(roots: &[Vec<ShipId>], children: impl Fn(ShipId) -> Vec<ShipId>) -> Vec<(ShipId, u8)> {
    let mut out: Vec<(ShipId, u8)> = Vec::new();
    for (side, ids) in roots.iter().enumerate() {
        let side = side as u8;
        let mut stack = ids.clone();
        while let Some(id) = stack.pop() {
            if !out.contains(&(id, side)) {
                out.push((id, side));
                stack.extend(children(id));
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn run_warmup(
    mut commands: Commands,
    mut warmup: ResMut<BattleWarmup>,
    rt: Res<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    defs: Option<Res<alamo_render::XmlDefs>>,
    models: Option<Res<alamo_render::ModelCache>>,
    waiting: Res<PipelinesWaiting>,
    camera: Query<Entity, With<BattleCamera>>,
    unbuilt: Query<(), Unbuilt>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    fs: Option<Res<alamo_render::AlamoFs>>,
    mut images: ResMut<Assets<Image>>,
    mut beam_art: ResMut<crate::effects::BeamArt>,
) {
    if warmup.done {
        return;
    }
    let Some(started) = warmup.started else {
        warmup.started = Some(Instant::now());
        // Scenes without a galactic request (the menu backdrop, demos) bring nothing in later and have no loading screen.
        let camera = camera.single().ok().filter(|_| settings.warm_up && rt.request.is_some());
        let Some(camera) = camera else {
            warmup.done = true;
            return;
        };
        let mut spawned = Vec::new();
        for (id, side) in unit_types(&rt) {
            let def = rt.catalog.ship(id);
            let vis = rt.visuals.ship(id);
            let Some(model) = vis.model.clone() else { continue };
            // Far enough that the whole model is in view: three radii of its hull.
            let distance = (def.radius * vis.scale.max(1.0) * 3.0).clamp(20.0, 20_000.0);
            spawned.push(
                commands
                    .spawn((
                        Name::new(format!("warm-up {}", def.name)),
                        BattleEntity,
                        WarmupInstance,
                        crate::ships::ship_model(def, model, rt.visuals.team_color(side)),
                        Transform::from_xyz(0.0, 0.0, -distance).with_scale(Vec3::splat(vis.scale)),
                        ChildOf(camera),
                    ))
                    .id(),
            );
        }
        // Missiles in flight are drawn as their models (see `missiles.rs`); a few name a particle system instead, which is no model.
        if let Some(defs) = &defs {
            let mut seen = Vec::new();
            for p in &rt.catalog.projectiles {
                let Some(model) = defs.projectile(&p.name).and_then(|d| d.model.clone()) else { continue };
                if crate::missiles::is_missile_model(&p.category, &model) && models.as_ref().is_some_and(|c| c.contains(&alamo_render::model::model_path(&model))) && !seen.contains(&model) {
                    seen.push(model.clone());
                    spawned.push(commands.spawn((Name::new("warm-up missile"), BattleEntity, WarmupInstance, AlamoModel::new(model), Transform::from_xyz(0.0, 0.0, -60.0), ChildOf(camera))).id());
                }
            }
        }
        // The hyperspace arrival flash and the arrival marker of a placed wave (transparent, nothing shows).
        let flash = crate::ships::FlashArt::new(&mut meshes, &mut images);
        spawned.push(
            commands
                .spawn((
                    Name::new("warm-up flash"),
                    BattleEntity,
                    WarmupInstance,
                    Mesh3d(flash.mesh.clone()),
                    MeshMaterial3d(materials.add(crate::ships::flash_material(Color::srgba(0.85, 0.95, 1.0, 0.0), flash.glow.clone()))),
                    Transform::from_xyz(0.0, 0.0, -100.0).with_scale(Vec3::splat(10.0)),
                    ChildOf(camera),
                ))
                .id(),
        );
        spawned.push(
            commands
                .spawn((
                    Name::new("warm-up marker"),
                    BattleEntity,
                    WarmupInstance,
                    Mesh3d(meshes.add(Annulus::new(9.2, 10.0))),
                    MeshMaterial3d(materials.add(crate::tactical_ui::marker_material())),
                    Transform::from_xyz(0.0, 0.0, -100.0),
                    ChildOf(camera),
                ))
                .id(),
        );
        // Super laser beams drawn with the data's art (the Eclipse's): the texture is decoded and the beam's pipeline built now.
        if let Some(fs) = &fs {
            for (id, _) in unit_types(&rt) {
                let Some(name) = rt.visuals.ship(id).super_laser.as_ref().and_then(|l| l.beam_texture.clone()) else { continue };
                let Some(texture) = beam_art.get_or_load(&fs.0, &mut images, &name) else { continue };
                let at = Vec3::new(0.0, 0.0, -100.0);
                if let Some(beam) = crate::effects::spawn_textured_beam(&mut commands, &mut meshes, &mut materials, texture, at, at + Vec3::X * 10.0, 1.0, Color::NONE, 1, MAX_WARMUP.as_secs_f32()) {
                    commands.entity(beam).insert((WarmupInstance, ChildOf(camera)));
                    spawned.push(beam);
                }
            }
        }
        info!("battle warm-up: {} instances (unit types, missiles, arrival effects, beams)", spawned.len());
        warmup.entities = spawned;
        return;
    };
    if unbuilt.is_empty() {
        warmup.settled += 1;
        warmup.built_at.get_or_insert_with(Instant::now);
    } else {
        warmup.settled = 0;
        warmup.built_at = None;
    }
    let pipelines = waiting.0.load(Ordering::Relaxed);
    let compiled = warmup.settled >= SETTLE_FRAMES && (pipelines == 0 || warmup.built_at.is_some_and(|t| t.elapsed() > PIPELINE_WAIT));
    let timed_out = started.elapsed() > MAX_WARMUP;
    if compiled || timed_out {
        if timed_out && !compiled {
            warn!("battle warm-up: gave up after {:.1?} ({pipelines} render pipelines still compiling)", started.elapsed());
        }
        info!("battle warm-up: done in {:.2?} ({pipelines} render pipelines still compiling)", started.elapsed());
        for e in warmup.entities.drain(..) {
            commands.entity(e).try_despawn();
        }
        warmup.done = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_a_side_can_field_is_warmed_once_per_side() {
        // Side 0 fields carrier 1 (launches squadron 3 of fighter 2, which also points back at the carrier); side 1 fields fighter 2.
        let children = |id: ShipId| match id {
            1 => vec![3],
            3 => vec![2, 1],
            _ => vec![],
        };
        let mut got = reachable(&[vec![1], vec![2]], children);
        got.sort();
        assert_eq!(got, vec![(1, 0), (2, 0), (2, 1), (3, 0)]);
    }
}
