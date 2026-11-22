use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::config::{
    BP_ALCHEMY2, BP_PHASE_DEF, BP_SCANNERS, BP_STARGATE, BP_TERRA, BP_WARP10, NUM_TECH_FIELDS,
};
use crate::types::{Doctrine, EmpireLoadoutView, RaceDef};

const TECHS_JSON: &str = include_str!("../../../data/craig-stars/techs.json");
const RULES_JSON: &str = include_str!("../../../data/craig-stars/rules.json");

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawEwRules {
    #[serde(default = "default_tachyon_reduction")]
    pub tachyon_cloak_reduction: f32,
    #[serde(default = "default_tachyon_max_reduction")]
    pub tachyon_max_cloak_reduction: f32,
}

fn default_tachyon_reduction() -> f32 {
    0.05
}

fn default_tachyon_max_reduction() -> f32 {
    0.81
}

/// Canonical Stars! piecewise conversion from cloak units to cloak percentage [0.0, 0.98]:
/// - 0..=100   -> 0%..50%
/// - 100..=300 -> 50%..75%
/// - 300..=600 -> 75%..87.5%
/// - >600      -> asymptotically up to 98% cap
pub fn cloak_units_to_pct(units: f32) -> f32 {
    if units <= 0.0 {
        0.0
    } else if units <= 100.0 {
        units / 200.0
    } else if units <= 300.0 {
        0.50 + (units - 100.0) / 800.0
    } else if units <= 600.0 {
        0.75 + (units - 300.0) / 2400.0
    } else {
        (0.875 + (units - 600.0) / 6000.0).min(0.98)
    }
}

// ---------------------------------------------------------------------------
// 10 Primary Racial Traits (PRTs) & 14 Lesser Racial Traits (LRTs)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrtMeta {
    pub code: &'static str,
    pub short_name: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LrtMeta {
    pub bit: u32,
    pub code: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
}

pub const LRT_IFE: u32 = 1 << 0;
pub const LRT_TT: u32 = 1 << 1;
pub const LRT_ARM: u32 = 1 << 2;
pub const LRT_ISB: u32 = 1 << 3;
pub const LRT_GR: u32 = 1 << 4;
pub const LRT_UR: u32 = 1 << 5;
pub const LRT_MA: u32 = 1 << 6;
pub const LRT_NRSE: u32 = 1 << 7;
pub const LRT_CE: u32 = 1 << 8;
pub const LRT_OBRM: u32 = 1 << 9;
pub const LRT_NAS: u32 = 1 << 10;
pub const LRT_LSP: u32 = 1 << 11;
pub const LRT_BET: u32 = 1 << 12;
pub const LRT_RS: u32 = 1 << 13;

pub const PRT_LIST: [PrtMeta; 10] = [
    PrtMeta {
        code: "PRT_HE",
        short_name: "HE",
        name: "Hyper-Expansion",
        desc: "2x population growth, cheap Mini-Colony ships, 50% max pop cap",
    },
    PrtMeta {
        code: "PRT_SS",
        short_name: "SS",
        name: "Super Stealth",
        desc: "Cloaked ships, superior scanners & evasion in deep space",
    },
    PrtMeta {
        code: "PRT_WM",
        short_name: "WM",
        name: "War Monger",
        desc: "+25% warship firepower, faster shipyards, elite capital hulls",
    },
    PrtMeta {
        code: "PRT_CA",
        short_name: "CA",
        name: "Claim Adjuster",
        desc: "Starts with Bio/Prop/Eng bonuses and automatic rapid terraforming",
    },
    PrtMeta {
        code: "PRT_IS",
        short_name: "IS",
        name: "Inner Strength",
        desc: "Fortified planetary bunkers, strong industry & ground defense",
    },
    PrtMeta {
        code: "PRT_SD",
        short_name: "SD",
        name: "Space Demolition",
        desc: "Minefields & defensive interdiction, strong starbase hulls",
    },
    PrtMeta {
        code: "PRT_PP",
        short_name: "PP",
        name: "Packet Physics",
        desc: "Mass drivers, cheap mineral logistics & high alchemy efficiency",
    },
    PrtMeta {
        code: "PRT_IT",
        short_name: "IT",
        name: "Interstellar Traveler",
        desc: "Early long-range Stargates & reduced empire sprawl penalty",
    },
    PrtMeta {
        code: "PRT_AR",
        short_name: "AR",
        name: "Alternate Reality",
        desc: "Orbital starbase economy, high energy output & remote mining",
    },
    PrtMeta {
        code: "PRT_JOAT",
        short_name: "JOAT",
        name: "Jack of All Trades",
        desc: "Starts at Tech Lv 3 in all 6 fields, +20% pop cap & built-in scanners",
    },
];

pub const LRT_LIST: [LrtMeta; 14] = [
    LrtMeta {
        bit: LRT_IFE,
        code: "IFE",
        name: "Improved Fuel Efficiency",
        desc: "Starts at PRO +1, unlocks fuel-miser engines (+10% fleet speed)",
    },
    LrtMeta {
        bit: LRT_TT,
        code: "TT",
        name: "Total Terraforming",
        desc: "Unlocks Total Terraform techs (+30% terraforming power)",
    },
    LrtMeta {
        bit: LRT_ARM,
        code: "ARM",
        name: "Advanced Remote Mining",
        desc: "Unlocks top mining robots (+20% mineral extraction)",
    },
    LrtMeta {
        bit: LRT_ISB,
        code: "ISB",
        name: "Improved Starbases",
        desc: "Unlocks Space Dock & Ultra Station (+20% planetary defense)",
    },
    LrtMeta {
        bit: LRT_GR,
        code: "GR",
        name: "Generalized Research",
        desc: "Spreads research bonus across all 6 fields (+12% total research)",
    },
    LrtMeta {
        bit: LRT_UR,
        code: "UR",
        name: "Ultimate Recycling",
        desc: "Higher debris salvage recovery (+30% salvage rate)",
    },
    LrtMeta {
        bit: LRT_MA,
        code: "MA",
        name: "Mineral Alchemy",
        desc: "2x faster mineral transmutation from surplus resources",
    },
    LrtMeta {
        bit: LRT_NRSE,
        code: "NRSE",
        name: "No Ram Scoop Engines",
        desc: "Cannot build ramscoop engines (frees racial points)",
    },
    LrtMeta {
        bit: LRT_CE,
        code: "CE",
        name: "Cheap Engines",
        desc: "12% cheaper ship construction, slightly higher engine failure risk",
    },
    LrtMeta {
        bit: LRT_OBRM,
        code: "OBRM",
        name: "Only Basic Remote Mining",
        desc: "Restricts advanced mining robots (+10% max population)",
    },
    LrtMeta {
        bit: LRT_NAS,
        code: "NAS",
        name: "No Advanced Scanners",
        desc: "Disables penetrating scanners, boosts standard scanner range",
    },
    LrtMeta {
        bit: LRT_LSP,
        code: "LSP",
        name: "Low Starting Population",
        desc: "Starts with 30% fewer colonists on homeworld",
    },
    LrtMeta {
        bit: LRT_BET,
        code: "BET",
        name: "Bleeding Edge Technology",
        desc: "Unlocks cutting-edge tech faster (+15% research speed)",
    },
    LrtMeta {
        bit: LRT_RS,
        code: "RS",
        name: "Regenerating Shields",
        desc: "+25% shield strength in combat",
    },
];

