//! Loading of the real game data and mapping it onto the renderer's [`GalaxyView`].

use std::{sync::Arc, time::Instant};

use alamo_data::{Campaign, GameData, Planet};
use alamo_fs::{GameFs, GameInstall};
use bevy::prelude::{Resource, Vec3, info, warn};

use crate::cli::Args;
use crate::galaxy_view::{
    FactionView, GalaxyCameraView, GalaxyLight, GalaxyView, LineStyleView, PlanetView, PropView, RouteStyleView, RouteView,
};

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

impl StartupLog {
    /// Writes the recorded lines through the (now available) logger.
    pub fn emit(&self) {
        for (is_warn, msg) in &self.0 {
            if *is_warn {
                warn!("{msg}");
            } else {
                info!("{msg}");
            }
        }
    }
}

/// Startup system that emits the recorded [`StartupLog`] through the real logger.
pub fn replay_startup_log(log: bevy::prelude::Res<StartupLog>) {
    log.emit();
}

/// Game files and parsed galaxy data, shared by all screens.
#[derive(Clone)]
pub struct GameAssets {
    /// Layered file system (mod over FoC over EaW), used to load models and sounds.
    #[allow(dead_code)] // used by tests and the upcoming battle integration
    pub fs: Arc<GameFs>,
    /// Campaigns, planets, routes, factions and the English string table.
    pub data: Arc<GameData>,
}

/// Why the real game could not be used (shown on the title screen until acknowledged).
#[derive(Resource, Default, Debug, Clone)]
pub struct StartupProblem(pub Option<String>);

/// Turns a technical error of the file system or the data loader into advice for the player.
pub fn describe_problem(error: &str, mod_id: &str) -> String {
    let lower = error.to_lowercase();
    let advice = if lower.contains("no empire at war install") {
        "Star Wars: Empire at War was not found.\n\nInstall Empire at War - Gold Pack (includes Forces of Corruption) from Steam,\nstart it once, or pass the folder with --game-dir <path> (or set EAW_GAME_DIR)."
            .to_owned()
    } else if lower.contains("mod data not found") {
        format!("The Empire at War Remake mod is not installed.\n\nSubscribe to Empire at War Remake 4.0 on the Steam Workshop (id {mod_id}),\nlet Steam download it, then start again. Use --mod <id> for another mod.")
    } else if lower.contains("no galactic conquest") {
        format!("The mod {mod_id} has no Galactic Conquest campaigns: its files are incomplete or a different version.\n\nLet Steam finish the download or verify the files of the mod, then start again.")
    } else {
        "The game files could not be read.\n\nVerify the game and the mod in Steam (right click, Properties, Installed Files), then start again.".to_owned()
    };
    format!("{advice}\n\nDetails: {error}")
}

/// The mod to load: only an explicit `--mod` selects anything but Remake 4.0, whatever else is installed.
pub fn default_mod_id(explicit: Option<&str>) -> &str {
    explicit.unwrap_or(DEFAULT_MOD_ID)
}

/// `--mod` alias of the Clone Wars stack (see `alamo_fs::GameFs::for_mods`: 3229239424, 2794270450, 1770851727).
pub const CLONE_WARS_ALIAS: &str = "fft";
/// Workshop ids the Clone Wars stack needs; all of them must be installed.
pub const CLONE_WARS_IDS: [&str; 3] = ["3229239424", "2794270450", DEFAULT_MOD_ID];

/// Which game content a session runs on. The choice is explicit (new-game screen or `--mod`), never implicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    /// Empire at War Remake 4.0 alone (the default).
    Remake,
    /// Remake Clone Wars submod on top of Remake 4.0: Republic and CIS are playable.
    CloneWars,
}

impl Content {
    /// Content selected by `--mod` (or a save's recorded key): the Clone Wars alias selects the submod, anything else the plain Remake files.
    pub fn from_mod_arg(mod_id: Option<&str>) -> Self {
        match mod_id {
            Some(m) if m.eq_ignore_ascii_case(CLONE_WARS_ALIAS) => Content::CloneWars,
            _ => Content::Remake,
        }
    }

