//! A loaded script goes on where it was saved (see `alamo_lua::persist`): the state machine does not enter its state again, a `Sleep` goes
//! on with the time it had left, a loop at its index, and what the game sees after the load is what it sees in a game that was never saved.
//! Synthetic scripts with the contract of the game's libraries (`PGBase.lua`, `PGStateMachine.lua`), in-memory files.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::rc::Rc;
use std::sync::Arc;

use alamo_fs::{FileSource, normalize};
use alamo_lua::{LuaVal, MockHost, PlayerId, PlayerInfo, RuntimeSnapshot, ScriptHost, ScriptRuntime};

struct MemFs(HashMap<String, Vec<u8>>);

impl FileSource for MemFs {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        self.0.get(&normalize(path)).cloned().ok_or_else(|| io::ErrorKind::NotFound.into())
    }
    fn exists(&self, path: &str) -> bool {
        self.0.contains_key(&normalize(path))
    }
    fn list(&self, dir: &str) -> Vec<String> {
        let p = normalize(dir);
        self.0.keys().filter(|k| k.starts_with(&p)).cloned().collect()
    }
}

/// `PumpEvents` and `Sleep` as `PGBase.lua` has them (the sleep start in `ThreadValue`), and the state machine of `PGStateMachine.lua`.
const LIBRARY: &str = r#"
function PumpEvents()
  if ThreadValue("InPumpEvents") then error("already in pump events") end
  ThreadValue.Set("InPumpEvents", true)
  coroutine.yield(true)
  ThreadValue.Set("InPumpEvents", false)
end
function Sleep(t)
  ThreadValue.Set("StartTime", GetCurrentTime())
  while GetCurrentTime() - ThreadValue("StartTime") < t do PumpEvents() end
end
function Define_State(state, f)
  StateMachine[state] = f
  if NextState == nil then NextState = state end
end
function Process_States()
  while NextState ~= nil do
    local _curproc = StateMachine[NextState]
    CurrentState = NextState
    _curproc(OnEnter)
    while CurrentState == NextState do
      _curproc(OnUpdate)
      if CurrentState == NextState then PumpEvents() end
    end
    _curproc(OnExit)
  end
end
function Base_Definitions()
  StateMachine = {}
  OnEnter, OnUpdate, OnExit = 1, 2, 3
  NextState, CurrentState = nil, nil
  if Definitions then Definitions() end
end
function bump(key) GlobalValue.Set(key, (GlobalValue.Get(key) or 0) + 1) end
function say(what) Game_Message(string.format("%s at %.2f", what, GetCurrentTime())) end
"#;

/// `Start` pays 100, counts itself and starts a worker; the worker pays 10, then beats every half second and moves the machine on to
/// `Second` when the game says `go`. Leaving `Start` and entering `Second` are counted.
const STORY: &str = r#"
function Definitions()
  Define_State("Start", Start)
  Define_State("Second", Second)
end
function main() Process_States() end
function Start(message)
  if message == OnEnter then
    Find_Player("Rebel").Give_Money(100)
    bump("start")
    worker = Create_Thread("Worker", 0.5)
  elseif message == OnExit then
    bump("left start")
  end
end
function Second(message)
  if message == OnEnter then bump("second"); say("second") end
end
function Worker(tick)
  Find_Player("Rebel").Give_Money(10)
  beats = 0
  while true do
    Sleep(tick)
    beats = beats + 1
    say("beat " .. beats)
    if GlobalValue.Get("go") == 1 then NextState = "Second" end
  end
end
"#;

/// The mock galaxy, with the messages the scripts show kept where the test can read them.
struct Recorder {
    inner: MockHost,
    messages: Rc<RefCell<Vec<String>>>,
}

impl ScriptHost for Recorder {
    fn players(&self) -> Vec<PlayerId> {
        self.inner.players()
    }
    fn player(&self, id: PlayerId) -> Option<PlayerInfo> {
        self.inner.player(id)
    }
    fn give_money(&mut self, player: PlayerId, amount: f32) {
        self.inner.give_money(player, amount);
        self.messages.borrow_mut().push(format!("paid {amount}"));
    }
    fn message(&mut self, text: &str) {
        self.messages.borrow_mut().push(text.to_string());
    }
}

struct Game {
    rt: ScriptRuntime,
    messages: Rc<RefCell<Vec<String>>>,
}

fn game(script: &str) -> Game {
    let files = HashMap::from([(normalize("Scripts/Story.lua"), format!("{LIBRARY}{script}").into_bytes())]);
    let messages = Rc::new(RefCell::new(Vec::new()));
    let host = Recorder { inner: MockHost::galactic_demo(), messages: messages.clone() };
    let mut rt = ScriptRuntime::new(Arc::new(MemFs(files)), Box::new(host));
    rt.spawn_script("Story", &[]).expect("the script starts");
    Game { rt, messages }
}

