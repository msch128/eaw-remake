//! The campaign's story objectives, read from the game data: the story plots of the human faction (`<Faction>_Story_Name`
//! in the campaign file) list events that open a story dialog chapter (`Story_Dialog` + `Story_Chapter`); the chapter
//! texts (`Scripts/Story/<Dialog>.TXT`) are what the original shows in the "Active Story Objectives" window.
//!
//! Only the events that start with the game (`STORY_ELAPSED`) or follow such an event (`Prereq`) are listed: the
//! Remake's welcome guide and overhaul chapters. Reacting to the simulation (completing objectives) is not done: the
//! scripted simulation does not forward `Add_Objective` calls.

use alamo_data::GameData;
use alamo_fs::{FileSource, GameFs};

/// A coloured piece of an objective's text.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Colour as `[r, g, b]` bytes.
    pub color: [u8; 3],
    /// The text (may contain line breaks).
    pub text: String,
}

/// One entry of the objectives list.
#[derive(Debug, Clone, PartialEq)]
pub struct Objective {
    /// Title shown in the list (the chapter's first text line).
    pub title: String,
    /// Colour of the title.
    pub color: [u8; 3],
    /// Story chapter number (the list shows the highest first).
    pub chapter: u32,
    /// The advisor transmission of the chapter.
    pub text: Vec<Segment>,
}

/// Objectives of `campaign` for the human `faction` (`Rebel`, `Empire`, `Underworld`), highest chapter first.
pub fn load(fs: &GameFs, data: &GameData, campaign: &str, faction: &str) -> Vec<Objective> {
    let Some(story) = data
        .campaign(campaign)
        .and_then(|c| c.faction(faction))
        .and_then(|f| f.story_name.clone())
        .or_else(|| data.campaign(campaign).and_then(|c| c.fields.get(&format!("{}_story_name", faction.to_ascii_lowercase())).cloned()))
    else {
        return Vec::new();
    };
    let read = |path: &str| fs.read(&format!("Data/{path}")).ok().map(|b| String::from_utf8_lossy(&b).into_owned());
    let Some(plots) = read(&format!("XML/{}", story.trim())) else { return Vec::new() };
    let mut out: Vec<Objective> = Vec::new();
    let mut seen: Vec<(String, u32)> = Vec::new();
    for plot in tag_values(&plots, "Active_Plot") {
        let Some(xml) = read(&format!("XML/{plot}")) else { continue };
        for (dialog, chapter) in dialog_events(&xml) {
            if seen.contains(&(dialog.to_ascii_lowercase(), chapter)) {
                continue;
            }
            seen.push((dialog.to_ascii_lowercase(), chapter));
            let Some(txt) = read(&format!("Scripts/Story/{dialog}.TXT")) else { continue };
            if let Some(o) = chapter_text(&txt, chapter, data) {
                out.push(o);
            }
        }
    }
    out.sort_by_key(|o| std::cmp::Reverse(o.chapter));
    out
}

/// Values of all `<tag>value</tag>` in `xml`, trimmed.
fn tag_values(xml: &str, tag: &str) -> Vec<String> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(a) = rest.find(&open) {
        let after = &rest[a + open.len()..];
        let Some(b) = after.find(&close) else { break };
        let value = after[..b].trim();
        if !value.is_empty() {
            out.push(value.to_owned());
        }
        rest = &after[b + close.len()..];
    }
    out
}

/// `(dialog, chapter)` of the events that start with the game or follow such an event, in file order.
fn dialog_events(xml: &str) -> Vec<(String, u32)> {
    struct Ev {
        name: String,
        elapsed: bool,
        prereq: Option<String>,
        dialog: Option<String>,
        chapter: Option<u32>,
    }
    let mut events: Vec<Ev> = Vec::new();
    let mut cur: Option<Ev> = None;
    for line in xml.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("<Event Name=\"") {
            let name = rest.split('"').next().unwrap_or("").to_owned();
            cur = Some(Ev { name, elapsed: false, prereq: None, dialog: None, chapter: None });
        } else if l.starts_with("</Event>") {
            events.extend(cur.take());
        } else if let Some(e) = cur.as_mut() {
            let value = |tag: &str| l.strip_prefix(&format!("<{tag}>")).and_then(|r| r.split('<').next()).map(|v| v.trim().to_owned());
            if let Some(t) = value("Event_Type") {
                e.elapsed = t.eq_ignore_ascii_case("STORY_ELAPSED");
            } else if let Some(p) = value("Prereq") {
                e.prereq = Some(p);
            } else if let Some(d) = value("Story_Dialog") {
                e.dialog = Some(d);
            } else if let Some(c) = value("Story_Chapter") {
                e.chapter = c.parse().ok();
            }
        }
    }
    let mut kept: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for e in &events {
        let reachable = e.elapsed || e.prereq.as_ref().is_some_and(|p| kept.iter().any(|k| k.eq_ignore_ascii_case(p)));
        if !reachable {
            continue;
        }
        kept.push(e.name.clone());
        if let (Some(d), Some(c)) = (&e.dialog, e.chapter) {
            out.push((d.clone(), c));
        }
    }
    out
}

