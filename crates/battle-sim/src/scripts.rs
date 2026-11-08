//! What battles need from the Remake's object scripts (`Data/Scripts/GameObject/*.lua`), read from the script text without running it.
//! No battle Lua runs in the engine; the script shapes below are recognised and turned into data:
//! * spawn scripts (`Reinforcements_*`, `Reinforcement_Subjugator-*`): `State_Init` rolls `GameRandom(lo, hi)` and calls
//!   `SpawnReinforcements(list, "Faction")` for the outcome (one of the `assault_list` tables), which spawns the list's units one by one
//!   at the objects `Find_All_Objects_Of_Type` returns, each with `Cinematic_Hyperspace_In(frames)` and `Sleep(a + GameRandom(b, c) / d)`
//!   between two units.

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Num(f32),
    Sym(char),
}

/// Splits Lua source into identifiers, strings, numbers and single-character symbols; comments are dropped.
fn lex(src: &str) -> Vec<Tok> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'-' && b.get(i + 1) == Some(&b'-') {
            i += 2;
            if b.get(i) == Some(&b'[') && b.get(i + 1) == Some(&b'[') {
                i = src[i..].find("]]").map_or(b.len(), |k| i + k + 2);
            } else {
                i = src[i..].find('\n').map_or(b.len(), |k| i + k + 1);
            }
        } else if c == b'"' || c == b'\'' {
            let mut s = String::new();
            i += 1;
            while i < b.len() && b[i] != c {
                if b[i] == b'\\' && i + 1 < b.len() {
                    i += 1;
                }
                s.push(b[i] as char);
                i += 1;
            }
            i += 1;
            out.push(Tok::Str(s));
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            out.push(Tok::Num(src[start..i].parse().unwrap_or(0.0)));
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push(Tok::Ident(src[start..i].to_string()));
        } else {
            out.push(Tok::Sym(c as char));
            i += 1;
        }
    }
    out
}

fn is_ident(t: Option<&Tok>, name: &str) -> bool {
    matches!(t, Some(Tok::Ident(n)) if n == name)
}

fn is_sym(t: Option<&Tok>, c: char) -> bool {
    matches!(t, Some(Tok::Sym(s)) if *s == c)
}

fn num(t: Option<&Tok>) -> Option<f32> {
    match t {
        Some(Tok::Num(v)) => Some(*v),
        _ => None,
    }
}

fn string(t: Option<&Tok>) -> Option<&str> {
    match t {
        Some(Tok::Str(s)) => Some(s),
        _ => None,
    }
}

/// Token range of the body of `function <name>`: from after the name to the next `function` keyword (or the end).
fn function_body<'a>(toks: &'a [Tok], name: &str) -> &'a [Tok] {
    let Some(start) = (0..toks.len().saturating_sub(1)).find(|&i| is_ident(toks.get(i), "function") && is_ident(toks.get(i + 1), name)) else {
        return &[];
    };
    let end = (start + 2..toks.len()).find(|&i| is_ident(toks.get(i), "function")).unwrap_or(toks.len());
    &toks[start + 2..end]
}

/// The first argument of every `name(...)` call in `toks`, as raw tokens, and the call's last string argument.
fn calls<'a>(toks: &'a [Tok], name: &str) -> Vec<(&'a [Tok], Option<&'a str>)> {
    let mut out = Vec::new();
    for i in 0..toks.len() {
        if !is_ident(toks.get(i), name) || !is_sym(toks.get(i + 1), '(') {
            continue;
        }
        let (mut depth, mut j, mut first_end) = (0i32, i + 1, None);
        while j < toks.len() {
            match &toks[j] {
                Tok::Sym('(' | '[' | '{') => depth += 1,
                Tok::Sym(')' | ']' | '}') => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                Tok::Sym(',') if depth == 1 && first_end.is_none() => first_end = Some(j),
                _ => {}
            }
            j += 1;
        }
        let first = &toks[i + 2..first_end.unwrap_or(j).max(i + 2)];
        let last = toks[i + 2..j.min(toks.len())].iter().rev().find_map(|t| string(Some(t)));
        out.push((first, last));
    }
    out
}

