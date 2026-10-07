//! Script inventory: compiles every script and lists the engine API calls scripts make.
//!
//! ```text
//! lua_survey [workshop_id] [--md API.md]
//! ```
//! Prints the report; with `--md` also writes it as Markdown.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_lua::scan::scan;

const AREAS: [&str; 9] = [
    "Story",
    "AI-Galactic",
    "AI-Space",
    "AI-Land",
    "GameObject",
    "Library",
    "Evaluators",
    "FreeStore",
    "Misc",
];

fn area(path: &str) -> &'static str {
    let p = path.to_ascii_uppercase();
    if p.starts_with("SCRIPTS/STORY/") {
        "Story"
    } else if p.starts_with("SCRIPTS/AI/LANDMODE/") {
        "AI-Land"
    } else if p.starts_with("SCRIPTS/AI/SPACEMODE/") {
        "AI-Space"
    } else if p.starts_with("SCRIPTS/AI/") {
        "AI-Galactic"
    } else if p.starts_with("SCRIPTS/GAMEOBJECT/") {
        "GameObject"
    } else if p.starts_with("SCRIPTS/LIBRARY/") {
        "Library"
    } else if p.starts_with("SCRIPTS/EVALUATORS/") {
        "Evaluators"
    } else if p.starts_with("SCRIPTS/FREESTORE/") {
        "FreeStore"
    } else {
        "Misc"
    }
}

