//! Index-file driven loader that assembles [`GameData`] from a [`FileSource`].

use std::collections::HashMap;

use alamo_fs::FileSource;

use crate::combat::CombatTable;
use crate::model::{Campaign, CampaignFaction, Faction, Planet, TradeRoute, TradeRouteLine};
use crate::objects::GameObjects;
use crate::text::TextTable;
use crate::xml::{self, Element, Warning};

/// Everything the galactic conquest layer needs, loaded from the XML data.
#[derive(Debug, Default)]
pub struct GameData {
    /// Campaigns in index order (`CampaignFiles.xml`), including menu-only entries
    /// (see [`Campaign::has_galaxy`]).
    pub campaigns: Vec<Campaign>,
    /// All `Planet` objects found in the files listed by `GameObjectFiles.XML`.
    pub planets: Vec<Planet>,
    /// All trade routes from `TradeRouteFiles.xml`.
    pub trade_routes: Vec<TradeRoute>,
    /// Trade route line styles from `TRADEROUTELINES.XML`.
    pub trade_route_lines: Vec<TradeRouteLine>,
    /// Factions from `FACTIONFILES.XML`.
    pub factions: Vec<Faction>,
    /// `Gameconstants.xml` as `lower-case tag -> trimmed text` (last occurrence wins).
    pub game_constants: HashMap<String, String>,
    /// Damage type vs armor type multipliers from `Gameconstants.xml`.
    pub combat: CombatTable,
    /// Localized strings (empty if the `.dat` was not found).
    pub text: TextTable,
    /// Every named object definition (units, hardpoints, projectiles, structures ...).
    pub objects: GameObjects,
    /// Files listed by `GameObjectFiles.XML`.
    pub game_object_files: Vec<String>,
    planet_index: HashMap<String, usize>,
    route_index: HashMap<String, usize>,
    faction_index: HashMap<String, usize>,
}

/// `(planet, factions)` pairs, see [`GameData::campaign_ownership_conflicts`].
pub type Conflicts = Vec<(String, Vec<String>)>;

fn key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

impl GameData {
    /// Planet by case-insensitive name.
    pub fn planet(&self, name: &str) -> Option<&Planet> {
        self.planet_index.get(&key(name)).map(|&i| &self.planets[i])
    }

    /// Trade route by case-insensitive name.
    pub fn trade_route(&self, name: &str) -> Option<&TradeRoute> {
        self.route_index
            .get(&key(name))
            .map(|&i| &self.trade_routes[i])
    }

    /// Faction by case-insensitive name.
    pub fn faction(&self, name: &str) -> Option<&Faction> {
        self.faction_index
            .get(&key(name))
            .map(|&i| &self.factions[i])
    }

