//! Empire at War Lua scripts running against [`galaxy_sim`].
//!
//! [`ScriptedSim`] steps the simulation and the script scheduler together:
//!
//! * the campaign's story plots (`Rebel_Story_Name`, `Empire_Story_Name`, `Underworld_Story_Name`) and
//!   `GameScoring` are started;
//! * simulation events become story notifications (`PlanetCaptured` -> `STORY_CONQUER`,
//!   `ConstructionComplete` -> `STORY_CONSTRUCT`, tech upgrades -> `STORY_TECH_LEVEL`);
//! * scripts are serviced at their `ServiceRate` in game time;
//! * what scripts do (spawn units, give credits, unlock objects, show dialogs / messages) is applied to
//!   the simulation by [`SimHost`] and reported as [`ScriptEvent`]s.
//!
//! What galaxy-sim cannot express yet is listed in `NEEDS.md`.

pub mod ai;
pub mod host;
mod save;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use alamo_data::{Campaign, GameData, GameObjects};
use alamo_fs::FileSource;
use alamo_lua::ScriptRuntime;
use galaxy_sim::{Event, ProductionKind, Sim, SimConfig, SimError};
use galaxy_ai::{AiTier, Personality};

pub use host::{HostStats, Inner, ScriptEvent, SimHost};
pub use save::{Checkpoint, Diagnostics, LoadProgress, SAVE_VERSION, SaveError, SaveFile, SaveHeader};

/// Options of a scripted match.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScriptedConfig {
    /// Run `GameScoring`.
    pub game_scoring: bool,
    /// Also start `GalacticFreeStore` for every faction (experimental, see `NEEDS.md`).
    pub freestore: bool,
    /// Script scheduler step in game seconds.
    pub script_step: f64,
    /// Faction controlled by the human; defaults to the campaign's `Starting_Active_Player`.
    pub human: Option<String>,
    /// Run the Remake AI (galaxy-ai, default) for every AI faction of the campaign instead of galaxy-sim's built-in AI (which snowballs: about 500 captures in 30 weeks).
    pub remake_ai: bool,
    /// Difficulty of the AI factions (credits, build times, battle multipliers, contrast, goal sleep); `None` = no multipliers. Default `Normal`.
    pub difficulty: Option<DifficultyTier>,
    /// Strategic layer of the Remake AI (influence maps, utility scoring, pincers, honest information); see `galaxy-ai/STRATEGY.md`.
    pub strategy: bool,
    /// Personality per faction name (lower case); factions without an entry use `default_personality`.
    pub personalities: std::collections::HashMap<String, Personality>,
    /// Match variety, 0 = every game identical (the defaults above), 1 = strong variation: per AI faction the personality values are
    /// jittered by up to +-40 % x `variety` and an opening (expand / turtle / raid / balanced, weighted by faction) is drawn, both from the
    /// match seed (`SimConfig::seed`), so a seed replays exactly.
    #[serde(default = "default_variety")]
    pub variety: f64,
}

fn default_variety() -> f64 {
    0.5
}

/// Opening weights per faction name (balanced, expand, turtle, raid); estimates.
fn opening_weights(faction: &str) -> [u32; 4] {
    match faction {
        "empire" => [3, 2, 1, 3],
        "rebel" => [3, 3, 2, 2],
        "underworld" => [2, 2, 2, 3],
        _ => [3, 2, 2, 2],
    }
}

/// Draws the personality jitter and the opening of a faction from the match seed (splitmix64, so it needs no RNG state and does not depend on faction order).
pub fn draw_character(seed: u64, faction: &str, base: Personality, variety: f64) -> (Personality, galaxy_ai::Opening) {
    if variety <= 0.0 {
        return (base, galaxy_ai::Opening::Balanced);
    }
    let mut x = seed ^ 0x9E37_79B9_7F4A_7C15;
    for b in faction.to_ascii_lowercase().bytes() {
        x = (x ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3);
    }
    let mut next = move || {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) as f64 / u64::MAX as f64
    };
    let mut jitter = |v: f64| (v * (1.0 + 0.4 * variety * (next() * 2.0 - 1.0))).clamp(0.5, 1.8);
    let p = Personality { aggression: jitter(base.aggression), expansion: jitter(base.expansion), caution: jitter(base.caution) };
    let w = opening_weights(&faction.to_ascii_lowercase());
    let mut pick = (next() * f64::from(w.iter().sum::<u32>())) as u32;
    let mut idx = 0;
    for (i, &wi) in w.iter().enumerate() {
        if pick < wi {
            idx = i;
            break;
        }
        pick -= wi;
    }
    (p, [galaxy_ai::Opening::Balanced, galaxy_ai::Opening::Expand, galaxy_ai::Opening::Turtle, galaxy_ai::Opening::Raid][idx])
}

