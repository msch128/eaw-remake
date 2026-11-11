//! Census of everything a space battle of the Remake can draw: every model and particle system that an object with a space model, its
//! hardpoints, its death clone, its projectiles and their effects name, read from the installed mod. Skipped when the mod is not installed.
//!
//! The battle view must never draw a stand-in for art it cannot resolve (a flat coloured mesh, an untextured square, a guessed blend
//! mode). The renderer leaves such a part out ([`part_draw`], [`emitter_draw`]); this census lists every part of the real data that is
//! left out for a gap, so that each one is a known gap of the data and a new one fails here.
use std::collections::{BTreeMap, BTreeSet};

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::Model;
use alamo_model::particle::{BlendMode, ParticleSystem};
use alamo_render::XmlDefs;
use alamo_render::model::{PartDraw, is_render_mesh, part_draw};
use alamo_render::particles::{EmitterDraw, emitter_draw, particle_path};
use alamo_render::texture::{image_from_bytes, read_texture_file};

fn remake_fs() -> Option<GameFs> {
    let install = GameInstall::detect().filter(|i| i.mod_data("1770851727").is_dir())?;
    GameFs::for_mod(&install, Some("1770851727")).ok()
}

fn model_path(name: &str) -> String {
    let name = name.trim();
    let name = if name.to_ascii_lowercase().ends_with(".alo") { name.to_string() } else { format!("{name}.alo") };
    if name.contains(['/', '\\']) { name } else { format!("Art/Models/{name}") }
}

/// What the census found.
#[derive(Default)]
struct Census {
    models: usize,
    systems: usize,
    emitters: usize,
    /// Visible parts that are drawn, and parts of meshes the data hides (built invisible; animations and shield hits show them).
    drawn_parts: usize,
    hidden_parts: usize,
    /// Sub-meshes left out by rule inside meshes that are drawn: collision and shadow geometry.
    helper_geometry: usize,
    /// Models and particle files that are named but missing or unreadable: nothing of them is drawn.
    missing_files: BTreeSet<String>,
    /// Particle object names that are no `<Particle>` object: nothing is played.
    missing_particle_objects: BTreeSet<String>,
    /// Visible parts whose named base texture cannot be loaded, `model | mesh #sub | shader | texture`: not drawn.
    parts_without_texture: BTreeSet<String>,
    /// Visible surfaces that name no base texture, `model | mesh #sub | shader`: not drawn.
    parts_without_texture_name: BTreeSet<String>,
    /// Visible parts with a shader this renderer does not know, `model | mesh #sub | shader`: not drawn.
    parts_with_unknown_shader: BTreeSet<String>,
    /// Sprite emitters whose texture cannot be loaded, `file | emitter | texture`: their batch is not drawn.
    emitters_without_texture: BTreeSet<String>,
    /// Emitters of a type without a blend mode here or without a texture name, `file | emitter | type`: not drawn.
    emitters_without_mode: BTreeSet<String>,
    /// Emitters by what is done with them.
    emitter_kinds: BTreeMap<String, usize>,
}