    /// Campaign by case-insensitive name.
    pub fn campaign(&self, name: &str) -> Option<&Campaign> {
        self.campaigns
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Localized string for a text key.
    pub fn text(&self, id: &str) -> Option<&str> {
        self.text.get(id)
    }

    /// Display name of a planet: its localized `Text_ID`, falling back to the object name.
    pub fn planet_display_name<'a>(&'a self, planet: &'a Planet) -> &'a str {
        planet
            .text_id
            .as_deref()
            .and_then(|id| self.text.get(id))
            .unwrap_or(&planet.name)
    }

    /// Planets of a campaign (its `Locations` that resolve to a planet definition).
    /// Includes dummies/art objects; filter with [`Planet::is_real_planet`].
    pub fn campaign_planets(&self, campaign: &Campaign) -> Vec<&Planet> {
        campaign
            .locations
            .iter()
            .filter_map(|n| self.planet(n))
            .collect()
    }

    /// Trade routes of a campaign (its `Trade_Routes` that resolve to a definition).
    pub fn campaign_trade_routes(&self, campaign: &Campaign) -> Vec<&TradeRoute> {
        campaign
            .trade_routes
            .iter()
            .filter_map(|n| self.trade_route(n))
            .collect()
    }

    /// Planets of `campaign` and the faction that owns each at game start, keyed by the planet
    /// object name. Planets missing from the map start neutral/unowned.
    ///
    /// Rule (derived from the Remake campaigns): a faction owns a planet when one of its
    /// `Starting_Forces` entries for that planet is a *structure* (`StarBase`, `SpecialStructure`,
    /// ...); ships, squadrons, heroes and companies alone confer no ownership. If several factions
    /// place structures on one planet, the first in file order wins (see
    /// [`GameData::campaign_ownership_conflicts`]). A faction's `Home_Location` is owned by it when
    /// no structure decided otherwise.
    pub fn campaign_planet_owners(&self, campaign: &Campaign) -> HashMap<String, String> {
        self.ownership(campaign).0
    }

    /// Planets where several factions place starting structures, as `(planet, factions)` with the
    /// winning (first) faction first.
    pub fn campaign_ownership_conflicts(&self, campaign: &Campaign) -> Conflicts {
        self.ownership(campaign).1
    }

    fn ownership(&self, campaign: &Campaign) -> (HashMap<String, String>, Conflicts) {
        let canon = |name: &str| {
            self.planet(name)
                .map_or_else(|| name.to_string(), |p| p.name.clone())
        };
        let mut owners: HashMap<String, String> = HashMap::new();
        let mut claims: Conflicts = Vec::new();
        for sf in &campaign.starting_forces {
            if !self.objects.is_structure(&sf.object) {
                continue;
            }
            let planet = canon(&sf.planet);
            match owners.get(&planet) {
                None => {
                    owners.insert(planet.clone(), sf.faction.clone());
                    claims.push((planet, vec![sf.faction.clone()]));
                }
                Some(_) => {
                    let entry = claims
                        .iter_mut()
                        .find(|(p, _)| *p == planet)
                        .expect("claim recorded with owner");
                    if !entry.1.iter().any(|f| f.eq_ignore_ascii_case(&sf.faction)) {
                        entry.1.push(sf.faction.clone());
                    }
                }
            }
        }
        for cf in &campaign.factions {
            if let Some(home) = &cf.home_location {
                owners.entry(canon(home)).or_insert_with(|| cf.name.clone());
            }
        }
        claims.retain(|(_, f)| f.len() > 1);
        (owners, claims)
    }

    fn rebuild_indexes(&mut self) {
        // First definition wins; duplicates were already reported while loading.
        self.planet_index.clear();
        for (i, p) in self.planets.iter().enumerate() {
            self.planet_index.entry(key(&p.name)).or_insert(i);
        }
        self.route_index.clear();
        for (i, r) in self.trade_routes.iter().enumerate() {
            self.route_index.entry(key(&r.name)).or_insert(i);
        }
        self.faction_index.clear();
        for (i, f) in self.factions.iter().enumerate() {
            self.faction_index.entry(key(&f.name)).or_insert(i);
        }
    }
}

/// Loads campaigns, planets, trade routes, factions and game constants from `src`
/// (a `Data` root), plus the English string table `Text/MasterTextFile_ENGLISH.DAT`.
///
/// Never fails: missing or malformed files produce [`Warning`]s and the corresponding
/// part of the result stays empty.
pub fn load_galaxy(src: &dyn FileSource) -> (GameData, Vec<Warning>) {
    let mut w = Vec::new();
    let mut data = GameData::default();

    // Campaigns.
    for file in index_files(src, "Xml/CampaignFiles.xml", &mut w) {
        let Some(root) = read_xml(src, &xml_path(&file), &mut w) else {
            continue;
        };
        for el in root.children_named("campaign") {
            let mut local = Vec::new();
            let c = Campaign::from_element(el, &file, &mut |line, msg| local.push((line, msg)));
            w.extend(local.into_iter().map(|(l, m)| Warning::new(&file, l, m)));
            match c {
                Some(c)
                    if data
                        .campaigns
                        .iter()
                        .any(|x| x.name.eq_ignore_ascii_case(&c.name)) =>
                {
                    w.push(Warning::new(
                        &file,
                        el.line,
                        format!("duplicate campaign '{}' ignored", c.name),
                    ));
                }
                Some(c) => data.campaigns.push(c),
                None => w.push(Warning::new(
                    &file,
                    el.line,
                    "campaign without Name attribute",
                )),
            }
        }
    }

    // Object definitions (units, structures, hardpoints, projectiles, planets ...).
    data.objects = GameObjects::load(src, &mut w);
    data.game_object_files = index_files(src, "Xml/GameObjectFiles.XML", &mut w);
    for obj in data.objects.of_kind("planet") {
        match Planet::from_element(&obj.element, &obj.source_file) {
            Some(p) => data.planets.push(p),
            None => w.push(Warning::new(
                &obj.source_file,
                obj.element.line,
                "planet without Name attribute",
            )),
        }
    }
    // Trade routes.
    for file in index_files(src, "Xml/TradeRouteFiles.xml", &mut w) {
        let path = xml_path(&file);
        let Some(root) = read_xml(src, &path, &mut w) else {
            continue;
        };
        for el in root.children_named("traderoute") {
            match TradeRoute::from_element(el, &file) {
                Some(r)
                    if data
                        .trade_routes
                        .iter()
                        .any(|x| x.name.eq_ignore_ascii_case(&r.name)) =>
                {
                    w.push(Warning::new(
                        &path,
                        el.line,
                        format!("duplicate trade route '{}' ignored", r.name),
                    ));
                }
                Some(r) => data.trade_routes.push(r),
                None => w.push(Warning::new(
                    &path,
                    el.line,
                    "trade route without Name attribute",
                )),
            }
        }
    }
    if let Some(root) = read_xml(src, "Xml/TRADEROUTELINES.XML", &mut w) {
        data.trade_route_lines = root
            .children_named("traderouteline")
            .filter_map(TradeRouteLine::from_element)
            .collect();
    }

    // Factions.
    for file in index_files(src, "Xml/FACTIONFILES.XML", &mut w) {
        let path = xml_path(&file);
        let Some(root) = read_xml(src, &path, &mut w) else {
            continue;
        };
        for el in root.children_named("faction") {
            match Faction::from_element(el, &file) {
                Some(f)
                    if data
                        .factions
                        .iter()
                        .any(|x| x.name.eq_ignore_ascii_case(&f.name)) =>
                {
                    w.push(Warning::new(
                        &path,
                        el.line,
                        format!("duplicate faction '{}' ignored", f.name),
                    ));
                }
                Some(f) => data.factions.push(f),
                None => w.push(Warning::new(
                    &path,
                    el.line,
                    "faction without Name attribute",
                )),
            }
        }
    }

    // Game constants (flat key/value list).
    if let Some(root) = read_xml(src, "Xml/Gameconstants.xml", &mut w) {
        for c in &root.children {
            data.game_constants
                .insert(c.name.clone(), c.text_trimmed().to_string());
        }
        data.combat = CombatTable::from_constants(&root);
    }

    // Localized strings.
    let text_path = "Text/MasterTextFile_ENGLISH.DAT".to_string();
    match TextTable::load(src, &text_path) {
        Ok(t) => data.text = t,
        Err(e) => w.push(Warning::new(
            &text_path,
            0,
            format!("string table not loaded: {e}"),
        )),
    }

    data.rebuild_indexes();
    (data, w)
}

/// Resolves a file name from an index file (relative to `Data/Xml`).
pub(crate) fn xml_path(file: &str) -> String {
    format!("Xml/{}", file.trim().replace('\\', "/"))
}

/// Reads and leniently parses an XML file, warning when it is missing or empty.
pub(crate) fn read_xml(src: &dyn FileSource, path: &str, w: &mut Vec<Warning>) -> Option<Element> {
    let bytes = match src.read(path) {
        Ok(b) => b,
        Err(e) => {
            w.push(Warning::new(path, 0, format!("cannot read file: {e}")));
            return None;
        }
    };
    let root = xml::parse(&bytes, path, w);
    if root.is_none() {
        w.push(Warning::new(path, 0, "no XML element found"));
    }
    root
}

/// Reads an index file (`<File>name</File>` entries) and returns the listed names.
pub(crate) fn index_files(src: &dyn FileSource, path: &str, w: &mut Vec<Warning>) -> Vec<String> {
    read_xml(src, path, w)
        .map(|root| {
            root.children_named("file")
                .map(|f| f.text_trimmed().to_string())
                .filter(|f| !f.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// One galactic conquest the player can start: a campaign with a galaxy and the faction the player takes (`Starting_Active_Player`).
#[derive(Debug, Clone, Copy)]
pub struct PlayableCampaign<'a> {
    /// The campaign (`GC_Full_Web_Way_Rebel`).
    pub campaign: &'a Campaign,
    /// The faction the player plays.
    pub faction: &'a Faction,
    /// The campaign's entry of that faction (credits, tech, home planet, story plot).
    pub slot: Option<&'a CampaignFaction>,
}

/// A campaign set as the menu shows it: the campaigns that share a galaxy and size (`Campaign_Set`, for example `Web_Way4` = the Full galaxy),
/// one per playable faction.
#[derive(Debug, Clone)]
pub struct CampaignSet<'a> {
    /// `Campaign_Set` value.
    pub name: String,
    /// Playable campaigns of the set, in menu order (`Sort_Order`, then file order).
    pub campaigns: Vec<PlayableCampaign<'a>>,
}

impl GameData {
    /// The campaigns the player can start, grouped by campaign set in file order. Only campaigns that define a galaxy and whose
    /// `Starting_Active_Player` is a known faction count; menu-only entries and the dummy / language entries are left out.
    pub fn campaign_sets(&self) -> Vec<CampaignSet<'_>> {
        let mut sets: Vec<CampaignSet<'_>> = Vec::new();
        for c in self.campaigns.iter().filter(|c| c.has_galaxy()) {
            let Some(faction) = c.starting_active_player.as_deref().and_then(|f| self.faction(f)) else { continue };
            let name = c.set.clone().unwrap_or_else(|| c.name.clone());
            let slot = c.factions.iter().find(|f| f.name.eq_ignore_ascii_case(&faction.name));
            let entry = PlayableCampaign { campaign: c, faction, slot };
            match sets.iter_mut().find(|s| s.name.eq_ignore_ascii_case(&name)) {
                Some(s) => s.campaigns.push(entry),
                None => sets.push(CampaignSet { name, campaigns: vec![entry] }),
            }
        }
        for s in &mut sets {
            s.campaigns.sort_by_key(|p| p.campaign.sort_order);
        }
        sets
    }

    /// Factions the player can pick in at least one campaign set, in first-seen order.
    pub fn playable_factions(&self) -> Vec<&Faction> {
        let mut out: Vec<&Faction> = Vec::new();
        for s in self.campaign_sets() {
            for p in s.campaigns {
                if !out.iter().any(|f| std::ptr::eq(*f, p.faction)) {
                    out.push(p.faction);
                }
            }
        }
        out
    }

    /// Factions that carry `Is_Playable` in the faction files but have no campaign of their own (the Remake lists many minor factions that way:
    /// they play as AI factions in every campaign, the player picks one of the campaign factions instead).
    pub fn flagged_playable_without_campaign(&self) -> Vec<&Faction> {
        let picked = self.playable_factions();
        self.factions.iter().filter(|f| f.is_playable && !f.name.to_ascii_lowercase().starts_with("dummy") && !picked.iter().any(|p| std::ptr::eq(*p, *f))).collect()
    }

    /// The other factions of a campaign (everything but the player's, without neutral and pirate placeholders), with their campaign entries.
    pub fn campaign_opponents<'a>(&'a self, campaign: &'a Campaign) -> Vec<(&'a Faction, &'a CampaignFaction)> {
        campaign
            .factions
            .iter()
            .filter(|cf| !campaign.starting_active_player.as_deref().is_some_and(|p| p.eq_ignore_ascii_case(&cf.name)))
            .filter_map(|cf| self.faction(&cf.name).map(|f| (f, cf)))
            .filter(|(f, _)| !f.is_neutral && !f.name.to_ascii_lowercase().starts_with("dummy"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    struct Mem(HashMap<String, Vec<u8>>);

    impl FileSource for Mem {
        fn read(&self, path: &str) -> io::Result<Vec<u8>> {
            self.0
                .get(&alamo_fs::normalize(path))
                .cloned()
                .ok_or_else(|| io::ErrorKind::NotFound.into())
        }
        fn exists(&self, path: &str) -> bool {
            self.0.contains_key(&alamo_fs::normalize(path))
        }
        fn list(&self, _dir: &str) -> Vec<String> {
            self.0.keys().cloned().collect()
        }
    }

    fn mem(files: &[(&str, &str)]) -> Mem {
        Mem(files
            .iter()
            .map(|(k, v)| (alamo_fs::normalize(k), v.as_bytes().to_vec()))
            .collect())
    }

    #[test]
    fn loads_small_galaxy() {
        let src = mem(&[
            (
                "Xml/CampaignFiles.xml",
                "<Campaign_Files><File>c.xml</File><!--<File>x.xml</File>--></Campaign_Files>",
            ),
            (
                "Xml/c.xml",
                "<Campaigns><Campaign Name=\"C1\"><Locations>A, B,\n Missing</Locations><Trade_Routes>R1</Trade_Routes>\
                 <Home_Location> Rebel, A </Home_Location><Starting_Credits>Rebel, 100</Starting_Credits>\
                 <Starting_Forces>Empire, B, Star_Base</Starting_Forces><Empire_Story_Name>s.xml</Empire_Story_Name>\
                 </Campaign></Campaigns>",
            ),
            (
                "Xml/GameObjectFiles.XML",
                "<Game_Object_Files><File>p.xml</File><File>gone.xml</File></Game_Object_Files>",
            ),
            (
                "Xml/p.xml",
                "<Planets><Planet Name=\"A\"><Behavior>SELECTABLE, PLANET</Behavior><Galactic_Position>1, 2.5, 3</Galactic_Position>\
                 <Planet_Credit_Value>50</Planet_Credit_Value></Planet><PLANET Name=\"b\"><Text_ID>T_B</Text_ID></PLANET></Planets>",
            ),
            ("Xml/TradeRouteFiles.xml", "<X><File>t.xml</File></X>"),
            (
                "Xml/t.xml",
                "<TradeRoutes><TradeRoute Name=\"R1\"><Point_A>A</Point_A><Point_B>B</Point_B></TradeRoute></TradeRoutes>",
            ),
            ("Xml/FACTIONFILES.XML", "<X><File>f.xml</File></X>"),
            (
                "Xml/f.xml",
                "<Factions><Faction Name=\"Rebel\"><Color>1,2,3,4</Color><Is_Playable>Yes</Is_Playable></Faction></Factions>",
            ),
        ]);
        let (d, w) = load_galaxy(&src);
        assert_eq!(d.campaigns.len(), 1);
        let c = &d.campaigns[0];
        assert_eq!(c.locations, ["A", "B", "Missing"]);
        assert_eq!(c.faction("rebel").unwrap().starting_credits, Some(100));
        assert_eq!(c.faction("Rebel").unwrap().presence_planets, ["A"]);
        assert_eq!(
            c.faction("empire").unwrap().story_name.as_deref(),
            Some("s.xml")
        );
        assert_eq!(c.faction("empire").unwrap().presence_planets, ["B"]);
        assert_eq!(d.campaign_planets(c).len(), 2);
        let a = d.planet("a").unwrap();
        assert!(a.is_real_planet());
        assert_eq!(a.position, [1.0, 2.5, 3.0]);
        assert_eq!(a.credit_value, 50);
        assert_eq!(d.campaign_trade_routes(c)[0].point_b, "B");
        assert_eq!(d.faction("REBEL").unwrap().color, Some([1, 2, 3, 4]));
        assert!(d.faction("rebel").unwrap().is_playable);
        assert!(
            w.iter()
                .any(|x| x.message.contains("gone") || x.file.contains("gone")),
            "{w:?}"
        );
    }
}
