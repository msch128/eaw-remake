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
    /// Visual radius multiplier relative to a "normal" planet (`1.0`).
    pub size: f32,
    /// Placeholder surface colour (linear-ish sRGB, 0..1) until the real `.alo` models are loaded.
    pub surface_color: [f32; 3],
}

/// A trade route / hyperspace lane between two planets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteView {
    /// Index into [`GalaxyView::planets`].
    pub a: usize,
    /// Index into [`GalaxyView::planets`].
    pub b: usize,
}

/// A playable faction.
#[derive(Debug, Clone, PartialEq)]
pub struct FactionView {
    /// Display name (e.g. `Galactic Empire`).
    pub name: String,
    /// Faction colour in sRGB, each channel 0..1.
    pub color: [f32; 3],
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
            FactionView { name: "Galactic Empire".into(), color: [0.30, 0.85, 0.45] },
            FactionView { name: "Rebel Alliance".into(), color: [1.00, 0.28, 0.22] },
            FactionView { name: "Underworld".into(), color: [1.00, 0.72, 0.15] },
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
            .map(|&(id, name, x, y, z, owner, income, size, surface)| PlanetView {
                id: id.to_owned(),
                display_name: name.to_owned(),
                position: Vec3::new(x, y, z),
                owner,
                income,
                size,
                surface_color: surface,
            })
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
            .filter_map(|&(a, b)| Some(RouteView { a: find(a)?, b: find(b)? }))
            .collect();

        Self { planets, routes, factions }
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
        assert!(g.routes.iter().all(|r| r.a < g.planets.len() && r.b < g.planets.len() && r.a != r.b));
        assert!(g.planets.iter().all(|p| p.owner.is_none_or(|o| o < g.factions.len())));
    }
}
