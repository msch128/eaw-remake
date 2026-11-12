//! Story plots (`Xml/Story_*.xml`): events with prerequisites and rewards, driving the story scripts.
//!
//! A plot is a list of `Event`s. An event becomes *active* when all its `Prereq` events have fired and
//! *fires* when its condition is met (`STORY_ELAPSED`: that many seconds after activation; every other
//! type: when notified, see [`StoryEngine::notify`]). Firing calls `Story_Event_Trigger(name)` in the
//! plot's Lua script and applies the rewards. Rewards that only manipulate the story graph
//! (`TRIGGER_EVENT`, `RESET_BRANCH`, `DISABLE_BRANCH`, `DISABLE_STORY_EVENT`) are handled here, all
//! others are handed to the host.
//!
//! **Semantics are reconstructed**, not verified against the engine. A `STORY_TRIGGER` with prerequisites fires once those have fired
//! (the Web Way plots chain heroes, locks, tech followers, shield grants and the 181st on prerequisites alone; nothing triggers them by name).
//! One with no prerequisites waits until a `TRIGGER_EVENT` names it. Several `Prereq` tags are alternatives (any one
//! tag). Names inside one tag, separated by spaces or commas, are all required.

use std::collections::HashMap;

use alamo_data::xml;

/// Names in one `<Prereq>` tag. Spaces and commas separate them; every name is required.
fn split_prereq(text: &str) -> Vec<String> {
    text.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()).map(str::to_string).collect()
}

/// True when there is no prerequisite, or one whole tag has fired.
fn prereqs_ready(groups: &[Vec<String>], fired: impl Fn(&str) -> bool) -> bool {
    groups.is_empty() || groups.iter().any(|g| g.iter().all(|p| fired(p)))
}

/// One reward of an event.
#[derive(Debug, Clone, PartialEq)]
pub struct Reward {
    /// `Reward_Type`.
    pub kind: String,
    /// `Reward_Param1`, `Reward_Param2`, ...
    pub params: Vec<String>,
}

/// One `<Event>` of a plot.
#[derive(Debug, Clone)]
pub struct PlotEvent {
    /// Event name.
    pub name: String,
    /// `Event_Type` (`STORY_ELAPSED`, `STORY_TRIGGER`, ...).
    pub kind: String,
    /// `Event_Param1..`.
    pub params: Vec<String>,
    /// `Prereq` groups. Any group may be met; every name inside a group must have fired.
    pub prereqs: Vec<Vec<String>>,
    /// Rewards in file order.
    pub rewards: Vec<Reward>,
    /// `Perpetual`: the event can fire again after firing.
    pub perpetual: bool,
    /// `Branch` tag (`RESET_BRANCH`/`DISABLE_BRANCH` name it).
    pub branch: Option<String>,
    /// `Story_Dialog` (name of a dialog text file such as `Dialog_Intro`) and `Story_Chapter`.
    pub dialog: Option<(String, i32)>,
}

/// A parsed plot file.
#[derive(Debug, Clone, Default)]
pub struct Plot {
    /// File name as referenced (`Story_SandBox_Rebel_Web_Way_Full.xml`).
    pub file: String,
    /// Events in file order.
    pub events: Vec<PlotEvent>,
}

impl Plot {
    /// Parses plot XML.
    pub fn parse(file: &str, bytes: &[u8], warnings: &mut Vec<xml::Warning>) -> Option<Plot> {
        let root = xml::parse(bytes, file, warnings)?;
        let mut events = Vec::new();
        for e in root.children_named("event") {
            let Some(name) = e.attr("name") else { continue };
            let mut ev = PlotEvent {
                name: name.to_string(),
                kind: e
                    .child_text("event_type")
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_uppercase(),
                params: Vec::new(),
                prereqs: Vec::new(),
                rewards: Vec::new(),
                perpetual: e.child_text("perpetual").is_some_and(xml::parse_bool),
                branch: e.child_text("branch").map(|b| b.trim().to_ascii_lowercase()).filter(|b| !b.is_empty()),
                dialog: e
                    .child_text("story_dialog")
                    .map(|d| d.trim().to_string())
                    .filter(|d| !d.is_empty())
                    .map(|d| {
                        (
                            d,
                            e.child_text("story_chapter")
                                .and_then(|c| c.trim().parse().ok())
                                .unwrap_or(0),
                        )
                    }),
            };
            for c in &e.children {
                let text = c.text_trimmed().to_string();
                match c.name.as_str() {
                    "prereq" => {
                        let names = split_prereq(&text);
                        if !names.is_empty() {
                            ev.prereqs.push(names);
                        }
                    }
                    "reward_type" => ev.rewards.push(Reward {
                        kind: text.to_ascii_uppercase(),
                        params: Vec::new(),
                    }),
                    n if n.starts_with("reward_param") => {
                        if let Some(r) = ev.rewards.last_mut() {
                            let idx: usize = n["reward_param".len()..].parse().unwrap_or(1);
                            while r.params.len() < idx {
                                r.params.push(String::new());
                            }
                            r.params[idx - 1] = text;
                        }
                    }
                    n if n.starts_with("event_param") => {
                        let idx: usize = n["event_param".len()..].parse().unwrap_or(1);
                        while ev.params.len() < idx {
                            ev.params.push(String::new());
                        }
                        ev.params[idx - 1] = text;
                    }
                    _ => {}
                }
            }
            events.push(ev);
        }
        Some(Plot {
            file: file.to_string(),
            events,
        })
    }
}