fn census(fs: &GameFs) -> Census {
    let defs = XmlDefs::load(fs);
    let mut out = Census::default();
    let mut models: BTreeSet<String> = BTreeSet::new();
    let mut objects: BTreeSet<String> = BTreeSet::new();
    let mut add_model = |m: &Option<String>| {
        if let Some(m) = m.as_deref().filter(|m| !m.trim().is_empty()) {
            models.insert(model_path(m).to_ascii_lowercase());
        }
    };
    // Every object with a space model (units, star bases, structures, death clones, break-off props), its hardpoints and its effects.
    // The death effects the battle view plays for a unit whose data names none (`battle_view::visuals::ship_death_effect`) and the
    // fallback explosions it preloads (`battle_view` `lib.rs`).
    objects.extend(
        [
            "Small_Explosion_Space_Fighter",
            "Medium_Explosion_Space",
            "Large_Explosion_Space",
            "Huge_Explosion_Space",
            "Large_Explosion_Space_Empire_pd_medium_3_150",
            "Large_Explosion_Space_Empire_pd_medium_3_300",
            "Large_Explosion_Space_Empire_pd_medium_3_750",
            "Large_Explosion_Space_Empire_pd_medium_3_1250",
        ]
        .map(str::to_ascii_lowercase),
    );
    for u in defs.units.values().filter(|u| u.model.is_some()) {
        add_model(&u.model);
        objects.extend(u.death_explosions.iter().map(|n| n.to_ascii_lowercase()));
        // Break-off fires, spin-away explosions, asteroid hits.
        objects.extend(u.other_particles.iter().map(|n| n.to_ascii_lowercase()));
        for h in u.hardpoints.iter().filter_map(|h| defs.hardpoint(h)) {
            add_model(&h.model);
            objects.extend(h.death_particles.iter().map(|n| n.to_ascii_lowercase()));
            // `Damage_Particles` is a particle object or a bone of the hull: only the objects count.
            objects.extend(h.damage_particles.iter().filter(|n| defs.particle(n).is_some()).map(|n| n.to_ascii_lowercase()));
        }
    }
    for p in defs.projectiles.values() {
        add_model(&p.model);
        // A detonation that names another projectile is its second-stage warhead, counted with the projectiles (its own effects).
        for n in [&p.object_detonation, &p.armor_detonation, &p.lifetime_detonation, &p.shield_absorb].into_iter().flatten() {
            if defs.projectile(n).is_none() {
                objects.insert(n.to_ascii_lowercase());
            }
        }
    }
    // Particle files: those of the particle objects, and those the models carry on proxies of their name.
    let mut systems: BTreeSet<String> = BTreeSet::new();
    for o in &objects {
        match defs.particle(o) {
            Some(p) => {
                systems.insert(particle_path(&p.model).to_ascii_lowercase());
            }
            None => {
                out.missing_particle_objects.insert(o.clone());
            }
        }
    }
    // A texture counts when the loader can make an image of it (found under one of the names it tries, and decodable).
    let mut texture_ok: BTreeMap<String, bool> = BTreeMap::new();
    let mut has_texture =
        |name: &str| *texture_ok.entry(name.to_ascii_lowercase()).or_insert_with(|| read_texture_file(fs, name).is_some_and(|d| image_from_bytes(&d).is_ok()));
    for path in &models {
        let Some(model) = fs.read(path).ok().and_then(|d| Model::parse(&d).ok()) else {
            // A particle file named as a model (projectiles do that) is a particle system.
            if fs.read(path).ok().is_some_and(|d| ParticleSystem::parse(&d).is_ok()) {
                systems.insert(path.clone());
            } else {
                out.missing_files.insert(path.clone());
            }
            continue;
        };
        out.models += 1;
        for p in &model.proxies {
            let file = particle_path(&p.name);
            if fs.exists(&file) {
                systems.insert(file.to_ascii_lowercase());
            }
        }
        for mesh in model.meshes.iter().filter(|m| is_render_mesh(m)) {
            for (si, sub) in mesh.sub_meshes.iter().enumerate() {
                if sub.vertices.is_empty() || sub.indices.is_empty() {
                    continue;
                }
                let base = sub.material.texture("BaseTexture").filter(|t| !t.trim().is_empty());
                let resolved = base.is_some_and(&mut has_texture);
                let draw = part_draw(&sub.material, resolved);
                if mesh.hidden {
                    out.hidden_parts += 1;
                    continue;
                }
                let at = format!("{path} | {} #{si} | {}", mesh.name, sub.material.shader.to_ascii_lowercase());
                match draw {
                    PartDraw::Draw(_) => out.drawn_parts += 1,
                    PartDraw::Nothing("base texture not found") => {
                        out.parts_without_texture.insert(format!("{at} | {}", base.unwrap_or_default()));
                    }
                    PartDraw::Nothing("no base texture named") => {
                        out.parts_without_texture_name.insert(at);
                    }
                    PartDraw::Nothing("unknown shader") => {
                        out.parts_with_unknown_shader.insert(at);
                    }
                    PartDraw::Nothing(_) => out.helper_geometry += 1,
                }
            }
        }
    }
    for path in &systems {
        let Some(system) = fs.read(path).ok().and_then(|d| ParticleSystem::parse(&d).ok()) else {
            out.missing_files.insert(path.clone());
            continue;
        };
        out.systems += 1;
        for e in &system.emitters {
            out.emitters += 1;
            let kind = match emitter_draw(e) {
                EmitterDraw::Nothing(why) => {
                    if matches!(e.blend, BlendMode::Other(_)) || e.texture.trim().is_empty() {
                        out.emitters_without_mode.insert(format!("{path} | {} | {:?}", e.name, e.blend));
                    }
                    format!("nothing: {why}")
                }
                EmitterDraw::Sprites => {
                    if !has_texture(&e.texture) {
                        out.emitters_without_texture.insert(format!("{path} | {} | {}", e.name, e.texture));
                    }
                    format!("sprites: {:?}", e.blend)
                }
            };
            *out.emitter_kinds.entry(kind).or_default() += 1;
        }
    }
    out
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn every_effect_and_model_of_a_space_battle_resolves_or_is_left_out() {
    let Some(fs) = remake_fs() else { return };
    let c = census(&fs);
    println!(
        "{} models ({} drawn parts, {} parts of hidden meshes, {} collision or shadow sub-meshes left out), {} particle systems, {} emitters",
        c.models, c.drawn_parts, c.hidden_parts, c.helper_geometry, c.systems, c.emitters
    );
    println!("emitters: {:#?}", c.emitter_kinds);
    println!("missing files ({}): {:#?}", c.missing_files.len(), c.missing_files);
    // The census covers the battle art of the mod, not a handful of files.
    assert!(c.models >= 2000 && c.systems >= 650 && c.emitters >= 4000 && c.drawn_parts >= 8000, "{} {} {} {}", c.models, c.systems, c.emitters, c.drawn_parts);

    // Particles. Every emitter has a blend mode this renderer draws (additive, alpha, light corona, frames) or is one of the kinds that
    // draw no sprite (heat haze, light volumes, type 11), and every sprite emitter's texture loads.
    assert_eq!(c.emitters_without_mode, set(&[]), "emitters of a type without a blend mode");
    assert_eq!(c.emitters_without_texture, set(&[]), "sprite emitters whose texture does not load");

    // Models. No visible part uses a shader this renderer does not know.
    assert_eq!(c.parts_with_unknown_shader, set(&[]), "parts with an unknown shader");
    // Two parts name a texture the mod does not ship; they are left out (they were drawn mid grey).
    assert_eq!(
        c.parts_without_texture,
        set(&[
            "art/models/empire_dominator_star_destroyer_deathclone.alo | Ion_Quad_Base #0 | meshbumpcolorize.fx | Empire_Dominator_Star_Destroyer_Super_Structure.dds",
            "art/models/neutral_dauntless_cruiser_d.alo | HullS_TOP #0 | rskinalpha.fx | Destruction_for_DCs.dds",
        ]),
        "parts whose base texture does not load"
    );
    // Twelve visible surfaces name no base texture: exporter helpers (unit boxes and zero-size meshes with `alDefault.fx`, the 17 unit
    // `downhandle` box of the asteroid mine) and one hull plate of the Liberator (`MeshGloss.fx` without a texture; what the original
    // shows there is not known). They are left out (they were drawn in a plain lit white).
    assert_eq!(
        c.parts_without_texture_name,
        set(&[
            "art/models/empire_imperial_sd_1_refit.alo | Empire_ISD_Bridge2 #0 | aldefault.fx",
            "art/models/empire_imperial_sd_2.alo | Empire_ISD_Bridge2 #0 | aldefault.fx",
            "art/models/empire_imperial_sd_2_errant_venture.alo | Empire_ISD_Bridge2 #0 | aldefault.fx",
            "art/models/empire_imperial_sd_2_voca.alo | Empire_ISD_Bridge2 #0 | aldefault.fx",
            "art/models/empire_tartan_turret.alo | Vertical #0 | aldefault.fx",
            "art/models/empire_tie_defender_gold.alo | Twin Ion Engines #0 | aldefault.fx",
            "art/models/empire_tie_defender_red.alo | Twin Ion Engines #0 | aldefault.fx",
            "art/models/nb_asteroidmining.alo | downhandle #0 | aldefault.fx",
            "art/models/nb_asteroidmining_full.alo | downhandle #0 | aldefault.fx",
            "art/models/nv_black_sun_fighter.alo | engine particles #0 | aldefault.fx",
            "art/models/rebel_liberator.alo | objHangar_Exterior #0 | meshgloss.fx",
            "art/models/uv_vengeance.alo | engine2 #0 | aldefault.fx",
        ]),
        "visible surfaces without a base texture name"
    );

    // Names without a file or object behind them draw nothing. Effect names of hardpoints and projectiles that are neither a `<Particle>`
    // nor a `<Projectile>` (a warhead):
    assert_eq!(
        c.missing_particle_objects,
        set(&["hardpoint_explosion_space", "proj_construction_pod_detonation"]),
        "effect names that are no particle object"
    );
    // and 80 model names have no file (cinematic sets, `turret_space_*`, `w_stars_newgalaxymodel_*`, ...).
    assert_eq!(c.missing_files.len(), 80, "named models and particle files without a file: {:#?}", c.missing_files);
}
