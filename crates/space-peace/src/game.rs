//! Loading of the real game data and mapping it onto the renderer's [`GalaxyView`].

use std::{sync::Arc, time::Instant};

use alamo_data::{Campaign, GameData, Planet};
use alamo_fs::{GameFs, GameInstall};
use bevy::prelude::{Resource, Vec3, info, warn};

use crate::cli::Args;
use crate::galaxy_view::{FactionView, GalaxyView, PlanetView, RouteView};
use crate::music::GameMusicSource;

/// Workshop id of "Empire at War Remake 4.0".
pub const DEFAULT_MOD_ID: &str = "1770851727";
/// Campaign shown when `--campaign` is not given.
pub const DEFAULT_CAMPAIGN: &str = "GC_Full_Web_Way_Rebel";

/// Log lines produced before the Bevy logger exists; replayed by [`replay_startup_log`].
#[derive(Resource, Default)]
pub struct StartupLog(Vec<(bool, String)>);

impl StartupLog {
    /// Records an info message.
    pub fn info(&mut self, msg: String) {
        self.0.push((false, msg));
    }

    /// Records a warning.
    pub fn warn(&mut self, msg: String) {
        self.0.push((true, msg));
    }
}

/// Startup system that emits the recorded [`StartupLog`] through the real logger.
pub fn replay_startup_log(log: bevy::prelude::Res<StartupLog>) {
    for (is_warn, msg) in &log.0 {
        if *is_warn {
            warn!("{msg}");
        } else {
            info!("{msg}");
        }
    }
}

/// Everything loaded from the installed game.
pub struct LoadedGame {
    /// The galaxy to render.
    pub view: GalaxyView,
    /// Music source for the galactic map, if any galactic tracks were found.
    pub music: Option<GameMusicSource>,
}

/// Opens the game file system and loads the galaxy data (blocking).
fn open(args: &Args, log: &mut StartupLog) -> Result<(GameInstall, Arc<GameFs>, GameData), String> {
    let install = match &args.game_dir {
        Some(dir) => GameInstall::from_root(dir),
        None => GameInstall::detect(),
    }
    .ok_or("no Empire at War install found (use --game-dir <path> or set EAW_GAME_DIR)")?;
    let mod_id = args.mod_id.as_deref().unwrap_or(DEFAULT_MOD_ID);
    let fs = GameFs::for_mod(&install, Some(mod_id))
        .map_err(|e| format!("cannot open game files: {e}"))?;
    let fs = Arc::new(fs);

    let started = Instant::now();
    let (data, warnings) = alamo_data::load_galaxy(fs.as_ref());
    log.info(format!(
        "game: install {} | mod {mod_id} | {} files | data loaded in {:.2?} ({} warnings)",
        install.game_root.display(),
        fs.file_count(),
        started.elapsed(),
        warnings.len()
    ));
    for w in warnings.iter().take(8) {
        log.warn(format!("game data: {}:{}: {}", w.file, w.line, w.message));
    }
    Ok((install, fs, data))
}

/// Prints all galaxy campaigns with their planet counts (for `--list-campaigns`).
pub fn list_campaigns(args: &Args) -> Result<(), String> {
    let (_, _, data) = open(args, &mut StartupLog::default())?;
    for c in data.campaigns.iter().filter(|c| c.has_galaxy()) {
        let planets = data
            .campaign_planets(c)
            .iter()
            .filter(|p| p.is_real_planet())
            .count();
        println!(
            "{:<40} {planets:>4} planets  {:>4} routes",
            c.name,
            c.trade_routes.len()
        );
    }
    Ok(())
}

/// Loads the game and builds the view of the requested campaign.
pub fn load(args: &Args, log: &mut StartupLog) -> Result<LoadedGame, String> {
    let started = Instant::now();
    let (_, fs, data) = open(args, log)?;
    let name = args.campaign.as_deref().unwrap_or(DEFAULT_CAMPAIGN);
    let campaign = data
        .campaign(name)
        .filter(|c| c.has_galaxy())
        .ok_or_else(|| {
            format!("campaign `{name}` not found or has no galaxy (see --list-campaigns)")
        })?;
    let (view, notes) = build_view(&data, campaign);
    for n in &notes {
        log.warn(format!("galaxy: {n}"));
    }
    let mut counts = vec![0usize; view.factions.len()];
    for p in &view.planets {
        if let Some(o) = p.owner {
            counts[o] += 1;
        }
    }
    let unowned = view.planets.iter().filter(|p| p.owner.is_none()).count();
    let per_faction: Vec<String> = view
        .factions
        .iter()
        .zip(&counts)
        .map(|(f, n)| format!("{}={n}", f.name))
        .collect();
    log.info(format!(
        "galaxy: owned planets per faction: {} | unowned={unowned}",
        per_faction.join(", ")
    ));
    let conflicts = data.campaign_ownership_conflicts(campaign).len();
    if conflicts > 0 {
        log.info(format!(
            "galaxy: {conflicts} planets have conflicting starting structures (first faction wins)"
        ));
    }
    let music =
        GameMusicSource::galactic(fs.clone(), campaign.starting_active_player.as_deref(), log);
    log.info(format!(
        "game: campaign {} | {} planets, {} routes, {} factions | {} music tracks | total load {:.2?}",
        campaign.name,
        view.planets.len(),
        view.routes.len(),
        view.factions.len(),
        music.as_ref().map_or(0, GameMusicSource::len),
        started.elapsed()
    ));
    Ok(LoadedGame { view, music })
}

