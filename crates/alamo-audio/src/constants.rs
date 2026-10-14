//! Audio-relevant game constants: `Xml/Audio.xml` (music peace delay, base-attack announcement delay) and
//! `Xml/Gameconstants.xml` (unit health thresholds for the low / critical health lines).

use alamo_fs::FileSource;


/// Music events a faction names in `Xml/Factions.xml` (`Music_Event_Battle_Load_Screen`, `Music_Event_Tactical_Win` and
/// `_Lose`, `Music_Event_Strategic_Win` / `_Lose`, their `_Vs_Faction` variants, `Music_Event_Space_Battle_Super_Weapon`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FactionMusic {
    /// Faction name, lower-case.
    pub faction: String,
    /// `Music_Event_Battle_Load_Screen`.
    pub battle_load: Option<String>,
    /// `Music_Event_Tactical_Win`.
    pub tactical_win: Option<String>,
    /// `Music_Event_Tactical_Lose`.
    pub tactical_lose: Option<String>,
    /// `Music_Event_Strategic_Win`.
    pub strategic_win: Option<String>,
    /// `Music_Event_Strategic_Lose`.
    pub strategic_lose: Option<String>,
    /// `Music_Event_Tactical_Win_Vs_Faction`: `(opponent lower-case, event)`.
    pub tactical_win_vs: Vec<(String, String)>,
    /// `Music_Event_Tactical_Lose_Vs_Faction`.
    pub tactical_lose_vs: Vec<(String, String)>,
    /// `Music_Event_Strategic_Win_Vs_Faction`.
    pub strategic_win_vs: Vec<(String, String)>,
    /// `Music_Event_Strategic_Lose_Vs_Faction`.
    pub strategic_lose_vs: Vec<(String, String)>,
    /// `Music_Event_Space_Battle_Super_Weapon` (battle theme while a super weapon is in the battle).
    pub space_battle_super_weapon: Option<String>,
}

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
    /// `Music_Event_*` tags of `Xml/Factions.xml` per faction.
    pub faction_music: Vec<FactionMusic>,
    /// Per-faction battle announcement events of `Xml/Factions.xml`: `(faction, tag, event)`, see [`AudioConstants::parse_faction_sfx`].
    pub faction_sfx: Vec<(String, String, String)>,
    /// Ability toggle sounds per faction, see [`AudioConstants::parse_toggle_sfx`].
    pub faction_toggle: Vec<(String, bool, String, String)>,
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
            faction_music: Vec::new(),
            faction_sfx: Vec::new(),
            faction_toggle: Vec::new(),
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



    /// The per-faction SFX tags of `Factions.xml` that carry battle announcements (`Space_Retreat_Enemy_Begin_SFXEvent` ...), as
    /// `(faction lower-case, tag lower-case, event)`.
    pub fn parse_faction_sfx(bytes: &[u8]) -> Vec<(String, String, String)> {
        const TAGS: [&str; 10] = [
            "space_retreat_begin_sfxevent",
            "space_retreat_cancel_sfxevent",
            "space_retreat_enemy_begin_sfxevent",
            "space_retreat_not_allowed_sfxevent",
            "space_retreat_not_allowed_reason_1_sfxevent",
            "space_surrender_sfxevent",
            "reinforcements_selection_sfxevent",
            "reinforcements_requesting_sfxevent",
            "reinforcements_enroute_sfxevent",
            "reinforcements_cancelled_sfxevent",
        ];
        let mut warnings = Vec::new();
        let mut out = Vec::new();
        if let Some(root) = alamo_data::xml::parse(bytes, "Factions.xml", &mut warnings) {
            for f in root.children_named("faction") {
                let Some(name) = f.attr("name") else { continue };
                for tag in TAGS {
                    if let Some(e) = f.child_text(tag).map(str::trim).filter(|e| !e.is_empty()) {
                        out.push((name.trim().to_ascii_lowercase(), tag.to_string(), e.to_string()));
                    }
                }
            }
        }
        out
    }

    /// Announcement event of `faction` for one of the tags of [`AudioConstants::parse_faction_sfx`] (lower-case tag name).
    pub fn faction_sfx(&self, faction: &str, tag: &str) -> Option<&str> {
        let f = faction.to_ascii_lowercase();
        self.faction_sfx.iter().find(|(ff, t, _)| *ff == f && t == tag).map(|(_, _, e)| e.as_str())
    }


    /// `SFXEvent_GUI_Toggle_Non_Hero_Ability_On` / `_Off` entries of every faction: `(faction lower-case, on, ability key upper-case, event)`.
    pub fn parse_toggle_sfx(bytes: &[u8]) -> Vec<(String, bool, String, String)> {
        let mut warnings = Vec::new();
        let mut out = Vec::new();
        if let Some(root) = alamo_data::xml::parse(bytes, "Factions.xml", &mut warnings) {
            for f in root.children_named("faction") {
                let Some(name) = f.attr("name") else { continue };
                for (tag, on) in [("sfxevent_gui_toggle_non_hero_ability_on", true), ("sfxevent_gui_toggle_non_hero_ability_off", false)] {
                    for e in f.children_named(tag) {
                        if let Some((key, event)) = e.text_trimmed().split_once(',') {
                            let (key, event) = (key.trim(), event.trim());
                            if !key.is_empty() && !event.is_empty() {
                                out.push((name.trim().to_ascii_lowercase(), on, key.to_ascii_uppercase(), event.to_string()));
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// The GUI sound when `faction` switches the non-hero ability `kind` (`AbilityDef::kind`) on or off.
    pub fn toggle_sfx(&self, faction: &str, kind: &str, on: bool) -> Option<&str> {
        let (f, k) = (faction.to_ascii_lowercase(), kind.to_ascii_uppercase());
        self.faction_toggle.iter().find(|(ff, o, kk, _)| *ff == f && *o == on && *kk == k).map(|(_, _, _, e)| e.as_str())
    }

    /// `Music_Event_*` tags of every faction in a `Factions.xml` file.
    pub fn parse_faction_music(bytes: &[u8]) -> Vec<FactionMusic> {
        let mut warnings = Vec::new();
        let mut out = Vec::new();
        let Some(root) = alamo_data::xml::parse(bytes, "Factions.xml", &mut warnings) else { return out };
        for f in root.children_named("faction") {
            let Some(name) = f.attr("name") else { continue };
            let one = |tag: &str| f.child_text(tag).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
            let vs = |tag: &str| -> Vec<(String, String)> {
                f.children_named(tag)
                    .filter_map(|e| {
                        let (opponent, event) = e.text_trimmed().split_once(',')?;
                        let (opponent, event) = (opponent.trim(), event.trim());
                        (!opponent.is_empty() && !event.is_empty()).then(|| (opponent.to_ascii_lowercase(), event.to_string()))
                    })
                    .collect()
            };
            out.push(FactionMusic {
                faction: name.trim().to_ascii_lowercase(),
                battle_load: one("music_event_battle_load_screen"),
                tactical_win: one("music_event_tactical_win"),
                tactical_lose: one("music_event_tactical_lose"),
                strategic_win: one("music_event_strategic_win"),
                strategic_lose: one("music_event_strategic_lose"),
                tactical_win_vs: vs("music_event_tactical_win_vs_faction"),
                tactical_lose_vs: vs("music_event_tactical_lose_vs_faction"),
                strategic_win_vs: vs("music_event_strategic_win_vs_faction"),
                strategic_lose_vs: vs("music_event_strategic_lose_vs_faction"),
                space_battle_super_weapon: one("music_event_space_battle_super_weapon"),
            });
        }
        out
    }

    fn faction_music(&self, faction: &str) -> Option<&FactionMusic> {
        let f = faction.to_ascii_lowercase();
        self.faction_music.iter().find(|m| m.faction == f)
    }

    /// Battle load screen music of `faction` (`Music_Event_Battle_Load_Screen`).
    pub fn battle_load_music(&self, faction: &str) -> Option<&str> {
        self.faction_music(faction)?.battle_load.as_deref()
    }

    /// Win / lose music of `faction`, in order of preference: the entry for the opponent (`..._Vs_Faction`), then the plain one.
    pub fn result_music(&self, faction: &str, versus: Option<&str>, win: bool, tactical: bool) -> Vec<&str> {
        let Some(m) = self.faction_music(faction) else { return Vec::new() };
        let (vs, plain) = match (tactical, win) {
            (true, true) => (&m.tactical_win_vs, &m.tactical_win),
            (true, false) => (&m.tactical_lose_vs, &m.tactical_lose),
            (false, true) => (&m.strategic_win_vs, &m.strategic_win),
            (false, false) => (&m.strategic_lose_vs, &m.strategic_lose),
        };
        let versus = versus.map(str::to_ascii_lowercase);
        let mut out: Vec<&str> = vs.iter().filter(|(o, _)| Some(o) == versus.as_ref()).map(|(_, e)| e.as_str()).collect();
        out.extend(plain.as_deref());
        out
    }

    /// Space battle theme while a super weapon is in the battle (`Music_Event_Space_Battle_Super_Weapon`).
    pub fn super_weapon_battle_music(&self, faction: &str) -> Option<&str> {
        self.faction_music(faction)?.space_battle_super_weapon.as_deref()
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
            c.faction_music = Self::parse_faction_music(&bytes);
            c.faction_sfx = Self::parse_faction_sfx(&bytes);
            c.faction_toggle = Self::parse_toggle_sfx(&bytes);
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
    fn faction_music_and_announcements_per_faction() {
        let xml = br#"<F><Faction Name="Rebel">
            <Music_Event_Battle_Load_Screen> Load_2 </Music_Event_Battle_Load_Screen>
            <Music_Event_Tactical_Lose_Vs_Faction> Empire,	 Lose_To_Empire_Event </Music_Event_Tactical_Lose_Vs_Faction>
            <Music_Event_Tactical_Lose> Rebel_Lose </Music_Event_Tactical_Lose>
            <Space_Retreat_Enemy_Begin_SFXEvent> Unit_Enemy_Retreat_Rebel_Soldier </Space_Retreat_Enemy_Begin_SFXEvent>
            <Space_Retreat_Not_Allowed_Reason_2_SFXEvent> </Space_Retreat_Not_Allowed_Reason_2_SFXEvent>
            </Faction></F>"#;
        let c = AudioConstants { faction_music: AudioConstants::parse_faction_music(xml), faction_sfx: AudioConstants::parse_faction_sfx(xml), ..Default::default() };
        assert_eq!(c.battle_load_music("rebel"), Some("Load_2"));
        assert_eq!(c.result_music("Rebel", Some("EMPIRE"), false, true), ["Lose_To_Empire_Event", "Rebel_Lose"]);
        assert_eq!(c.result_music("Rebel", Some("Pirates"), false, true), ["Rebel_Lose"]);
        assert!(c.result_music("Rebel", None, true, true).is_empty());
        assert_eq!(c.faction_sfx("Rebel", "space_retreat_enemy_begin_sfxevent"), Some("Unit_Enemy_Retreat_Rebel_Soldier"));
        assert_eq!(c.faction_sfx("Rebel", "space_retreat_not_allowed_reason_1_sfxevent"), None);
    }

    #[test]
    fn start_campaign_sound_per_faction() {
        let t = AudioConstants::parse_start_campaign(b"<F><Faction Name=\"Rebel\"><SFXEvent_GUI_Start_Campaign> GUI_Select_Rebel_SFX </SFXEvent_GUI_Start_Campaign></Faction><Faction Name=\"Neutral\"/></F>");
        let c = AudioConstants { start_campaign: t, ..Default::default() };
        assert_eq!(c.start_campaign_sfx("rebel"), Some("GUI_Select_Rebel_SFX"));
        assert_eq!(c.start_campaign_sfx("Neutral"), None);
    }
}