/// Difficulty tiers. `Easy`, `Normal` and `Hard` read `Xml/DifficultyAdjustments.xml` (`<Tier>_Default` for the major
/// factions, `<Tier>_Minor` for minor ones). `Brutal` and `Fair` are our own estimates, not part of the mod data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DifficultyTier {
    /// `Easy_*`.
    Easy,
    /// `Normal_*`.
    Normal,
    /// `Hard_*`.
    Hard,
    /// Estimate: `Hard_*` with credits x4 (instead of x3) and full map knowledge.
    Brutal,
    /// Estimate: no economic or combat cheats at all (credits 1.0, build time 1.0, battle multipliers 1.0), `Normal` contrast, honest information.
    Fair,
}

/// Default personalities (estimates): the Empire presses harder, the Rebels expand and stay careful, the Underworld is cautious.
pub fn default_personalities() -> std::collections::HashMap<String, Personality> {
    let p = |aggression, expansion, caution| Personality { aggression, expansion, caution };
    [("empire", p(1.25, 1.0, 0.9)), ("rebel", p(0.95, 1.2, 1.0)), ("underworld", p(1.0, 1.0, 1.1))].into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

impl Default for ScriptedConfig {
    fn default() -> Self {
        Self {
            game_scoring: true,
            freestore: false,
            script_step: 0.2,
            human: None,
            remake_ai: true,
            difficulty: Some(DifficultyTier::Normal),
            strategy: true,
            personalities: default_personalities(),
            variety: default_variety(),
        }
    }
}

/// Result of one [`ScriptedSim::step`].
#[derive(Debug, Default)]
pub struct StepReport {
    /// Simulation events of the step.
    pub sim_events: Vec<Event>,
    /// What scripts produced during the step.
    pub script_events: Vec<ScriptEvent>,
}

/// Accumulated cost (thread CPU time on Windows).
#[derive(Debug, Default, Clone, Copy)]
pub struct Timing {
    /// Time spent in `Sim::step`.
    pub sim: Duration,
    /// Time spent running scripts.
    pub scripts: Duration,
    /// Time spent in the Remake AI's goal evaluation and task force commands (plan scripts count as scripts).
    pub ai: Duration,
}

/// Simulation plus scripts.
pub struct ScriptedSim {
    inner: Rc<RefCell<Inner>>,
    runtime: ScriptRuntime,
    script_step: f64,
    script_clock: f64,
    /// Cost so far.
    pub timing: Timing,
    /// Errors of scripts that failed to start.
    pub start_errors: Vec<String>,
    /// Plans that could not be started.
    pub ai_errors: Vec<String>,
    /// Setup recorded in saves.
    header: save::SaveHeader,
    /// Parts of a loaded save that did not fit the game (changed mod data); empty for a new game.
    pub load_warnings: Vec<String>,
}

/// CPU cost of the calling thread, from the thread cycle counter (fine grained and unaffected by other
/// processes competing for the cores, unlike wall time). Converted at an assumed 3 GHz, so only relative
/// numbers are meaningful.
#[cfg(windows)]
fn thread_cpu_time() -> Duration {
    unsafe extern "system" {
        fn GetCurrentThread() -> isize;
        fn QueryThreadCycleTime(thread: isize, cycles: *mut u64) -> i32;
    }
    let mut cycles = 0u64;
    // SAFETY: plain Win32 call with a valid out pointer for the current thread.
    unsafe {
        QueryThreadCycleTime(GetCurrentThread(), &mut cycles);
    }
    Duration::from_nanos(cycles / 3)
}
#[cfg(not(windows))]
fn thread_cpu_time() -> Duration {
    Duration::ZERO
}

fn read_dialog(fs: &Arc<dyn FileSource>) -> host::DialogReader {
    let fs = fs.clone();
    Rc::new(move |name: &str| {
        let file = if name.to_ascii_lowercase().ends_with(".txt") {
            name.to_string()
        } else {
            format!("{name}.txt")
        };
        fs.read(&format!("Scripts/Story/{file}"))
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    })
}

impl ScriptedSim {
    /// Builds the simulation for `campaign` and starts the story scripts.
    pub fn new(
        fs: Arc<dyn FileSource>,
        data: Arc<GameData>,
        campaign: &Campaign,
        sim_config: SimConfig,
        config: ScriptedConfig,
    ) -> Result<ScriptedSim, SimError> {
        let sim = Sim::new(&data, campaign, sim_config)?;
        Ok(Self::from_sim(sim, fs, data, campaign, config))
    }

    /// Wraps an already built simulation (`Sim` is `Send`, `ScriptedSim` is not: build the `Sim` anywhere and call this on the thread that
    /// runs the match) and starts the story scripts. The sim must be fresh (time 0).
    pub fn from_sim(sim: Sim, fs: Arc<dyn FileSource>, data: Arc<GameData>, campaign: &Campaign, config: ScriptedConfig) -> ScriptedSim {
        let header_sim_config = sim.world().config.clone();
        let header_config = config.clone();
        let world = sim.world().clone();
        let human_name = config
            .human
            .clone()
            .or_else(|| campaign.starting_active_player.clone())
            .unwrap_or_default();
        let human = if human_name.eq_ignore_ascii_case("none") {
            u32::MAX // nobody is human (all factions AI controlled)
        } else {
            world.faction_index.get(&human_name.to_ascii_lowercase()).copied().unwrap_or(0)
        };
        let fs_for_ai = fs.clone();
        let mut warnings = Vec::new();
        let objects = GameObjects::load(&*fs, &mut warnings);
        let inner = Rc::new(RefCell::new(Inner::new(
            sim,
            human,
            objects,
            data.clone(),
            read_dialog(&fs),
        )));
        let mut runtime = ScriptRuntime::new(
            fs,
            Box::new(SimHost {
                inner: inner.clone(),
            }),
        );
        let mut start_errors = Vec::new();

        for (faction, key) in [
            ("Rebel", "rebel_story_name"),
            ("Empire", "empire_story_name"),
            ("Underworld", "underworld_story_name"),
        ] {
            let value = campaign
                .fields
                .get(key)
                .or_else(|| campaign.fields.get(&key.to_ascii_uppercase()));
            if let Some(file) = value
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                && let Err(e) = runtime.load_story_plots(&file, Some(faction))
            {
                start_errors.push(format!("{faction} story {file}: {e}"));
            }
        }
        if config.game_scoring
            && let Err(e) = runtime.spawn_script("GameScoring", &[])
        {
            start_errors.push(format!("GameScoring: {e}"));
        }
        if config.freestore {
            for f in 0..world.factions.len() as u32 {
                if let Err(e) = runtime.spawn_script(
                    "GalacticFreeStore",
                    &[("PlayerObject", alamo_lua::ScriptValue::Player(f))],
                ) {
                    start_errors.push(format!(
                        "GalacticFreeStore({}): {e}",
                        world.factions[f as usize].name
                    ));
                    break;
                }
            }
        }
        let mut sim = ScriptedSim {
            inner,
            runtime,
            script_step: config.script_step.max(0.01),
            script_clock: 0.0,
            timing: Timing::default(),
            start_errors,
            ai_errors: Vec::new(),
            header: save::header(&campaign.name, &header_sim_config, &header_config, &world),
            load_warnings: Vec::new(),
        };
        if config.remake_ai {
            sim.start_ai(&fs_for_ai, campaign, &config);
        }
        sim
    }

    /// Switches from galaxy-sim's built-in AI to the Remake AI for all AI factions of the campaign.
    fn start_ai(&mut self, fs: &Arc<dyn FileSource>, campaign: &Campaign, config: &ScriptedConfig) {
        let data = galaxy_ai::AiData::load(&**fs);
        let plans = galaxy_ai::discover_plans(&**fs);
        let mut engine = galaxy_ai::AiEngine::new(Arc::new(data), plans);
        let (world, human) = {
            let i = self.inner.borrow();
            (i.world.clone(), i.human)
        };
        // AI factions follow the human tech level through the human faction story plot; a human without a story plot (a minor) gets no such plot.
        let human_story = world.factions.get(human as usize).is_some_and(|f| campaign.fields.get(&format!("{}_story_name", f.name.to_ascii_lowercase())).or_else(|| campaign.fields.get(&format!("{}_STORY_NAME", f.name.to_ascii_uppercase()))).is_some_and(|v| !v.trim().is_empty()));
        engine.follow_human_tech = human_story;
        let mut added = std::collections::HashSet::new();
        let mut default_type = None;
        for (faction, ai_type) in galaxy_ai::campaign_ai_players(&**fs, campaign) {
            let Some(&id) = world.faction_index.get(&faction.to_ascii_lowercase()) else { continue };
            if id == human {
                continue;
            }
            // `SandboxHuman` is the template of the human slot; when nobody is human the faction plays with its own AI type.
            let ai_type = if ai_type.eq_ignore_ascii_case("SandboxHuman") { format!("GC_{faction}Ai") } else { ai_type };
            if !ai_type.eq_ignore_ascii_case("GC_MinorAi") {
                default_type.get_or_insert_with(|| ai_type.clone());
            }
            added.insert(id);
            if let Err(e) = engine.add_player(id, &ai_type) {
                self.start_errors.push(e);
            }
        }
        // The campaign's own active player has no `AI_Player_Control` entry; when nobody is human (headless AI-vs-AI
        // runs) it plays like the others. Neutral forces never act.
        if let Some(ai_type) = default_type {
            for (id, f) in world.factions.iter().enumerate() {
                let id = id as u32;
                let passive = ["neutral", "pirates", "hostile"].iter().any(|n| f.name.eq_ignore_ascii_case(n));
                if id != human
                    && !added.contains(&id)
                    && !passive
                    && let Err(e) = engine.add_player(id, &ai_type)
                {
                    self.start_errors.push(e);
                }
            }
        }
        // Difficulty and strategic layer.
        let table = alamo_data::DifficultyTable::load(&**fs, &mut Vec::new());
        let minor: std::collections::HashSet<u32> = galaxy_ai::campaign_ai_players(&**fs, campaign)
            .into_iter()
            .filter(|(_, t)| t.eq_ignore_ascii_case("GC_MinorAi"))
            .filter_map(|(f, _)| world.faction_index.get(&f.to_ascii_lowercase()).copied())
            .collect();
        let mut i = self.inner.borrow_mut();
        let seed = i.sim.world().config.seed;
        engine.strategy.config.enabled = config.strategy;
        for (id, f) in world.factions.iter().enumerate() {
            let id = id as u32;
            if id == human || ["neutral", "hostile"].iter().any(|n| f.name.eq_ignore_ascii_case(n)) {
                continue;
            }
            let base = config.personalities.get(&f.name.to_ascii_lowercase()).copied().unwrap_or_default();
            let (pers, opening) = draw_character(seed, &f.name, base, config.variety);
            engine.strategy.config.personalities.insert(id, pers);
            engine.strategy.config.openings.insert(id, opening);
            if let Some(tier) = config.difficulty {
                let (diff, ai_tier) = tier_settings(&table, tier, minor.contains(&id));
                i.sim.set_faction_difficulty(id, diff);
                engine.strategy.tiers.insert(id, ai_tier);
            }
        }
        i.sim.set_ai_factions(std::iter::empty());
        i.ai = Some(engine);
    }

    /// Runs `f` with the simulation.
    pub fn with_sim<R>(&self, f: impl FnOnce(&Sim) -> R) -> R {
        f(&self.inner.borrow().sim)
    }

    /// Runs `f` with the simulation for commands.
    /// Runtime-only setup that is not part of the game state (tactical catalog and arenas).
    pub fn with_sim_runtime<R>(&self, f: impl FnOnce(&mut Sim) -> R) -> R {
        f(&mut self.inner.borrow_mut().sim)
    }

    /// Mutable access to the simulation (tests and tools; the next save contains the changes).
    pub fn with_sim_mut<R>(&self, f: impl FnOnce(&mut Sim) -> R) -> R {
        f(&mut self.inner.borrow_mut().sim)
    }

    /// DebugMessage lines of scripts.
    pub fn script_log(&self) -> Vec<String> {
        self.inner.borrow().script_log.clone()
    }

    /// Counters of script effects.
    pub fn host_stats(&self) -> HostStats {
        self.inner.borrow().stats.clone()
    }

    /// Mutable runtime access (for example `set_idle_backoff`).
    pub fn runtime_mut(&mut self) -> &mut ScriptRuntime {
        &mut self.runtime
    }

    /// The Lua runtime (statistics, errors, log).
    pub fn runtime(&self) -> &ScriptRuntime {
        &self.runtime
    }

    /// Personality (after the seeded variation) and opening of an AI faction; `None` for the human, neutral factions or without the Remake AI.
    pub fn ai_character(&self, faction: &str) -> Option<(Personality, galaxy_ai::Opening)> {
        let i = self.inner.borrow();
        let id = i.world.faction_index.get(&faction.to_ascii_lowercase()).copied()?;
        let ai = i.ai.as_ref()?;
        let cfg = &ai.strategy.config;
        Some((*cfg.personalities.get(&id)?, cfg.openings.get(&id).copied().unwrap_or_default()))
    }

    /// Advances simulation and scripts by `dt` game seconds.
    pub fn step(&mut self, dt: f32) -> StepReport {
        let t0 = thread_cpu_time();
        let sim_events = {
            let mut g = self.inner.borrow_mut();
            let mut events = std::mem::take(&mut g.pending_events);
            events.extend(g.sim.step(dt));
            events
        };
        self.timing.sim += thread_cpu_time() - t0;

        let t1 = thread_cpu_time();
        self.notify_story(&sim_events);
        self.script_clock += f64::from(dt);
        while self.script_clock >= self.script_step {
            self.runtime.update(self.script_step);
            self.script_clock -= self.script_step;
        }
        self.timing.scripts += thread_cpu_time() - t1;
        let t2 = thread_cpu_time();
        self.step_ai();
        self.timing.ai += thread_cpu_time() - t2;

        let script_events = std::mem::take(&mut self.inner.borrow_mut().events);
        StepReport {
            sim_events,
            script_events,
        }
    }

    /// Goal evaluation, plan start and plan bookkeeping of the Remake AI.
    fn step_ai(&mut self) {
        let (requests, expired, now) = {
            let mut g = self.inner.borrow_mut();
            let Inner { sim, world, human, allies, ai, pending_events, .. } = &mut *g;
            let Some(ai) = ai.as_mut() else { return };
            let mut adapter = ai::SimAdapter { sim, world: world.clone(), human: *human, allies, pending: pending_events };
            let now = galaxy_ai::AiSim::time(&adapter);
            let requests = ai.update(&mut adapter);
            (requests, ai.take_expired(), now)
        };
        for r in requests {
            let target = r.target.map(|p| ai::PLANET_OBJECT_TAG | u64::from(p));
            if let Err(e) = self.runtime.spawn_plan(&r.script, r.plan, r.player, target) {
                self.ai_errors.push(format!("{}: {e}", r.script));
            }
        }
        for p in expired {
            self.runtime.kill_plan(p);
        }
        let finished = self.runtime.take_finished_plans();
        if let Some(ai) = self.inner.borrow_mut().ai.as_mut() {
            for p in finished {
                ai.plan_finished(p, now);
            }
        }
    }

    /// Statistics of the Remake AI (None when it is not enabled).
    pub fn ai_stats(&self) -> Option<galaxy_ai::AiStats> {
        self.inner.borrow().ai.as_ref().map(galaxy_ai::AiEngine::stats)
    }

    /// Goals the Remake AI of a faction is working on: `(goal, target planet, seconds running)`.
    pub fn ai_active_goals(&self, player: u32) -> Vec<(String, Option<u32>, f64)> {
        let i = self.inner.borrow();
        let now = i.sim.galaxy().time;
        i.ai.as_ref().map(|a| a.active_goals(player, now)).unwrap_or_default()
    }

    fn notify_story(&mut self, events: &[Event]) {
        // Collect first: notifications call back into the host, which borrows the shared state.
        let mut notes: Vec<(&'static str, String)> = Vec::new();
        let mut battles: Vec<(bool, String)> = Vec::new();
        {
            let s = self.inner.borrow();
            let w = &s.world;
            for e in events {
                match e {
                    Event::PlanetCaptured { planet, .. } => {
                        let full = w.planets[*planet as usize].name.clone();
                        let short = full
                            .rsplit_once('_')
                            .map_or(full.clone(), |(a, _)| a.to_string());
                        notes.push(("STORY_CONQUER", short));
                        notes.push(("STORY_CONQUER", full));
                    }
                    Event::BattleWon { faction, .. } => battles.push((true, w.factions[*faction as usize].name.clone())),
                    Event::BattleLost { faction, .. } => battles.push((false, w.factions[*faction as usize].name.clone())),
                    // The Remake's hero respawn and ship-phase plots wait for these (`Story_Sandbox_Hero_Timers_*.xml`: STORY_TACTICAL_DESTROY, then a 300 s timer, then SPAWN_HERO).
                    Event::UnitsDestroyed { unit, .. } => notes.push(("STORY_TACTICAL_DESTROY", w.units[*unit as usize].name.clone())),
                    Event::ConstructionComplete { faction, kind, .. } => {
                        match kind {
                            ProductionKind::Unit(u) => {
                                notes.push(("STORY_CONSTRUCT", w.units[*u as usize].name.clone()))
                            }
                            ProductionKind::Structure(st) => notes
                                .push(("STORY_CONSTRUCT", w.structures[*st as usize].name.clone())),
                            ProductionKind::Starbase(sb) => notes
                                .push(("STORY_CONSTRUCT", w.starbases[*sb as usize].name.clone())),
                            ProductionKind::Tech(_) => {
                                let level = s.sim.galaxy().factions[*faction as usize].tech;
                                notes.push(("STORY_TECH_LEVEL", level.to_string()));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        for (kind, param) in notes {
            self.runtime.story_notify(kind, &param);
        }
        for (won, faction) in battles {
            self.runtime.story_battle(won, &faction);
        }
    }

    /// API call counts that reached unimplemented stubs, with the number of calls.
    pub fn unimplemented_calls(&self) -> BTreeMap<String, usize> {
        let hits = self.runtime.api_hits();
        self.runtime
            .unimplemented()
            .into_iter()
            .map(|n| (hits.get(&n).copied().unwrap_or(0), n))
            .map(|(c, n)| (n, c))
            .collect()
    }
}

/// Maps a tier to the sim multipliers and the AI tier (see [`DifficultyTier`]).
fn tier_settings(table: &alamo_data::DifficultyTable, tier: DifficultyTier, minor: bool) -> (galaxy_sim::Difficulty, AiTier) {
    let suffix = if minor { "Minor" } else { "Default" };
    let entry = |name: &str| table.get(&format!("{name}_{suffix}")).cloned().unwrap_or_default();
    let ai = |name: &str, a: &alamo_data::DifficultyAdjustment, full: bool| AiTier { name: name.into(), contrast: f64::from(a.galactic_contrast), goal_sleep: f64::from(a.galactic_goal_sleep), full_map_knowledge: full };
    match tier {
        DifficultyTier::Easy | DifficultyTier::Normal | DifficultyTier::Hard => {
            let name = match tier { DifficultyTier::Easy => "Easy", DifficultyTier::Normal => "Normal", _ => "Hard" };
            let a = entry(name);
            (galaxy_sim::Difficulty::from_adjustment(&a), ai(name, &a, false))
        }
        DifficultyTier::Brutal => {
            let a = entry("Hard");
            let mut d = galaxy_sim::Difficulty::from_adjustment(&a);
            d.credit = 4.0;
            (d, ai("Brutal", &a, true))
        }
        DifficultyTier::Fair => {
            let a = entry("Normal");
            (galaxy_sim::Difficulty::default(), ai("Fair", &a, false))
        }
    }
}
