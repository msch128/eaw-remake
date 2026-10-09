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
        }
    }
}

impl AudioConstants {
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
}
