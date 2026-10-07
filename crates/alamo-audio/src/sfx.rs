//! SFX event definitions (`Xml/SFXEventFiles.xml` -> `SFXEvents*.xml`).
//!
//! Events may inherit from presets via `Use_Preset` (a preset is just an event with `Is_Preset`);
//! values of the event itself win, then its preset, then the preset's preset. Later definitions of the
//! same name replace earlier ones, as in the engine's file order.
//!
//! Units: volumes and pitches are **percent** as written in the XML (100 = unchanged, volumes above 100
//! amplify); distances are game units; predelays are milliseconds. `Priority` 1 is the most important
//! (GUI and big explosions use 1, ambience 5).

use std::collections::HashMap;

use alamo_data::xml::{self, Element, Warning, parse_bool};
use alamo_fs::FileSource;

/// One fully resolved SFX event (preset values already merged in).
#[derive(Debug, Clone)]
pub struct SfxEvent {
    /// Name as written (look-ups are case-insensitive).
    pub name: String,
    /// Definition file (relative to `Data/Xml`).
    pub source_file: String,
    /// True for `Is_Preset` events (not meant to be played).
    pub is_preset: bool,
    /// Sample paths exactly as in the XML (`Samples`); see [`crate::SampleResolver`].
    pub samples: Vec<String>,
    /// Samples played before the main sample (`Pre_Samples`).
    pub pre_samples: Vec<String>,
    /// Samples played after the main sample (`Post_Samples`).
    pub post_samples: Vec<String>,
    /// `Play_Sequentially`: walk the list in order instead of picking randomly.
    pub play_sequentially: bool,
    /// Minimum / maximum volume in percent (randomized per play).
    pub volume: (f32, f32),
    /// Minimum / maximum pitch in percent (randomized per play).
    pub pitch: (f32, f32),
    /// 1 = most important .. 5 = least.
    pub priority: u8,
    /// `Max_Instances`: how many copies of this event may play at once (`None` = unlimited).
    pub max_instances: Option<u32>,
    /// Chance (percent, 0..100) that the event plays at all.
    pub probability: f32,
    /// `Volume_Saturation_Distance`: full volume inside this distance, attenuating outside.
    pub saturation_distance: Option<f32>,
    /// True for events without spatialization (`Is_2D` or `Is_3D = No`).
    pub is_2d: bool,
    /// `Is_GUI`.
    pub is_gui: bool,
    /// `Is_HUD_VO` (advisor / HUD voice-over).
    pub is_hud_vo: bool,
    /// `Is_Unit_Response_VO` (select / move / attack acknowledgements).
    pub is_unit_response_vo: bool,
    /// `Is_Ambient_VO` (battle chatter).
    pub is_ambient_vo: bool,
    /// `Play_Count = -1`: loops until stopped.
    pub looping: bool,
    /// `Loop_Fade_In_Seconds` / `Loop_Fade_Out_Seconds`.
    pub loop_fade: (f32, f32),
    /// `Overlap_Test`: events sharing this group name do not overlap each other (it is the name of a
    /// "voice" such as `Unit_Rebel_Space_Station`).
    pub overlap_group: Option<String>,
    /// `Localize`: the samples are language specific (speech).
    pub localize: bool,
    /// `Kills_Previous_Object_SFX`: starting this event stops the object's earlier sounds.
    pub kills_previous: bool,
    /// `Chained_SFXEvent`: event started when this one has finished.
    pub chained: Option<String>,
    /// `Min_Predelay` / `Max_Predelay` in milliseconds before the sound starts.
    pub predelay_ms: (f32, f32),
    /// `Text_ID`s (subtitles), parallel to `samples`.
    pub text_ids: Vec<String>,
}

/// All SFX events of the active game / mod.
#[derive(Debug, Default)]
pub struct SfxLibrary {
    events: HashMap<String, SfxEvent>,
}

