//! Audio-relevant game constants: `Xml/Audio.xml` (music peace delay, base-attack announcement delay) and
//! `Xml/Gameconstants.xml` (unit health thresholds for the low / critical health lines).

use alamo_fs::FileSource;

/// Values read from the game data; the defaults are the shipped numbers of the Remake 4.0 files.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioConstants {
    /// `Music_Space_Battle_To_Ambient_Peace_Seconds` (Audio.xml): calm time before space battle music ends.
    pub space_battle_to_ambient_seconds: f32,
    /// `Delay_Between_Space_Base_Attack_Announcement_Seconds` (Audio.xml).
    pub space_base_attack_announcement_delay: f32,
    /// `Health_Low_Percent_Threshold` (Gameconstants.xml), fraction of max health.
    pub health_low: f32,
    /// `Health_Critical_Percent_Threshold` (Gameconstants.xml), fraction of max health.
    pub health_critical: f32,
    /// `Audio_Space_3D_Saturation_Distance_Mod` (Audio.xml): scales every event's saturation distance in space battles.
    pub space_saturation_mod: f32,
    /// `Audio_Space_3D_Rolloff_Distance_Mod` (Audio.xml): higher = quicker rolloff over distance.
    pub space_rolloff_mod: f32,
    /// `SFXEvent_GUI_Negative_Feedback` (Audio.xml): the general "refused" sound.
    pub negative_feedback: Option<String>,
    /// `SFXEvent_Planet_Revealed` (Audio.xml).
    pub planet_revealed: Option<String>,
    /// `SFXEvent_Planet_Capture_Bonus_Reward` (Audio.xml): one-time reward when a planet is captured.
    pub planet_capture_bonus: Option<String>,
    /// `SFXEvent_Unit_Type_Spotted` of `Xml/Factions.xml`: `(faction, unit type, event)`, faction and unit lower-case.
    /// The faction's soldiers announce the first sighting of that enemy unit type (heroes, Star Destroyers ...).
    pub unit_type_spotted: Vec<(String, String, String)>,
    /// `SFXEvent_GUI_Start_Campaign` of `Xml/Factions.xml`: `(faction lower-case, event)`, played when "Start Game" is pressed.
    pub start_campaign: Vec<(String, String)>,
}

impl Default for AudioConstants {
    fn default() -> Self {
        Self {
            space_battle_to_ambient_seconds: 15.0,
            space_base_attack_announcement_delay: 60.0,
            health_low: 0.33,
            health_critical: 0.10,
            space_saturation_mod: 1.5,
            space_rolloff_mod: 1.0,
            negative_feedback: Some("GUI_Bad_Sound".into()),
            planet_revealed: Some("GUI_Planet_Mouse_Over".into()),
            planet_capture_bonus: Some("Unit_Light_Saber_On".into()),
            unit_type_spotted: Vec::new(),
            start_campaign: Vec::new(),
        }
    }
}

impl AudioConstants {
    /// Announcement `faction` makes when it first sees an enemy of unit type `unit` (`SFXEvent_Unit_Type_Spotted`).
    pub fn spotted_line(&self, faction: &str, unit: &str) -> Option<&str> {
        let (f, u) = (faction.to_ascii_lowercase(), unit.to_ascii_lowercase());
        self.unit_type_spotted.iter().find(|(ff, uu, _)| *ff == f && *uu == u).map(|(_, _, e)| e.as_str())
    }

    /// Sound of the "Start Game" button for the side `faction` (`SFXEvent_GUI_Start_Campaign`).
    pub fn start_campaign_sfx(&self, faction: &str) -> Option<&str> {
        let f = faction.to_ascii_lowercase();
        self.start_campaign.iter().find(|(ff, _)| *ff == f).map(|(_, e)| e.as_str())
    }

    /// `SFXEvent_GUI_Start_Campaign` of every faction in a `Factions.xml` file.
    pub fn parse_start_campaign(bytes: &[u8]) -> Vec<(String, String)> {
        let mut warnings = Vec::new();
        let mut out = Vec::new();
        if let Some(root) = alamo_data::xml::parse(bytes, "Factions.xml", &mut warnings) {
            for f in root.children_named("faction") {
                if let (Some(name), Some(e)) = (f.attr("name"), f.child_text("sfxevent_gui_start_campaign")) && !e.is_empty() {
                    out.push((name.trim().to_ascii_lowercase(), e.to_string()));
                }
            }
        }
        out
    }

    /// `SFXEvent_Unit_Type_Spotted` entries of every faction in a `Factions.xml` file.
    pub fn parse_spotted(bytes: &[u8]) -> Vec<(String, String, String)> {
        let mut warnings = Vec::new();
        let mut out = Vec::new();
        if let Some(root) = alamo_data::xml::parse(bytes, "Factions.xml", &mut warnings) {
            for f in root.children_named("faction") {
                let Some(name) = f.attr("name") else { continue };
                for e in f.children_named("sfxevent_unit_type_spotted") {
                    if let Some((unit, event)) = e.text_trimmed().split_once(',') {
                        let (unit, event) = (unit.trim(), event.trim());
                        if !unit.is_empty() && !event.is_empty() {
                            out.push((name.trim().to_ascii_lowercase(), unit.to_ascii_lowercase(), event.to_string()));
                        }
                    }
                }
            }
        }
        out
    }