/// The chapter `n` of a story dialog text file.
fn chapter_text(txt: &str, n: u32, data: &GameData) -> Option<Objective> {
    let header = format!("[CHAPTER {n}]");
    let mut inside = false;
    let mut color = [255u8, 255, 255];
    let mut segments: Vec<Segment> = Vec::new();
    let mut title: Option<(String, [u8; 3])> = None;
    for line in txt.lines() {
        let l = line.trim();
        if l.starts_with("[CHAPTER") {
            if inside {
                break;
            }
            inside = l.eq_ignore_ascii_case(&header);
            continue;
        }
        if !inside || l.is_empty() {
            continue;
        }
        let (cmd, arg) = l.split_once(char::is_whitespace).map_or((l, ""), |(a, b)| (a, b.trim()));
        match cmd.to_ascii_uppercase().as_str() {
            "TEXTCOLOR" => {
                let v: Vec<u8> = arg.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                if v.len() >= 3 {
                    color = [v[0], v[1], v[2]];
                }
            }
            "TEXT" => {
                let text = data.text(arg).unwrap_or(arg).replace("\\n", "\n");
                if title.is_none() {
                    title = Some((text.clone(), color));
                }
                segments.push(Segment { color, text });
            }
            "NEWLINE" => {
                let count: usize = arg.parse().unwrap_or(1);
                segments.push(Segment { color, text: "\n".repeat(count.max(1)) });
            }
            _ => {}
        }
    }
    let (title, color) = title?;
    Some(Objective { title, color, chapter: n, text: segments })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_chain_from_the_elapsed_start() {
        let xml = r#"<Story>
<Event Name="A"><Event_Type>STORY_ELAPSED</Event_Type></Event>
<Event Name="Intro_Start">
<Event_Type>STORY_ELAPSED</Event_Type>
<Story_Dialog>Dialog_Intro</Story_Dialog>
<Story_Chapter>0</Story_Chapter>
</Event>
<Event Name="Intro_2">
<Event_Type>STORY_TRIGGER</Event_Type>
<Story_Dialog>Dialog_Intro</Story_Dialog>
<Story_Chapter>1</Story_Chapter>
<Prereq>Intro_Start</Prereq>
</Event>
<Event Name="Later">
<Event_Type>STORY_TRIGGER</Event_Type>
<Story_Dialog>Dialog_Other</Story_Dialog>
<Story_Chapter>7</Story_Chapter>
<Prereq>Unknown</Prereq>
</Event>
</Story>"#;
        assert_eq!(dialog_events(xml), vec![("Dialog_Intro".to_owned(), 0), ("Dialog_Intro".to_owned(), 1)]);
    }

    /// The Remake's welcome guide and overhaul chapters of the video appear for the Web Way campaign (skips without a game install).
    #[test]
    fn web_way_campaign_lists_the_welcome_guide() {
        let Some(install) = alamo_fs::GameInstall::detect() else { return };
        let Ok(fs) = GameFs::for_mod(&install, Some("1770851727")) else { return };
        let (data, _) = alamo_data::load_galaxy(&fs);
        let list = load(&fs, &data, "GC_Full_Web_Way_Rebel", "Rebel");
        let titles: Vec<&str> = list.iter().map(|o| o.title.as_str()).collect();
        assert!(titles.contains(&"Welcome Guide"), "{titles:?}");
        assert!(titles.iter().any(|t| t.contains("Galactic Conquest Overhauls")), "{titles:?}");
        let welcome = list.iter().find(|o| o.title == "Welcome Guide").unwrap();
        assert!(welcome.text.iter().any(|s| s.text.contains("first time player")));
    }

    #[test]
    fn tag_values_are_trimmed() {
        assert_eq!(tag_values("<A> x </A><A>y</A>", "A"), vec!["x", "y"]);
    }
}