impl SfxLibrary {
    /// Loads every file listed in `Xml/SFXEventFiles.xml`. Problems are reported as warnings.
    pub fn load(src: &dyn FileSource, warnings: &mut Vec<Warning>) -> Self {
        let mut raw: HashMap<String, (Element, String)> = HashMap::new();
        let mut order = Vec::new();
        let files = match src.read("Xml/SFXEventFiles.xml") {
            Ok(b) => xml::parse(&b, "SFXEventFiles.xml", warnings)
                .map(|root| {
                    root.children_named("file")
                        .map(|f| f.text_trimmed().to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            Err(e) => {
                warnings.push(Warning {
                    file: "SFXEventFiles.xml".into(),
                    line: 0,
                    message: e.to_string(),
                });
                Vec::new()
            }
        };
        for file in files {
            let path = format!("Xml/{file}");
            let Ok(bytes) = src.read(&path) else {
                warnings.push(Warning {
                    file: file.clone(),
                    line: 0,
                    message: "SFX event file not found".into(),
                });
                continue;
            };
            let Some(root) = xml::parse(&bytes, &file, warnings) else {
                continue;
            };
            for ev in root.children {
                if ev.name != "sfxevent" {
                    continue;
                }
                let Some(name) = ev.attr("name").map(str::to_string) else {
                    continue;
                };
                let key = name.to_ascii_lowercase();
                if !raw.contains_key(&key) {
                    order.push(key.clone());
                }
                raw.insert(key, (ev, file.clone()));
            }
        }
        let events = order
            .iter()
            .map(|key| {
                let (el, file) = &raw[key];
                (key.clone(), build_event(el, file, &raw))
            })
            .collect();
        Self { events }
    }

    /// Looks an event up by name (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&SfxEvent> {
        self.events.get(&name.to_ascii_lowercase())
    }

    /// Number of events including presets.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True if no events were loaded.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Iterates all events (unordered).
    pub fn iter(&self) -> impl Iterator<Item = &SfxEvent> {
        self.events.values()
    }

    /// Builds a library from already-parsed event elements (used by tests).
    pub fn from_elements(elements: Vec<(Element, String)>) -> Self {
        let raw: HashMap<String, (Element, String)> = elements
            .into_iter()
            .filter_map(|(e, f)| Some((e.attr("name")?.to_ascii_lowercase(), (e, f))))
            .collect();
        let events = raw
            .iter()
            .map(|(k, (e, f))| (k.clone(), build_event(e, f, &raw)))
            .collect();
        Self { events }
    }
}

/// First value of `tag` along the preset chain.
fn lookup<'a>(
    el: &'a Element,
    raw: &'a HashMap<String, (Element, String)>,
    tag: &str,
) -> Option<&'a str> {
    let mut cur = el;
    for _ in 0..8 {
        if let Some(v) = cur.child_text(tag) {
            return Some(v.trim());
        }
        let preset = cur.child_text("use_preset")?.trim().to_ascii_lowercase();
        cur = &raw.get(&preset)?.0;
    }
    None
}

fn words(s: &str) -> Vec<String> {
    xml::split_list(s)
}