/// Progress of one story event.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum State {
    Inactive,
    Active(f64),
    Fired,
    Disabled,
}

struct Running {
    plot: Plot,
    /// Index of the Lua script instance that receives `Story_Event_Trigger`.
    script: usize,
    faction: Option<String>,
    started: f64,
    states: Vec<State>,
    /// STORY_TRIGGER events that were triggered by name before they became active.
    triggered: Vec<bool>,
    /// The event fired since its branch was last reset (a perpetual event stays `Active` but still satisfies prerequisites).
    done: Vec<bool>,
    /// Battles counted so far by `STORY_WIN_BATTLES` / `STORY_LOSE_BATTLES` events.
    battles: Vec<u32>,
    by_name: HashMap<String, usize>,
}


/// State of one running plot (the plot itself is re-read from the file).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlotSnapshot {
    /// Plot file (`Story_Sandbox_Hero_Timers_Rebel.xml`).
    pub file: String,
    /// Time the plot was activated.
    pub started: f64,
    /// Event states in file order.
    pub states: Vec<State>,
    /// Events triggered by name before they became active.
    pub triggered: Vec<bool>,
    /// Events that fired since their branch was last reset.
    pub done: Vec<bool>,
    /// Battle counters of the `STORY_WIN_BATTLES` / `STORY_LOSE_BATTLES` events.
    pub battles: Vec<u32>,
}

/// State of the story engine: every plot's event states, counters and timers.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct StorySnapshot {
    /// Plots in activation order.
    pub plots: Vec<PlotSnapshot>,
    /// Re-evaluate on the next tick.
    pub dirty: bool,
    /// Earliest due time of an elapsed event (`None` = none; JSON has no infinity).
    pub next_due: Option<f64>,
}

impl StoryEngine {
    /// Copies the dynamic state.
    pub fn snapshot(&self) -> StorySnapshot {
        StorySnapshot {
            plots: self
                .plots
                .iter()
                .map(|r| PlotSnapshot { file: r.plot.file.clone(), started: r.started, states: r.states.clone(), triggered: r.triggered.clone(), done: r.done.clone(), battles: r.battles.clone() })
                .collect(),
            dirty: self.dirty,
            next_due: self.next_due.is_finite().then_some(self.next_due),
        }
    }

    /// Applies a snapshot to the plots activated in the same order from the same files. Plots whose file or event count differ (a changed mod) keep their fresh state
    /// and are reported in the returned warnings.
    pub fn restore(&mut self, snap: &StorySnapshot) -> Vec<String> {
        let mut warnings = Vec::new();
        for (i, p) in snap.plots.iter().enumerate() {
            let Some(r) = self.plots.get_mut(i).filter(|r| r.plot.file == p.file && r.states.len() == p.states.len()) else {
                warnings.push(format!("story plot {} of the save does not match the loaded plots", p.file));
                continue;
            };
            r.started = p.started;
            r.states = p.states.clone();
            r.triggered = p.triggered.clone();
            r.done = p.done.clone();
            r.battles = p.battles.clone();
        }
        if snap.plots.len() != self.plots.len() {
            warnings.push(format!("the save has {} story plots, the game {}", snap.plots.len(), self.plots.len()));
        }
        // The plots go on as they were saved. Only where a plot did not fit is everything looked at again.
        (self.dirty, self.next_due) = if warnings.is_empty() { (snap.dirty, snap.next_due.unwrap_or(f64::INFINITY)) } else { (true, 0.0) };
        warnings
    }
}

/// An event that fired: the runtime calls the script and hands the rewards to the host.
#[derive(Debug, Clone)]
pub struct Fired {
    /// Plot file the event belongs to.
    pub file: String,
    /// Script instance to notify.
    pub script: usize,
    /// Event name.
    pub event: String,
    /// Plot owner faction.
    pub faction: Option<String>,
    /// Rewards for the host.
    pub host_rewards: Vec<Reward>,
    /// `TRIGGER_EVENT`, `RESET_BRANCH`, `DISABLE_BRANCH`, `DISABLE_STORY_EVENT`: already applied here.
    pub graph_rewards: Vec<Reward>,
    /// Dialog (file name, chapter) the event shows.
    pub dialog: Option<(String, i32)>,
}