/// `name = { "A", "B" }` tables: a flat table gives one list, a table of tables (`{ {"A"}, {"B"} }`) one list per inner table.
fn tables(toks: &[Tok]) -> Vec<(String, Vec<Vec<String>>, bool)> {
    let mut out = Vec::new();
    for i in 0..toks.len() {
        let (Some(Tok::Ident(name)), true, true) = (toks.get(i), is_sym(toks.get(i + 1), '='), is_sym(toks.get(i + 2), '{')) else { continue };
        let (mut depth, mut flat, mut inner, mut nested) = (0, Vec::new(), Vec::new(), Vec::new());
        for t in &toks[i + 2..] {
            match t {
                Tok::Sym('{') => depth += 1,
                Tok::Sym('}') => {
                    depth -= 1;
                    if depth == 1 {
                        nested.push(std::mem::take(&mut inner));
                    }
                    if depth == 0 {
                        break;
                    }
                }
                Tok::Str(s) if depth == 1 => flat.push(s.clone()),
                Tok::Str(s) if depth == 2 => inner.push(s.clone()),
                _ => {}
            }
        }
        if nested.is_empty() {
            out.push((name.clone(), vec![flat], false));
        } else {
            out.push((name.clone(), nested, true));
        }
    }
    out
}

/// `sleep_time = base + (GameRandom(lo, hi) / div)` between two spawned units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnGap {
    /// Fixed part in seconds.
    pub base: f32,
    /// Lowest roll.
    pub lo: i32,
    /// Highest roll.
    pub hi: i32,
    /// The roll is divided by this.
    pub div: f32,
}

/// A spawn script as the battle runs it.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnScript {
    /// One entry per outcome of the script's roll, in roll order: the unit names that outcome spawns (empty = the outcome spawns nothing).
    pub choices: Vec<Vec<String>>,
    /// `Find_All_Objects_Of_Type` argument: the object type (or map marker name) the units are spawned at.
    pub marker: String,
    /// `Find_Player` faction the units are spawned for.
    pub faction: String,
    /// Seconds `State_Init` sleeps before the first unit.
    pub delay: f32,
    /// `Cinematic_Hyperspace_In` frames.
    pub hyperspace_frames: f32,
    /// Pause between two units.
    pub gap: SpawnGap,
}

impl SpawnScript {
    /// Every unit name the script can spawn.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.choices.iter().flatten().map(String::as_str)
    }
}

