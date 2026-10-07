//! Decoupled view model of the galactic conquest map.
//!
//! The renderer, camera, UI and picking code only ever read a [`GalaxyView`]. Real game data
//! (parsed from the Empire at War XML by `alamo-data`) is mapped into these plain types, so the
//! shell never depends on the data layer. [`GalaxyView::demo`] provides hand-placed stand-in data.

use bevy::prelude::{Resource, Vec3};

/// Everything the galaxy renderer needs to draw the conquest map.
#[derive(Resource, Debug, Clone, Default)]
pub struct GalaxyView {
    /// All planets; [`RouteView`] and [`PlanetView::owner`] index into / alongside these vectors.
    pub planets: Vec<PlanetView>,
    /// Hyperspace trade routes between planets (undirected).
    pub routes: Vec<RouteView>,
    /// Factions; [`PlanetView::owner`] indexes into this vector.
    pub factions: Vec<FactionView>,
    /// Trade route line styles; [`RouteView::style`] indexes into this vector.
    pub route_styles: Vec<RouteStyleView>,
    /// The human player's faction (index into [`GalaxyView::factions`]), if known.
    pub player_faction: Option<usize>,
    /// Decorative objects that are part of the galaxy scene (galaxy disc art, nebulae, logos).
    pub props: Vec<PropView>,
    /// Galactic map lighting.
    pub light: GalaxyLight,
}

/// A decorative (non-selectable) model placed on the galaxy map.
#[derive(Debug, Clone, PartialEq)]
pub struct PropView {
    /// Object name from the data, e.g. `Galaxy_Core_Art_Model_Web_Way`.
    pub id: String,
    /// Model file name, e.g. `w_galaxy00.alo`.
    pub model: String,
    /// Position in EaW galactic coordinates (see [`PlanetView::position`]).
    pub position: Vec3,
    /// Uniform model scale (`Scale_Factor`).
    pub scale: f32,
}

/// Lighting of the galactic map (the single sun shared by all planets).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GalaxyLight {
    /// Light angle in degrees as in `Gameconstants.xml` (`Galactic_Zoom_Out_Light_Angle`).
    pub angle_deg: Vec3,
    /// Light strength, 1.0 is full white (`Galactic_Zoom_Light_Level`).
    pub level: f32,
}

impl Default for GalaxyLight {
    fn default() -> Self {
        Self { angle_deg: Vec3::new(0.0, 60.0, -30.0), level: 0.75 }
    }
}

/// One planet on the galactic map.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanetView {
    /// Stable internal identifier (the XML object name, e.g. `CORUSCANT`).
    pub id: String,
    /// Localised, human readable name shown in labels and the info panel.
    pub display_name: String,
    /// Position in Empire at War galactic coordinates: `x`/`y` span the galaxy plane
    /// (`y` is "north" on the map) and `z` is the height above the plane.
    pub position: Vec3,
    /// Index into [`GalaxyView::factions`] of the owning faction, `None` for neutral planets.
    pub owner: Option<usize>,
    /// Credits income per turn shown in the info panel.
    pub income: i32,
    /// Visual radius multiplier relative to a "normal" planet (`1.0`); used for placeholders.
    pub size: f32,
    /// Galactic model file name (`Galactic_Model_Name`), `None` for demo data.
    pub model: Option<String>,
    /// Model scale (`Scale_Factor`); the model's own radius times this is the world radius.
    pub model_scale: f32,
    /// Placeholder surface colour (linear-ish sRGB, 0..1) until the real `.alo` models are loaded.
    pub surface_color: [f32; 3],
    /// Number of space structure (starbase) slots, `Max_Space_Base`.
    pub max_space_base: i32,
    /// Extra special structure slots in space.
    pub special_slots: i32,
    /// Planet ability as `(name, description)` in English, if the planet has one.
    pub ability: Option<(String, String)>,
    /// Display names of the structures/forces the campaign places here at the start.
    pub starting_forces: Vec<String>,
}

/// A trade route / hyperspace lane between two planets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteView {
    /// Index into [`GalaxyView::planets`].
    pub a: usize,
    /// Index into [`GalaxyView::planets`].
    pub b: usize,
    /// Index into [`GalaxyView::route_styles`].
    pub style: usize,
}