/// The set of active plots.
#[derive(Default)]
pub struct StoryEngine {
    plots: Vec<Running>,
    /// Something changed that tick must re-evaluate (a plot was added, an event fired or was reset).
    dirty: bool,
    /// Earliest time an active STORY_ELAPSED event becomes due.
    next_due: f64,
}

impl StoryEngine {
    /// Activates a plot.
    pub fn activate(&mut self, plot: Plot, script: usize, faction: Option<String>, now: f64) {
        let n = plot.events.len();
        let by_name = plot
            .events
            .iter()
            .enumerate()
            .map(|(i, e)| (e.name.to_ascii_lowercase(), i))
            .collect();
        self.dirty = true;
        self.plots.push(Running {
            plot,
            script,
            faction,
            started: now,
            states: vec![State::Inactive; n],
            triggered: vec![false; n],
            done: vec![false; n],
            battles: vec![0; n],
            by_name,
        });
    }

    /// Number of active plots.
    pub fn plot_count(&self) -> usize {
        self.plots.len()
    }

    /// Total number of events that have fired.
    pub fn fired_count(&self) -> usize {
        self.plots
            .iter()
            .flat_map(|p| &p.states)
            .filter(|s| **s == State::Fired)
            .count()
    }

    /// Advances time: activates events whose prerequisites fired and fires elapsed ones. Chain reactions
    /// (events firing others) resolve within the call.
    pub fn tick(&mut self, now: f64) -> Vec<Fired> {
        // Nothing can happen between state changes and the next elapsed-timer deadline.
        if !self.dirty && now < self.next_due {
            return Vec::new();
        }
        let mut out = Vec::new();
        loop {
            let before = out.len();
            for pi in 0..self.plots.len() {
                self.update_plot(pi, now, None, &mut out);
            }
            if out.len() == before {
                break;
            }
        }
        self.dirty = false;
        self.next_due = self.compute_next_due();
        out
    }

    fn compute_next_due(&self) -> f64 {
        let mut due = f64::INFINITY;
        for r in &self.plots {
            for (ei, ev) in r.plot.events.iter().enumerate() {
                if let State::Active(since) = r.states[ei]
                    && ev.kind == "STORY_ELAPSED"
                {
                    let secs: f64 = ev
                        .params
                        .first()
                        .and_then(|p| p.parse().ok())
                        .unwrap_or(0.0);
                    let base = if ev.prereqs.is_empty() {
                        r.started
                    } else {
                        since
                    };
                    due = due.min(base + secs);
                }
            }
        }
        due
    }

    /// Notifies the story of something that happened (`Story_Event("X")` from Lua, or the host reporting
    /// `STORY_CONQUER` / `STORY_CONSTRUCT` / ...). Fires matching *active* events of the given kinds;
    /// `kinds` empty means any non-elapsed kind. A match is an event whose name or `Event_Param1` equals
    /// `param` (case-insensitive).
    pub fn notify(&mut self, kinds: &[&str], param: &str, now: f64) -> Vec<Fired> {
        let mut out = Vec::new();
        for pi in 0..self.plots.len() {
            self.update_plot(pi, now, Some((kinds, param)), &mut out);
        }
        out.extend(self.tick(now));
        out
    }

    /// [`StoryEngine::notify`] for something that happened to `faction` only: the plots of other owners do not see it
    /// (`STORY_TECH_LEVEL` is the plot owner's own tech level). Plots without an owner see it as with `notify`.
    pub fn notify_for(&mut self, kinds: &[&str], param: &str, faction: &str, now: f64) -> Vec<Fired> {
        let mut out = Vec::new();
        for pi in 0..self.plots.len() {
            if self.plots[pi].faction.as_deref().is_none_or(|f| f.eq_ignore_ascii_case(faction)) {
                self.update_plot(pi, now, Some((kinds, param)), &mut out);
            }
        }
        out.extend(self.tick(now));
        out
    }