const STD: &[&str] = &[
    "pairs",
    "ipairs",
    "type",
    "tostring",
    "tonumber",
    "unpack",
    "require",
    "pcall",
    "error",
    "print",
    "assert",
    "next",
    "select",
    "rawget",
    "rawset",
    "setmetatable",
    "getmetatable",
    "loadstring",
    "dofile",
    "xpcall",
    "collectgarbage",
];
const LIB_METHODS: &[&str] = &[
    "getn", "insert", "remove", "format", "sort", "concat", "foreach", "foreachi", "len", "sub",
    "find", "gsub", "gfind", "gmatch", "lower", "upper", "floor", "ceil", "abs", "max", "min",
    "sqrt", "random", "sin", "cos", "rep", "byte", "char",
];

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let md = args.iter().position(|a| a == "--md").map(|i| {
        args.remove(i);
        args.remove(i)
    });
    let rs = args.iter().position(|a| a == "--rs").map(|i| {
        args.remove(i);
        args.remove(i)
    });
    let id = args.first().cloned();
    let install = GameInstall::detect().expect("game not found");
    let fs = GameFs::for_mod(&install, id.as_deref()).expect("fs");
    let files: Vec<String> = fs
        .list("Scripts")
        .into_iter()
        .filter(|f| f.ends_with(".LUA"))
        .collect();
    let mut report = String::new();
    let mut by_area: BTreeMap<&str, usize> = BTreeMap::new();
    let mut defined = BTreeSet::new();
    let mut assigned = BTreeSet::new();
    let mut scans = Vec::new();
    let mut all_reads: BTreeMap<String, usize> = BTreeMap::new();
    let lua = mlua::Lua::new();
    let (mut ok, mut bad) = (0, 0);
    let mut fails = Vec::new();
    for f in &files {
        let text = String::from_utf8_lossy(&fs.read(f).unwrap()).into_owned();
        *by_area.entry(area(f)).or_default() += 1;
        match lua.load(&text).set_name(f.as_str()).into_function() {
            Ok(_) => ok += 1,
            Err(e) => {
                bad += 1;
                fails.push(format!(
                    "{f}: {}",
                    e.to_string().lines().next().unwrap_or("")
                ));
            }
        }
        let s = scan(&text);
        defined.extend(s.defined.iter().cloned());
        assigned.extend(s.assigned.iter().cloned());
        for (k, n) in &s.reads {
            *all_reads.entry(k.clone()).or_default() += n;
        }
        scans.push((area(f), s));
    }
    let mut calls: BTreeMap<String, BTreeMap<&'static str, usize>> = BTreeMap::new();
    for (ar, s) in scans {
        for (k, n) in s.calls {
            let key = match k.split_once('.') {
                Some((pre, rest)) if assigned.contains(pre) => {
                    format!(":{}", rest.rsplit('.').next().unwrap())
                }
                _ => k,
            };
            *calls.entry(key).or_default().entry(ar).or_default() += n;
        }
    }
    let mut api: Vec<(String, usize, BTreeMap<&str, usize>)> = Vec::new();
    for (k, v) in &calls {
        let bare = k.trim_start_matches(':');
        let last = bare.rsplit('.').next().unwrap();
        if STD.contains(&k.as_str())
            || k.starts_with("string.")
            || k.starts_with("table.")
            || k.starts_with("math.")
            || k.starts_with("coroutine.")
            || k.starts_with("os.")
            || (k.starts_with(':') && LIB_METHODS.contains(&last))
            || defined.contains(k)
            || defined.contains(bare)
            || defined.contains(last)
        {
            continue;
        }
        api.push((k.clone(), v.values().sum(), v.clone()));
    }
    api.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    // Which XML tags reference script files?
    let basenames: BTreeSet<String> = files
        .iter()
        .map(|f| {
            f.rsplit('/')
                .next()
                .unwrap()
                .trim_end_matches(".LUA")
                .to_ascii_lowercase()
        })
        .collect();
    let mut tag_refs: BTreeMap<String, (usize, BTreeSet<String>)> = BTreeMap::new();
    for x in fs.list("Xml").iter().filter(|f| f.ends_with(".XML")) {
        let text = String::from_utf8_lossy(&fs.read(x).unwrap()).into_owned();
        let mut rest = text.as_str();
        while let Some(p) = rest.find('<') {
            rest = &rest[p + 1..];
            let Some(gt) = rest.find('>') else { break };
            let tag = &rest[..gt];
            if tag.starts_with('/')
                || tag.starts_with('!')
                || tag.starts_with('?')
                || tag.contains(' ')
            {
                continue;
            }
            let after = &rest[gt + 1..];
            if let Some(end) = after.find('<') {
                let val = after[..end].trim();
                if !val.is_empty()
                    && val.len() < 80
                    && basenames.contains(&val.to_ascii_lowercase())
                {
                    let e = tag_refs.entry(tag.to_ascii_lowercase()).or_default();
                    e.0 += 1;
                    e.1.insert(val.to_ascii_lowercase());
                }
            }
        }
    }

    let _ = writeln!(
        report,
        "# Empire at War Lua scripts: inventory ({})\n",
        id.as_deref().unwrap_or("base game")
    );
    let _ = writeln!(
        report,
        "Generated by `cargo run -p alamo-lua --example lua_survey -- <workshop_id> --md API.md`.\n"
    );
    let _ = writeln!(report, "## Lua version\n");
    let _ = writeln!(
        report,
        "The engine embeds **Lua 5.0**. The scripts use only 5.0 features that Lua 5.1 still accepts: implicit `arg` \
in vararg functions (`string.format(unpack(arg))` in PGDebug/GameScoring), `table.getn`, `table.foreach`/`foreachi`, \
no `%upvalue` syntax, no `string.gfind`, no 5.0-only globals (`strlen`, `tinsert`, ...). Lua 5.1 (vendored through mlua) \
compiled **{ok} of {} scripts** ({bad} failures) with these shims in the runtime: none required for compilation; \
`arg` relies on LUA_COMPAT_VARARG which the vendored 5.1 enables (checked at runtime in the tests).\n",
        files.len()
    );
    for f in &fails {
        let _ = writeln!(report, "- compile failure: {f}");
    }
    let _ = writeln!(
        report,
        "## Script files by area ({} total, Remake mod + base game)\n",
        files.len()
    );
    for a in AREAS {
        let _ = writeln!(report, "- {a}: {}", by_area.get(a).unwrap_or(&0));
    }
    let _ = writeln!(
        report,
        "\n## Entry points: XML tags that reference scripts\n"
    );
    let _ = writeln!(
        report,
        "| XML tag | references | distinct scripts | examples |\n|---|---|---|---|"
    );
    let mut tr: Vec<_> = tag_refs.iter().collect();
    tr.sort_by_key(|(_, v)| std::cmp::Reverse(v.0));
    for (tag, (n, set)) in tr.iter().take(25) {
        let ex: Vec<_> = set.iter().take(4).cloned().collect();
        let _ = writeln!(
            report,
            "| `{tag}` | {n} | {} | {} |",
            set.len(),
            ex.join(", ")
        );
    }
    let _ = writeln!(
        report,
        "\nThe engine contract (see `Library/PGStateMachine.lua`, `PGStoryMode.lua`): a script file is loaded, \
`Base_Definitions()` runs (which calls the script's `Definitions()`), then `main()` runs as the script's thread. \
`Story_Event_Trigger(name)` is called on story scripts when a plot event fires; `PumpEvents()` yields to the engine; \
`ServiceRate` sets the update period; `ScriptExit()` ends the script.\n"
    );
    let called_roots: BTreeSet<String> = calls
        .keys()
        .map(|k| k.split(['.', ':']).next().unwrap_or(k).to_string())
        .collect();
    let mut globals_read: Vec<(&String, &usize)> = all_reads
        .iter()
        .filter(|(k, n)| {
            (**n >= 3 || k.starts_with('_'))
                && !assigned.contains(*k)
                && !defined.contains(*k)
                && !called_roots.contains(*k)
                && !STD.contains(&k.as_str())
                && ![
                    "string",
                    "table",
                    "math",
                    "os",
                    "io",
                    "coroutine",
                    "_G",
                    "_VERSION",
                    "arg",
                ]
                .contains(&k.as_str())
        })
        .collect();
    globals_read.sort_by(|a, b| b.1.cmp(a.1));
    let _ = writeln!(
        report,
        "## Engine global variables ({})\n",
        globals_read.len()
    );
    let _ = writeln!(
        report,
        "Names that scripts read but never assign, define or call: values the engine provides (script-private \
variables such as `Object`, `Target`, `PlayerObject` that the libraries reset to nil are listed too when only read).\n"
    );
    for (k, n) in globals_read.iter().take(80) {
        let _ = writeln!(report, "- `{k}` ({n})");
    }
    let _ = writeln!(report);
    let _ = writeln!(report, "## Engine API ({} functions)\n", api.len());
    let _ = writeln!(
        report,
        "Names after `:` are methods on engine objects (game objects, players, planets, task forces, ...). \
Calls through a plain table prefix (`GlobalValue.Set`) are namespaced globals. Counts are call sites in all scripts. \
Columns: per-area counts; **G** = used by galactic-relevant scripts (Story, AI-Galactic, Misc, Library, Evaluators, FreeStore), \
**S** = used in space-mode AI or GameObject scripts, **L** = land-only (AI-Land).\n"
    );
    let _ = writeln!(
        report,
        "| calls | name | G | S | L | areas |\n|---:|---|:-:|:-:|:-:|---|"
    );
    for (name, n, v) in &api {
        let g = [
            "Story",
            "AI-Galactic",
            "Misc",
            "Library",
            "Evaluators",
            "FreeStore",
        ]
        .iter()
        .any(|a| v.contains_key(a));
        let s = v.contains_key("AI-Space") || v.contains_key("GameObject");
        let l = v.contains_key("AI-Land") && v.len() == 1;
        let areas: Vec<String> = v.iter().map(|(a, c)| format!("{a} {c}")).collect();
        let _ = writeln!(
            report,
            "| {n} | `{name}` | {} | {} | {} | {} |",
            if g { "G" } else { "" },
            if s { "S" } else { "" },
            if l { "L" } else { "" },
            areas.join(", ")
        );
    }
    if let Some(p) = rs {
        let mut globals = Vec::new();
        let mut spaces: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (name, _, _) in &api {
            if name.starts_with(':') {
                continue;
            }
            match name.split_once('.') {
                Some((ns, f)) => spaces
                    .entry(ns.to_string())
                    .or_default()
                    .push(f.to_string()),
                None => globals.push(name.clone()),
            }
        }
        let mut s = String::from(
            "// Generated by examples/lua_survey.rs --rs. Engine API names seen in the scripts.\n\n",
        );
        s += "pub const GLOBAL_STUBS: &[&str] = &[\n";
        for g in &globals {
            s += &format!("    \"{g}\",\n");
        }
        s += "];\n\npub const NAMESPACE_STUBS: &[(&str, &[&str])] = &[\n";
        for (ns, fs_) in &spaces {
            let list: Vec<String> = fs_.iter().map(|f| format!("\"{f}\"")).collect();
            s += &format!("    (\"{ns}\", &[{}]),\n", list.join(", "));
        }
        s += "];\n";
        std::fs::write(p, s).unwrap();
    }
    println!("{report}");
    if let Some(p) = md {
        std::fs::write(p, report).unwrap();
    }
}
