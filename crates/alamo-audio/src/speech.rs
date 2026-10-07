//! Speech events (`Xml/SpeechEvents.xml`) and classification of unit-response SFX events.
//!
//! Campaign / advisor / cinematic lines live in `SpeechEvents.xml` (one file per line, in
//! `Data/Audio/Speech/English` via `EnglishSpeech.meg`). The per-unit acknowledgements ("Unit_Select_*",
//! "Unit_Move_*", ...) are ordinary SFX events flagged `Is_Unit_Response_VO`, see [`UnitResponseKind`].

use std::collections::HashMap;

use alamo_data::xml::{self, Warning};
use alamo_fs::FileSource;

/// A line of speech (`SpeechEvent`).
#[derive(Debug, Clone)]
pub struct SpeechEvent {
    /// Event name.
    pub name: String,
    /// Sample file names (usually one, bare `*_ENG.MP3` names).
    pub files: Vec<String>,
    /// `Text_ID` of the subtitle.
    pub text_id: String,
    /// Playback volume in percent.
    pub volume_percent: f32,
}

/// All speech events.
#[derive(Debug, Default)]
pub struct SpeechLibrary {
    events: HashMap<String, SpeechEvent>,
}

impl SpeechLibrary {
    /// Loads `Xml/SpeechEvents.xml`.
    pub fn load(src: &dyn FileSource, warnings: &mut Vec<Warning>) -> Self {
        let mut events = HashMap::new();
        match src.read("Xml/SpeechEvents.xml") {
            Ok(bytes) => {
                if let Some(root) = xml::parse(&bytes, "SpeechEvents.xml", warnings) {
                    for e in root.children_named("speechevent") {
                        let Some(name) = e.attr("name") else { continue };
                        let ev = SpeechEvent {
                            name: name.to_string(),
                            files: e
                                .child_text("files")
                                .map(xml::split_list)
                                .unwrap_or_default(),
                            text_id: e
                                .child_text("text_id")
                                .unwrap_or_default()
                                .trim()
                                .to_string(),
                            volume_percent: e
                                .child_text("volume_percent")
                                .and_then(|v| v.trim().parse().ok())
                                .unwrap_or(100.0),
                        };
                        events.insert(name.to_ascii_lowercase(), ev);
                    }
                }
            }
            Err(e) => warnings.push(Warning {
                file: "SpeechEvents.xml".into(),
                line: 0,
                message: e.to_string(),
            }),
        }
        Self { events }
    }

    /// Looks a line up by name (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&SpeechEvent> {
        self.events.get(&name.to_ascii_lowercase())
    }

    /// Number of lines.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True if empty.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Iterates all lines.
    pub fn iter(&self) -> impl Iterator<Item = &SpeechEvent> {
        self.events.values()
    }
}

/// What a unit says; derived from the SFX event name prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnitResponseKind {
    /// `Unit_Select_*`.
    Select,
    /// `Unit_Move_*`.
    Move,
    /// `Unit_Fleet_Move_*` (fleet-wide move order).
    FleetMove,
    /// `Unit_Attack_*`.
    Attack,
    /// `Unit_Guard_*`.
    Guard,
    /// `Unit_Stop_*` / `Unit_Halt_*`.
    Stop,
    /// `Unit_Complete_*` (production finished).
    Complete,
    /// `Unit_Lock_Attack_*` / `Unit_Fleet_Attack_*` and similar attack variants are folded into `Attack`.
    Other,
}

impl UnitResponseKind {
    /// Classifies an event name such as `Unit_Fleet_Move_Star_Destroyer`.
    pub fn from_event_name(name: &str) -> Option<Self> {
        let n = name.to_ascii_lowercase();
        let rest = n.strip_prefix("unit_")?;
        Some(if rest.starts_with("select_") {
            Self::Select
        } else if rest.starts_with("fleet_move_") {
            Self::FleetMove
        } else if rest.starts_with("move_") {
            Self::Move
        } else if rest.starts_with("attack_")
            || rest.contains("attack_") && rest.starts_with("fleet_")
        {
            Self::Attack
        } else if rest.starts_with("guard_") {
            Self::Guard
        } else if rest.starts_with("stop_") || rest.starts_with("halt_") {
            Self::Stop
        } else if rest.starts_with("complete_") {
            Self::Complete
        } else {
            Self::Other
        })
    }

    /// The event name for this kind and unit suffix, e.g. `(Select, "Tie_Fighter")` -> `Unit_Select_Tie_Fighter`.
    pub fn event_name(self, unit: &str) -> Option<String> {
        let prefix = match self {
            Self::Select => "Unit_Select_",
            Self::Move => "Unit_Move_",
            Self::FleetMove => "Unit_Fleet_Move_",
            Self::Attack => "Unit_Attack_",
            Self::Guard => "Unit_Guard_",
            Self::Stop => "Unit_Stop_",
            Self::Complete => "Unit_Complete_",
            Self::Other => return None,
        };
        Some(format!("{prefix}{unit}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_names() {
        assert_eq!(
            UnitResponseKind::from_event_name("Unit_Select_Tie_Fighter"),
            Some(UnitResponseKind::Select)
        );
        assert_eq!(
            UnitResponseKind::from_event_name("unit_fleet_move_x"),
            Some(UnitResponseKind::FleetMove)
        );
        assert_eq!(
            UnitResponseKind::from_event_name("Unit_Move_A"),
            Some(UnitResponseKind::Move)
        );
        assert_eq!(UnitResponseKind::from_event_name("GUI_Click"), None);
        assert_eq!(
            UnitResponseKind::Attack.event_name("X").as_deref(),
            Some("Unit_Attack_X")
        );
    }
}