impl Game {
    fn run(&mut self, seconds: f64) {
        for _ in 0..(seconds * 10.0).round() as u32 {
            self.rt.update(0.1);
        }
        assert!(self.rt.errors().is_empty(), "{:?}", self.rt.errors());
    }

    /// What the scripts showed and paid so far.
    fn messages(&self) -> Vec<String> {
        self.messages.borrow().clone()
    }

    /// Credits the scripts paid the Rebel player (5000 at the start of the mock galaxy).
    fn paid(&self) -> f32 {
        self.rt.with_host(|h| h.player(0).expect("Rebel").credits) - 5000.0
    }

    fn global(&self, key: &str) -> Option<f64> {
        self.rt.global_number(key)
    }
}

fn loaded(script: &str, snap: &RuntimeSnapshot) -> Game {
    let mut g = game(script);
    let warnings = g.rt.restore(snap);
    assert!(warnings.is_empty(), "{warnings:?}");
    g
}

fn global<'a>(snap: &'a RuntimeSnapshot, key: &str) -> Option<&'a LuaVal> {
    snap.scripts[0].globals.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn text(s: &str) -> LuaVal {
    LuaVal::Str(s.into())
}

/// Plays `script` for `before` seconds, saves, and plays on for `after` seconds, once in the same game and once in a game loaded from the
/// save. Returns the save, the game that was never saved and the loaded game.
fn save_and_load(script: &str, before: f64, after: f64) -> (RuntimeSnapshot, Game, Game) {
    let mut original = game(script);
    original.run(before);
    let snap = original.rt.snapshot();
    let mut again = loaded(script, &snap);
    assert_eq!(again.rt.snapshot(), snap, "a load changes nothing");
    let seen = original.messages().len();
    original.messages.borrow_mut().drain(..seen);
    original.run(after);
    again.run(after);
    (snap, original, again)
}

#[test]
fn a_script_saved_in_an_entered_state_goes_on_in_it() {
    let (snap, original, again) = save_and_load(STORY, 2.05, 3.0);
    assert_eq!((global(&snap, "CurrentState"), global(&snap, "beats")), (Some(&text("Start")), Some(&LuaVal::Num(3.0))));
    let threads: Vec<(Option<&str>, usize)> = snap.scripts[0].threads.iter().flatten().map(|t| (t.function.as_deref(), t.stack.len())).collect();
    assert_eq!(threads, [(Some("main"), 3), (Some("Worker"), 3)], "main waits in PumpEvents of Process_States, the worker in PumpEvents of its Sleep");
    assert_eq!(again.paid(), 0.0, "neither the entry code of the state nor the start of the worker paid again");
    assert_eq!(again.global("start"), Some(1.0), "the state was not entered again");
    assert_eq!(again.messages(), original.messages(), "the beats come at the times of the game that was never saved");
    assert!(again.messages().len() >= 4, "the worker beats on: {:?}", again.messages());
    assert_eq!(again.rt.snapshot(), original.rt.snapshot(), "and both games are in the same place");
}

#[test]
fn a_state_reached_after_the_load_is_entered_once() {
    let (_, mut original, mut again) = save_and_load(STORY, 2.05, 1.0);
    for g in [&mut original, &mut again] {
        g.rt.set_global_value("go", 1.0);
        g.run(2.0);
        assert_eq!((g.global("start"), g.global("left start"), g.global("second")), (Some(1.0), Some(1.0), Some(1.0)));
    }
    assert_eq!(again.messages(), original.messages());
    // Saved in the new state and loaded again: it is not entered a second time.
    let snap = again.rt.snapshot();
    assert_eq!(global(&snap, "CurrentState"), Some(&text("Second")));
    let mut third = loaded(STORY, &snap);
    third.run(2.0);
    assert_eq!((third.global("start"), third.global("second")), (Some(1.0), Some(1.0)));
    assert!(!third.messages().iter().any(|m| m.starts_with("second")), "{:?}", third.messages());
}