fn build_event(el: &Element, file: &str, raw: &HashMap<String, (Element, String)>) -> SfxEvent {
    let get = |tag: &str| lookup(el, raw, tag);
    let num = |tag: &str| get(tag).and_then(|v| v.parse::<f32>().ok());
    let flag = |tag: &str| get(tag).map(parse_bool);
    let min_vol = num("min_volume");
    let max_vol = num("max_volume");
    let (vmin, vmax) = match (min_vol, max_vol) {
        (Some(a), Some(b)) => (a.min(b), a.max(b)),
        (Some(a), None) | (None, Some(a)) => (a, a),
        _ => (100.0, 100.0),
    };
    let (pmin, pmax) = match (num("min_pitch"), num("max_pitch")) {
        (Some(a), Some(b)) => (a.min(b), a.max(b)),
        (Some(a), None) | (None, Some(a)) => (a, a),
        _ => (100.0, 100.0),
    };
    let (dmin, dmax) = match (num("min_predelay"), num("max_predelay")) {
        (Some(a), Some(b)) => (a.min(b), a.max(b)),
        (Some(a), None) | (None, Some(a)) => (a, a),
        _ => (0.0, 0.0),
    };
    let is_2d = flag("is_2d").unwrap_or(false) || flag("is_3d") == Some(false);
    SfxEvent {
        name: el.attr("name").unwrap_or_default().to_string(),
        source_file: file.to_string(),
        is_preset: el.child_text("is_preset").map(parse_bool).unwrap_or(false),
        samples: get("samples").map(words).unwrap_or_default(),
        pre_samples: get("pre_samples").map(words).unwrap_or_default(),
        post_samples: get("post_samples").map(words).unwrap_or_default(),
        play_sequentially: flag("play_sequentially").unwrap_or(false),
        volume: (vmin, vmax),
        pitch: (pmin, pmax),
        priority: num("priority")
            .map(|p| p.clamp(1.0, 255.0) as u8)
            .unwrap_or(3),
        max_instances: num("max_instances").map(|n| n.max(1.0) as u32),
        probability: num("probability").unwrap_or(100.0).clamp(0.0, 100.0),
        saturation_distance: num("volume_saturation_distance"),
        is_2d,
        is_gui: flag("is_gui").unwrap_or(false),
        is_hud_vo: flag("is_hud_vo").unwrap_or(false),
        is_unit_response_vo: flag("is_unit_response_vo").unwrap_or(false),
        is_ambient_vo: flag("is_ambient_vo").unwrap_or(false),
        looping: num("play_count").is_some_and(|c| c < 0.0),
        loop_fade: (
            num("loop_fade_in_seconds").unwrap_or(0.0),
            num("loop_fade_out_seconds").unwrap_or(0.0),
        ),
        overlap_group: get("overlap_test")
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        localize: flag("localize").unwrap_or(false),
        kills_previous: flag("kills_previous_object_sfx").unwrap_or(false),
        chained: get("chained_sfxevent")
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        predelay_ms: (dmin, dmax),
        text_ids: get("text_id").map(words).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib(xml_text: &str) -> SfxLibrary {
        let mut w = Vec::new();
        let root = xml::parse_str(xml_text, "t.xml", &mut w).unwrap();
        SfxLibrary::from_elements(
            root.children
                .into_iter()
                .map(|e| (e, "t.xml".to_string()))
                .collect(),
        )
    }

    #[test]
    fn preset_inheritance_and_overrides() {
        let l = lib(r#"<SFXEvents>
            <SFXEvent Name="Preset_A"><Is_Preset>Yes</Is_Preset><Is_3D>No</Is_3D><Priority>1</Priority>
              <Min_Volume>80</Min_Volume><Max_Volume>90</Max_Volume><Max_Instances>2</Max_Instances></SFXEvent>
            <SFXEvent Name="Preset_B"><Use_Preset>Preset_A</Use_Preset><Priority>4</Priority></SFXEvent>
            <SFXEvent Name="Boom"><Use_Preset>Preset_B</Use_Preset>
              <Samples>Data\Audio\SFX\a.wav, b.wav
                 c.wav</Samples><Play_Sequentially>Yes</Play_Sequentially><Play_Count>-1</Play_Count>
              <Probability>60</Probability></SFXEvent></SFXEvents>"#);
        let e = l.get("BOOM").unwrap();
        assert_eq!(e.samples.len(), 3);
        assert!(e.is_2d && e.play_sequentially && e.looping);
        assert_eq!(e.priority, 4);
        assert_eq!(e.volume, (80.0, 90.0));
        assert_eq!(e.max_instances, Some(2));
        assert_eq!(e.probability, 60.0);
        assert!(l.get("preset_a").unwrap().is_preset);
    }
}