    /// Key written into save files and passed to `--mod` when the game restarts on this content.
    pub fn key(self) -> &'static str {
        match self {
            Content::Remake => DEFAULT_MOD_ID,
            Content::CloneWars => CLONE_WARS_ALIAS,
        }
    }

    /// Name shown on the new-game screen.
    pub fn label(self) -> &'static str {
        match self {
            Content::Remake => "Remake 4.0",
            Content::CloneWars => "Clone Wars (submod)",
        }
    }
}

/// The running content and whether the submod can be offered (all its workshop folders exist).
#[derive(Resource, Debug, Clone, Copy)]
pub struct ContentChoice {
    /// Content this session was started with.
    pub active: Content,
    /// The Clone Wars folders are installed.
    pub clone_wars_available: bool,
}

impl Default for ContentChoice {
    fn default() -> Self {
        Self { active: Content::Remake, clone_wars_available: false }
    }
}

/// True when every workshop folder of the Clone Wars stack exists.
pub fn clone_wars_installed(install: &GameInstall) -> bool {
    CLONE_WARS_IDS.iter().all(|id| install.mod_data(id).is_dir())
}

/// Starts this program again on `content` (the layered file system is fixed at start-up). The original arguments are passed on
/// except the ones that pick the content, the screen and the campaign; `extra` is appended. Returns false if the start failed.
pub fn restart_with(content: Content, extra: &[String]) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let mut args: Vec<String> = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--mod" | "--screen" | "--campaign" | "--load-from" => {
                it.next();
            }
            "--via-menu" => {}
            _ => args.push(a),
        }
    }
    args.extend(["--mod".to_owned(), content.key().to_owned()]);
    args.extend(extra.iter().cloned());
    match std::process::Command::new(exe).args(&args).spawn() {
        Ok(_) => true,
        Err(e) => {
            warn!("cannot restart on {}: {e}", content.label());
            false
        }
    }
}

/// The installed game, or `None` when only the demo galaxy is available.
#[derive(Resource, Clone, Default)]
pub struct Game(pub Option<GameAssets>);

/// The installed game: the folder of `--game-dir`, else `EAW_GAME_DIR`, else the Steam install found through the registry and `libraryfolders.vdf`.
pub fn find_install(args: &Args) -> Option<GameInstall> {
    match &args.game_dir {
        Some(dir) => GameInstall::from_root(dir),
        None => GameInstall::detect(),
    }
}

/// The mod folder named by `--mod-dir` or `EAW_MOD_DIR`; `None` selects the Workshop folder of the mod id.
pub fn explicit_mod_dir(args: &Args) -> Option<std::path::PathBuf> {
    args.mod_dir.clone().or_else(|| std::env::var_os("EAW_MOD_DIR").map(std::path::PathBuf::from))
}

/// The `Data` folder of an explicit mod folder: the folder itself when it has no `Data` folder below it.
pub fn explicit_mod_data(dir: &std::path::Path) -> std::path::PathBuf {
    if dir.join("Data").is_dir() { dir.join("Data") } else { dir.to_path_buf() }
}

/// Locates the install and opens the layered game file system (fast: reads archive indexes only).
pub fn open_fs(args: &Args, log: &mut StartupLog) -> Result<(GameInstall, Arc<GameFs>), String> {
    let started = Instant::now();
    let install = find_install(args).ok_or("no Empire at War install found (use --game-dir <path> or set EAW_GAME_DIR)")?;
    let (mod_desc, fs) = if let Some(dir) = explicit_mod_dir(args) {
        let mod_data = explicit_mod_data(&dir);
        let fs = alamo_fs::GameFs::from_stacks(&[
            vec![mod_data],
            vec![install.foc_data.clone()],
            vec![install.eaw_data.clone()],
        ])
        .map_err(|e| format!("cannot open mod files from {}: {e}", dir.display()))?;
        (dir.display().to_string(), fs)
    } else {
        let mod_id = default_mod_id(args.mod_id.as_deref());
        let fs = GameFs::for_mod(&install, Some(mod_id))
            .map_err(|e| format!("cannot open game files: {e}"))?;
        (format!("mod {mod_id}"), fs)
    };
    log.info(format!(
        "game: install {} | {mod_desc} | {} files | opened in {:.2?}",
        install.game_root.display(),
        fs.file_count(),
        started.elapsed()
    ));
    Ok((install, Arc::new(fs)))
}