/// The worker asked for the next state after `main` had waited in this service; the save came before `main` ran again. The loaded game
/// leaves `Start` and enters `Second` once, as the game that was never saved does.
#[test]
fn a_change_of_state_asked_for_before_the_save_happens_after_the_load() {
    let mut original = game(STORY);
    original.run(1.0);
    original.rt.set_global_value("go", 1.0);
    let snap = loop {
        original.run(0.1);
        let snap = original.rt.snapshot();
        if global(&snap, "NextState") == Some(&text("Second")) {
            break snap;
        }
    };
    assert_eq!(global(&snap, "CurrentState"), Some(&text("Start")));
    let mut again = loaded(STORY, &snap);
    original.messages.borrow_mut().clear();
    original.run(1.0);
    again.run(1.0);
    for g in [&original, &again] {
        assert_eq!((g.global("start"), g.global("left start"), g.global("second")), (Some(1.0), Some(1.0), Some(1.0)));
    }
    assert_eq!(again.messages(), original.messages());
    assert_eq!(again.paid(), 0.0);
}

/// Saved while the entry code of a state sleeps: after the load it finishes once, at the time it would have.
#[test]
fn entry_code_that_was_still_running_finishes_once() {
    const SLOW: &str = r#"
function Definitions() Define_State("Start", Start) end
function main() Process_States() end
function Start(message)
  if message == OnEnter then
    Find_Player("Rebel").Give_Money(1)
    Sleep(3.0)
    Find_Player("Rebel").Give_Money(1000)
    say("entered")
  end
end
"#;
    let (snap, original, again) = save_and_load(SLOW, 1.0, 4.0);
    assert_eq!(snap.scripts[0].threads.as_ref().unwrap()[0].stack.len(), 5, "PumpEvents, Sleep, Start, Process_States, main");
    assert_eq!(again.paid(), 1000.0, "the first payment is not made again, the second is made");
    assert_eq!(again.messages(), original.messages());
    assert_eq!(again.messages(), ["paid 1000", "entered at 3.50"]);
}

/// A script without a state machine (an object script, `GameScoring`): a counted loop with sleeps goes on at its index, with the time its
/// sleep had left, and a thread it started goes on too.
#[test]
fn a_loop_goes_on_at_its_index() {
    const LOOP: &str = r#"
function main()
  Create_Thread(Ticker)
  for i = 1, 6 do
    Find_Player("Rebel").Give_Money(i)
    say("step " .. i)
    Sleep(0.7)
  end
  say("done")
end
function Ticker()
  local n = 0
  while true do
    n = n + 1
    if n % 5 == 0 then say("tick " .. n) end
    PumpEvents()
  end
end
"#;
    let (snap, original, again) = save_and_load(LOOP, 1.65, 4.0);
    let ticker = &snap.scripts[0].threads.as_ref().unwrap()[1];
    assert_eq!(ticker.function.as_deref(), Some("Ticker"), "a thread started with a function value is saved by the global that names it");
    assert_eq!(again.messages(), original.messages());
    assert_eq!((again.paid(), original.paid()), (4.0 + 5.0 + 6.0, 21.0), "steps 4 to 6 are paid after the load, 1 to 3 are not paid again");
    assert!(again.messages().iter().any(|m| m.starts_with("done")));
}

/// A save of a build that kept no threads. The entry code of the state is run with the game shut off: it pays nothing and counts nothing
/// a second time, but it starts the worker again, which no other part of the save could bring back.
#[test]
fn a_save_without_threads_repeats_no_entry_code() {
    let mut original = game(STORY);
    original.run(2.05);
    let mut snap = original.rt.snapshot();
    for s in &mut snap.scripts {
        (s.threads, s.service) = (None, None);
    }
    snap.next_thread = None;
    let mut again = loaded(STORY, &snap);
    let before = again.messages().len();
    again.run(2.0);
    assert_eq!(again.paid(), 0.0, "nothing reached the game a second time");
    assert_eq!(again.global("start"), Some(1.0));
    assert!(again.messages().len() >= before + 3, "the worker runs again: {:?}", again.messages());
    let after = again.rt.snapshot();
    let threads = after.scripts[0].threads.as_ref().expect("from here on the save keeps the threads");
    assert_eq!(threads.len(), 2, "main and one worker");
    // The worker started over with its global as saved: beats counts on from 4.
    assert!(again.messages()[0].starts_with("beat 4"), "{:?}", again.messages());
}

/// A script that had ended does not run again in the loaded game.
#[test]
fn an_ended_script_stays_ended() {
    const ONCE: &str = r#"
function main()
  Find_Player("Rebel").Give_Money(7)
end
"#;
    let mut original = game(ONCE);
    original.run(1.0);
    assert_eq!(original.paid(), 7.0);
    let snap = original.rt.snapshot();
    assert!(snap.scripts[0].ended);
    let mut again = loaded(ONCE, &snap);
    again.run(1.0);
    assert_eq!(again.paid(), 0.0);
    assert_eq!(again.rt.alive_scripts(), 0);
}