/// Visual settings of a trade route line for one relation to the player.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineStyleView {
    /// RGBA (sRGB, 0..1) when zoomed far out.
    pub color_far: [f32; 4],
    /// RGBA (sRGB, 0..1) when zoomed in close.
    pub color_near: [f32; 4],
    /// Line width in the game's units (`Width`, typically 0.5 to 6).
    pub width: f32,
}

/// A named trade route line style (`TradeRouteLine`) with one look per relation.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteStyleView {
    /// Style name, e.g. `Web_Way_Main`.
    pub name: String,
    /// Route controlled by the player (`Ally` settings).
    pub own: LineStyleView,
    /// Route controlled by an enemy (`Enemy` settings).
    pub enemy: LineStyleView,
    /// Neutral or contested route (`Neutral` settings).
    pub neutral: LineStyleView,
}

/// A playable faction.
#[derive(Debug, Clone, PartialEq)]
pub struct FactionView {
    /// Display name (e.g. `Galactic Empire`).
    pub name: String,
    /// Faction colour in sRGB, each channel 0..1.
    pub color: [f32; 3],
    /// Credits the faction has (the campaign's starting credits until a game state exists).
    pub credits: i64,
}

impl RouteStyleView {
    /// Dark teal line used when no style data is available.
    pub fn fallback() -> Self {
        let line = LineStyleView { color_far: [0.05, 0.12, 0.15, 1.0], color_near: [0.05, 0.12, 0.15, 1.0], width: 1.0 };
        Self { name: "Default".into(), own: line, enemy: line, neutral: line }
    }
}

impl GalaxyView {
    /// Returns the faction that owns `planet`, if any.
    pub fn owner_of(&self, planet: usize) -> Option<&FactionView> {
        self.planets
            .get(planet)
            .and_then(|p| p.owner)
            .and_then(|f| self.factions.get(f))
    }

