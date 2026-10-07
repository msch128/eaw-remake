//! Loading of the AI definition files under `Data/Xml/AI`.
//!
//! | directory | content |
//! |---|---|
//! | `Players` | `AIPlayerType`: goal proposal function sets, templates, free store scripts |
//! | `GoalFunctions` | function sets: goal name -> desire equation name |
//! | `Goals` | goal definitions: application flags, category, reachability, time limit, `Is_Like` |
//! | `PerceptualEquations` | named equations (see [`crate::equation`]) |
//! | `Templates` | budget allocation per goal category and the categories that are switched on |
//!
//! See `AI.md` for how these fit together.

use std::collections::{BTreeMap, HashMap};

use alamo_data::xml::{self, Element, Warning};
use alamo_fs::FileSource;

use crate::equation::EquationSet;

/// An `AIPlayerType` (`Players/*.xml`).
#[derive(Debug, Clone, Default)]
pub struct AiPlayerDef {
    /// `Name`.
    pub name: String,
    /// `GoalProposalFunctionSets`, in order.
    pub function_sets: Vec<String>,
    /// `Templates/Galactic` template names.
    pub galactic_templates: Vec<String>,
    /// `GalacticFreeStoreScript`.
    pub galactic_free_store: Option<String>,
    /// `Personality/Aggressiveness`.
    pub aggressiveness: f32,
    /// `Personality/Focus`.
    pub focus: f32,
    /// `GoalSetExtensionSize`: how many goals may be active at once beyond the first (engine tuning).
    pub goal_set_extension_size: u32,
}

/// A goal definition (`Goals/*.xml`).
#[derive(Debug, Clone, Default)]
pub struct GoalDef {
    /// Goal name.
    pub name: String,
    /// `AIGoalApplicationFlags`: which targets the goal applies to (`Friendly`, `Enemy`, `Neutral`).
    pub flags: Vec<String>,
    /// `GameMode` (`Galactic`, `Space`, `Land`).
    pub game_mode: String,
    /// `Category` (`Offensive`, `Defensive`, `Infrastructure`, `Information`, `Hero`, `StoryArc`, `MajorItem`, ...).
    pub category: String,
    /// `Reachability` (`Any`, `Enemy_Destination`, `Single_Hop_Disconnected`, ...).
    pub reachability: String,
    /// `Time_Limit` in seconds.
    pub time_limit: f32,
    /// `Build_Time_Delay_Tolerance`.
    pub build_time_delay_tolerance: f32,
    /// `Is_Like`: goals this one shares plan learning with.
    pub is_like: Vec<String>,
    /// `Per_Failure_Desire_Adjust`.
    pub per_failure_desire_adjust: f32,
    /// `Tracking_Duration` of failures, seconds.
    pub tracking_duration: f32,
    /// `Per_Activation_Failure_Desire_Adjust`.
    pub per_activation_failure_desire_adjust: f32,
    /// `Activation_Tracking_Duration`, seconds.
    pub activation_tracking_duration: f32,
}

/// An AI template (`Templates/*.xml`): budget equations per category.
#[derive(Debug, Clone, Default)]
pub struct AiTemplate {
    /// Template name.
    pub name: String,
    /// `Budget/<Category>` -> equation name.
    pub budget: BTreeMap<String, String>,
    /// `Turn_On/Goals/Category` list.
    pub goal_categories: Vec<String>,
    /// `Turn_On/Plans/Goal_Category` list.
    pub plan_categories: Vec<String>,
}

/// Everything loaded from `Xml/AI`.
#[derive(Debug, Default)]
pub struct AiData {
    /// AI player types by lower-cased name.
    pub players: HashMap<String, AiPlayerDef>,
    /// Function sets: set name (lower) -> `(goal, function)` pairs.
    pub function_sets: HashMap<String, Vec<(String, String)>>,
    /// Goal definitions by lower-cased name.
    pub goals: HashMap<String, GoalDef>,
    /// All perceptual equations.
    pub equations: EquationSet,
    /// Templates by lower-cased name.
    pub templates: HashMap<String, AiTemplate>,
    /// XML problems.
    pub warnings: Vec<Warning>,
    /// Files read.
    pub files: usize,
}

fn text(e: &Element, tag: &str) -> Option<String> {
    e.child_text(tag).map(|t| t.trim().to_string())
}

fn words(s: &str) -> Vec<String> {
    xml::split_list(s)
}

fn num(e: &Element, tag: &str, default: f32) -> f32 {
    text(e, tag).and_then(|t| t.parse().ok()).unwrap_or(default)
}