/// Opens the game file system and loads the galaxy data (blocking).
fn open(args: &Args, log: &mut StartupLog) -> Result<(GameInstall, Arc<GameFs>, GameData), String> {
    let (install, fs) = open_fs(args, log)?;
    let started = Instant::now();
    let (data, warnings) = alamo_data::load_galaxy(fs.as_ref());
    log.info(format!(
        "game: data loaded in {:.2?} ({} warnings)",
        started.elapsed(),
        warnings.len()
    ));
    for w in warnings.iter().take(8) {
        log.warn(format!("game data: {}:{}: {}", w.file, w.line, w.message));
    }
    Ok((install, fs, data))
}
/// Locates and loads the game (blocking).
pub fn load_assets(args: &Args, log: &mut StartupLog) -> Result<GameAssets, String> {
    let (_, fs, data) = open(args, log)?;
    Ok(GameAssets { fs, data: Arc::new(data) })
}

/// Prints all galaxy campaigns with their planet counts (for `--list-campaigns`).
pub fn list_campaigns(args: &Args) -> Result<(), String> {
    let assets = load_assets(args, &mut StartupLog::default())?;
    for c in campaign_entries(&assets.data) {
        println!("{:<40} {:>4} planets  {}  [{}]", c.name, c.planets, c.title, c.group_title);
    }
    Ok(())
}

/// A playable galactic conquest campaign as shown in the campaign selection.
#[derive(Debug, Clone, PartialEq)]
pub struct CampaignEntry {
    /// Campaign name (key for [`build_campaign`]).
    pub name: String,
    /// Localised title of the campaign (falls back to the name).
    pub title: String,
    /// Group key: campaigns with the same group differ only in the human faction.
    pub group: String,
    /// Display name of the group, e.g. `Full` or `Standard Casual`.
    pub group_title: String,
    /// Human faction of the campaign (`Rebel`, `Empire`, `Underworld`).
    pub faction: String,
    /// Number of planets on the map.
    pub planets: usize,
}

/// Name used by the built-in demo galaxy when no game is installed.
pub const DEMO_CAMPAIGN: &str = "__demo__";

/// Entry for the built-in demo galaxy.
pub fn demo_entry() -> CampaignEntry {
    CampaignEntry {
        name: DEMO_CAMPAIGN.into(),
        title: "Demo galaxy".into(),
        group: "demo".into(),
        group_title: "Demo".into(),
        faction: "Rebel".into(),
        planets: GalaxyView::demo().planets.len(),
    }
}

/// Lists the campaigns with a galaxy and a human player, grouped by map size.
///
/// Observer campaigns (`OBS`, AI versus AI) are left out. The group is the campaign name without
/// the faction token; entries are ordered by planet count, then name, with Rebel, Empire and
/// Underworld variants in that order inside a group.
pub fn campaign_entries(data: &GameData) -> Vec<CampaignEntry> {
    let faction_of = |token: &str| match token.to_ascii_lowercase().as_str() {
        "rebel" | "rebels" => Some("Rebel"),
        "imperial" | "empire" => Some("Empire"),
        "underworld" => Some("Underworld"),
        "republic" => Some("Republic"),
        "cis" => Some("CIS"),
        _ => None,
    };
    let mut out = Vec::new();
    for c in data.campaigns.iter().filter(|c| c.has_galaxy()) {
        let tokens: Vec<&str> = c.name.split('_').collect();
        if tokens.iter().any(|t| t.eq_ignore_ascii_case("obs")) {
            continue;
        }
        let Some(pos) = tokens.iter().position(|t| faction_of(t).is_some()) else { continue };
        let faction = faction_of(tokens[pos]).expect("checked").to_owned();
        let rest: Vec<&str> = tokens
            .iter()
            .enumerate()
            .filter(|&(i, t)| {
                i != pos && !["gc", "web", "way"].iter().any(|k| t.eq_ignore_ascii_case(k))
            })
            .map(|(_, t)| *t)
            .collect();
        let planets = data.campaign_planets(c).iter().filter(|p| is_map_planet(p)).count();
        let title = c.text_id.as_deref().and_then(|id| data.text(id)).filter(|t| !t.is_empty());
        out.push(CampaignEntry {
            name: c.name.clone(),
            title: title.unwrap_or(&c.name).to_owned(),
            group: c.name.replace(tokens[pos], "{faction}"),
            group_title: rest.join(" "),
            faction,
            planets,
        });
    }
    let rank = |f: &str| match f {
        "Rebel" => 0,
        "Empire" => 1,
        "Underworld" => 2,
        "Republic" => 3,
        "CIS" => 4,
        _ => 5,
    };
    out.sort_by(|a, b| {
        (a.planets, a.group_title.split(' ').count(), &a.group, rank(&a.faction))
            .cmp(&(b.planets, b.group_title.split(' ').count(), &b.group, rank(&b.faction)))
    });
    out
}

