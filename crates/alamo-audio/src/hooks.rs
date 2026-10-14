//! Static audit of the sound hooks the game data defines.
//!
//! Every XML tag whose name mentions `SFX` or `Music` (`SFXEvent_Select`, `Click_SFX`, `Fire_SFXEvent`,
//! `Music_Event_Tactical_Win`, ...) names an SFX or music event. [`scan_hooks`] collects those references from all
//! `Xml/**/*.xml` files of the layered file system and [`audit_hooks`] checks that each named event exists, has samples and
//! that the samples resolve to files. Results are grouped per tag, so a tag whose events are all broken (or that the game
//! code never reads) stands out. Nothing here plays a sound.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use alamo_fs::FileSource;

use crate::{MusicLibrary, SampleResolver, SfxLibrary};


/// Hook tags the game code reads and plays (battle: `battle-view/src/visuals.rs` + `audio.rs`; galaxy: `space-peace/src/ui_sounds.rs`;
/// `alamo-audio/src/constants.rs` for `Audio.xml` / `Factions.xml`). Keep in sync when a new hook is wired; the audit marks every
/// other tag "NO". Music is selected by event-name convention (`MusicPlanner`), not through the `Music_Event_*` faction tags.
pub const WIRED_TAGS: &[&str] = &[
    // unit responses and warnings
    "sfxevent_select",
    "sfxevent_move",
    "sfxevent_fleet_move",
    "sfxevent_attack",
    "sfxevent_attack_hardpoint",
    "sfxevent_guard",
    "sfxevent_stop",
    "sfxevent_unit_lost",
    "sfxevent_unit_under_attack",
    "sfxevent_health_low_warning",
    "sfxevent_health_critical_warning",
    "sfxevent_enemy_damaged_health_low_warning",
    "sfxevent_enemy_damaged_health_critical_warning",
    "sfxevent_hardpoint_destroyed",
    "sfxevent_gui_unit_ability_activated",
    "sfxevent_ability_activated",
    "sfxevent_special_ability",
    // weapons, impacts, deaths
    "fire_sfxevent",
    "sfxevent_fire",
    "projectile_sfxevent_detonate",
    "projectile_sfxevent_detonate_reduced_by_armor",
    "projectile_sfxevent_detonate_shield",
    "projectile_sfxevent_shield_detonate",
    "projectile_sfxevent_detonate_reduced_by_shield",
    "death_sfxevent_start_die",
    "death_explosion_sfxevent",
    // loops
    "sfxevent_engine_idle_loop",
    "sfxevent_engine_moving_loop",
    "sfxevent_ambient_loop",
    "sfxevent_arrive_from_hyperspace",
    "sfxevent_exit_into_hyperspace",
    // galaxy and GUI
    "click_sfx",
    "mouse_over_sfx",
    "sfxevent_build_started",
    "sfxevent_build_cancelled",
    "sfxevent_build_complete",
    "sfxevent_unit_ready",
    "sfxevent_fleet_ready",
    "sfxevent_target_affected",
    "sfxevent_gui_start_campaign",
    "sfxevent_unit_type_spotted",
    "sfxevent_planet_revealed",
    "sfxevent_planet_capture_bonus_reward",
    "sfxevent_gui_negative_feedback",
    "tsw_sfxevent_gui_bttton_press",
    "tsw_sfxevent_activate_voice",
    "tsw_sfxevent_weapon_power_up",
    "tsw_musicevent_activation_start",
    "space_retreat_enemy_begin_sfxevent",
    "space_retreat_not_allowed_sfxevent",
    "reinforcements_selection_sfxevent",
    "reinforcements_cancelled_sfxevent",
    "sfxevent_gui_unit_ability_deactivated",
    "sfxevent_target_ability",
    "sfxevent_gui_toggle_non_hero_ability_on",
    "sfxevent_gui_toggle_non_hero_ability_off",
    "sfxevent_group_move",
    "sfxevent_group_attack",
    "sfxevent_assist_move",
    "sfxevent_assist_attack",
    "death_sfxevent_end_die",
    "build_music_completed",
];

/// True if the game code plays events named by `tag` (lower case).
pub fn is_wired(tag: &str) -> bool {
    WIRED_TAGS.contains(&tag)
}

/// One reference from an XML file to an SFX / music event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRef {
    /// Lower-case tag name.
    pub tag: String,
    /// The event name as written.
    pub event: String,
    /// File the reference is in (normalized path).
    pub file: String,
}

