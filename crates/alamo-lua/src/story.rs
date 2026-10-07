//! Story plots (`Xml/Story_*.xml`): events with prerequisites and rewards, driving the story scripts.
//!
//! A plot is a list of `Event`s. An event becomes *active* when all its `Prereq` events have fired and
//! *fires* when its condition is met (`STORY_ELAPSED`: that many seconds after activation; every other
//! type: when notified, see [`StoryEngine::notify`]). Firing calls `Story_Event_Trigger(name)` in the
//! plot's Lua script and applies the rewards. Rewards that only manipulate the story graph
//! (`TRIGGER_EVENT`, `RESET_BRANCH`, `DISABLE_BRANCH`, `DISABLE_STORY_EVENT`) are handled here, all
//! others are handed to the host.
//!
//! **Semantics are reconstructed**, not verified against the engine: in particular `STORY_TRIGGER` events
//! fire only when another event triggers them by name, and several `Prereq` tags mean "all of them".

use std::collections::HashMap;

use alamo_data::xml;

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
    /// `Prereq` event names.
    pub prereqs: Vec<String>,
    /// Rewards in file order.
    pub rewards: Vec<Reward>,
    /// `Perpetual`: the event can fire again after firing.
    pub perpetual: bool,
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
                    "prereq" => ev.prereqs.push(text),
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

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
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
    by_name: HashMap<String, usize>,
}

/// An event that fired: the runtime calls the script and hands the rewards to the host.
#[derive(Debug, Clone)]
pub struct Fired {
    /// Script instance to notify.
    pub script: usize,
    /// Event name.
    pub event: String,
    /// Plot owner faction.
    pub faction: Option<String>,
    /// Rewards for the host (graph-only rewards already applied).
    pub host_rewards: Vec<Reward>,
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
                    let ready = r.plot.events[ei].prereqs.iter().all(|p| {
                        r.by_name
                            .get(&p.to_ascii_lowercase())
                            .is_some_and(|&i| r.states[i] == State::Fired)
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
                "STORY_TRIGGER" => r.triggered[ei],
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
        let rewards = self.plots[pi].plot.events[ei].rewards.clone();
        let mut host_rewards = Vec::new();
        for rw in rewards {
            let p1 = rw.params.first().cloned().unwrap_or_default();
            match rw.kind.as_str() {
                "TRIGGER_EVENT" => {
                    for r in &mut self.plots {
                        if let Some(&i) = r.by_name.get(&p1.to_ascii_lowercase()) {
                            r.triggered[i] = true;
                        }
                    }
                }
                "RESET_BRANCH" => self.reset_branch(&p1, now),
                "DISABLE_BRANCH" | "DISABLE_STORY_EVENT" => {
                    self.disable(&p1, rw.kind == "DISABLE_BRANCH")
                }
                _ => host_rewards.push(rw),
            }
        }
        let r = &self.plots[pi];
        out.push(Fired {
            script: r.script,
            event: r.plot.events[ei].name.clone(),
            faction: r.faction.clone(),
            host_rewards,
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
                if !set.contains(&ei) && e.prereqs.iter().any(|p| p.to_ascii_lowercase() == name) {
                    set.push(ei);
                }
            }
            i += 1;
        }
        set
    }

    fn reset_branch(&mut self, name: &str, now: f64) {
        for r in &mut self.plots {
            if let Some(&root) = r.by_name.get(&name.to_ascii_lowercase()) {
                for i in Self::branch(r, root) {
                    if r.states[i] != State::Disabled {
                        r.states[i] = State::Inactive;
                        r.triggered[i] = false;
                    }
                }
                let _ = now;
            }
        }
    }

    fn disable(&mut self, name: &str, whole_branch: bool) {
        for r in &mut self.plots {
            if let Some(&root) = r.by_name.get(&name.to_ascii_lowercase()) {
                let set = if whole_branch {
                    Self::branch(r, root)
                } else {
                    vec![root]
                };
                for i in set {
                    r.states[i] = State::Disabled;
                }
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
}
