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

pub mod host;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alamo_data::{Campaign, GameData, GameObjects};
use alamo_fs::FileSource;
use alamo_lua::ScriptRuntime;
use galaxy_sim::{Event, ProductionKind, Sim, SimConfig, SimError};

pub use host::{HostStats, Inner, ScriptEvent, SimHost};

/// Options of a scripted match.
#[derive(Debug, Clone)]
pub struct ScriptedConfig {
    /// Run `GameScoring`.
    pub game_scoring: bool,
    /// Also start `GalacticFreeStore` for every faction (experimental, see `NEEDS.md`).
    pub freestore: bool,
    /// Script scheduler step in game seconds.
    pub script_step: f64,
    /// Faction controlled by the human; defaults to the campaign's `Starting_Active_Player`.
    pub human: Option<String>,
}

impl Default for ScriptedConfig {
    fn default() -> Self {
        Self {
            game_scoring: true,
            freestore: false,
            script_step: 0.2,
            human: None,
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
        let world = sim.world().clone();
        let human_name = config
            .human
            .clone()
            .or_else(|| campaign.starting_active_player.clone())
            .unwrap_or_default();
        let human = world
            .faction_index
            .get(&human_name.to_ascii_lowercase())
            .copied()
            .unwrap_or(0);
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
        Ok(ScriptedSim {
            inner,
            runtime,
            script_step: config.script_step.max(0.01),
            script_clock: 0.0,
            timing: Timing::default(),
            start_errors,
        })
    }

    /// Runs `f` with the simulation.
    pub fn with_sim<R>(&self, f: impl FnOnce(&Sim) -> R) -> R {
        f(&self.inner.borrow().sim)
    }

    /// Runs `f` with the simulation for commands.
    pub fn with_sim_mut<R>(&self, f: impl FnOnce(&mut Sim) -> R) -> R {
        f(&mut self.inner.borrow_mut().sim)
    }

    /// Counters of script effects.
    pub fn host_stats(&self) -> HostStats {
        self.inner.borrow().stats.clone()
    }

    /// The Lua runtime (statistics, errors, log).
    pub fn runtime(&self) -> &ScriptRuntime {
        &self.runtime
    }

    /// Advances simulation and scripts by `dt` game seconds.
    pub fn step(&mut self, dt: f32) -> StepReport {
        let t0 = thread_cpu_time();
        let sim_events = self.inner.borrow_mut().sim.step(dt);
        self.timing.sim += thread_cpu_time() - t0;

        let t1 = thread_cpu_time();
        self.notify_story(&sim_events);
        self.script_clock += f64::from(dt);
        while self.script_clock >= self.script_step {
            self.runtime.update(self.script_step);
            self.script_clock -= self.script_step;
        }
        self.timing.scripts += thread_cpu_time() - t1;

        let script_events = std::mem::take(&mut self.inner.borrow_mut().events);
        StepReport {
            sim_events,
            script_events,
        }
    }

    fn notify_story(&mut self, events: &[Event]) {
        // Collect first: notifications call back into the host, which borrows the shared state.
        let mut notes: Vec<(&'static str, String)> = Vec::new();
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


