//! `CommandBarComponents.xml`: the definition of every UI element of the original interface.
//!
//! The XML contains the *look* of each component (textures per state, font, text placement,
//! tooltip text ids, flags). Where a component sits on screen is not in the XML but in the shell
//! models, see [`crate::shell`].

use std::collections::HashMap;

use alamo_data::xml::{self, Element, Warning};
use alamo_fs::FileSource;
use bevy::prelude::*;

/// Component type (`<Type>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentKind {
    /// A 3D/2D shell model that holds the frame art and the placement of other components.
    Shell,
    /// An icon button with state textures.
    Button,
    /// A button that also carries text (labels, counters and also most pads).
    TextButton,
    /// A discrete-level status bar (health, shield, build progress).
    Bar,
    /// A static icon.
    Icon,
    /// Anything else.
    Other,
}

/// One `<CommandBarComponent>`.
#[derive(Debug, Clone, Default)]
pub struct CommandBarComponent {
    /// Component name as written in the XML (original capitalization).
    pub name: String,
    /// Component type.
    pub kind: Option<ComponentKind>,
    /// `<Group>`.
    pub group: String,
    /// `<Model_Name>` (shells).
    pub model: Option<String>,
    /// `<Blank_Texture_Name>`: background pad; one entry per faction when several are listed.
    pub blank: Vec<String>,
    /// `<Icon_Texture_Name>`.
    pub icon: Vec<String>,
    /// `<Icon_Alternate_Texture_Name>`: icon variants selected by an index (faction, state ...).
    pub icon_alt: Vec<String>,
    /// `<Mouse_Over_Texture_Name>`.
    pub mouse_over: Vec<String>,
    /// `<Selected_Texture_Name>` (pressed / toggled on).
    pub selected: Vec<String>,
    /// `<Disabled_Texture_Name>`.
    pub disabled: Vec<String>,
    /// `<Build_Texture_Name>` (progress dial / clock).
    pub build: Vec<String>,
    /// `<Flash_Texture_Name>`.
    pub flash: Vec<String>,
    /// `<Overlay_Texture_Name>`.
    pub overlay: Vec<String>,
    /// `<Overlay2_Texture_Name>`.
    pub overlay2: Vec<String>,
    /// `<Lower_Effect_Texture_Name>`.
    pub lower_effect: Vec<String>,
    /// `<Bar_Texture_Name>`: one frame texture per bar level.
    pub bar_texture: Vec<String>,
    /// `<Bar_Overlay_Name>`: one fill texture per bar level.
    pub bar_overlay: Vec<String>,
    /// `<Max_Bar_Level>`.
    pub max_bar_level: u32,
    /// `<Scale>`: scale of the texture relative to its natural size.
    pub scale: f32,
    /// `<Base_Layer>`: draw order inside the shell.
    pub base_layer: i32,
    /// `<Font_Name>`.
    pub font_name: Option<String>,
    /// `<Font_Point_Size>`.
    pub font_size: f32,
    /// `<Text_Offset>`.
    pub text_offset: Vec2,
    /// `<Text_Offset2>` (second text line).
    pub text_offset2: Vec2,
    /// `<Text_Color>` (RGBA 0..255).
    pub text_color: Option<[u8; 4]>,
    /// `<Text_Color2>`.
    pub text_color2: Option<[u8; 4]>,
    /// `<Icon_Offset>`.
    pub icon_offset: Vec2,
    /// `<Offset>`.
    pub offset: Vec2,
    /// `<Size>`.
    pub size: Option<Vec2>,
    /// `<Color>` tint (RGBA 0..255).
    pub color: Option<[u8; 4]>,
    /// `<Tooltip_Text>`: text ids (several for state dependent tooltips).
    pub tooltips: Vec<String>,
    /// `<Mouse_Over_SFX>`.
    pub mouse_over_sfx: Option<String>,
    /// `<Click_SFX>`.
    pub click_sfx: Option<String>,
    /// `<Left_Justified>`.
    pub left_justified: bool,
    /// `<Right_Justified>`.
    pub right_justified: bool,
    /// `<Text_Outline>`.
    pub text_outline: bool,
    /// `<Toggle>`: stays pressed after a click.
    pub toggle: bool,
    /// `<Tab>`: member of an exclusive tab group.
    pub tab: bool,
    /// `<Selected_Alpha>`.
    pub selected_alpha: bool,
    /// `<Swap_Texture>`.
    pub swap_texture: bool,
    /// `<Hidden>`.
    pub hidden: bool,
    /// `<Disabled>`.
    pub start_disabled: bool,
    /// `<Drag_And_Drop>`.
    pub drag_and_drop: bool,
    /// Every tag as `lower-case name -> trimmed text`, for anything not typed above.
    pub raw: HashMap<String, String>,
}