/// Builds the galaxy of campaign `name`; also returns its human faction (e.g. `Rebel`).
pub fn build_campaign(
    assets: &GameAssets,
    name: &str,
    log: &mut StartupLog,
) -> Result<(GalaxyView, Option<String>), String> {
    let started = Instant::now();
    let data = &*assets.data;
    let campaign = data
        .campaign(name)
        .filter(|c| c.has_galaxy())
        .ok_or_else(|| {
            format!("campaign `{name}` not found or has no galaxy (see --list-campaigns)")
        })?;
    let (view, notes) = build_view(data, campaign);
    for n in &notes {
        // Routes to planets outside the campaign (e.g. Allanteen in the Mini/Standard galaxies) are data, not errors.
        if n.starts_with("trade route") || n.contains("trade routes skipped") {
            log.info(format!("galaxy: {n}"));
        } else {
            log.warn(format!("galaxy: {n}"));
        }
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
    log.info(format!(
        "game: campaign {} | {} planets, {} routes, {} factions | built in {:.2?}",
        campaign.name,
        view.planets.len(),
        view.routes.len(),
        view.factions.len(),
        started.elapsed()
    ));
    Ok((view, campaign.starting_active_player.clone()))
}
/// Human readable name for a game object type: string table when known, else a prettified id.
fn object_display_name(data: &GameData, object: &str) -> String {
    let up = object.to_ascii_uppercase();
    for key in [format!("TEXT_UNIT_{up}"), format!("TEXT_STRUCTURE_{up}"), format!("TEXT_{up}"), up] {
        if let Some(t) = data.text(&key).filter(|t| !t.is_empty()) {
            return t.to_owned();
        }
    }
    object.replace('_', " ")
}

/// English planet ability `(name, description)`; `None` for planets without one (`NA`).
fn planet_ability(data: &GameData, p: &Planet) -> Option<(String, String)> {
    let name = p.ability_name.as_deref().filter(|n| !n.eq_ignore_ascii_case("na"))?;
    let name = data.text(name).filter(|t| !t.is_empty())?;
    let desc = p.ability_description.as_deref().and_then(|d| data.text(d)).unwrap_or("");
    Some((name.to_owned(), desc.to_owned()))
}

fn line_style(s: &alamo_data::LineSettings) -> LineStyleView {
    let c = |v: Option<[u8; 4]>| v.map_or([0.05, 0.12, 0.15, 1.0], |c| c.map(|x| x as f32 / 255.0));
    LineStyleView { color_far: c(s.color_zoomed_out), color_near: c(s.color_zoomed_in), width: s.width }
}

/// Maps a `TradeRouteLine` to the view style (`Ally` = own, `Enemy`, `Neutral`).
fn route_style(line: &alamo_data::TradeRouteLine) -> RouteStyleView {
    let find = |rel: &str| line.settings.iter().find(|s| s.relation.eq_ignore_ascii_case(rel)).map(line_style);
    let neutral = find("Neutral").unwrap_or(RouteStyleView::fallback().neutral);
    RouteStyleView {
        name: line.name.clone(),
        own: find("Ally").unwrap_or(neutral),
        enemy: find("Enemy").unwrap_or(neutral),
        neutral,
    }
}

/// Models that are intentionally invisible placeholders.
fn is_invisible_model(model: &str) -> bool {
    model.to_ascii_lowercase().contains("invisible")
}

/// True for planets that appear on the map. Faction "dummy" planets (invisible model, no name)
/// only hold starting forces and ownership and are stacked in one corner of the galaxy.
fn is_map_planet(p: &Planet) -> bool {
    p.is_real_planet()
        && !p.galactic_model.as_deref().is_some_and(is_invisible_model)
        && !p.name.to_ascii_lowercase().contains("dummy")
}

/// Sun settings from `Gameconstants.xml` (galactic zoom-out light angle and level).
fn galaxy_light(data: &GameData) -> GalaxyLight {
    let mut light = GalaxyLight::default();
    if let Some(a) = data.game_constants.get("galactic_zoom_out_light_angle") {
        let v = alamo_data::xml::parse_floats(a);
        if let [x, y, z, ..] = v[..] {
            light.angle_deg = Vec3::new(x, y, z);
        }
    }
    if let Some(l) = data.game_constants.get("galactic_zoom_light_level").and_then(|s| s.trim().parse().ok()) {
        light.level = l;
    }
    light
}

/// The camera tilt of `Gameconstants.xml`: `GMC_InitialPitchAngleDegrees` is negative for a camera that looks down (-50 in the Remake), so
/// the sign is dropped; a missing, unreadable or flat (under 5 degrees) value leaves the default.
fn galaxy_camera(data: &GameData) -> GalaxyCameraView {
    let mut camera = GalaxyCameraView::default();
    if let Some(p) = data.game_constants.get("gmc_initialpitchangledegrees").and_then(|s| s.trim().parse::<f32>().ok()).map(f32::abs)
        && (5.0..=89.0).contains(&p)
    {
        camera.pitch_deg = p;
    }
    camera
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
            credits: cf.starting_credits.unwrap_or(0),
            icon: def.alternate_icon_name.clone().or_else(|| def.icon_name.clone()),
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
        .filter(|p| is_map_planet(p) && p.scale_factor > 0.0)
        .map(|p| p.scale_factor)
        .collect();
    scales.sort_by(f32::total_cmp);
    let median_scale = scales.get(scales.len() / 2).copied().unwrap_or(1.0);

    // Starting forces per planet, shown in the info panel.
    let mut forces = std::collections::HashMap::<String, Vec<(String, usize)>>::new();
    for sf in &campaign.starting_forces {
        let list = forces.entry(sf.planet.to_ascii_lowercase()).or_default();
        let name = object_display_name(data, &sf.object);
        match list.iter_mut().find(|(n, _)| *n == name) {
            Some((_, count)) => *count += 1,
            None => list.push((name, 1)),
        }
    }

    // Planets.
    let mut planets = Vec::new();
    let mut index = std::collections::HashMap::<String, usize>::new();
    for p in data
        .campaign_planets(campaign)
        .into_iter()
        .filter(|p| is_map_planet(p))
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
            model: p.galactic_model.clone(),
            model_scale: if p.scale_factor > 0.0 { p.scale_factor } else { 1.0 },
            surface_color: terrain_color(p),
            max_space_base: p.max_space_base,
            max_ground_base: p.max_ground_base,
            terrain: p.terrain.clone(),
            info: planet_info(data, p),
            population_bonus: p.fields.get("additional_population_capacity").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
            land_slots: p.special_structures_land,
            special_slots: p.special_structures_space,
            ability: planet_ability(data, p),
            starting_forces: forces
                .remove(&key)
                .unwrap_or_default()
                .into_iter()
                .map(|(n, c)| if c > 1 { format!("{n} x{c}") } else { n })
                .collect(),
        });
    }

    // Decorative objects: every other location with a visible model (galaxy disc art, nebulae,
    // faction logos, far-away background planets).
    let props: Vec<PropView> = data
        .campaign_planets(campaign)
        .into_iter()
        .filter(|p| !is_map_planet(p) && !p.is_real_planet())
        .filter_map(|p| {
            let model = p.galactic_model.clone().filter(|m| !is_invisible_model(m))?;
            Some(PropView {
                id: p.name.clone(),
                model,
                position: Vec3::from(p.position),
                scale: if p.scale_factor > 0.0 { p.scale_factor } else { 1.0 },
            })
        })
        .collect();

    // Route styles (TRADEROUTELINES.XML) and routes.
    let mut route_styles: Vec<RouteStyleView> = data.trade_route_lines.iter().map(route_style).collect();
    let default_style = route_styles.iter().position(|s| s.name.eq_ignore_ascii_case("default"));
    if route_styles.is_empty() {
        route_styles.push(RouteStyleView::fallback());
    }
    let default_style = default_style.unwrap_or(0);
    let mut routes = Vec::new();
    let mut unresolved = 0;
    for r in data.campaign_trade_routes(campaign) {
        let find = |n: &str| index.get(&n.trim().to_ascii_lowercase()).copied();
        match (find(&r.point_a), find(&r.point_b)) {
            (Some(a), Some(b)) if a != b => {
                let style = r
                    .visible_line_name
                    .as_deref()
                    .and_then(|n| route_styles.iter().position(|s| s.name.eq_ignore_ascii_case(n)))
                    .unwrap_or(default_style);
                routes.push(RouteView { a, b, style });
            }
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
            route_styles,
            player_faction: campaign
                .starting_active_player
                .as_deref()
                .and_then(|h| faction_names.iter().position(|n| n.eq_ignore_ascii_case(h))),
            props,
            light: galaxy_light(data),
            camera: galaxy_camera(data),
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
        assert!(view.player_faction.is_some(), "human faction resolved");
        assert_eq!(view.camera.pitch_deg, 50.0, "Gameconstants.xml GMC_InitialPitchAngleDegrees -50 of the Remake");
        assert!(view.planets.iter().all(|p| !p.id.to_ascii_lowercase().contains("dummy")), "dummy planets filtered");
        assert!(view.planets.iter().all(|p| p.model.is_some() && p.model_scale > 0.0));
        assert!(view.props.iter().any(|p| p.model.eq_ignore_ascii_case("w_galaxy00.alo")), "galaxy art prop");
        assert!(!view.route_styles.is_empty());
        assert!(view.routes.iter().all(|r| r.style < view.route_styles.len()));
        assert!(view.planets.iter().any(|p| p.ability.is_some()), "some planet has an ability text");
        assert!(view.planets.iter().any(|p| !p.starting_forces.is_empty()));
        assert!(view.factions.iter().any(|f| f.credits > 0));
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
            .filter(|p| is_map_planet(p))
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
    #[test]
    fn lists_campaigns_by_group() {
        let args = Args::default();
        let Ok(assets) = load_assets(&args, &mut StartupLog::default()) else {
            eprintln!("skipped: no game install");
            return;
        };
        let entries = campaign_entries(&assets.data);
        for e in &entries {
            eprintln!("{:<42} {:<10} {:<22} {:>4} {}", e.name, e.faction, e.group_title, e.planets, e.title);
        }
        assert!(entries.iter().any(|e| e.name == DEFAULT_CAMPAIGN && e.faction == "Rebel"));
        assert!(entries.iter().all(|e| !e.name.to_ascii_lowercase().contains("obs")));
        // Remake 4.0 offers the Rebel, Empire and Underworld variants (Republic/CIS campaigns only come with another mod stack, --mod).
        let full: Vec<_> = entries.iter().filter(|e| e.group_title == "Full").map(|e| e.faction.as_str()).collect();
        assert_eq!(full, ["Rebel", "Empire", "Underworld"]);
    }

    #[test]
    fn builds_default_campaign_and_music_events_exist() {
        let args = Args::default();
        let mut log = StartupLog::default();
        let Ok(assets) = load_assets(&args, &mut log) else {
            eprintln!("skipped: no game install");
            return;
        };
        let (view, faction) = build_campaign(&assets, DEFAULT_CAMPAIGN, &mut log).expect("campaign");
        assert_eq!(view.planets.len(), 145);
        assert_eq!(faction.as_deref(), Some("Rebel"));
        // The music director needs a galactic playlist for the faction and a main menu theme.
        let mut w = Vec::new();
        let music = alamo_audio::MusicLibrary::load(assets.fs.as_ref(), &mut w);
        assert!(music.get("Galactic_Map_Rebel_Music_Event").is_some());
        assert!(music.get("Main_Menu_Music_Event").is_some());
    }
}

#[cfg(test)]
mod problem_tests {
    use super::{DEFAULT_MOD_ID, default_mod_id, describe_problem};

    #[test]
    fn default_selection_is_remake_only_and_never_a_stack() {
        assert_eq!(default_mod_id(None), DEFAULT_MOD_ID);
        assert_eq!(default_mod_id(Some("fft")), "fft");
        assert_eq!(super::Content::from_mod_arg(None), super::Content::Remake);
        assert_eq!(super::Content::from_mod_arg(Some("fft")), super::Content::CloneWars);
        assert_eq!(super::Content::from_mod_arg(Some("1770851727")), super::Content::Remake);
        assert_eq!(super::Content::CloneWars.key(), "fft");
    }

    #[test]
    fn problems_get_actionable_advice() {
        let install = describe_problem("no Empire at War install found (use --game-dir <path>)", "123");
        assert!(install.contains("--game-dir") && install.contains("Steam"));
        let missing_mod = describe_problem("cannot open game files: mod data not found: X", "123");
        assert!(missing_mod.contains("Workshop") && missing_mod.contains("123"));
        let incomplete = describe_problem("no galactic conquest campaigns found", "123");
        assert!(incomplete.contains("incomplete"));
        assert!(describe_problem("weird", "1").contains("Verify the game"));
    }
}

/// The planet panel text of the original: the `Describe_*` lines (system, climate, population, minerals ...) followed by
/// the history paragraph of the encyclopedia.
fn planet_info(data: &GameData, p: &alamo_data::Planet) -> Vec<String> {
    // The Remake panel (CS9VdxglewY f0009) prints the planet's `Encyclopedia_Text` list in order: "This is a System ...", climate, population,
    // deposits, orbital capacity, then the history; `TEXT_LINE` is a blank line. Planets without that list keep the older description lines.
    let mut ordered: Vec<String> = Vec::new();
    for key in &p.encyclopedia_text {
        if key.eq_ignore_ascii_case("TEXT_LINE") {
            if ordered.last().is_some_and(|l| !l.is_empty()) {
                ordered.push(String::new());
            }
        } else if let Some(t) = data.text(key) {
            ordered.extend(t.replace("\\n", "\n").trim().lines().map(str::to_owned));
        }
    }
    while ordered.last().is_some_and(|l| l.is_empty()) {
        ordered.pop();
    }
    if ordered.len() > 1 {
        return ordered;
    }
    let mut lines: Vec<String> = p
        .description_ids
        .iter()
        .filter_map(|id| data.text(id))
        .flat_map(|t| t.replace("\\n", "\n").lines().map(str::to_owned).collect::<Vec<_>>())
        .collect();
    let history = p
        .encyclopedia_text
        .iter()
        .find(|k| k.to_ascii_uppercase().ends_with("_INFO_HIS_01"))
        .or_else(|| p.encyclopedia_text.iter().find(|k| k.to_ascii_uppercase().ends_with("_INFO_HIS")))
        .and_then(|k| data.text(k));
    // Texts may repeat (the same history under two keys, shipyard names twice): keep the first of each.
    let mut seen = std::collections::HashSet::new();
    lines.retain(|l| l.is_empty() || seen.insert(l.clone()));
    if let Some(h) = history
        && !lines.iter().any(|l| l == h.trim())
    {
        lines.push(String::new());
        lines.push(h.trim().to_owned());
    }
    lines
}

#[cfg(test)]
mod camera_constant_tests {
    use super::*;

    fn data(value: Option<&str>) -> GameData {
        let mut d = GameData::default();
        if let Some(v) = value {
            d.game_constants.insert("gmc_initialpitchangledegrees".into(), v.into());
        }
        d
    }

    #[test]
    fn the_camera_looks_down_at_the_angle_of_the_constant_whatever_its_sign() {
        assert_eq!(galaxy_camera(&data(Some("-50.0"))).pitch_deg, 50.0);
        assert_eq!(galaxy_camera(&data(Some(" -35 "))).pitch_deg, 35.0);
        assert_eq!(galaxy_camera(&data(Some("42"))).pitch_deg, 42.0);
    }

    #[test]
    fn a_missing_or_unreadable_constant_keeps_the_remakes_fifty_degrees() {
        assert_eq!(galaxy_camera(&data(None)).pitch_deg, 50.0);
        assert_eq!(galaxy_camera(&data(Some("steep"))).pitch_deg, 50.0);
        assert_eq!(galaxy_camera(&data(Some("0"))).pitch_deg, 50.0, "a camera that looks along the plane is no map view");
        assert_eq!(galaxy_camera(&data(Some("-90"))).pitch_deg, 50.0);
    }
}
