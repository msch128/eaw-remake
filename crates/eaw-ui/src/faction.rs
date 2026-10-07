//! Faction skins of the interface.

/// The three playable factions of Forces of Corruption with their own interface art.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UiFaction {
    /// Galactic Empire.
    #[default]
    Empire,
    /// Rebel Alliance.
    Rebel,
    /// Zann Consortium (Underworld).
    Underworld,
}

impl UiFaction {
    /// Parses a faction object name (`Empire`, `Rebel`, `Underworld` ...), case-insensitive.
    /// Unknown factions use the Empire skin.
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_lowercase().as_str() {
            "rebel" | "rebels" | "alliance" => Self::Rebel,
            "underworld" | "zann" | "pirates" | "hutts" => Self::Underworld,
            _ => Self::Empire,
        }
    }

    /// The `_ALTn` suffix of this faction's frame meshes in the shell models
    /// (`Empire_Faceplate_ALT0`, `Rebel_Faceplate_ALT1`, `Underworld_Faceplate_ALT2`).
    pub fn mesh_alt(self) -> usize {
        match self {
            Self::Empire => 0,
            Self::Rebel => 1,
            Self::Underworld => 2,
        }
    }

    /// Index into per-faction texture lists of `CommandBarComponents.xml`
    /// (`i_button_pad_rebel.tga i_button_pad_empire.tga`: Rebel, Empire, Underworld ...).
    pub fn list_index(self) -> usize {
        match self {
            Self::Rebel => 0,
            Self::Empire => 1,
            Self::Underworld => 2,
        }
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Empire => "Empire",
            Self::Rebel => "Rebel",
            Self::Underworld => "Underworld",
        }
    }
}