/// Tags that contain `sfx` / `music` but are not event references (flags, timings, containers, definitions).
fn is_hook_tag(tag: &str) -> bool {
    if !(tag.contains("sfx") || tag.contains("music")) {
        return false;
    }
    const NOT_HOOKS: [&str; 12] = [
        "sfxevent",
        "sfxevents",
        "sfxevent_files",
        "musicevent",
        "musicevents",
        "play_sfxevent_on_sighting",
        "halt_game_music",
        "kills_previous_object_sfx",
        "sfxevent_ambient_moving_min_delay_seconds",
        "sfxevent_ambient_moving_max_delay_seconds",
        "sfxevent_frame",
        "sfx_event_files",
    ];
    !NOT_HOOKS.contains(&tag) && !tag.ends_with("_seconds") && !tag.ends_with("_frame")
}

/// Removes `<!-- ... -->` comments.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
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

/// Event names in a tag value: comma separated, flags, `NONE` / `SFX_NA` and `HARD_POINT_*` selectors dropped.
/// Tags that hold `Key, Event` pairs (music environment, unit type, weather, ...) return only the event.
fn event_names(tag: &str, value: &str) -> Vec<String> {
    let mut tokens: Vec<&str> = value.split(',').map(str::trim).filter(|t| !t.is_empty()).collect();
    // `KEY,` with a blank event: the key alone is not an event.
    if value.trim_end().ends_with(',') {
        tokens.pop();
    }
    // `Key, Event` pairs: music environment, unit type, weather, control-point type, versus-faction.
    let pair = tag.starts_with("music_event_list")
        || tag.ends_with("_control")
        || tag.contains("unit_type_spotted")
        || tag.ends_with("_vs_faction");
    if pair && tokens.len() == 2 {
        tokens.remove(0);
    }
    let last = tokens.len().saturating_sub(1);
    tokens
        .into_iter()
        .enumerate()
        // An all-upper-case token before the last one is a key (`RAIN, 0.75, Event`, `DEFEND, Event`).
        .filter(|(i, t)| *i == last || t.chars().any(char::is_lowercase))
        .map(|(_, t)| t)
        .filter(|t| {
            let l = t.to_ascii_lowercase();
            !t.contains(char::is_whitespace)
                && !matches!(l.as_str(), "true" | "false" | "yes" | "no" | "none" | "sfx_na" | "0" | "1")
                && !l.starts_with("hard_point_")
                && t.parse::<f64>().is_err()
        })
        .map(str::to_string)
        .collect()
}

/// Collects every hook reference in the XML text of one file.
pub fn scan_text(file: &str, text: &str) -> Vec<HookRef> {
    let text = strip_comments(text);
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(off) = text[i..].find('<') {
        let start = i + off + 1;
        let Some(end_rel) = text[start..].find('>') else { break };
        let head = &text[start..start + end_rel];
        i = start + end_rel + 1;
        if head.starts_with('/') || head.starts_with('?') || head.starts_with('!') || head.ends_with('/') {
            continue;
        }
        let tag = head.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        if !is_hook_tag(&tag) {
            continue;
        }
        let close = text[i..].find('<').map_or(bytes.len(), |c| i + c);
        for event in event_names(&tag, &text[i..close]) {
            out.push(HookRef { tag: tag.clone(), event, file: file.to_string() });
        }
    }
    out
}

/// Collects every hook reference of all `Xml/**/*.xml` files of `src`.
pub fn scan_hooks(src: &dyn FileSource) -> Vec<HookRef> {
    let mut files = src.list("Xml");
    files.sort();
    let mut refs = Vec::new();
    for f in files.iter().filter(|f| f.to_ascii_lowercase().ends_with(".xml")) {
        let Ok(bytes) = src.read(f) else { continue };
        refs.extend(scan_text(f, &String::from_utf8_lossy(&bytes)));
    }
    refs
}

/// Result of auditing all references of one tag.
#[derive(Debug, Default, Clone)]
pub struct TagAudit {
    /// Number of references.
    pub refs: usize,
    /// Distinct event names.
    pub events: BTreeSet<String>,
    /// Events that play: defined, have samples and every sample resolves.
    pub ok: BTreeSet<String>,
    /// Events not defined in the data.
    pub unknown: BTreeSet<String>,
    /// Events defined without any sample (and no chained event).
    pub no_samples: BTreeSet<String>,
    /// `(event, sample)` pairs whose file is not in the file system.
    pub missing: BTreeSet<(String, String)>,
    /// First file the tag appears in.
    pub example_file: String,
}

/// The audit, per tag.
#[derive(Debug, Default)]
pub struct HookAudit {
    /// Tag (lower case) -> result.
    pub tags: BTreeMap<String, TagAudit>,
}

