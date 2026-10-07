//! Difficulty adjustments from `Xml/DifficultyAdjustments.xml` (`Easy_Default`, `Normal_Minor`, ...).

use alamo_fs::FileSource;

use crate::xml::{self, Warning};

/// One `Difficulty_Adjustment` entry. Missing fields keep their neutral value (1.0 / 0.0).
#[derive(Debug, Clone, PartialEq)]
pub struct DifficultyAdjustment {
    /// Entry name, e.g. `Normal_Default`.
    pub name: String,
    /// `Credit_Multiplier`: applied to the income of AI players.
    pub credit_multiplier: f32,
    /// `Galactic_AI_Contrast_Multiplier`: scales the margin the galactic AI asks for before attacking (below 1 = bolder).
    pub galactic_contrast: f32,
    /// `Space_AI_Contrast_Multiplier`.
    pub space_contrast: f32,
    /// `Land_AI_Contrast_Multiplier`.
    pub land_contrast: f32,
    /// `Galactic_Build_Time_Multiplier`.
    pub galactic_build_time: f32,
    /// `Space_Build_Time_Multiplier`.
    pub space_build_time: f32,
    /// `Land_Build_Time_Multiplier`.
    pub land_build_time: f32,
    /// `Damage_Multiplier` (AI units in tactical battles).
    pub damage: f32,
    /// `Health_Multiplier`.
    pub health: f32,
    /// `Shield_Multiplier`.
    pub shield: f32,
    /// `Galactic_AI_Goal_Cycle_Sleep_Duration`: extra seconds between galactic goal evaluations, in units of the AI interval.
    pub galactic_goal_sleep: f32,
    /// `Space_AI_Goal_Cycle_Sleep_Duration`.
    pub space_goal_sleep: f32,
    /// `Land_AI_Goal_Cycle_Sleep_Duration`.
    pub land_goal_sleep: f32,
}

impl Default for DifficultyAdjustment {
    fn default() -> Self {
        Self {
            name: String::new(),
            credit_multiplier: 1.0,
            galactic_contrast: 1.0,
            space_contrast: 1.0,
            land_contrast: 1.0,
            galactic_build_time: 1.0,
            space_build_time: 1.0,
            land_build_time: 1.0,
            damage: 1.0,
            health: 1.0,
            shield: 1.0,
            galactic_goal_sleep: 0.0,
            space_goal_sleep: 0.0,
            land_goal_sleep: 0.0,
        }
    }
}

/// All adjustments of the mod.
#[derive(Debug, Clone, Default)]
pub struct DifficultyTable {
    /// Entries in file order.
    pub entries: Vec<DifficultyAdjustment>,
}

impl DifficultyTable {
    /// Loads `Xml/DifficultyAdjustments.xml`; an unreadable file gives an empty table.
    pub fn load(src: &dyn FileSource, warnings: &mut Vec<Warning>) -> Self {
        let Ok(bytes) = src.read("Xml/DifficultyAdjustments.xml") else { return Self::default() };
        let Some(root) = xml::parse(&bytes, "DifficultyAdjustments.xml", warnings) else { return Self::default() };
        let mut entries = Vec::new();
        for e in root.children.iter().filter(|c| c.name == "difficulty_adjustment") {
            let Some(name) = e.attr("name") else { continue };
            let num = |k: &str, d: f32| e.child(&k.to_ascii_lowercase()).and_then(|c| c.text_trimmed().parse::<f32>().ok()).unwrap_or(d);
            entries.push(DifficultyAdjustment {
                name: name.to_string(),
                credit_multiplier: num("Credit_Multiplier", 1.0),
                galactic_contrast: num("Galactic_AI_Contrast_Multiplier", 1.0),
                space_contrast: num("Space_AI_Contrast_Multiplier", 1.0),
                land_contrast: num("Land_AI_Contrast_Multiplier", 1.0),
                galactic_build_time: num("Galactic_Build_Time_Multiplier", 1.0),
                space_build_time: num("Space_Build_Time_Multiplier", 1.0),
                land_build_time: num("Land_Build_Time_Multiplier", 1.0),
                damage: num("Damage_Multiplier", 1.0),
                health: num("Health_Multiplier", 1.0),
                shield: num("Shield_Multiplier", 1.0),
                galactic_goal_sleep: num("Galactic_AI_Goal_Cycle_Sleep_Duration", 0.0),
                space_goal_sleep: num("Space_AI_Goal_Cycle_Sleep_Duration", 0.0),
                land_goal_sleep: num("Land_AI_Goal_Cycle_Sleep_Duration", 0.0),
            });
        }
        Self { entries }
    }

    /// Entry by name, case-insensitive (`Normal_Default`).
    pub fn get(&self, name: &str) -> Option<&DifficultyAdjustment> {
        self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }
}