    /// Reports a finished battle of `faction` (`kind` is `STORY_WIN_BATTLES` or `STORY_LOSE_BATTLES`). Events with `FILTER_FRIENDLY_ONLY`
    /// count the battles of the plot's own faction; `Event_Param1` is the number needed, `Event_Param2` the mode (only `SPACE` counts here).
    pub fn notify_battle(&mut self, kind: &str, faction: &str, now: f64) -> Vec<Fired> {
        let mut out = Vec::new();
        for pi in 0..self.plots.len() {
            let r = &mut self.plots[pi];
            if !r.faction.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(faction)) {
                continue;
            }
            for ei in 0..r.plot.events.len() {
                let ev = &r.plot.events[ei];
                if ev.kind == kind && matches!(r.states[ei], State::Active(_)) && ev.params.get(1).is_none_or(|m| m.eq_ignore_ascii_case("space") || m.is_empty()) {
                    r.battles[ei] += 1;
                }
            }
            let _ = pi;
        }
        for pi in 0..self.plots.len() {
            self.update_plot(pi, now, Some((&[kind], "{0}battle")), &mut out);
        }
        out.extend(self.tick(now));
        out
    }

    fn update_plot(
        &mut self,
        pi: usize,
        now: f64,
        notify: Option<(&[&str], &str)>,
        out: &mut Vec<Fired>,
    ) {
        let n = self.plots[pi].plot.events.len();
        for ei in 0..n {
            // Activation.
            {
                let r = &mut self.plots[pi];
                if r.states[ei] == State::Inactive {
                    let ready = prereqs_ready(&r.plot.events[ei].prereqs, |p| {
                        r.by_name.get(&p.to_ascii_lowercase()).is_some_and(|&i| r.states[i] == State::Fired || r.done[i])
                    });
                    if ready {
                        r.states[ei] = State::Active(now);
                    }
                }
            }
            let State::Active(since) = self.plots[pi].states[ei] else {
                continue;
            };
            let r = &self.plots[pi];
            let ev = &r.plot.events[ei];
            let fire = match ev.kind.as_str() {
                "STORY_ELAPSED" => {
                    let secs: f64 = ev
                        .params
                        .first()
                        .and_then(|p| p.parse().ok())
                        .unwrap_or(0.0);
                    let base = if ev.prereqs.is_empty() {
                        r.started
                    } else {
                        since
                    };
                    now - base >= secs
                }
                // Prerequisites are the gate. A trigger with none waits for `TRIGGER_EVENT`.
                "STORY_TRIGGER" => r.triggered[ei] || (!ev.prereqs.is_empty() && !r.done[ei]),
                "STORY_WIN_BATTLES" | "STORY_LOSE_BATTLES" => {
                    let need: u32 = ev.params.first().and_then(|p| p.parse().ok()).unwrap_or(1);
                    notify.is_some_and(|(_, param)| param == "{0}battle") && r.battles[ei] >= need.max(1)
                }
                _ => notify.is_some_and(|(kinds, param)| {
                    (kinds.is_empty() || kinds.iter().any(|k| k.eq_ignore_ascii_case(&ev.kind)))
                        && (ev.name.eq_ignore_ascii_case(param)
                            || ev
                                .params
                                .iter()
                                .any(|p| !p.is_empty() && p.eq_ignore_ascii_case(param)))
                }),
            };
            if fire {
                self.fire(pi, ei, now, out);
            }
        }
    }

    fn fire(&mut self, pi: usize, ei: usize, now: f64, out: &mut Vec<Fired>) {
        // Perpetual events re-arm immediately (so they can fire again on the next notification).
        let rearm = self.plots[pi].plot.events[ei].perpetual
            && self.plots[pi].plot.events[ei].kind != "STORY_ELAPSED";
        self.dirty = true;
        self.plots[pi].states[ei] = if rearm {
            State::Active(now)
        } else {
            State::Fired
        };
        self.plots[pi].triggered[ei] = false;
        self.plots[pi].done[ei] = true;
        self.plots[pi].battles[ei] = 0;
        let rewards = self.plots[pi].plot.events[ei].rewards.clone();
        let mut host_rewards = Vec::new();
        let mut graph_rewards = Vec::new();
        for rw in rewards {
            let p1 = rw.params.first().cloned().unwrap_or_default();
            match rw.kind.as_str() {
                "TRIGGER_EVENT" => {
                    for r in &mut self.plots {
                        if let Some(&i) = r.by_name.get(&p1.to_ascii_lowercase()) {
                            r.triggered[i] = true;
                        }
                    }
                    graph_rewards.push(rw);
                }
                "RESET_BRANCH" => {
                    self.reset_branch(&p1, now);
                    graph_rewards.push(rw);
                }
                "DISABLE_BRANCH" => {
                    self.set_branch(&p1, rw.params.get(1).map(String::as_str).unwrap_or("1"));
                    graph_rewards.push(rw);
                }
                "DISABLE_STORY_EVENT" => {
                    self.disable_event(&p1);
                    graph_rewards.push(rw);
                }
                _ => host_rewards.push(rw),
            }
        }
        let r = &self.plots[pi];
        out.push(Fired {
            file: r.plot.file.clone(),
            script: r.script,
            event: r.plot.events[ei].name.clone(),
            faction: r.faction.clone(),
            host_rewards,
            graph_rewards,
            dialog: self.plots[pi].plot.events[ei].dialog.clone(),
        });
    }

    /// Events reachable from `root` through prerequisite edges (root included).
    fn branch(r: &Running, root: usize) -> Vec<usize> {
        let mut set = vec![root];
        let mut i = 0;
        while i < set.len() {
            let name = r.plot.events[set[i]].name.to_ascii_lowercase();
            for (ei, e) in r.plot.events.iter().enumerate() {
                if !set.contains(&ei) && e.prereqs.iter().flatten().any(|p| p.eq_ignore_ascii_case(&name)) {
                    set.push(ei);
                }
            }
            i += 1;
        }
        set
    }

    fn reset_branch(&mut self, name: &str, now: f64) {
        for r in &mut self.plots {
            let lower = name.to_ascii_lowercase();
            let mut set: Vec<usize> = (0..r.plot.events.len()).filter(|&i| r.plot.events[i].branch.as_deref() == Some(lower.as_str())).collect();
            if let Some(&root) = r.by_name.get(&lower) {
                set.push(root);
            }
            // Everything downstream of a branch member (through prerequisites) restarts with it.
            for i in set.clone() {
                set.extend(Self::branch(r, i));
            }
            set.sort_unstable();
            set.dedup();
            for i in set {
                if r.states[i] != State::Disabled {
                    r.states[i] = State::Inactive;
                    r.triggered[i] = false;
                    r.done[i] = false;
                    r.battles[i] = 0;
                }
            }
            let _ = now;
        }
    }

    /// `DISABLE_BRANCH`. `Reward_Param2` `0` or `false` turns the branch back on (`Disabled` becomes `Inactive`, and a firing
    /// already recorded stays recorded). Any other value, including a missing parameter, disables it. The branch is every event
    /// whose `Branch` tag is `name`, plus an event of that name. Live Story plots: 51 calls pass `1`, 38 pass `0`, none omit it.
    fn set_branch(&mut self, name: &str, param: &str) {
        let enable = param.eq_ignore_ascii_case("0") || param.eq_ignore_ascii_case("false");
        let lower = name.to_ascii_lowercase();
        for r in &mut self.plots {
            for i in 0..r.plot.events.len() {
                let on_branch = r.plot.events[i].branch.as_deref() == Some(lower.as_str()) || r.plot.events[i].name.eq_ignore_ascii_case(name);
                if !on_branch {
                    continue;
                }
                if enable {
                    if r.states[i] == State::Disabled {
                        r.states[i] = State::Inactive;
                    }
                } else {
                    r.states[i] = State::Disabled;
                }
            }
        }
    }

    /// `DISABLE_STORY_EVENT`: one event, by name.
    fn disable_event(&mut self, name: &str) {
        for r in &mut self.plots {
            if let Some(&i) = r.by_name.get(&name.to_ascii_lowercase()) {
                r.states[i] = State::Disabled;
            }
        }
    }

    /// Index of the running plot with this file name.
    pub fn plot_index(&self, file: &str) -> Option<usize> {
        self.plots
            .iter()
            .position(|r| r.plot.file.eq_ignore_ascii_case(file))
    }

    /// Index of an event in a running plot.
    pub fn event_index(&self, plot: usize, name: &str) -> Option<usize> {
        self.plots
            .get(plot)?
            .by_name
            .get(&name.to_ascii_lowercase())
            .copied()
    }

    /// Sets `Reward_Param<idx+1>` of the first reward of an event (`event.Set_Reward_Parameter(idx, v)`).
    pub fn set_reward_param(&mut self, plot: usize, event: usize, idx: usize, value: &str) {
        if let Some(r) = self.plots.get_mut(plot)
            && let Some(rw) = r
                .plot
                .events
                .get_mut(event)
                .and_then(|e| e.rewards.first_mut())
        {
            while rw.params.len() <= idx {
                rw.params.push(String::new());
            }
            rw.params[idx] = value.to_string();
        }
    }

    /// Resets a plot by index.
    pub fn reset_plot_index(&mut self, plot: usize, now: f64) {
        self.dirty = true;
        if let Some(r) = self.plots.get_mut(plot) {
            r.started = now;
            r.states.fill(State::Inactive);
            r.triggered.fill(false);
            r.done.fill(false);
        }
    }

    /// Resets the plot with this file name (all events inactive) or all plots if the name is empty.
    pub fn reset_plot(&mut self, file: &str, now: f64) {
        self.dirty = true;
        for r in &mut self.plots {
            if file.is_empty() || r.plot.file.eq_ignore_ascii_case(file) {
                r.started = now;
                r.states.fill(State::Inactive);
                r.triggered.fill(false);
                r.done.fill(false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plot(xml_text: &str) -> Plot {
        let mut w = Vec::new();
        Plot::parse("t.xml", xml_text.as_bytes(), &mut w).unwrap()
    }

    #[test]
    fn hero_respawn_chain_runs_twice() {
        let p = plot(
            r#"<Story>
            <Event Name="Death"><Event_Type>STORY_TACTICAL_DESTROY</Event_Type><Event_Param1>Hero</Event_Param1><Branch>B</Branch><Perpetual>True</Perpetual></Event>
            <Event Name="Spawn"><Event_Type>STORY_ELAPSED</Event_Type><Event_Param1>300</Event_Param1><Reward_Type>SPAWN_HERO</Reward_Type><Reward_Param1>Hero</Reward_Param1><Prereq>Death</Prereq><Branch>B</Branch></Event>
            <Event Name="Done"><Event_Type>STORY_TRIGGER</Event_Type><Reward_Type>REMOVE_STORY_GOAL</Reward_Type><Reward_Param1>T</Reward_Param1><Prereq>Spawn</Prereq><Perpetual>True</Perpetual></Event>
            <Event Name="Reset"><Event_Type>STORY_TRIGGER</Event_Type><Reward_Type>RESET_BRANCH</Reward_Type><Reward_Param1>B</Reward_Param1><Prereq>Done</Prereq><Branch>B</Branch><Perpetual>true</Perpetual></Event>
            </Story>"#,
        );
        let mut s = StoryEngine::default();
        s.activate(p, 0, Some("Rebel".into()), 0.0);
        assert!(s.tick(0.0).is_empty());
        for round in 0..2 {
            let t0 = f64::from(round) * 1000.0 + 10.0;
            assert!(s.notify(&["STORY_TACTICAL_DESTROY"], "Hero", t0).iter().any(|f| f.event == "Death"), "round {round}");
            assert!(s.tick(t0 + 299.0).is_empty());
            let names: Vec<_> = s.tick(t0 + 301.0).into_iter().map(|f| f.event).collect();
            assert_eq!(names, vec!["Spawn", "Done", "Reset"], "round {round}");
        }
    }

    #[test]
    fn battle_events_count_the_plot_factions_space_battles() {
        let p = plot(
            r#"<Story>
            <Event Name="Won"><Event_Type>STORY_WIN_BATTLES</Event_Type><Event_Param1>2</Event_Param1><Event_Param2>SPACE</Event_Param2><Event_Param3>FILTER_FRIENDLY_ONLY</Event_Param3><Perpetual>true</Perpetual></Event>
            </Story>"#,
        );
        let mut s = StoryEngine::default();
        s.activate(p, 0, Some("Rebel".into()), 0.0);
        s.tick(0.0);
        assert!(s.notify_battle("STORY_WIN_BATTLES", "Empire", 1.0).is_empty(), "not our battle");
        assert!(s.notify_battle("STORY_LOSE_BATTLES", "Rebel", 2.0).is_empty(), "a defeat");
        assert!(s.notify_battle("STORY_WIN_BATTLES", "Rebel", 3.0).is_empty(), "one of two");
        assert_eq!(s.notify_battle("STORY_WIN_BATTLES", "Rebel", 4.0).len(), 1);
        assert!(s.notify_battle("STORY_WIN_BATTLES", "Empire", 5.0).is_empty(), "the counter starts again");
        assert!(s.notify_battle("STORY_WIN_BATTLES", "Rebel", 6.0).is_empty());
        assert_eq!(s.notify_battle("STORY_WIN_BATTLES", "Rebel", 7.0).len(), 1);
    }

    /// `STORY_TECH_LEVEL` is the plot owner's level (`Research/Story/Tech_Levels_R_WW.xml` is the Rebel player's plot): another faction
    /// reaching the level fires nothing.
    #[test]
    fn tech_level_events_follow_the_plot_factions_own_level() {
        let p = plot(
            r#"<Story>
            <Event Name="Rebel_Tech_2"><Event_Type>STORY_TECH_LEVEL</Event_Type><Event_Param1>2</Event_Param1></Event>
            <Event Name="E_Tech-2_Auto"><Event_Type>STORY_TRIGGER</Event_Type><Reward_Type>SET_TECH_LEVEL</Reward_Type><Reward_Param1>Empire</Reward_Param1><Reward_Param2>2</Reward_Param2><Prereq>Rebel_Tech_2</Prereq></Event>
            </Story>"#,
        );
        let mut s = StoryEngine::default();
        s.activate(p, 0, Some("Rebel".into()), 0.0);
        s.tick(0.0);
        assert!(s.notify_for(&["STORY_TECH_LEVEL"], "2", "Empire", 1.0).is_empty(), "the Empire's level is not the Rebel's");
        assert!(s.notify_for(&["STORY_TECH_LEVEL"], "3", "rebel", 2.0).is_empty(), "another level");
        let names: Vec<_> = s.notify_for(&["STORY_TECH_LEVEL"], "2", "rebel", 3.0).into_iter().map(|f| f.event).collect();
        assert_eq!(names, vec!["Rebel_Tech_2", "E_Tech-2_Auto"]);
    }

    #[test]
    fn elapsed_chain_and_rewards() {
        let p = plot(
            r#"<Story>
            <Event Name="Start"><Event_Type>STORY_ELAPSED</Event_Type><Event_Param1>0</Event_Param1></Event>
            <Event Name="Later"><Event_Type>STORY_ELAPSED</Event_Type><Event_Param1>10</Event_Param1><Prereq>Start</Prereq>
              <Reward_Type>TRIGGER_EVENT</Reward_Type><Reward_Param1>Boom</Reward_Param1>
              <Reward_Type>CREDITS</Reward_Type><Reward_Param1>500</Reward_Param1></Event>
            <Event Name="Boom"><Event_Type>STORY_TRIGGER</Event_Type><Event_Param1>0</Event_Param1><Prereq>Later</Prereq></Event>
            </Story>"#,
        );
        let mut s = StoryEngine::default();
        s.activate(p, 0, Some("Rebel".into()), 0.0);
        let f = s.tick(0.0);
        assert_eq!(
            f.iter().map(|x| x.event.as_str()).collect::<Vec<_>>(),
            vec!["Start"]
        );
        assert!(s.tick(5.0).is_empty());
        let f = s.tick(10.5);
        let names: Vec<_> = f.iter().map(|x| x.event.as_str()).collect();
        assert_eq!(names, vec!["Later", "Boom"]);
        assert_eq!(
            f[0].host_rewards,
            vec![Reward {
                kind: "CREDITS".into(),
                params: vec!["500".into()]
            }]
        );
        assert_eq!(s.fired_count(), 3);
    }

    #[test]
    fn notify_matches_param_and_requires_activation() {
        let p = plot(
            r#"<Story>
            <Event Name="A"><Event_Type>STORY_ELAPSED</Event_Type><Event_Param1>100</Event_Param1></Event>
            <Event Name="G"><Event_Type>STORY_GENERIC</Event_Type><Event_Param1>Force_X</Event_Param1><Prereq>A</Prereq></Event>
            <Event Name="G0"><Event_Type>STORY_GENERIC</Event_Type><Event_Param1>Free</Event_Param1></Event>
            </Story>"#,
        );
        let mut s = StoryEngine::default();
        s.activate(p, 0, None, 0.0);
        s.tick(0.0);
        assert!(
            s.notify(&[], "Force_X", 1.0).is_empty(),
            "prereq A has not fired"
        );
        assert_eq!(s.notify(&["STORY_GENERIC"], "free", 1.0).len(), 1);
    }

    #[test]
    fn hero_spawns_fire_once_their_prerequisite_has() {
        let p = plot(
            r#"<Story>
            <Event Name="Universal_Story_Start"><Event_Type>STORY_ELAPSED</Event_Type><Event_Param1>0</Event_Param1></Event>
            <Event Name="Spawn_Tyber"><Event_Type>STORY_TRIGGER</Event_Type><Reward_Type>SPAWN_HERO</Reward_Type>
              <Reward_Param1>Tyber_Zann_Javelin</Reward_Param1><Reward_Param2>Ryloth_WW</Reward_Param2><Prereq>Universal_Story_Start</Prereq></Event>
            <Event Name="Grant"><Event_Type>STORY_TRIGGER</Event_Type><Reward_Type>NEW_POWER_FOR_ALL</Reward_Type>
              <Reward_Param1>CR-90_Corvette</Reward_Param1><Reward_Param2>Reflector_Shield</Reward_Param2><Prereq>Universal_Story_Start</Prereq></Event>
            <Event Name="Waits"><Event_Type>STORY_TRIGGER</Event_Type><Reward_Type>CREDITS</Reward_Type><Reward_Param1>5</Reward_Param1></Event>
            </Story>"#,
        );
        let mut s = StoryEngine::default();
        s.activate(p, 0, Some("Underworld".into()), 0.0);
        let mut fired: Vec<String> = s.tick(0.0).into_iter().map(|f| f.event).collect();
        fired.extend(s.tick(1.0).into_iter().map(|f| f.event));
        assert!(fired.iter().any(|e| e == "Spawn_Tyber"), "the hero spawn fires after the story start: {fired:?}");
        assert!(fired.iter().any(|e| e == "Grant"), "a gated trigger fires whatever its reward is: {fired:?}");
        assert!(!fired.iter().any(|e| e == "Waits"), "a STORY_TRIGGER with no prerequisite waits to be triggered by name");
    }

    #[test]
    fn prereq_tags_are_alternatives_and_names_in_one_tag_are_all_required() {
        let p = plot(
            r#"<Story>
            <Event Name="Win"><Event_Type>STORY_ELAPSED</Event_Type><Event_Param1>0</Event_Param1></Event>
            <Event Name="Lose"><Event_Type>STORY_ELAPSED</Event_Type><Event_Param1>100</Event_Param1></Event>
            <Event Name="Reset"><Event_Type>STORY_ELAPSED</Event_Type><Event_Param1>1</Event_Param1>
              <Prereq>Win</Prereq><Prereq>Lose</Prereq></Event>
            <Event Name="Both"><Event_Type>STORY_ELAPSED</Event_Type><Event_Param1>1</Event_Param1>
              <Prereq>Win, Lose</Prereq></Event>
            </Story>"#,
        );
        let mut s = StoryEngine::default();
        s.activate(p, 0, None, 0.0);
        assert_eq!(s.tick(0.0).iter().map(|f| f.event.as_str()).collect::<Vec<_>>(), vec!["Win"]);
        let at_one: Vec<_> = s.tick(1.0).into_iter().map(|f| f.event).collect();
        assert!(at_one.iter().any(|e| e == "Reset"), "one of two Prereq tags is enough: {at_one:?}");
        assert!(!at_one.iter().any(|e| e == "Both"), "Win, Lose in one tag needs both: {at_one:?}");
        assert!(s.tick(100.0).iter().any(|f| f.event == "Lose"));
        assert!(s.tick(101.0).iter().any(|f| f.event == "Both"), "both names of the one tag have fired");
    }

    #[test]
    fn disable_branch_uses_the_branch_tag_and_param2() {
        // Story_Sandbox_Empire_Dreadnoughts.xml: repair sets DISABLE_BRANCH of Piett_P2_Death_Branch to 1, the spawn sets it back to 0.
        let p = plot(
            r#"<Story>
            <Event Name="Phase2_Piett_Death">
              <Event_Type>STORY_TACTICAL_DESTROY</Event_Type>
              <Event_Param1>Piett_Executor_P2</Event_Param1>
              <Reward_Type>REMOVE_UNIT</Reward_Type><Reward_Param1>Piett_Executor_P2</Reward_Param1>
              <Branch>Piett_P2_Death_Branch</Branch></Event>
            <Event Name="Construct_Repair_Piett_P2">
              <Event_Type>STORY_ELAPSED</Event_Type><Event_Param1>0</Event_Param1>
              <Reward_Type>DISABLE_BRANCH</Reward_Type>
              <Reward_Param1>Piett_P2_Death_Branch</Reward_Param1><Reward_Param2>1</Reward_Param2></Event>
            <Event Name="Enable_Phase1_Piett_Death">
              <Event_Type>STORY_ELAPSED</Event_Type><Event_Param1>10</Event_Param1>
              <Reward_Type>DISABLE_BRANCH</Reward_Type>
              <Reward_Param1>Piett_P2_Death_Branch</Reward_Param1><Reward_Param2>0</Reward_Param2>
              <Prereq>Construct_Repair_Piett_P2</Prereq></Event>
            <Event Name="Reset_Phase1_Piett_Death">
              <Event_Type>STORY_ELAPSED</Event_Type><Event_Param1>5</Event_Param1>
              <Reward_Type>RESET_BRANCH</Reward_Type><Reward_Param1>Piett_P2_Death_Branch</Reward_Param1>
              <Prereq>Enable_Phase1_Piett_Death</Prereq></Event>
            </Story>"#,
        );
        let mut s = StoryEngine::default();
        s.activate(p, 0, Some("Empire".into()), 0.0);
        s.tick(0.0);
        assert!(s.notify(&["STORY_TACTICAL_DESTROY"], "Piett_Executor_P2", 1.0).is_empty(), "param 1 disables the branch");
        s.tick(10.0);
        let back = s.notify(&["STORY_TACTICAL_DESTROY"], "Piett_Executor_P2", 11.0);
        assert!(back.iter().any(|f| f.event == "Phase2_Piett_Death"), "param 0 allows the death event again: {back:?}");
        assert!(s.notify(&["STORY_TACTICAL_DESTROY"], "Piett_Executor_P2", 12.0).is_empty(), "it has fired");
        s.tick(15.0);
        let again = s.notify(&["STORY_TACTICAL_DESTROY"], "Piett_Executor_P2", 16.0);
        assert!(again.iter().any(|f| f.event == "Phase2_Piett_Death"), "RESET_BRANCH clears the firing so the next destroy counts: {again:?}");
    }
}