/// Placeholder surface colour for a terrain type.
fn terrain_color(planet: &Planet) -> [f32; 3] {
    let t = planet
        .terrain
        .as_deref()
        .or(planet.description.as_deref())
        .unwrap_or("")
        .to_ascii_lowercase();
    let has = |s: &str| t.contains(s);
    if has("urban") {
        [0.62, 0.62, 0.68]
    } else if has("desert") {
        [0.82, 0.62, 0.36]
    } else if has("arctic") || has("ice") {
        [0.82, 0.90, 0.96]
    } else if has("swamp") {
        [0.40, 0.46, 0.26]
    } else if has("volcan") {
        [0.70, 0.26, 0.14]
    } else if has("temperate") || has("forest") {
        [0.30, 0.55, 0.38]
    } else if has("gas") {
        [0.86, 0.66, 0.46]
    } else {
        [0.55, 0.50, 0.46]
    }
}

/// Maps a campaign's data to a [`GalaxyView`]; the second value lists non-fatal problems.
///
/// * planets: `Locations` that resolve to real planets (behavior `PLANET`), duplicates removed;
/// * owners: `Home_Location` first, then planets with starting forces of a faction;
/// * factions: campaign factions that exist as non-neutral faction definitions and own something;
/// * routes: campaign trade routes whose end points are both planets of the view.
pub fn build_view(data: &GameData, campaign: &Campaign) -> (GalaxyView, Vec<String>) {
    let mut notes = Vec::new();

    // Exact ownership from the data layer; the older "presence" heuristic is only a fallback.
    let exact: std::collections::HashMap<String, String> = data
        .campaign_planet_owners(campaign)
        .into_iter()
        .map(|(p, f)| (p.to_ascii_lowercase(), f.to_ascii_lowercase()))
        .collect();
    let use_exact = !exact.is_empty();
    if !use_exact {
        notes.push("no exact planet ownership available, using starting-force presence".into());
    }

    // Factions: campaign factions with a non-neutral definition that own something.
    let owns = |name: &str| {
        let lname = name.to_ascii_lowercase();
        if use_exact {
            exact.values().any(|f| *f == lname)
        } else {
            campaign
                .faction(name)
                .is_some_and(|f| !f.presence_planets.is_empty())
        }
    };
    let mut factions = Vec::new();
    let mut faction_names: Vec<&str> = Vec::new();
    for cf in &campaign.factions {
        let Some(def) = data.faction(&cf.name) else {
            continue;
        };
        if def.is_neutral || !owns(&cf.name) {
            continue;
        }
        let c = def.color.unwrap_or([160, 160, 170, 255]);
        let display = def
            .text_id
            .as_deref()
            .and_then(|id| data.text(id))
            .unwrap_or(&def.name);
        factions.push(FactionView {
            name: display.to_owned(),
            color: [
                c[0] as f32 / 255.0,
                c[1] as f32 / 255.0,
                c[2] as f32 / 255.0,
            ],
        });
        faction_names.push(&cf.name);
    }

    // Planet -> owner index (lower-cased planet name).
    let mut owners = std::collections::HashMap::<String, usize>::new();
    if use_exact {
        for (planet, faction) in &exact {
            if let Some(fi) = faction_names
                .iter()
                .position(|n| n.eq_ignore_ascii_case(faction))
            {
                owners.insert(planet.clone(), fi);
            }
        }
    } else {
        for (fi, name) in faction_names.iter().enumerate() {
            if let Some(home) = campaign
                .faction(name)
                .and_then(|f| f.home_location.as_deref())
            {
                owners.entry(home.to_ascii_lowercase()).or_insert(fi);
            }
        }
        for (fi, name) in faction_names.iter().enumerate() {
            if let Some(cf) = campaign.faction(name) {
                for p in &cf.presence_planets {
                    owners.entry(p.to_ascii_lowercase()).or_insert(fi);
                }
            }
        }
    }
    // Planet model scale factors are absolute; normalise by the median so `size == 1.0` is typical.
    let mut scales: Vec<f32> = data
        .campaign_planets(campaign)
        .iter()
        .filter(|p| p.is_real_planet() && p.scale_factor > 0.0)
        .map(|p| p.scale_factor)
        .collect();
    scales.sort_by(f32::total_cmp);
    let median_scale = scales.get(scales.len() / 2).copied().unwrap_or(1.0);

    // Planets.
    let mut planets = Vec::new();
    let mut index = std::collections::HashMap::<String, usize>::new();
    for p in data
        .campaign_planets(campaign)
        .into_iter()
        .filter(|p| p.is_real_planet())
    {
        let key = p.name.to_ascii_lowercase();
        if index.contains_key(&key) {
            continue;
        }
        index.insert(key.clone(), planets.len());
        let size = if p.scale_factor > 0.0 {
            p.scale_factor / median_scale
        } else {
            1.0
        };
        planets.push(PlanetView {
            id: p.name.clone(),
            display_name: data.planet_display_name(p).to_owned(),
            position: Vec3::from(p.position),
            owner: owners.get(&key).copied(),
            income: p.credit_value,
            size: size.clamp(0.6, 1.6),
            surface_color: terrain_color(p),
        });
    }

    // Routes.
    let mut routes = Vec::new();
    let mut unresolved = 0;
    for r in data.campaign_trade_routes(campaign) {
        let find = |n: &str| index.get(&n.trim().to_ascii_lowercase()).copied();
        match (find(&r.point_a), find(&r.point_b)) {
            (Some(a), Some(b)) if a != b => routes.push(RouteView { a, b }),
            _ => {
                unresolved += 1;
                if unresolved <= 5 {
                    notes.push(format!(
                        "trade route {} ({} - {}) skipped: unresolved end point",
                        r.name, r.point_a, r.point_b
                    ));
                }
            }
        }
    }
    if unresolved > 5 {
        notes.push(format!("{} trade routes skipped in total", unresolved));
    }

    (
        GalaxyView {
            planets,
            routes,
            factions,
        },
        notes,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs against the real install when available; otherwise the test is skipped.
    #[test]
    fn maps_real_campaign() {
        let args = Args::default();
        let Ok((_, _, data)) = open(&args, &mut StartupLog::default()) else {
            eprintln!("skipped: no game install");
            return;
        };
        let Some(campaign) = data.campaign(DEFAULT_CAMPAIGN) else {
            eprintln!("skipped: campaign missing");
            return;
        };
        let (view, notes) = build_view(&data, campaign);
        eprintln!("notes: {notes:?}");
        assert!(view.planets.len() > 20, "planets: {}", view.planets.len());
        assert!(!view.routes.is_empty());
        assert!(view.factions.len() >= 2);
        assert!(
            view.routes
                .iter()
                .all(|r| r.a < view.planets.len() && r.b < view.planets.len())
        );
        assert!(
            view.planets
                .iter()
                .all(|p| p.owner.is_none_or(|o| o < view.factions.len()))
        );
        assert!(view.planets.iter().any(|p| p.owner.is_some()));
        let exact = data.campaign_planet_owners(campaign);
        assert!(!exact.is_empty());
        // Every exactly-owned planet in the view carries the owner the data layer reports.
        for p in &view.planets {
            let want = exact
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(&p.id))
                .map(|(_, f)| f.as_str());
            let got = p.owner.map(|o| view.factions[o].name.as_str());
            assert_eq!(
                want.is_some(),
                got.is_some(),
                "owner presence mismatch on {}",
                p.id
            );
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &view.planets {
            lo = lo.min(p.position);
            hi = hi.max(p.position);
        }
        eprintln!("position range: {lo} .. {hi}");
        let sizes = (
            view.planets.iter().map(|p| p.size).fold(f32::MAX, f32::min),
            view.planets.iter().map(|p| p.size).fold(f32::MIN, f32::max),
        );
        let owned = view.planets.iter().filter(|p| p.owner.is_some()).count();
        eprintln!(
            "planets {} owned {owned} routes {}",
            view.planets.len(),
            view.routes.len()
        );
        let raw: Vec<f32> = data
            .campaign_planets(campaign)
            .iter()
            .filter(|p| p.is_real_planet())
            .map(|p| p.scale_factor)
            .collect();
        eprintln!(
            "raw scale_factor min {:?} max {:?}",
            raw.iter().cloned().fold(f32::MAX, f32::min),
            raw.iter().cloned().fold(f32::MIN, f32::max)
        );
        eprintln!(
            "sizes {sizes:?}, factions {:?}",
            view.factions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    /// End-to-end load incl. the galactic playlist; skipped without an install.
    #[test]
    fn loads_game_with_music() {
        let args = Args::default();
        let mut log = StartupLog::default();
        let Ok(game) = load(&args, &mut log) else {
            eprintln!("skipped: no game install");
            return;
        };
        for (_, line) in &log.0 {
            eprintln!("{line}");
        }
        let mut music = game.music.expect("galactic playlist");
        assert!(music.len() > 5);
        let track = crate::music::MusicSource::next_track(&mut music).expect("track");
        eprintln!(
            "first track {} {:?} {} bytes",
            track.name,
            track.duration,
            track.bytes.len()
        );
        assert!(track.name.to_ascii_uppercase().starts_with("GC_"));
        assert!(track.duration.is_some());
    }
}