impl CommandBarComponent {
    fn from_element(el: &Element) -> Option<Self> {
        let name = el.attr("name")?.to_string();
        let mut raw = HashMap::new();
        for c in &el.children {
            raw.insert(c.name.clone(), c.text_trimmed().to_string());
        }
        let list = |k: &str| raw.get(k).map(|s| xml::split_list(s)).unwrap_or_default();
        let flag = |k: &str| raw.get(k).is_some_and(|s| xml::parse_bool(s));
        let num = |k: &str, d: f32| {
            raw.get(k)
                .and_then(|s| s.trim().parse::<f32>().ok())
                .unwrap_or(d)
        };
        let vec2 = |k: &str| {
            raw.get(k)
                .map(|s| xml::parse_floats(s))
                .filter(|v| v.len() >= 2)
                .map(|v| Vec2::new(v[0], v[1]))
        };
        let rgba = |k: &str| {
            raw.get(k)
                .map(|s| xml::parse_floats(s))
                .filter(|v| v.len() >= 3)
                .map(|v| {
                    [
                        v[0] as u8,
                        v[1] as u8,
                        v[2] as u8,
                        v.get(3).map_or(255, |a| *a as u8),
                    ]
                })
        };
        let kind = raw.get("type").map(|t| match t.to_lowercase().as_str() {
            "shell" => ComponentKind::Shell,
            "button" => ComponentKind::Button,
            "textbutton" => ComponentKind::TextButton,
            "bar" => ComponentKind::Bar,
            "icon" => ComponentKind::Icon,
            _ => ComponentKind::Other,
        });
        Some(Self {
            name,
            kind,
            group: raw.get("group").cloned().unwrap_or_default(),
            model: raw.get("model_name").filter(|s| !s.is_empty()).cloned(),
            blank: list("blank_texture_name"),
            icon: list("icon_texture_name"),
            icon_alt: list("icon_alternate_texture_name"),
            mouse_over: list("mouse_over_texture_name"),
            selected: list("selected_texture_name"),
            disabled: list("disabled_texture_name"),
            build: list("build_texture_name"),
            flash: list("flash_texture_name"),
            overlay: list("overlay_texture_name"),
            overlay2: list("overlay2_texture_name"),
            lower_effect: list("lower_effect_texture_name"),
            bar_texture: list("bar_texture_name"),
            bar_overlay: list("bar_overlay_name"),
            max_bar_level: num("max_bar_level", 0.0) as u32,
            scale: num("scale", 1.0),
            base_layer: num("base_layer", 0.0) as i32,
            font_name: raw.get("font_name").filter(|s| !s.is_empty()).cloned(),
            font_size: num("font_point_size", 7.0),
            text_offset: vec2("text_offset").unwrap_or_default(),
            text_offset2: vec2("text_offset2").unwrap_or_default(),
            text_color: rgba("text_color"),
            text_color2: rgba("text_color2"),
            icon_offset: vec2("icon_offset").unwrap_or_default(),
            offset: vec2("offset").unwrap_or_default(),
            size: vec2("size"),
            color: rgba("color"),
            tooltips: list("tooltip_text"),
            mouse_over_sfx: raw.get("mouse_over_sfx").cloned(),
            click_sfx: raw.get("click_sfx").cloned(),
            left_justified: flag("left_justified"),
            right_justified: flag("right_justified"),
            text_outline: flag("text_outline"),
            toggle: flag("toggle"),
            tab: flag("tab"),
            selected_alpha: flag("selected_alpha"),
            swap_texture: flag("swap_texture"),
            hidden: flag("hidden"),
            start_disabled: flag("disabled"),
            drag_and_drop: flag("drag_and_drop"),
            raw,
        })
    }

    /// Faction dependent entry of a texture list: single-entry lists apply to everybody.
    pub fn pick<'a>(list: &'a [String], index: usize) -> Option<&'a str> {
        list.get(index).or_else(|| list.first()).map(String::as_str)
    }
}

/// All components by lower-cased name (resource).
#[derive(Resource, Default, Debug)]
pub struct CommandBarDb {
    components: HashMap<String, CommandBarComponent>,
}

impl CommandBarDb {
    /// Loads `CommandBarComponentFiles.xml` and every file it lists (falling back to
    /// `CommandBarComponents.xml`). Problems are returned as warnings.
    pub fn load(src: &dyn FileSource) -> (Self, Vec<Warning>) {
        let mut warnings = Vec::new();
        let mut files = Vec::new();
        if let Ok(bytes) = src.read("Xml/CommandBarComponentFiles.xml")
            && let Some(root) = xml::parse(&bytes, "CommandBarComponentFiles.xml", &mut warnings)
        {
            files.extend(
                root.children_named("file")
                    .map(|f| f.text_trimmed().to_string()),
            );
        }
        if files.is_empty() {
            files.push("CommandBarComponents.xml".to_string());
        }
        let mut db = Self::default();
        for file in files {
            let path = format!("Xml/{file}");
            let Ok(bytes) = src.read(&path) else {
                warnings.push(Warning {
                    file: path,
                    line: 0,
                    message: "file not found".into(),
                });
                continue;
            };
            let Some(root) = xml::parse(&bytes, &file, &mut warnings) else {
                continue;
            };
            for el in root.children_named("commandbarcomponent") {
                if let Some(c) = CommandBarComponent::from_element(el) {
                    db.components.insert(c.name.to_lowercase(), c);
                }
            }
        }
        (db, warnings)
    }

    /// Looks a component up by name (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&CommandBarComponent> {
        self.components.get(&name.to_lowercase())
    }

    /// Number of components.
    pub fn len(&self) -> usize {
        self.components.len()
    }

    /// True if no component was loaded.
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    /// Iterates all components.
    pub fn iter(&self) -> impl Iterator<Item = &CommandBarComponent> {
        self.components.values()
    }
}

