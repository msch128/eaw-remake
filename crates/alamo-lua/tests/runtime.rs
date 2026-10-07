//! Runtime behaviour tests with synthetic scripts and an in-memory file system.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use alamo_fs::{FileSource, normalize};
use alamo_lua::{MockHost, ScriptRuntime, ScriptValue};

struct MemFs(HashMap<String, Vec<u8>>);

impl FileSource for MemFs {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        self.0
            .get(&normalize(path))
            .cloned()
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }
    fn exists(&self, path: &str) -> bool {
        self.0.contains_key(&normalize(path))
    }
    fn list(&self, dir: &str) -> Vec<String> {
        let p = normalize(dir);
        self.0
            .keys()
            .filter(|k| k.starts_with(&p))
            .cloned()
            .collect()
    }
}

fn runtime(files: &[(&str, &str)]) -> ScriptRuntime {
    let map = files
        .iter()
        .map(|(p, c)| (normalize(p), c.as_bytes().to_vec()))
        .collect();
    ScriptRuntime::new(Arc::new(MemFs(map)), Box::new(MockHost::galactic_demo()))
}

fn run(rt: &mut ScriptRuntime, seconds: f64) {
    let mut t = 0.0;
    while t < seconds {
        rt.update(0.1);
        t += 0.1;
    }
}

const PUMP: &str = r#"
function PumpEvents() coroutine.yield(true) end
function Sleep(s)
  local start = GetCurrentTime()
  while GetCurrentTime() - start < s do PumpEvents() end
end
"#;

#[test]
fn require_returns_module_value_and_arg_compat_works() {
    let mut rt = runtime(&[
        ("Scripts/Library/Mod.lua", "return { answer = 42 }"),
        (
            "Scripts/Test.lua",
            r#"
            local m = require("mod")
            assert(m.answer == 42)
            assert(require("MOD") == m)          -- cached, case-insensitive
            function f(...) return table.getn(arg) end   -- Lua 5.0 style vararg
            assert(f(1, 2, 3) == 3)
            collectgarbage(128)                   -- Lua 5.0 style
            function main() GlobalValue.Set("done", 1) end
            "#,
        ),
    ]);
    rt.spawn_script("Test", &[]).unwrap();
    run(&mut rt, 1.0);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(rt.alive_scripts(), 0, "main returned, script ends");
    assert_eq!(rt.api_hits().get("GlobalValue.Set"), Some(&1));
}

#[test]
fn threads_sleep_and_scheduler() {
    let mut rt = runtime(&[(
        "Scripts/T.lua",
        &format!(
            r#"{PUMP}
            function Worker(tag)
              Sleep(1.0)
              GlobalValue.Set("worker_" .. tag, GetCurrentTime())
            end
            function main()
              Create_Thread("Worker", "a")
              Create_Thread(Worker, "b")
              Sleep(3.0)
              GlobalValue.Set("main_done", GetCurrentTime())
            end
            "#
        ),
    )]);
    rt.spawn_script("T", &[]).unwrap();
    run(&mut rt, 2.0);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(rt.alive_scripts(), 1);
    run(&mut rt, 3.0);
    assert_eq!(rt.alive_scripts(), 0);
    let hits = rt.api_hits();
    assert_eq!(
        hits.get("GlobalValue.Set"),
        Some(&3),
        "two workers and main finished: {hits:?}"
    );
}

#[test]
fn unknown_api_is_logged_once_and_returns_nil() {
    let mut rt = runtime(&[(
        "Scripts/U.lua",
        r#"
        function main()
          local a = Find_Hint("x", "y")        -- known engine name, not implemented
          local b = Find_Hint("x", "y")
          assert(a == nil and b == nil)
          Fade_Screen_In(1)                     -- presentation call: ignored silently
          local p = Find_Player("Rebel")
          assert(p.Get_Faction_Name() == "REBEL")
          assert(p:Get_Faction_Name() == "REBEL")   -- colon syntax works too
          assert(p.Some_Unknown_Method() == nil)
          assert(p.Is_Valid())
        end
        "#,
    )]);
    rt.spawn_script("U", &[]).unwrap();
    run(&mut rt, 1.0);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let unknown = rt.unimplemented();
    assert!(
        unknown.contains("Find_Hint") && unknown.contains(":Some_Unknown_Method"),
        "{unknown:?}"
    );
    assert!(!unknown.contains("Fade_Screen_In"));
    assert_eq!(rt.api_hits().get("Find_Hint"), Some(&2));
    assert_eq!(
        rt.log().iter().filter(|l| l.contains("Find_Hint")).count(),
        1,
        "logged once"
    );
}