impl AiData {
    /// Loads all AI definitions visible in `src` (mod files override base files of the same name, as the
    /// layered file system resolves).
    pub fn load(src: &dyn FileSource) -> AiData {
        let mut d = AiData::default();
        for path in src.list("Xml/AI") {
            if !path.ends_with(".XML") {
                continue;
            }
            let Some(sub) = path.strip_prefix("XML/AI/") else { continue };
            let dir = sub.split('/').next().unwrap_or("");
            let Ok(bytes) = src.read(&path) else { continue };
            let Some(root) = xml::parse(&bytes, &path, &mut d.warnings) else { continue };
            d.files += 1;
            match dir {
                "PLAYERS" => d.add_player(&root),
                "GOALFUNCTIONS" => d.add_function_set(&path, &root),
                "GOALS" => d.add_goals(&root),
                "PERCEPTUALEQUATIONS" => {
                    for e in &root.children {
                        d.equations.insert(&e.name, e.text_trimmed());
                    }
                }
                "TEMPLATES" => d.add_templates(&root),
                _ => {}
            }
        }
        d
    }

    fn add_player(&mut self, root: &Element) {
        let name = text(root, "name").unwrap_or_default();
        if name.is_empty() {
            return;
        }
        let def = AiPlayerDef {
            name: name.clone(),
            function_sets: root.child_text("goalproposalfunctionsets").map(words).unwrap_or_default(),
            galactic_templates: root.child("templates").and_then(|t| t.child_text("galactic")).map(words).unwrap_or_default(),
            galactic_free_store: text(root, "galacticfreestorescript"),
            aggressiveness: root.child("personality").map_or(1.0, |p| num(p, "aggressiveness", 1.0)),
            focus: root.child("personality").map_or(1.0, |p| num(p, "focus", 1.0)),
            goal_set_extension_size: num(root, "goalsetextensionsize", 0.0) as u32,
        };
        self.players.insert(name.to_ascii_lowercase(), def);
    }

    fn add_function_set(&mut self, path: &str, root: &Element) {
        let file = path.rsplit('/').next().unwrap_or(path).trim_end_matches(".XML").to_ascii_lowercase();
        let pairs = root
            .children
            .iter()
            .filter_map(|e| Some((text(e, "goal")?, text(e, "function")?)))
            .collect::<Vec<_>>();
        self.function_sets.entry(file).or_default().extend(pairs);
    }

    fn add_goals(&mut self, root: &Element) {
        for e in &root.children {
            let goal = GoalDef {
                name: e.raw_name.clone(),
                flags: e.child_text("aigoalapplicationflags").map(words).unwrap_or_default(),
                game_mode: text(e, "gamemode").unwrap_or_default(),
                category: text(e, "category").unwrap_or_default(),
                reachability: text(e, "reachability").unwrap_or_default(),
                time_limit: num(e, "time_limit", 0.0),
                build_time_delay_tolerance: num(e, "build_time_delay_tolerance", 1.0),
                is_like: e.child_text("is_like").map(words).unwrap_or_default(),
                per_failure_desire_adjust: num(e, "per_failure_desire_adjust", 0.0),
                tracking_duration: num(e, "tracking_duration", 0.0),
                per_activation_failure_desire_adjust: num(e, "per_activation_failure_desire_adjust", 0.0),
                activation_tracking_duration: num(e, "activation_tracking_duration", 0.0),
            };
            self.goals.insert(e.name.clone(), goal);
        }
    }

    fn add_templates(&mut self, root: &Element) {
        for e in &root.children {
            let mut t = AiTemplate { name: e.raw_name.clone(), ..AiTemplate::default() };
            if let Some(b) = e.child("budget") {
                for c in &b.children {
                    t.budget.insert(c.raw_name.clone(), c.text_trimmed().to_string());
                }
            }
            if let Some(on) = e.child("turn_on") {
                if let Some(g) = on.child("goals") {
                    t.goal_categories = g.children_named("category").map(|c| c.text_trimmed().to_string()).collect();
                }
                if let Some(p) = on.child("plans") {
                    t.plan_categories = p.children_named("goal_category").map(|c| c.text_trimmed().to_string()).collect();
                }
            }
            self.templates.insert(e.name.clone(), t);
        }
    }

    /// The desire equation name of a goal in the given function sets (first set that defines it wins).
    pub fn goal_function<'a>(&'a self, sets: &[String], goal: &str) -> Option<&'a str> {
        sets.iter().find_map(|s| {
            self.function_sets
                .get(&s.to_ascii_lowercase())?
                .iter()
                .find(|(g, _)| g.eq_ignore_ascii_case(goal))
                .map(|(_, f)| f.as_str())
        })
    }
}

/// `AI_Player_Control` lines of a campaign: `(faction, ai player type)`.
pub fn campaign_ai_players(src: &dyn FileSource, campaign: &alamo_data::Campaign) -> Vec<(String, String)> {
    let Ok(bytes) = src.read(&format!("Xml/{}", campaign.source_file)) else { return Vec::new() };
    let mut w = Vec::new();
    let Some(root) = xml::parse(&bytes, &campaign.source_file, &mut w) else { return Vec::new() };
    let Some(c) = root.children.iter().find(|c| c.attr("name").is_some_and(|n| n.eq_ignore_ascii_case(&campaign.name))) else {
        return Vec::new();
    };
    c.children_named("ai_player_control")
        .filter_map(|e| {
            let parts = words(e.text_trimmed());
            Some((parts.first()?.clone(), parts.get(1)?.clone()))
        })
        .collect()
}