pub fn classic_races() -> Vec<RaceDef> {
    vec![
        RaceDef {
            id: "humanoids".to_string(),
            name: "Humanoids".to_string(),
            color: "#38bdf8".to_string(),
            prt: "PRT_JOAT".to_string(),
            lrt_mask: 0,
            is_custom: false,
            doctrine: Doctrine {
                fac_share: 0.48,
                ship_share: 0.24,
                tech_share: 0.16,
                col_pop_req: 0.32,
                dist_exp: 1.5,
                qual_exp: 1.5,
                cruiser_share: 0.40,
                bship_share: 0.35,
                bomber_share: 0.25,
                intercept_bias: 0.55,
                tech_alloc: [0.17, 0.17, 0.17, 0.17, 0.16, 0.16],
                kills: 0,
                gates: 0,
                last_fit: 0,
                win_rate: 17,
            },
        },
        RaceDef {
            id: "rabbitoids".to_string(),
            name: "Rabbitoids".to_string(),
            color: "#4ade80".to_string(),
            prt: "PRT_IT".to_string(),
            lrt_mask: LRT_IFE | LRT_TT | LRT_CE | LRT_LSP,
            is_custom: false,
            doctrine: Doctrine {
                fac_share: 0.46,
                ship_share: 0.28,
                tech_share: 0.14,
                col_pop_req: 0.26,
                dist_exp: 1.8,
                qual_exp: 1.3,
                cruiser_share: 0.45,
                bship_share: 0.30,
                bomber_share: 0.25,
                intercept_bias: 0.65,
                tech_alloc: [0.14, 0.14, 0.28, 0.18, 0.12, 0.14],
                kills: 0,
                gates: 0,
                last_fit: 0,
                win_rate: 17,
            },
        },
        RaceDef {
            id: "insectoids".to_string(),
            name: "Insectoids".to_string(),
            color: "#f87171".to_string(),
            prt: "PRT_WM".to_string(),
            lrt_mask: LRT_ISB | LRT_RS | LRT_CE,
            is_custom: false,
            doctrine: Doctrine {
                fac_share: 0.44,
                ship_share: 0.32,
                tech_share: 0.14,
                col_pop_req: 0.30,
                dist_exp: 1.6,
                qual_exp: 1.3,
                cruiser_share: 0.30,
                bship_share: 0.45,
                bomber_share: 0.25,
                intercept_bias: 0.60,
                tech_alloc: [0.16, 0.28, 0.16, 0.22, 0.10, 0.08],
                kills: 0,
                gates: 0,
                last_fit: 0,
                win_rate: 17,
            },
        },
        RaceDef {
            id: "nucleids".to_string(),
            name: "Nucleids".to_string(),
            color: "#c084fc".to_string(),
            prt: "PRT_SS".to_string(),
            lrt_mask: LRT_IFE | LRT_ARM | LRT_BET,
            is_custom: false,
            doctrine: Doctrine {
                fac_share: 0.46,
                ship_share: 0.22,
                tech_share: 0.22,
                col_pop_req: 0.34,
                dist_exp: 1.5,
                qual_exp: 1.6,
                cruiser_share: 0.45,
                bship_share: 0.20,
                bomber_share: 0.35,
                intercept_bias: 0.72,
                tech_alloc: [0.20, 0.18, 0.18, 0.12, 0.20, 0.12],
                kills: 0,
                gates: 0,
                last_fit: 0,
                win_rate: 17,
            },
        },
        RaceDef {
            id: "silicanoids".to_string(),
            name: "Silicanoids".to_string(),
            color: "#facc15".to_string(),
            prt: "PRT_HE".to_string(),
            lrt_mask: LRT_OBRM | LRT_BET | LRT_UR,
            is_custom: false,
            doctrine: Doctrine {
                fac_share: 0.50,
                ship_share: 0.26,
                tech_share: 0.14,
                col_pop_req: 0.24,
                dist_exp: 1.9,
                qual_exp: 1.1,
                cruiser_share: 0.50,
                bship_share: 0.25,
                bomber_share: 0.25,
                intercept_bias: 0.55,
                tech_alloc: [0.18, 0.16, 0.20, 0.18, 0.14, 0.14],
                kills: 0,
                gates: 0,
                last_fit: 0,
                win_rate: 16,
            },
        },
        RaceDef {
            id: "antethereals".to_string(),
            name: "Antethereals".to_string(),
            color: "#fb923c".to_string(),
            prt: "PRT_SD".to_string(),
            lrt_mask: LRT_ARM | LRT_ISB | LRT_GR,
            is_custom: false,
            doctrine: Doctrine {
                fac_share: 0.52,
                ship_share: 0.20,
                tech_share: 0.18,
                col_pop_req: 0.35,
                dist_exp: 1.4,
                qual_exp: 1.7,
                cruiser_share: 0.30,
                bship_share: 0.40,
                bomber_share: 0.30,
                intercept_bias: 0.50,
                tech_alloc: [0.22, 0.16, 0.14, 0.20, 0.12, 0.16],
                kills: 0,
                gates: 0,
                last_fit: 0,
                win_rate: 16,
            },
        },
    ]
}

// ---------------------------------------------------------------------------
// Raw JSON schema matching data/craig-stars/techs.json unmodified
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TechLevelReq {
    #[serde(default)]
    pub energy: i32,
    #[serde(default)]
    pub weapons: i32,
    #[serde(default)]
    pub propulsion: i32,
    #[serde(default)]
    pub construction: i32,
    #[serde(default)]
    pub electronics: i32,
    #[serde(default)]
    pub biotechnology: i32,
}