/// Checks every reference against the libraries and the file system.
pub fn audit_hooks(
    refs: &[HookRef],
    sfx: &SfxLibrary,
    music: &MusicLibrary,
    resolver: &SampleResolver,
    src: &dyn FileSource,
) -> HookAudit {
    let mut audit = HookAudit::default();
    for r in refs {
        let t = audit.tags.entry(r.tag.clone()).or_default();
        if t.example_file.is_empty() {
            t.example_file = r.file.clone();
        }
        t.refs += 1;
        if !t.events.insert(r.event.clone()) {
            continue;
        }
        let is_music = r.tag.contains("music");
        let samples: Option<Vec<String>> = if is_music {
            music.get(&r.event).map(|e| e.files.clone())
        } else {
            sfx.get(&r.event).map(|e| {
                if e.is_preset {
                    Vec::new()
                } else {
                    e.samples.iter().chain(&e.pre_samples).chain(&e.post_samples).cloned().collect()
                }
            })
        };
        let chained = !is_music && sfx.get(&r.event).is_some_and(|e| e.chained.is_some());
        match samples {
            None => {
                t.unknown.insert(r.event.clone());
            }
            Some(s) if s.is_empty() && !chained => {
                t.no_samples.insert(r.event.clone());
            }
            Some(s) => {
                let mut broken = false;
                for sample in s {
                    if resolver.resolve(src, &sample).is_none() {
                        t.missing.insert((r.event.clone(), sample));
                        broken = true;
                    }
                }
                if !broken {
                    t.ok.insert(r.event.clone());
                }
            }
        }
    }
    audit
}

impl HookAudit {
    /// Events named by a tag for which `wired` is true that do not play, one line per event: `event: unknown event`,
    /// `event: no samples` or `event: missing file <first missing sample>`. Sorted, de-duplicated across tags.
    pub fn broken_wired(&self, wired: impl Fn(&str) -> bool) -> Vec<String> {
        let mut v = BTreeSet::new();
        for (_, t) in self.tags.iter().filter(|(tag, _)| wired(tag)) {
            v.extend(t.unknown.iter().map(|e| format!("{e}: unknown event")));
            v.extend(t.no_samples.iter().map(|e| format!("{e}: no samples")));
            let mut first: BTreeMap<&str, &str> = BTreeMap::new();
            for (e, s) in &t.missing {
                first.entry(e).or_insert(s);
            }
            v.extend(first.into_iter().map(|(e, s)| format!("{e}: missing file {s}")));
        }
        v.into_iter().collect()
    }

    /// Markdown table: tag, references, distinct events, playable / unknown / sampleless / missing-file counts, wired flag.
    pub fn to_markdown(&self, wired: impl Fn(&str) -> bool) -> String {
        let mut s = String::from("| Tag | Refs | Events | Play | Unknown | No sample | Missing file | In game code |\n|---|---:|---:|---:|---:|---:|---:|---|\n");
        let mut rows: Vec<_> = self.tags.iter().collect();
        rows.sort_by_key(|(tag, t)| (wired(tag), std::cmp::Reverse(t.refs)));
        for (tag, t) in rows {
            let _ = writeln!(
                s,
                "| `{tag}` | {} | {} | {} | {} | {} | {} | {} |",
                t.refs,
                t.events.len(),
                t.ok.len(),
                t.unknown.len(),
                t.no_samples.len(),
                t.missing.len(),
                if wired(tag) { "yes" } else { "NO" }
            );
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_tags_values_and_skips_comments_flags_and_selectors() {
        let xml = r#"<Root>
            <Unit>
              <SFXEvent_Select> Unit_Select_Vader </SFXEvent_Select>
              <!-- <SFXEvent_Move> Hidden </SFXEvent_Move> -->
              <SFXEvent_Hardpoint_Destroyed> HARD_POINT_ENGINE, Unit_Lost_Engines, HARD_POINT_WEAPON_LASER, SFX_NA </SFXEvent_Hardpoint_Destroyed>
              <Play_SFXEvent_On_Sighting>True</Play_SFXEvent_On_Sighting>
              <Music_Event_List_Battle> Space, Space_Map_Rebel_Battle_Music_Event </Music_Event_List_Battle>
              <Click_SFX/>
              <SFXEvent_GUI_Toggle_Non_Hero_Ability_On>SPREAD_OUT, </SFXEvent_GUI_Toggle_Non_Hero_Ability_On>
            </Unit></Root>"#;
        let refs = scan_text("t.xml", xml);
        let got: Vec<_> = refs.iter().map(|r| (r.tag.as_str(), r.event.as_str())).collect();
        assert_eq!(
            got,
            vec![
                ("sfxevent_select", "Unit_Select_Vader"),
                ("sfxevent_hardpoint_destroyed", "Unit_Lost_Engines"),
                ("music_event_list_battle", "Space_Map_Rebel_Battle_Music_Event"),
            ]
        );
    }
}