    /// Hand-placed demo galaxy (about 40 recognisable planets) used until real data is plugged in.
    #[allow(clippy::type_complexity)]
    pub fn demo() -> Self {
        const EMPIRE: Option<usize> = Some(0);
        const REBEL: Option<usize> = Some(1);
        const UNDER: Option<usize> = Some(2);
        const NONE: Option<usize> = None;

        // Placeholder surface palettes.
        const URBAN: [f32; 3] = [0.62, 0.62, 0.68];
        const DESERT: [f32; 3] = [0.82, 0.62, 0.36];
        const OCEAN: [f32; 3] = [0.25, 0.48, 0.80];
        const FOREST: [f32; 3] = [0.28, 0.58, 0.30];
        const ICE: [f32; 3] = [0.82, 0.90, 0.96];
        const GAS: [f32; 3] = [0.86, 0.66, 0.46];
        const VOLCANIC: [f32; 3] = [0.70, 0.26, 0.14];
        const SWAMP: [f32; 3] = [0.40, 0.46, 0.26];
        const BARREN: [f32; 3] = [0.55, 0.50, 0.46];

        let factions = vec![
            FactionView {
                name: "Galactic Empire".into(),
                color: [0.30, 0.85, 0.45],
                credits: 40_000,
            },
            FactionView {
                name: "Rebel Alliance".into(),
                color: [1.00, 0.28, 0.22],
                credits: 15_000,
            },
            FactionView {
                name: "Underworld".into(),
                color: [1.00, 0.72, 0.15],
                credits: 20_000,
            },
        ];

        // (id, name, x, y, z, owner, income, size, surface)
        #[rustfmt::skip]
        let raw: &[(&str, &str, f32, f32, f32, Option<usize>, i32, f32, [f32; 3])] = &[
            ("CORUSCANT",     "Coruscant",      -10.0,   5.0,  0.0, EMPIRE, 1500, 1.3, URBAN),
            ("KUAT",          "Kuat",            24.0,  28.0,  2.0, EMPIRE, 1100, 1.1, URBAN),
            ("CORELLIA",      "Corellia",       -34.0, -26.0, -2.0, EMPIRE,  900, 1.1, OCEAN),
            ("CHANDRILA",     "Chandrila",      -52.0,  34.0,  1.0, EMPIRE,  500, 1.0, FOREST),
            ("ANAXES",        "Anaxes",          46.0,  -6.0,  0.0, EMPIRE,  600, 1.0, URBAN),
            ("BYSS",          "Byss",            -4.0,  62.0,  3.0, EMPIRE,  700, 1.0, VOLCANIC),
            ("ENDOR",         "Endor",         -190.0, -58.0,  0.0, EMPIRE,  300, 0.9, FOREST),
            ("FONDOR",        "Fondor",          60.0, -34.0, -2.0, EMPIRE,  600, 1.0, URBAN),
            ("BORLEIAS",      "Borleias",       -78.0, -44.0,  2.0, EMPIRE,  350, 0.9, DESERT),
            ("MUSTAFAR",      "Mustafar",       -92.0,  70.0, -3.0, EMPIRE,  400, 0.9, VOLCANIC),
            ("SALEUCAMI",     "Saleucami",      -16.0, -78.0,  0.0, EMPIRE,  300, 0.9, BARREN),
            ("YAVIN",         "Yavin 4",        150.0,-122.0,  0.0, REBEL,   400, 1.0, FOREST),
            ("HOTH",          "Hoth",           204.0,-172.0,  4.0, REBEL,   250, 0.9, ICE),
            ("DANTOOINE",     "Dantooine",      108.0, -92.0, -2.0, REBEL,   350, 1.0, FOREST),
            ("MON_CALAMARI",  "Mon Calamari",   120.0,  74.0,  0.0, REBEL,   900, 1.2, OCEAN),
            ("SULLUST",       "Sullust",         70.0, -82.0,  2.0, REBEL,   800, 1.0, VOLCANIC),
            ("ALDERAAN_REM",  "Alderaan",       -62.0,  96.0, -2.0, REBEL,   600, 1.0, FOREST),
            ("HOLOTHAN",      "Ord Mantell",    -70.0, -86.0, -1.0, REBEL,   300, 0.9, BARREN),
            ("KESSEL",        "Kessel",         112.0,  38.0,  0.0, UNDER,   450, 0.9, BARREN),
            ("NAL_HUTTA",     "Nal Hutta",      144.0, 160.0,  3.0, UNDER,   800, 1.2, SWAMP),
            ("TATOOINE",      "Tatooine",       214.0,  84.0,  0.0, UNDER,   500, 1.0, DESERT),
            ("NAR_SHADDAA",   "Nar Shaddaa",    174.0, 120.0, -2.0, UNDER,   700, 1.0, URBAN),
            ("TERRIDON",      "Teth",           -150.0, 118.0, 1.0, UNDER,   350, 0.9, SWAMP),
            ("KASHYYYK",      "Kashyyyk",       -96.0, -12.0,  0.0, NONE,    450, 1.2, FOREST),
            ("NABOO",         "Naboo",           34.0, -34.0,  0.0, NONE,    500, 1.0, OCEAN),
            ("BESPIN",        "Bespin",         190.0, -34.0, -3.0, NONE,    450, 1.4, GAS),
            ("DAGOBAH",       "Dagobah",       -122.0,-142.0,  0.0, NONE,    100, 0.8, SWAMP),
            ("GEONOSIS",      "Geonosis",       136.0,   2.0,  0.0, NONE,    300, 0.9, DESERT),
            ("ALZOC_III",     "Alzoc III",      -38.0, 128.0,  2.0, NONE,    200, 0.9, ICE),
            ("UMBARA",        "Umbara",          18.0, 104.0, -2.0, NONE,    250, 0.9, BARREN),
            ("RYLOTH",        "Ryloth",         168.0,  44.0,  2.0, NONE,    300, 0.9, DESERT),
            ("DATHOMIR",      "Dathomir",       -48.0, -134.0, 0.0, NONE,    250, 0.9, VOLCANIC),
            ("MANDALORE",     "Mandalore",      -30.0,  -2.0, -4.0, NONE,    450, 1.0, BARREN),
            ("ABREGADO_RAE",  "Abregado-Rae",    -2.0, -44.0,  0.0, NONE,    300, 0.9, URBAN),
            ("CATO_NEIMOIDIA","Cato Neimoidia",  88.0,  92.0,  0.0, NONE,    350, 0.9, FOREST),
            ("UTAPAU",        "Utapau",         -118.0,  38.0, -1.0, NONE,   200, 0.9, BARREN),
            ("FELUCIA",       "Felucia",         86.0,-136.0,  2.0, NONE,    300, 1.0, SWAMP),
            ("MUUNILINST",    "Muunilinst",      30.0,  -92.0, 0.0, NONE,    500, 1.0, URBAN),
            ("HYPORI",        "Hypori",         -82.0, 136.0,  0.0, NONE,    150, 0.8, BARREN),
            ("JAKKU",         "Jakku",          232.0, -96.0, -2.0, NONE,    100, 0.8, DESERT),
        ];

        let planets: Vec<PlanetView> = raw
            .iter()
            .map(
                |&(id, name, x, y, z, owner, income, size, surface)| PlanetView {
                    id: id.to_owned(),
                    display_name: name.to_owned(),
                    position: Vec3::new(x, y, z),
                    owner,
                    income,
                    size,
                    model: None,
                    model_scale: 1.0,
                    surface_color: surface,
                    max_space_base: 3,
                    special_slots: 6,
                    ability: None,
                    starting_forces: Vec::new(),
                },
            )
            .collect();

        #[rustfmt::skip]
        let route_names: &[(&str, &str)] = &[
            ("CORUSCANT", "KUAT"), ("CORUSCANT", "CORELLIA"), ("CORUSCANT", "CHANDRILA"),
            ("CORUSCANT", "BYSS"), ("CORUSCANT", "MANDALORE"), ("CORUSCANT", "ANAXES"),
            ("KUAT", "ANAXES"), ("KUAT", "BYSS"), ("KUAT", "CATO_NEIMOIDIA"),
            ("ANAXES", "FONDOR"), ("ANAXES", "GEONOSIS"), ("FONDOR", "NABOO"),
            ("NABOO", "SULLUST"), ("NABOO", "ABREGADO_RAE"), ("CORELLIA", "ABREGADO_RAE"),
            ("CORELLIA", "KASHYYYK"), ("CORELLIA", "MANDALORE"), ("KASHYYYK", "BORLEIAS"),
            ("KASHYYYK", "UTAPAU"), ("BORLEIAS", "HOLOTHAN"), ("BORLEIAS", "ENDOR"),
            ("HOLOTHAN", "DAGOBAH"), ("HOLOTHAN", "SALEUCAMI"), ("SALEUCAMI", "DATHOMIR"),
            ("SALEUCAMI", "ABREGADO_RAE"), ("SALEUCAMI", "MUUNILINST"), ("MUUNILINST", "SULLUST"),
            ("SULLUST", "DANTOOINE"), ("DANTOOINE", "YAVIN"), ("YAVIN", "HOTH"),
            ("YAVIN", "FELUCIA"), ("SULLUST", "FELUCIA"), ("GEONOSIS", "KESSEL"),
            ("KESSEL", "MON_CALAMARI"), ("KESSEL", "RYLOTH"), ("RYLOTH", "TATOOINE"),
            ("RYLOTH", "BESPIN"), ("BESPIN", "JAKKU"), ("BESPIN", "HOTH"),
            ("BESPIN", "GEONOSIS"), ("MON_CALAMARI", "CATO_NEIMOIDIA"), ("MON_CALAMARI", "NAR_SHADDAA"),
            ("NAR_SHADDAA", "NAL_HUTTA"), ("NAR_SHADDAA", "TATOOINE"), ("BYSS", "UMBARA"),
            ("UMBARA", "ALZOC_III"), ("ALZOC_III", "ALDERAAN_REM"), ("ALDERAAN_REM", "HYPORI"),
            ("CHANDRILA", "ALDERAAN_REM"), ("CHANDRILA", "MUSTAFAR"), ("MUSTAFAR", "UTAPAU"),
            ("UTAPAU", "TERRIDON"), ("HYPORI", "TERRIDON"), ("DANTOOINE", "GEONOSIS"),
        ];
        let find = |id: &str| planets.iter().position(|p| p.id == id);
        let routes = route_names
            .iter()
            .filter_map(|&(a, b)| {
                Some(RouteView {
                    a: find(a)?,
                    b: find(b)?,
                    style: 0,
                })
            })
            .collect();

        Self {
            planets,
            routes,
            factions,
            route_styles: vec![RouteStyleView::fallback()],
            player_faction: Some(1),
            props: Vec::new(),
            light: GalaxyLight::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_is_consistent() {
        let g = GalaxyView::demo();
        assert!(g.planets.len() >= 40);
        // every route name resolved
        assert_eq!(g.routes.len(), 54);
        assert!(
            g.routes
                .iter()
                .all(|r| r.a < g.planets.len() && r.b < g.planets.len() && r.a != r.b)
        );
        assert!(
            g.planets
                .iter()
                .all(|p| p.owner.is_none_or(|o| o < g.factions.len()))
        );
    }
}