impl TechLevelReq {
    pub fn as_array(&self) -> [i32; NUM_TECH_FIELDS] {
        [
            self.energy,
            self.weapons,
            self.propulsion,
            self.construction,
            self.electronics,
            self.biotechnology,
        ]
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TechCost {
    #[serde(default)]
    pub ironium: i32,
    #[serde(default)]
    pub boranium: i32,
    #[serde(default)]
    pub germanium: i32,
    #[serde(default)]
    pub resources: i32,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TechRequirements {
    #[serde(default)]
    pub tech_level: TechLevelReq,
    #[serde(default)]
    pub prts_required: Vec<String>,
    #[serde(default)]
    pub prts_denied: Vec<String>,
    #[serde(default)]
    pub lrts_required: u32,
    #[serde(default)]
    pub lrts_denied: u32,
    #[serde(default)]
    pub hulls_allowed: Vec<String>,
    #[serde(default)]
    pub acquirable: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TechMeta {
    pub name: String,
    #[serde(default)]
    pub cost: TechCost,
    #[serde(default)]
    pub requirements: TechRequirements,
    #[serde(default)]
    pub ranking: i32,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub tags: HashMap<String, bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawPlanetaryScanner {
    pub tech: TechMeta,
    #[serde(default)]
    pub scan_range: i32,
    #[serde(default)]
    pub scan_range_pen: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawTerraform {
    pub tech: TechMeta,
    #[serde(default)]
    pub ability: i32,
    #[serde(default)]
    pub hab_type: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawDefense {
    pub tech: TechMeta,
    #[serde(default)]
    pub defense_coverage: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawPlanetary {
    pub tech: TechMeta,
    #[serde(default)]
    pub reset_planet: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawHullComponent {
    pub tech: TechMeta,
    #[serde(default)]
    pub mass: i32,
    #[serde(default)]
    pub ideal_speed: i32,
    #[serde(default)]
    pub free_speed: i32,
    #[serde(default)]
    pub max_safe_speed: i32,
    #[serde(default)]
    pub fuel_usage: Vec<i32>,
    #[serde(default)]
    pub fuel_bonus: i32,
    #[serde(default)]
    pub fuel_generation: i32,
    #[serde(default)]
    pub cloak_units: i32,
    #[serde(default)]
    pub cloak_unarmed_only: bool,
    #[serde(default)]
    pub reduce_cloaking: bool,
    #[serde(default)]
    pub torpedo_jamming: f32,
    #[serde(default)]
    pub safe_hull_mass: i64,
    #[serde(default)]
    pub safe_range: i64,
    #[serde(default)]
    pub max_hull_mass: i64,
    #[serde(default)]
    pub max_range: i64,
    #[serde(default)]
    pub packet_speed: i32,
    #[serde(default)]
    pub mining_rate: i32,
    #[serde(default)]
    pub kill_rate: f32,
    #[serde(default)]
    pub structure_destroy_rate: f32,
    #[serde(default)]
    pub smart: bool,
    #[serde(default)]
    pub scan_range: i32,
    #[serde(default)]
    pub scan_range_pen: i32,
    #[serde(default)]
    pub armor: i32,
    #[serde(default)]
    pub shield: i32,
    #[serde(default)]
    pub power: i32,
    #[serde(default)]
    pub range: i32,
    #[serde(default)]
    pub initiative: i32,
    #[serde(default)]
    pub accuracy: i32,
    #[serde(default)]
    pub mine_laying_rate: i32,
    #[serde(default)]
    pub cargo_bonus: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawHullSlot {
    #[serde(default)]
    pub capacity: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawHull {
    pub tech: TechMeta,
    #[serde(rename = "type", default)]
    pub hull_type: String,
    #[serde(default)]
    pub mass: i32,
    #[serde(default)]
    pub armor: i32,
    #[serde(default)]
    pub fuel_capacity: i32,
    #[serde(default)]
    pub cargo_capacity: i32,
    #[serde(default)]
    pub initiative: i32,
    #[serde(default)]
    pub starbase: bool,
    #[serde(default)]
    pub slots: Vec<RawHullSlot>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTechFile {
    planetary_scanners: Vec<RawPlanetaryScanner>,
    terraforms: Vec<RawTerraform>,
    defenses: Vec<RawDefense>,
    planetaries: Vec<RawPlanetary>,
    hull_components: Vec<RawHullComponent>,
    hulls: Vec<RawHull>,
}

// ---------------------------------------------------------------------------
// Unified Catalog Entry (239 items) & Static TechStore
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub id: u16,
    pub name: String,
    pub category: String,
    pub category_label: String,
    pub origin: String,
    pub cost: TechCost,
    pub req_levels: [i32; NUM_TECH_FIELDS],
    pub prts_required: Vec<String>,
    pub prts_denied: Vec<String>,
    pub lrts_required: u32,
    pub lrts_denied: u32,
    pub acquirable: bool,
    pub stats_summary: String,
}

pub struct TechStore {
    pub entries: Vec<CatalogEntry>,
    pub planetary_scanners: Vec<(u16, RawPlanetaryScanner)>,
    pub terraforms: Vec<(u16, RawTerraform)>,
    pub defenses: Vec<(u16, RawDefense)>,
    pub planetaries: Vec<(u16, RawPlanetary)>,
    pub hull_components: Vec<(u16, RawHullComponent)>,
    pub hulls: Vec<(u16, RawHull)>,
    pub ew_rules: RawEwRules,
}

static TECH_STORE: OnceLock<TechStore> = OnceLock::new();

pub fn tech_store() -> &'static TechStore {
    TECH_STORE.get_or_init(TechStore::load)
}

fn category_label(cat: &str) -> &'static str {
    match cat {
        "TECH_CATEGORY_PLANETARY_SCANNER" => "Planetary Scanner",
        "TECH_CATEGORY_TERRAFORMING" => "Terraforming",
        "TECH_CATEGORY_PLANETARY_DEFENSE" => "Defense",
        "TECH_CATEGORY_PLANETARY" => "Planetary",
        "TECH_CATEGORY_ENGINE" => "Engine",
        "TECH_CATEGORY_ORBITAL" => "Orbital",
        "TECH_CATEGORY_MINE_ROBOT" => "Mining Robot",
        "TECH_CATEGORY_BOMB" => "Bomb",
        "TECH_CATEGORY_SCANNER" => "Scanner",
        "TECH_CATEGORY_ARMOR" => "Armor",
        "TECH_CATEGORY_ELECTRICAL" => "Electrical",
        "TECH_CATEGORY_MINE_LAYER" => "Mine Layer",
        "TECH_CATEGORY_MECHANICAL" => "Mechanical",
        "TECH_CATEGORY_BEAM_WEAPON" => "Beam Weapon",
        "TECH_CATEGORY_TORPEDO" => "Torpedo",
        "TECH_CATEGORY_SHIELD" => "Shield",
        "TECH_CATEGORY_SHIP_HULL" => "Ship Hull",
        "TECH_CATEGORY_STARBASE_HULL" => "Starbase",
        _ => "Other",
    }
}

fn make_entry(id: u16, meta: &TechMeta, stats_summary: String) -> CatalogEntry {
    CatalogEntry {
        id,
        name: meta.name.clone(),
        category: meta.category.clone(),
        category_label: category_label(&meta.category).to_string(),
        origin: meta.origin.clone(),
        cost: meta.cost,
        req_levels: meta.requirements.tech_level.as_array(),
        prts_required: meta.requirements.prts_required.clone(),
        prts_denied: meta.requirements.prts_denied.clone(),
        lrts_required: meta.requirements.lrts_required,
        lrts_denied: meta.requirements.lrts_denied,
        acquirable: meta.requirements.acquirable,
        stats_summary,
    }
}

impl TechStore {
    fn load() -> Self {
        let raw: RawTechFile =
            serde_json::from_str(TECHS_JSON).expect("Failed to parse data/craig-stars/techs.json");
        let ew_rules: RawEwRules =
            serde_json::from_str(RULES_JSON).expect("Failed to parse data/craig-stars/rules.json");

        let mut entries = Vec::with_capacity(240);
        let mut next_id = 0u16;

        let mut planetary_scanners = Vec::with_capacity(raw.planetary_scanners.len());
        for item in raw.planetary_scanners {
            let id = next_id;
            next_id += 1;
            let summary = if item.scan_range_pen > 0 {
                format!("Scan {}ly / Pen {}ly", item.scan_range, item.scan_range_pen)
            } else {
                format!("Scan {}ly", item.scan_range)
            };
            entries.push(make_entry(id, &item.tech, summary));
            planetary_scanners.push((id, item));
        }

        let mut terraforms = Vec::with_capacity(raw.terraforms.len());
        for item in raw.terraforms {
            let id = next_id;
            next_id += 1;
            let scope = match item.hab_type.as_str() {
                "TERRAFORM_HAB_TYPE_ALL" => "All",
                "TERRAFORM_HAB_TYPE_GRAV" => "Grav",
                "TERRAFORM_HAB_TYPE_TEMP" => "Temp",
                "TERRAFORM_HAB_TYPE_RAD" => "Rad",
                _ => "Hab",
            };
            let summary = format!("±{}% ({scope})", item.ability);
            entries.push(make_entry(id, &item.tech, summary));
            terraforms.push((id, item));
        }

        let mut defenses = Vec::with_capacity(raw.defenses.len());
        for item in raw.defenses {
            let id = next_id;
            next_id += 1;
            let summary = format!("Coverage {:.2}% / unit", item.defense_coverage);
            entries.push(make_entry(id, &item.tech, summary));
            defenses.push((id, item));
        }

        let mut planetaries = Vec::with_capacity(raw.planetaries.len());
        for item in raw.planetaries {
            let id = next_id;
            next_id += 1;
            let summary = if item.reset_planet {
                "Resets planet hab & minerals".to_string()
            } else {
                "Planetary installation".to_string()
            };
            entries.push(make_entry(id, &item.tech, summary));
            planetaries.push((id, item));
        }

        let mut hull_components = Vec::with_capacity(raw.hull_components.len());
        for item in raw.hull_components {
            let id = next_id;
            next_id += 1;
            let summary = match item.tech.category.as_str() {
                "TECH_CATEGORY_ENGINE" => format!(
                    "Warp {} ideal / {} max safe",
                    item.ideal_speed, item.max_safe_speed
                ),
                "TECH_CATEGORY_BEAM_WEAPON" => format!(
                    "Pow {} | Rng {} | Init {}",
                    item.power, item.range, item.initiative
                ),
                "TECH_CATEGORY_TORPEDO" => format!(
                    "Pow {} | Rng {} | Acc {}%",
                    item.power, item.range, item.accuracy
                ),
                "TECH_CATEGORY_BOMB" => format!(
                    "Kill {:.1}% | Struct {:.0}{}",
                    item.kill_rate,
                    item.structure_destroy_rate,
                    if item.smart { " | Smart" } else { "" }
                ),
                "TECH_CATEGORY_ARMOR" => {
                    if item.shield > 0 {
                        format!("Armor {} | Shield {}", item.armor, item.shield)
                    } else {
                        format!("Armor {} | Mass {}", item.armor, item.mass)
                    }
                }
                "TECH_CATEGORY_SHIELD" => {
                    if item.armor > 0 {
                        format!("Shield {} | Armor {}", item.shield, item.armor)
                    } else {
                        format!("Shield {}", item.shield)
                    }
                }
                "TECH_CATEGORY_SCANNER" => format!(
                    "Scan {}ly{}",
                    item.scan_range,
                    if item.scan_range_pen > 0 {
                        format!(" / Pen {}ly", item.scan_range_pen)
                    } else {
                        String::new()
                    }
                ),
                "TECH_CATEGORY_ORBITAL" => {
                    if item.packet_speed > 0 {
                        format!("Mass Driver Warp {}", item.packet_speed)
                    } else {
                        let rng = if item.safe_range >= 1_000_000 {
                            "∞".to_string()
                        } else {
                            format!("{}ly", item.safe_range)
                        };
                        let mass = if item.safe_hull_mass >= 1_000_000 {
                            "∞".to_string()
                        } else {
                            format!("{}kT", item.safe_hull_mass)
                        };
                        format!("Stargate {mass} / {rng}")
                    }
                }
                "TECH_CATEGORY_MINE_ROBOT" => format!("Mining {}/yr", item.mining_rate),
                "TECH_CATEGORY_MINE_LAYER" => format!("Mines {}/yr", item.mine_laying_rate),
                "TECH_CATEGORY_ELECTRICAL" => {
                    let mut parts: Vec<String> = Vec::new();
                    if item.cloak_units > 0 {
                        let pct = (cloak_units_to_pct(item.cloak_units as f32) * 100.0).round();
                        let tag = if item.cloak_unarmed_only { " unarmed" } else { "" };
                        parts.push(format!("Cloak {} ({:.0}%{})", item.cloak_units, pct, tag));
                    }
                    if item.reduce_cloaking {
                        parts.push("Tachyon (-cloak)".to_string());
                    }
                    if item.torpedo_jamming > 0.0 {
                        parts.push(format!("Jammer {:.0}%", item.torpedo_jamming * 100.0));
                    }
                    if parts.is_empty() {
                        format!("Mass {}kT", item.mass)
                    } else {
                        parts.join(" | ")
                    }
                }
                _ => {
                    if item.cargo_bonus > 0 {
                        format!("Cargo +{}kT", item.cargo_bonus)
                    } else {
                        format!("Mass {}kT", item.mass)
                    }
                }
            };
            entries.push(make_entry(id, &item.tech, summary));
            hull_components.push((id, item));
        }

        let mut hulls = Vec::with_capacity(raw.hulls.len());
        for item in raw.hulls {
            let id = next_id;
            next_id += 1;
            let summary = format!(
                "Armor {} | Fuel {} | Cargo {} | {} slots",
                item.armor,
                item.fuel_capacity,
                item.cargo_capacity,
                item.slots.len()
            );
            entries.push(make_entry(id, &item.tech, summary));
            hulls.push((id, item));
        }

        Self {
            entries,
            planetary_scanners,
            terraforms,
            defenses,
            planetaries,
            hull_components,
            hulls,
            ew_rules,
        }
    }

    pub fn is_unlocked(
        &self,
        meta: &TechMeta,
        int_levels: &[i32; NUM_TECH_FIELDS],
        prt: &str,
        lrt_mask: u32,
        ignore_traits: bool,
    ) -> bool {
        let req = &meta.requirements;
        if req.acquirable {
            return false;
        }
        let need = req.tech_level.as_array();
        for f in 0..NUM_TECH_FIELDS {
            if int_levels[f] < need[f] {
                return false;
            }
        }
        if !ignore_traits {
            if !req.prts_required.is_empty() && !req.prts_required.iter().any(|p| p == prt) {
                return false;
            }
            if req.prts_denied.iter().any(|p| p == prt) {
                return false;
            }
            if req.lrts_required != 0 && (lrt_mask & req.lrts_required) != req.lrts_required {
                return false;
            }
            if req.lrts_denied != 0 && (lrt_mask & req.lrts_denied) != 0 {
                return false;
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Per-Empire Cached Best-in-Slot Tech Loadout
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct HullSpec {
    pub name: String,
    pub armor: f32,
    pub mass: f32,
    pub fuel_capacity: f32,
    pub cargo: f32,
    pub iron: f32,
    pub bora: f32,
    pub germ: f32,
    pub res: f32,
}

#[derive(Debug, Clone)]
pub struct EmpireTechCache {
    pub unlocked_bits: [u32; 8],
    pub unlocked_count: u16,
    pub perk_mask: u8,

    pub engine_name: String,
    pub engine_warp: f32,
    pub engine_free_speed: i32,
    pub engine_fuel_usage: f32,

    pub weapon_name: String,
    pub weapon_power: f32,
    pub weapon_is_torpedo: bool,

    pub bomb_name: String,
    pub bomb_power: f32,
    pub bomb_smart: bool,

    pub armor_name: String,
    pub armor_val: f32,

    pub shield_name: String,
    pub shield_val: f32,

    pub scanner_name: String,
    pub scanner_range: f32,
    pub scanner_pen: f32,

    // Electronic Warfare (Cloak, Tachyon Detector, Torpedo Jammer & ECM Scanner Jamming)
    pub cloak_units_armed: f32,
    pub cloak_units_unarmed: f32,
    pub cloak_armed_pct: f32,
    pub cloak_unarmed_pct: f32,
    pub tachyon_cloak_reduction: f32,
    pub torpedo_jamming: f32,
    pub ecm_jammer_strength: f32,
    pub ecm_jammer_range: f32,

    pub defense_name: String,
    pub defense_coverage: f32,

    pub terraform_name: String,
    pub terraform_ability: f32,

    pub stargate_name: Option<String>,
    pub stargate_max_range: f32,
    pub stargate_cost_iron: f32,
    pub stargate_cost_germ: f32,

    pub mining_rate: f32,
    pub fuel_tank_bonus: f32,
    pub fuel_gen: f32,

    pub hull_col: HullSpec,
    pub hull_freight: HullSpec,
    pub hull_cruiser: HullSpec,
    pub hull_battleship: HullSpec,
    pub hull_bomber: HullSpec,

    pub range_col: f32,
    pub range_freight: f32,
    pub range_cruiser: f32,
    pub range_battleship: f32,
    pub range_bomber: f32,

    pub best_starbase_tier: u8,
    pub starbase_name: String,
}

impl Default for EmpireTechCache {
    fn default() -> Self {
        Self::resolve(&[0; NUM_TECH_FIELDS], "PRT_JOAT", 0, false)
    }
}

fn hull_from_raw(h: &RawHull) -> HullSpec {
    HullSpec {
        name: h.tech.name.clone(),
        armor: h.armor.max(10) as f32,
        mass: h.mass.max(8) as f32,
        fuel_capacity: h.fuel_capacity.max(50) as f32,
        cargo: h.cargo_capacity.max(10) as f32,
        iron: h.tech.cost.ironium.max(2) as f32,
        bora: h.tech.cost.boranium.max(1) as f32,
        germ: h.tech.cost.germanium.max(2) as f32,
        res: h.tech.cost.resources.max(10) as f32,
    }
}

fn compute_hull_range(
    hull: &HullSpec,
    engine_warp: f32,
    engine_free_speed: i32,
    engine_fuel_usage: f32,
    fuel_tank_bonus: f32,
    fuel_gen: f32,
    lrt_mask: u32,
    is_he_colonizer: bool,
) -> f32 {
    // Stars!-inspired operational fuel range scaled to the 1000x560 galaxy map:
    // Tech 0 ships (~75-95 ly radius) only reach immediate neighboring stars;
    // mid-game hulls/tanks/Fuel Mizer reach ~160-260 ly;
    // ramscoops, Fuel Generators, and Capital Ships project across sectors (450-850+ ly).
    let eff_fuel = hull.fuel_capacity + fuel_tank_bonus * 0.25 + fuel_gen * 4.0;
    let eff_mass = hull.mass + 30.0;
    let usage = engine_fuel_usage.max(35.0) * if lrt_mask & LRT_IFE != 0 { 0.85 } else { 1.0 };
    let base_ly =
        32.0 + (eff_fuel / eff_mass).sqrt() * (engine_warp * 5.2) * (100.0 / usage).powf(0.55);

    // Ramscoops (free_speed >= 4) & HE Settler's Delight extend cruise range
    let ramscoop_mult = if is_he_colonizer {
        1.35
    } else if engine_free_speed >= 8 {
        2.3
    } else if engine_free_speed >= 6 {
        1.75
    } else if engine_free_speed >= 4 {
        1.30
    } else {
        1.0
    };

    (base_ly * ramscoop_mult).clamp(65.0, 1100.0)
}

impl EmpireTechCache {
    pub fn resolve(
        int_levels: &[i32; NUM_TECH_FIELDS],
        prt: &str,
        lrt_mask: u32,
        ignore_traits: bool,
    ) -> Self {
        let store = tech_store();
        let mut unlocked_bits = [0u32; 8];
        let mut unlocked_count = 0u16;

        let mark = |id: u16, bits: &mut [u32; 8], count: &mut u16| {
            let word = (id as usize) / 32;
            let bit = (id as usize) % 32;
            if word < 8 {
                bits[word] |= 1u32 << bit;
                *count += 1;
            }
        };

        // Defaults (tech 0 baseline)
        let mut engine_name = "Quick Jump 5".to_string();
        let mut engine_warp = 5.0f32;
        let mut engine_free_speed = 1i32;
        let mut engine_fuel_usage = 100.0f32;
        let mut weapon_name = "Laser".to_string();
        let mut weapon_power = 10.0f32;
        let mut weapon_is_torpedo = false;
        let mut bomb_name = "Lady Finger Bomb".to_string();
        let mut bomb_power = 0.6f32;
        let mut bomb_smart = false;
        let mut armor_name = "Tritanium".to_string();
        let mut armor_val = 50.0f32;
        let mut shield_name = "Mole-skin Shield".to_string();
        let mut shield_val = 25.0f32;
        let mut scanner_name = "Bat Scanner".to_string();
        let mut scanner_range = 60.0f32;
        let mut scanner_pen = 0.0f32;
        let mut best_cloak_armed = 0.0f32;
        let mut best_cloak_unarmed = 0.0f32;
        let mut has_tachyon = false;
        let mut best_torpedo_jamming = 0.0f32;
        let mut defense_name = "SDI".to_string();
        let mut defense_coverage = 0.99f32;
        let mut terraform_name = "None".to_string();
        let mut terraform_ability = 0.0f32;
        let mut stargate_name: Option<String> = None;
        let mut stargate_max_range = 0.0f32;
        let mut stargate_cost_iron = 50.0f32;
        let mut stargate_cost_germ = 50.0f32;
        let mut mining_rate = 2.0f32;
        let mut fuel_tank_bonus = 0.0f32;
        let mut fuel_gen = 0.0f32;

        // 1. Planetary Scanners
        for (id, item) in &store.planetary_scanners {
            if store.is_unlocked(&item.tech, int_levels, prt, lrt_mask, ignore_traits) {
                mark(*id, &mut unlocked_bits, &mut unlocked_count);
                if (item.scan_range as f32) > scanner_range {
                    scanner_range = item.scan_range as f32;
                    scanner_name = item.tech.name.clone();
                }
                if (item.scan_range_pen as f32) > scanner_pen {
                    scanner_pen = item.scan_range_pen as f32;
                }
            }
        }

        // 2. Terraforming
        for (id, item) in &store.terraforms {
            if store.is_unlocked(&item.tech, int_levels, prt, lrt_mask, ignore_traits) {
                mark(*id, &mut unlocked_bits, &mut unlocked_count);
                let eff = if item.hab_type == "TERRAFORM_HAB_TYPE_ALL" {
                    item.ability as f32 * 1.15
                } else {
                    item.ability as f32
                };
                if eff > terraform_ability {
                    terraform_ability = eff;
                    terraform_name = item.tech.name.clone();
                }
            }
        }

        // 3. Planetary Defenses
        for (id, item) in &store.defenses {
            if store.is_unlocked(&item.tech, int_levels, prt, lrt_mask, ignore_traits) {
                mark(*id, &mut unlocked_bits, &mut unlocked_count);
                if item.defense_coverage > defense_coverage {
                    defense_coverage = item.defense_coverage;
                    defense_name = item.tech.name.clone();
                }
            }
        }

        // 4. Planetaries
        for (id, item) in &store.planetaries {
            if store.is_unlocked(&item.tech, int_levels, prt, lrt_mask, ignore_traits) {
                mark(*id, &mut unlocked_bits, &mut unlocked_count);
            }
        }

        // 5. Hull Components
        for (id, item) in &store.hull_components {
            if !store.is_unlocked(&item.tech, int_levels, prt, lrt_mask, ignore_traits) {
                continue;
            }
            mark(*id, &mut unlocked_bits, &mut unlocked_count);

            // Electronic Warfare & hybrid component attributes (Electrical, Shield, Armor, Mechanical, etc.)
            if item.cloak_units > 0 {
                let u = item.cloak_units as f32;
                if u > best_cloak_unarmed {
                    best_cloak_unarmed = u;
                }
                if !item.cloak_unarmed_only && u > best_cloak_armed {
                    best_cloak_armed = u;
                }
            }
            if item.reduce_cloaking {
                has_tachyon = true;
            }
            if item.torpedo_jamming > best_torpedo_jamming {
                best_torpedo_jamming = item.torpedo_jamming;
            }

            match item.tech.category.as_str() {
                "TECH_CATEGORY_ENGINE" => {
                    // Prefer highest ideal_speed (cruise warp) and ramscoop free_speed on tie
                    if item.tech.requirements.hulls_allowed.is_empty() {
                        let spd = item.ideal_speed.max(5) as f32;
                        let ideal_idx = (item.ideal_speed.max(1) as usize).min(item.fuel_usage.len().saturating_sub(1));
                        let usage = item
                            .fuel_usage
                            .get(ideal_idx)
                            .copied()
                            .unwrap_or(100) as f32;
                        let better_engine = spd > engine_warp
                            || ((spd - engine_warp).abs() < 0.01 && item.free_speed > engine_free_speed);
                        if better_engine {
                            engine_warp = spd;
                            engine_free_speed = item.free_speed;
                            engine_fuel_usage = if usage <= 0.0 { 40.0 } else { usage };
                            engine_name = item.tech.name.clone();
                        }
                    }
                }
                "TECH_CATEGORY_BEAM_WEAPON" | "TECH_CATEGORY_TORPEDO" => {
                    let is_torp = item.tech.category == "TECH_CATEGORY_TORPEDO";
                    let eff_pow = if is_torp {
                        (item.power as f32) * ((item.accuracy.max(35) as f32) / 100.0) * 1.35
                    } else {
                        item.power as f32 * (1.0 + (item.range as f32) * 0.15)
                    };
                    if eff_pow > weapon_power {
                        weapon_power = eff_pow;
                        weapon_name = item.tech.name.clone();
                        weapon_is_torpedo = is_torp;
                    }
                }
                "TECH_CATEGORY_BOMB" => {
                    let eff = item.kill_rate + item.structure_destroy_rate * 0.35 + if item.smart { 1.5 } else { 0.0 };
                    if eff > bomb_power {
                        bomb_power = eff;
                        bomb_name = item.tech.name.clone();
                        bomb_smart = item.smart;
                    }
                }
                "TECH_CATEGORY_ARMOR" => {
                    if (item.armor as f32) > armor_val {
                        armor_val = item.armor as f32;
                        armor_name = item.tech.name.clone();
                    }
                }
                "TECH_CATEGORY_SHIELD" => {
                    if (item.shield as f32) > shield_val {
                        shield_val = item.shield as f32;
                        shield_name = item.tech.name.clone();
                    }
                }
                "TECH_CATEGORY_SCANNER" => {
                    if (item.scan_range as f32) > scanner_range {
                        scanner_range = item.scan_range as f32;
                        scanner_name = item.tech.name.clone();
                    }
                    if (item.scan_range_pen as f32) > scanner_pen {
                        scanner_pen = item.scan_range_pen as f32;
                    }
                }
                "TECH_CATEGORY_ORBITAL" => {
                    if item.safe_range > 0 {
                        let rng = if item.safe_range >= 1_000_000 {
                            2000.0
                        } else {
                            item.safe_range as f32
                        };
                        if rng > stargate_max_range {
                            stargate_max_range = rng;
                            stargate_name = Some(item.tech.name.clone());
                            stargate_cost_iron = (item.tech.cost.ironium.max(25) as f32).min(90.0);
                            stargate_cost_germ = (item.tech.cost.germanium.max(25) as f32).min(90.0);
                        }
                    }
                }
                "TECH_CATEGORY_MINE_ROBOT" => {
                    if (item.mining_rate as f32) > mining_rate {
                        mining_rate = item.mining_rate as f32;
                    }
                }
                "TECH_CATEGORY_MECHANICAL" => {
                    if (item.fuel_bonus as f32) > fuel_tank_bonus {
                        fuel_tank_bonus = item.fuel_bonus as f32;
                    }
                    if (item.fuel_generation as f32) > fuel_gen {
                        fuel_gen = item.fuel_generation as f32;
                    }
                }
                _ => {}
            }
        }

        // 6. Hulls (resolve best unlocked hull per strategic role)
        let mut hull_col = HullSpec {
            name: "Colony Ship".to_string(),
            armor: 20.0,
            mass: 20.0,
            fuel_capacity: 200.0,
            cargo: 25.0,
            iron: 10.0,
            bora: 4.0,
            germ: 15.0,
            res: 20.0,
        };
        let mut hull_freight = HullSpec {
            name: "Small Freighter".to_string(),
            armor: 25.0,
            mass: 25.0,
            fuel_capacity: 130.0,
            cargo: 70.0,
            iron: 12.0,
            bora: 2.0,
            germ: 17.0,
            res: 20.0,
        };
        let mut hull_cruiser = HullSpec {
            name: "Scout".to_string(),
            armor: 20.0,
            mass: 8.0,
            fuel_capacity: 50.0,
            cargo: 0.0,
            iron: 4.0,
            bora: 2.0,
            germ: 4.0,
            res: 10.0,
        };
        let mut hull_battleship = HullSpec {
            name: "Destroyer".to_string(),
            armor: 200.0,
            mass: 30.0,
            fuel_capacity: 280.0,
            cargo: 0.0,
            iron: 15.0,
            bora: 3.0,
            germ: 5.0,
            res: 35.0,
        };
        let mut hull_bomber = HullSpec {
            name: "Mini Bomber".to_string(),
            armor: 50.0,
            mass: 28.0,
            fuel_capacity: 120.0,
            cargo: 0.0,
            iron: 20.0,
            bora: 5.0,
            germ: 10.0,
            res: 35.0,
        };

        let mut best_col_cargo = 0i32;
        let mut best_freight_cargo = 0i32;
        let mut best_cruiser_armor = 0i32;
        let mut best_bship_armor = 0i32;
        let mut best_bomber_armor = 0i32;
        let mut best_starbase_tier: u8 = 1;
        let mut starbase_name = "Orbital Fort".to_string();

        for (id, h) in &store.hulls {
            if !store.is_unlocked(&h.tech, int_levels, prt, lrt_mask, ignore_traits) {
                continue;
            }
            if h.starbase {
                let tier = match h.tech.name.as_str() {
                    "Death Star" => 5,
                    "Ultra Station" => 4,
                    "Space Station" => 3,
                    "Space Dock" => 2,
                    _ => 1,
                };
                if tier >= best_starbase_tier {
                    best_starbase_tier = tier;
                    starbase_name = h.tech.name.clone();
                }
                continue;
            }
            mark(*id, &mut unlocked_bits, &mut unlocked_count);

            match h.hull_type.as_str() {
                "TECH_HULL_TYPE_COLONIZER" => {
                    if h.cargo_capacity >= best_col_cargo {
                        best_col_cargo = h.cargo_capacity;
                        hull_col = hull_from_raw(h);
                    }
                }
                "TECH_HULL_TYPE_FREIGHTER" | "TECH_HULL_TYPE_MULTI_PURPOSE_FREIGHTER" => {
                    if h.cargo_capacity > best_freight_cargo {
                        best_freight_cargo = h.cargo_capacity;
                        hull_freight = hull_from_raw(h);
                    }
                }
                "TECH_HULL_TYPE_SCOUT" | "TECH_HULL_TYPE_FIGHTER" => {
                    if h.armor > best_cruiser_armor {
                        best_cruiser_armor = h.armor;
                        hull_cruiser = hull_from_raw(h);
                    }
                }
                "TECH_HULL_TYPE_CAPITAL_SHIP" => {
                    if h.armor > best_bship_armor {
                        best_bship_armor = h.armor;
                        hull_battleship = hull_from_raw(h);
                    }
                }
                "TECH_HULL_TYPE_BOMBER" => {
                    if h.armor > best_bomber_armor {
                        best_bomber_armor = h.armor;
                        hull_bomber = hull_from_raw(h);
                    }
                }
                _ => {}
            }
        }

        // If no capital ship unlocked yet, use best fighter hull for battleship role
        if best_bship_armor == 0 && best_cruiser_armor > 0 {
            hull_battleship = hull_cruiser.clone();
        }

        // Compute HUD milestone perk badges backed by real craig-stars unlocks
        let mut perk_mask = 0u8;
        if scanner_pen > 0.0 || scanner_range >= 150.0 {
            perk_mask |= BP_SCANNERS;
        }
        if engine_warp >= 9.0 {
            perk_mask |= BP_WARP10;
        }
        if stargate_name.is_some() {
            perk_mask |= BP_STARGATE;
        }
        if defense_coverage >= 2.35 {
            perk_mask |= BP_PHASE_DEF;
        }
        if terraform_ability >= 10.0 {
            perk_mask |= BP_TERRA;
        }
        if int_levels[0] >= 10 || (lrt_mask & LRT_MA != 0 && int_levels[0] >= 5) {
            perk_mask |= BP_ALCHEMY2;
        }

        let is_he_col = prt == "PRT_HE" || ignore_traits;
        let range_col = compute_hull_range(
            &hull_col,
            engine_warp,
            engine_free_speed,
            engine_fuel_usage,
            fuel_tank_bonus,
            fuel_gen,
            lrt_mask,
            is_he_col,
        );
        let range_freight = compute_hull_range(
            &hull_freight,
            engine_warp,
            engine_free_speed,
            engine_fuel_usage,
            fuel_tank_bonus,
            fuel_gen,
            lrt_mask,
            false,
        );
        let range_cruiser = compute_hull_range(
            &hull_cruiser,
            engine_warp,
            engine_free_speed,
            engine_fuel_usage,
            fuel_tank_bonus,
            fuel_gen,
            lrt_mask,
            false,
        );
        let range_battleship = compute_hull_range(
            &hull_battleship,
            engine_warp,
            engine_free_speed,
            engine_fuel_usage,
            fuel_tank_bonus,
            fuel_gen,
            lrt_mask,
            false,
        );
        let range_bomber = compute_hull_range(
            &hull_bomber,
            engine_warp,
            engine_free_speed,
            engine_fuel_usage,
            fuel_tank_bonus,
            fuel_gen,
            lrt_mask,
            false,
        );

        // Resolve Electronic Warfare stats from unlocked components + racial traits + rules.json
        let innate_ss_cloak = if !ignore_traits && prt == "PRT_SS" {
            300.0 // Super Stealth innate 75% cloak baseline
        } else {
            0.0
        };
        let cloak_units_armed = best_cloak_armed + innate_ss_cloak;
        let cloak_units_unarmed = best_cloak_unarmed.max(best_cloak_armed) + innate_ss_cloak;
        let cloak_armed_pct = cloak_units_to_pct(cloak_units_armed);
        let cloak_unarmed_pct = cloak_units_to_pct(cloak_units_unarmed);

        // Tachyon Detector: scales from ~25% at initial unlock up to ~62% on late-game starbases/capitals
        let tachyon_cloak_reduction = if has_tachyon {
            let per_det = store.ew_rules.tachyon_cloak_reduction.clamp(0.01, 0.25);
            let max_red = store.ew_rules.tachyon_max_cloak_reduction.clamp(0.25, 0.90);
            let eff_detectors = (5.0 + (int_levels[4] - 6).max(0) as f32 * 0.65).min(18.0);
            (1.0 - (1.0 - per_det).powf(eff_detectors)).min(max_red).min(0.65)
        } else {
            0.0
        };

        // Torpedo Jammer (capped at 75%) + Planetary ECM Scanner Jamming (unlocked from Jammer 20/30/50)
        let torpedo_jamming = (best_torpedo_jamming * 1.35).min(0.75);
        let (ecm_jammer_strength, ecm_jammer_range) = if best_torpedo_jamming >= 0.20 {
            let ss_mult = if !ignore_traits && prt == "PRT_SS" { 1.25 } else { 1.0 };
            let strength = (best_torpedo_jamming * 1.15 * ss_mult).min(0.85);
            let range = (55.0 + best_torpedo_jamming * 130.0) * ss_mult;
            (strength, range)
        } else {
            (0.0, 0.0)
        };

        Self {
            unlocked_bits,
            unlocked_count,
            perk_mask,
            engine_name,
            engine_warp,
            engine_free_speed,
            engine_fuel_usage,
            weapon_name,
            weapon_power,
            weapon_is_torpedo,
            bomb_name,
            bomb_power,
            bomb_smart,
            armor_name,
            armor_val,
            shield_name,
            shield_val,
            scanner_name,
            scanner_range,
            scanner_pen,
            cloak_units_armed,
            cloak_units_unarmed,
            cloak_armed_pct,
            cloak_unarmed_pct,
            tachyon_cloak_reduction,
            torpedo_jamming,
            ecm_jammer_strength,
            ecm_jammer_range,
            defense_name,
            defense_coverage,
            terraform_name,
            terraform_ability,
            stargate_name,
            stargate_max_range,
            stargate_cost_iron,
            stargate_cost_germ,
            mining_rate,
            fuel_tank_bonus,
            fuel_gen,
            hull_col,
            hull_freight,
            hull_cruiser,
            hull_battleship,
            hull_bomber,
            range_col,
            range_freight,
            range_cruiser,
            range_battleship,
            range_bomber,
            best_starbase_tier,
            starbase_name,
        }
    }

    pub fn to_loadout_view(&self) -> EmpireLoadoutView {
        EmpireLoadoutView {
            unlocked_count: self.unlocked_count,
            unlocked_bits: self.unlocked_bits,
            engine: format!("{} (W{:.0})", self.engine_name, self.engine_warp),
            weapon: format!("{} ({:.0})", self.weapon_name, self.weapon_power),
            bomb: self.bomb_name.clone(),
            armor: format!("{} ({:.0})", self.armor_name, self.armor_val),
            shield: format!("{} ({:.0})", self.shield_name, self.shield_val),
            scanner: format!("{} ({:.0}ly)", self.scanner_name, self.scanner_range),
            defense: format!("{} ({:.2}%)", self.defense_name, self.defense_coverage),
            terraform: self.terraform_name.clone(),
            stargate: self
                .stargate_name
                .clone()
                .unwrap_or_else(|| "Locked".to_string()),
            cruiser_hull: self.hull_cruiser.name.clone(),
            battleship_hull: self.hull_battleship.name.clone(),
            bomber_hull: self.hull_bomber.name.clone(),
            range_col: self.range_col,
            range_cruiser: self.range_cruiser,
            range_battleship: self.range_battleship,
            range_bomber: self.range_bomber,
            scanner_range: self.scanner_range,
            cloak_armed_pct: self.cloak_armed_pct,
            cloak_unarmed_pct: self.cloak_unarmed_pct,
            tachyon_reduction_pct: self.tachyon_cloak_reduction,
            torpedo_jamming_pct: self.torpedo_jamming,
            ecm_jammer_range: self.ecm_jammer_range,
            ecm_jammer_strength: self.ecm_jammer_strength,
        }
    }
}

pub fn starting_tech_for_traits(prt: &str, lrt_mask: u32) -> [i32; NUM_TECH_FIELDS] {
    let mut lv = [0i32; NUM_TECH_FIELDS];
    match prt {
        "PRT_JOAT" => lv = [3, 3, 3, 3, 3, 3],
        "PRT_CA" => {
            lv[0] = 1;
            lv[1] = 1;
            lv[2] = 1;
            lv[3] = 2;
            lv[5] = 6;
        }
        "PRT_WM" => {
            lv[0] = 1;
            lv[1] = 6;
            lv[2] = 1;
        }
        "PRT_SS" => {
            lv[4] = 5;
        }
        "PRT_IT" => {
            lv[2] = 5;
            lv[3] = 5;
        }
        "PRT_PP" => {
            lv[0] = 4;
        }
        "PRT_SD" => {
            lv[2] = 2;
            lv[5] = 2;
        }
        "PRT_AR" => {
            lv[0] = 1;
        }
        _ => {}
    }
    if lrt_mask & LRT_IFE != 0 {
        lv[2] = lv[2].max(1);
    }
    lv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_239_craig_stars_techs() {
        let store = tech_store();
        assert_eq!(store.entries.len(), 239);
        assert_eq!(store.planetary_scanners.len(), 9);
        assert_eq!(store.terraforms.len(), 20);
        assert_eq!(store.defenses.len(), 5);
        assert_eq!(store.planetaries.len(), 1);
        assert_eq!(store.hull_components.len(), 167);
        assert_eq!(store.hulls.len(), 37);
    }

    #[test]
    fn resolves_upgrades_across_tech_levels() {
        let low = EmpireTechCache::resolve(&[0; 6], "PRT_JOAT", 0, false);
        let mid = EmpireTechCache::resolve(&[10; 6], "PRT_IT", LRT_IFE | LRT_TT, false);
        let max = EmpireTechCache::resolve(&[26; 6], "PRT_WM", LRT_ISB | LRT_RS, true);

        assert!(mid.unlocked_count > low.unlocked_count);
        assert!(max.unlocked_count >= 220);
        assert!(mid.stargate_name.is_some());
        assert!(max.weapon_power > mid.weapon_power);
        assert!(max.engine_warp >= 9.0);
        assert!(low.range_col < 110.0, "early colonizer range should be local (<110ly), got {}", low.range_col);
        assert!(mid.range_col > low.range_col, "mid-game tech should expand ship range");
        assert!(max.range_battleship > 450.0, "late-game ramscoop capital range should span sectors");
        assert_eq!(classic_races().len(), 6);
    }

    #[test]
    fn cloak_curve_and_electronic_warfare_resolution() {
        assert_eq!(cloak_units_to_pct(0.0), 0.0);
        assert!((cloak_units_to_pct(100.0) - 0.50).abs() < 1e-5);
        assert!((cloak_units_to_pct(300.0) - 0.75).abs() < 1e-5);
        assert!((cloak_units_to_pct(600.0) - 0.875).abs() < 1e-5);
        assert!((cloak_units_to_pct(5000.0) - 0.98).abs() < 1e-5);

        // PRT_SS starts with innate 75% cloak baseline
        let ss_start = EmpireTechCache::resolve(&[0, 0, 0, 0, 5, 0], "PRT_SS", 0, false);
        assert!(ss_start.cloak_armed_pct >= 0.75);
        assert!(ss_start.cloak_unarmed_pct >= ss_start.cloak_armed_pct);

        // Low tech has no Tachyon Detector or ECM jammer
        let low = EmpireTechCache::resolve(&[0; 6], "PRT_JOAT", 0, false);
        assert_eq!(low.tachyon_cloak_reduction, 0.0);
        assert_eq!(low.ecm_jammer_strength, 0.0);

        // Mid/high Electronics unlocks Jammer 20/30/50 for JOAT and Tachyon Detector for PRT_IS
        let high_joat = EmpireTechCache::resolve(&[14, 14, 14, 14, 18, 14], "PRT_JOAT", 0, false);
        assert_eq!(high_joat.tachyon_cloak_reduction, 0.0);
        assert!(high_joat.torpedo_jamming > 0.30 && high_joat.torpedo_jamming <= 0.75);
        assert!(high_joat.ecm_jammer_strength > 0.20 && high_joat.ecm_jammer_range > 80.0);

        let high_is = EmpireTechCache::resolve(&[10, 10, 10, 10, 18, 10], "PRT_IS", 0, false);
        assert!(high_is.tachyon_cloak_reduction > 0.35 && high_is.tachyon_cloak_reduction <= 0.65);

        let sandbox = EmpireTechCache::resolve(&[14, 14, 14, 14, 18, 14], "PRT_JOAT", 0, true);
        assert!(sandbox.tachyon_cloak_reduction > 0.35 && sandbox.tachyon_cloak_reduction <= 0.65);
    }
}