#[test]
fn spawn_and_object_methods_go_through_the_host() {
    let mut rt = runtime(&[(
        "Scripts/S.lua",
        r#"
        function main()
          local rebel = Find_Player("Rebel")
          local empire = Find_Player("Empire")
          local planet = FindPlanet("Coruscant")
          assert(planet.Get_Owner() == empire)
          local ships = Spawn_Unit(Find_Object_Type("Star_Destroyer"), planet, rebel)
          assert(table.getn(ships) == 1)
          local s = ships[1]
          assert(s.Get_Owner() == rebel and s.Get_Planet_Location() == planet)
          s.Change_Owner(empire)
          assert(s.Get_Owner() == empire)
          assert(table.getn(Find_All_Objects_Of_Type("Star_Destroyer")) == 1)
          s.Despawn()
          assert(not s.Is_Valid())
          rebel.Give_Money(250)
          assert(rebel.Get_Credits() == 5250)
          Game_Message("hello")
        end
        "#,
    )]);
    rt.spawn_script("S", &[]).unwrap();
    run(&mut rt, 1.0);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt.with_host(|h| {
        let _ = h;
    });
}

#[test]
fn story_plot_drives_story_event_trigger() {
    let mut rt = runtime(&[
        (
            "Xml/Story_Plots_T.xml",
            "<Story_Mode_Plots><Lua_Script>Story_T</Lua_Script><Active_Plot>Plot_T.xml</Active_Plot></Story_Mode_Plots>",
        ),
        (
            "Xml/Plot_T.xml",
            r#"<Story>
              <Event Name="Universal_Story_Start"><Event_Type>STORY_ELAPSED</Event_Type><Event_Param1>0</Event_Param1></Event>
              <Event Name="Later"><Event_Type>STORY_ELAPSED</Event_Type><Event_Param1>5</Event_Param1><Prereq>Universal_Story_Start</Prereq>
                <Reward_Type>CREDITS</Reward_Type><Reward_Param1>100</Reward_Param1></Event>
              <Event Name="Ping"><Event_Type>STORY_AI_NOTIFICATION</Event_Type><Event_Param2>PING_ME</Event_Param2>
                <Reward_Type>TRIGGER_AI</Reward_Type><Reward_Param1>PongFlag</Reward_Param1><Reward_Param2>Rebel</Reward_Param2>
                <Perpetual>True</Perpetual></Event>
            </Story>"#,
        ),
        (
            "Scripts/Story/Story_T.lua",
            &format!(
                r#"{PUMP}
                seen = {{}}
                function Story_Event_Trigger(name) table.insert(seen, name) end
                function main()
                  while true do
                    if table.getn(seen) >= 2 and not pinged then
                      pinged = true
                      Story_Event("PING_ME")
                      Story_Event("PING_ME")
                    end
                    if Check_Story_Flag(Find_Player("Rebel"), "PongFlag", nil, true) then
                      GlobalValue.Set("pong", table.getn(seen))
                    end
                    PumpEvents()
                  end
                end
                "#
            ),
        ),
    ]);
    assert_eq!(
        rt.load_story_plots("Story_Plots_T.xml", Some("Rebel"))
            .unwrap(),
        1
    );
    run(&mut rt, 8.0);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert_eq!(
        rt.story_fired(),
        2,
        "Start and Later; the perpetual Ping re-arms instead of staying fired"
    );
    assert_eq!(
        rt.api_hits().get("GlobalValue.Set"),
        Some(&1),
        "flag is consumed on first read"
    );
}

#[test]
fn script_errors_are_reported_not_fatal() {
    let mut rt = runtime(&[
        (
            "Scripts/Bad.lua",
            &format!("{PUMP}\nfunction main() PumpEvents() local x = nil; x.y = 1 end"),
        ),
        (
            "Scripts/Good.lua",
            &format!("{PUMP}\nfunction main() while true do PumpEvents() end end"),
        ),
    ]);
    rt.spawn_script("Bad", &[]).unwrap();
    rt.spawn_script("Good", &[("Object", ScriptValue::Str("x".into()))])
        .unwrap();
    run(&mut rt, 2.0);
    assert_eq!(rt.errors().len(), 1);
    assert_eq!(rt.errors()[0].script, "Bad");
    assert_eq!(rt.alive_scripts(), 1);
}