    /// Reads both files from `src`; missing files or tags keep the defaults.
    pub fn load(src: &dyn FileSource) -> Self {
        let mut c = Self::default();
        let read = |p: &str| src.read(p).ok().map(|b| strip_comments(&String::from_utf8_lossy(&b)));
        if let Some(t) = read("Xml/Audio.xml") {
            if let Some(v) = tag_f32(&t, "Music_Space_Battle_To_Ambient_Peace_Seconds") {
                c.space_battle_to_ambient_seconds = v;
            }
            if let Some(v) = tag_f32(&t, "Delay_Between_Space_Base_Attack_Announcement_Seconds") {
                c.space_base_attack_announcement_delay = v;
            }
            if let Some(v) = tag_f32(&t, "Audio_Space_3D_Saturation_Distance_Mod") {
                c.space_saturation_mod = v;
            }
            if let Some(v) = tag_f32(&t, "Audio_Space_3D_Rolloff_Distance_Mod") {
                c.space_rolloff_mod = v;
            }
            c.negative_feedback = tag_str(&t, "SFXEvent_GUI_Negative_Feedback").or(c.negative_feedback);
            c.planet_revealed = tag_str(&t, "SFXEvent_Planet_Revealed").or(c.planet_revealed);
            c.planet_capture_bonus = tag_str(&t, "SFXEvent_Planet_Capture_Bonus_Reward").or(c.planet_capture_bonus);
        }
        if let Ok(bytes) = src.read("Xml/Factions.xml") {
            c.unit_type_spotted = Self::parse_spotted(&bytes);
            c.start_campaign = Self::parse_start_campaign(&bytes);
        }
        if let Some(t) = read("Xml/Gameconstants.xml") {
            if let Some(v) = tag_f32(&t, "Health_Low_Percent_Threshold") {
                c.health_low = v;
            }
            if let Some(v) = tag_f32(&t, "Health_Critical_Percent_Threshold") {
                c.health_critical = v;
            }
        }
        c
    }
}

fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("<!--") {
        out.push_str(&rest[..i]);
        match rest[i..].find("-->") {
            Some(j) => rest = &rest[i + j + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn tag_str(text: &str, tag: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let open = format!("<{}>", tag.to_ascii_lowercase());
    let start = lower.find(&open)? + open.len();
    let end = lower[start..].find('<')? + start;
    Some(text[start..end].trim().to_string()).filter(|s| !s.is_empty())
}

fn tag_f32(text: &str, tag: &str) -> Option<f32> {
    let lower = text.to_ascii_lowercase();
    let open = format!("<{}>", tag.to_ascii_lowercase());
    let start = lower.find(&open)? + open.len();
    let end = lower[start..].find('<')? + start;
    text[start..end].trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tags_and_skips_comments() {
        let t = strip_comments("<!-- <A>1</A> --><A>2</A><B> 0.5 </B>");
        assert_eq!(tag_f32(&t, "A"), Some(2.0));
        assert_eq!(tag_f32(&t, "b"), Some(0.5));
        assert_eq!(tag_f32(&t, "C"), None);
    }

    #[test]
    fn spotted_lines_per_faction() {
        let t = AudioConstants::parse_spotted(
            br#"<Factions><Faction Name="Rebel">
            <!--<SFXEvent_Unit_Type_Spotted>Hidden, Nope</SFXEvent_Unit_Type_Spotted>-->
            <SFXEvent_Unit_Type_Spotted>Darth_Vader,   Unit_Announce_Vader_Rebel_Soldier</SFXEvent_Unit_Type_Spotted>
            </Faction><Faction Name="Empire"><SFXEvent_Unit_Type_Spotted>Yoda, Unit_Announce_Yoda_Empire_Soldier</SFXEvent_Unit_Type_Spotted></Faction></Factions>"#,
        );
        let c = AudioConstants { unit_type_spotted: t, ..Default::default() };
        assert_eq!(c.spotted_line("Rebel", "darth_vader"), Some("Unit_Announce_Vader_Rebel_Soldier"));
        assert_eq!(c.spotted_line("Empire", "Yoda"), Some("Unit_Announce_Yoda_Empire_Soldier"));
        assert_eq!(c.spotted_line("Rebel", "Yoda"), None);
        assert_eq!(c.spotted_line("Rebel", "Hidden"), None);
    }

    #[test]
    fn start_campaign_sound_per_faction() {
        let t = AudioConstants::parse_start_campaign(b"<F><Faction Name=\"Rebel\"><SFXEvent_GUI_Start_Campaign> GUI_Select_Rebel_SFX </SFXEvent_GUI_Start_Campaign></Faction><Faction Name=\"Neutral\"/></F>");
        let c = AudioConstants { start_campaign: t, ..Default::default() };
        assert_eq!(c.start_campaign_sfx("rebel"), Some("GUI_Select_Rebel_SFX"));
        assert_eq!(c.start_campaign_sfx("Neutral"), None);
    }
}