/// Reads a spawn script; `None` when the source is not one (no `SpawnReinforcements` with `Spawn_Unit`, or no unit list).
pub fn parse_spawn_script(src: &str) -> Option<SpawnScript> {
    let toks = lex(src);
    let spawner = function_body(&toks, "SpawnReinforcements");
    if !spawner.iter().any(|t| is_ident(Some(t), "Spawn_Unit")) {
        return None;
    }
    let lists = tables(&toks);
    let list = |name: &str| lists.iter().find(|l| l.0 == name);
    let init = function_body(&toks, "State_Init");
    let spawns = calls(init, "SpawnReinforcements");
    let first_call = (0..init.len()).find(|&i| is_ident(init.get(i), "SpawnReinforcements"))?;
    let faction = spawns.iter().find_map(|c| c.1)?.to_string();
    // The roll and the branch each call sits in.
    let mut roll: Option<(i32, i32)> = None;
    let mut branch: Option<i32> = None;
    let mut by_branch: Vec<(Option<i32>, &[Tok])> = Vec::new();
    let mut delay = 0.0;
    let mut k: usize = 0;
    while k < init.len() {
        let t = init.get(k);
        if is_ident(t, "GameRandom") && is_sym(init.get(k + 1), '(') && is_sym(init.get(k.wrapping_sub(1)), '=') && !is_sym(init.get(k.wrapping_sub(2)), '=') {
            if let (Some(lo), Some(hi)) = (num(init.get(k + 2)), num(init.get(k + 4))) {
                roll = Some((lo as i32, hi as i32));
            }
        } else if is_sym(t, '=') && is_sym(init.get(k + 1), '=') {
            branch = num(init.get(k + 2)).map(|v| v as i32);
            k += 1;
        } else if is_ident(t, "Sleep") && k < first_call {
            delay += num(init.get(k + 2)).unwrap_or(0.0);
        } else if is_ident(t, "SpawnReinforcements") && is_sym(init.get(k + 1), '(') {
            let arg = calls(&init[k..], "SpawnReinforcements").first().map_or(&[][..], |c| c.0);
            by_branch.push((branch, arg));
        }
        k += 1;
    }
    let lookup = |arg: &[Tok]| -> Vec<Vec<String>> {
        let Some(Tok::Ident(name)) = arg.first() else { return Vec::new() };
        let Some((_, entries, nested)) = list(name) else { return Vec::new() };
        if is_sym(arg.get(1), '[') {
            // `list[GameRandom(1, table.getn(list))]`: one entry at random.
            if *nested { entries.clone() } else { entries.iter().flatten().map(|n| vec![n.clone()]).collect() }
        } else {
            vec![entries.iter().flatten().cloned().collect()]
        }
    };
    let choices: Vec<Vec<String>> = match roll {
        Some((lo, hi)) if by_branch.iter().any(|b| b.0.is_some()) => {
            (lo..=hi).map(|v| by_branch.iter().find(|b| b.0 == Some(v)).map(|b| lookup(b.1).into_iter().flatten().collect()).unwrap_or_default()).collect()
        }
        _ => by_branch.first().map(|b| lookup(b.1)).unwrap_or_default(),
    };
    if choices.iter().all(Vec::is_empty) {
        return None;
    }
    let after = |name: &str, n: usize| (0..spawner.len()).find(|&i| is_ident(spawner.get(i), name)).and_then(|i| spawner.get(i + n));
    let marker = string(after("Find_All_Objects_Of_Type", 2))?.to_string();
    let hyperspace_frames = num(after("Cinematic_Hyperspace_In", 2)).unwrap_or(0.0);
    let gap = (0..spawner.len())
        .find(|&i| is_ident(spawner.get(i), "sleep_time") && is_sym(spawner.get(i + 1), '='))
        .and_then(|i| {
            let n: Vec<f32> = spawner[i + 2..].iter().take(12).filter_map(|t| num(Some(t))).collect();
            (n.len() >= 4).then(|| SpawnGap { base: n[0], lo: n[1] as i32, hi: n[2] as i32, div: n[3].max(1.0) })
        })
        .unwrap_or(SpawnGap { base: 0.0, lo: 0, hi: 0, div: 1.0 });
    Some(SpawnScript { choices, marker, faction, delay, hyperspace_frames, gap })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hand-written scripts in the shape of the Remake's (no game text).
    const ESCORT: &str = r#"
require("PGStateMachine")
function Definitions()
    Define_State("State_Init", State_Init);
    list_a = {"Ship_A", "Ship_B"}
    list_b = {"Ship_C"}
    chances = nil
end
function State_Init(message)
    if Get_Game_Mode() ~= "Space" then
        ScriptExit()
    end
    chances = GameRandom(1, 3)
    if chances == 1 then
        SpawnReinforcements(list_a, "Rebel") -- first
    end
    if chances == 3 then
        SpawnReinforcements(list_b, "Rebel")
    end
    Sleep(10)
    ScriptExit()
end
function SpawnReinforcements(garrison_list, faction_name)
    marker_list = Find_All_Objects_Of_Type("Hero_Ship")
    for _,unit in pairs(garrison_list) do
        spawn = Spawn_Unit(Find_Object_Type(unit), marker, Find_Player(faction_name))
        spawn[1].Cinematic_Hyperspace_In(45)
        sleep_time = 2 + (GameRandom(1,4) / 5)
        Sleep(sleep_time)
    end
end"#;

    #[test]
    fn escort_script_gives_one_choice_per_roll_outcome() {
        let s = parse_spawn_script(ESCORT).unwrap();
        assert_eq!(s.choices, vec![vec!["Ship_A".to_string(), "Ship_B".into()], vec![], vec!["Ship_C".into()]]);
        assert_eq!((s.marker.as_str(), s.faction.as_str(), s.delay, s.hyperspace_frames), ("Hero_Ship", "Rebel", 0.0, 45.0));
        assert_eq!(s.gap, SpawnGap { base: 2.0, lo: 1, hi: 4, div: 5.0 });
    }

    #[test]
    fn indexed_table_of_tables_gives_one_choice_per_entry() {
        let src = r#"
function Definitions()
    groups = {
        -- one group each
        {"Ship_A"},
        {"Ship_B", "Ship_C"},
    }
end
function State_Init(message)
    Sleep(4)
    SpawnReinforcements(groups[GameRandom(1,table.getn(groups))], "Pirates")
end
function SpawnReinforcements(garrison_list, faction_name)
    marker_list = Find_All_Objects_Of_Type("Defending Forces Position")
    spawn = Spawn_Unit(Find_Object_Type(unit), marker, Find_Player(faction_name))
end"#;
        let s = parse_spawn_script(src).unwrap();
        assert_eq!(s.choices, vec![vec!["Ship_A".to_string()], vec!["Ship_B".into(), "Ship_C".into()]]);
        assert_eq!((s.delay, s.marker.as_str()), (4.0, "Defending Forces Position"));
    }

    #[test]
    fn scripts_without_spawns_are_not_spawn_scripts() {
        assert!(parse_spawn_script("function State_Init(message)\n Object.Take_Damage(1)\nend").is_none());
    }
}
