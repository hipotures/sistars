use serde::{Deserialize, Serialize};

use crate::galaxy::{GalaxyShape, GalaxySize};
use crate::types::ScenarioModifiers;

pub const WORLD_W: f32 = 1000.0;
pub const WORLD_H: f32 = 560.0;
pub const DT: f32 = 0.05;
pub const MAX_EMPIRES: usize = 16;
pub const NUM_TECH_FIELDS: usize = 6;
pub const MAX_TECH_LEVEL: usize = 26;
pub const MAX_POWER_HISTORY: usize = 300;
pub const MAX_DEBRIS_FIELDS: usize = 45;
pub const MAX_EVENT_LOG: usize = 5;
pub const SHIP_COST_SCALE: f32 = 0.25;

/// Cumulative research points required to reach each integer tech level 0..=26
/// (Stars! progressive curve scaled for continuous-time SI-Stars pacing).
pub const TECH_LEVEL_COSTS: [f32; MAX_TECH_LEVEL + 1] = [
    0.0, 20.0, 45.0, 80.0, 130.0, 200.0, 290.0, 410.0, 560.0, 750.0, 990.0, 1290.0, 1660.0,
    2120.0, 2680.0, 3360.0, 4180.0, 5170.0, 6360.0, 7790.0, 9500.0, 11540.0, 13980.0, 16890.0,
    20360.0, 24500.0, 29450.0,
];

/// Converts accumulated research points into a continuous tech level `0.0..=26.0`.
pub fn tech_level_from_points(pts: f32) -> f32 {
    if pts <= 0.0 {
        return 0.0;
    }
    for lv in 0..MAX_TECH_LEVEL {
        let next = TECH_LEVEL_COSTS[lv + 1];
        if pts < next {
            let cur = TECH_LEVEL_COSTS[lv];
            return (lv as f32) + (pts - cur) / (next - cur);
        }
    }
    MAX_TECH_LEVEL as f32
}

/// Returns cumulative points needed to start at a given integer level.
pub fn points_for_tech_level(lv: i32) -> f32 {
    let idx = (lv.max(0) as usize).min(MAX_TECH_LEVEL);
    TECH_LEVEL_COSTS[idx]
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct TechFieldMeta {
    pub index: usize,
    pub key: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    pub color: &'static str,
}

pub const TECH_FIELDS: [TechFieldMeta; NUM_TECH_FIELDS] = [
    TechFieldMeta {
        index: 0,
        key: "ENE",
        name: "Energy",
        desc: "Shields, Defenses & Alchemy",
        color: "#38bdf8",
    },
    TechFieldMeta {
        index: 1,
        key: "WEP",
        name: "Weapons",
        desc: "Beams, Torpedoes & Bombs",
        color: "#f87171",
    },
    TechFieldMeta {
        index: 2,
        key: "PRO",
        name: "Propulsion",
        desc: "Engines & Stargates",
        color: "#a78bfa",
    },
    TechFieldMeta {
        index: 3,
        key: "CON",
        name: "Construction",
        desc: "Ship Hulls, Armor & Starbases",
        color: "#fbbf24",
    },
    TechFieldMeta {
        index: 4,
        key: "ELE",
        name: "Electronics",
        desc: "Scanners, Cloaks & Sprawl",
        color: "#22d3ee",
    },
    TechFieldMeta {
        index: 5,
        key: "BIO",
        name: "Biotechnology",
        desc: "Terraforming & Growth",
        color: "#4ade80",
    },
];

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BreakthroughMeta {
    pub id: &'static str,
    pub bit: u8,
    pub icon: &'static str,
    pub name: &'static str,
    pub req_txt: &'static str,
    pub desc: &'static str,
}

pub const BP_SCANNERS: u8 = 1 << 0;
pub const BP_WARP10: u8 = 1 << 1;
pub const BP_STARGATE: u8 = 1 << 2;
pub const BP_PHASE_DEF: u8 = 1 << 3;
pub const BP_TERRA: u8 = 1 << 4;
pub const BP_ALCHEMY2: u8 = 1 << 5;

pub const BREAKTHROUGHS: [BreakthroughMeta; 6] = [
    BreakthroughMeta {
        id: "scanners",
        bit: BP_SCANNERS,
        icon: "📡",
        name: "Deep Scanners",
        req_txt: "Pen-Scanner / 150ly",
        desc: "Long-range penetrating scanners for fleet interception",
    },
    BreakthroughMeta {
        id: "warp10",
        bit: BP_WARP10,
        icon: "🚀",
        name: "Warp 9+ Drive",
        req_txt: "Warp 9+ Engine",
        desc: "High-speed interstellar propulsion unlocked",
    },
    BreakthroughMeta {
        id: "stargate",
        bit: BP_STARGATE,
        icon: "🌀",
        name: "Stargate Network",
        req_txt: "Orbital Stargate",
        desc: "Instantaneous fleet transit between gated worlds",
    },
    BreakthroughMeta {
        id: "phaseDef",
        bit: BP_PHASE_DEF,
        icon: "🛡️",
        name: "Laser / Shield Def",
        req_txt: "Defense >= 2.35%",
        desc: "Advanced planetary defense batteries & shields",
    },
    BreakthroughMeta {
        id: "terra",
        bit: BP_TERRA,
        icon: "🧬",
        name: "Terraforming ±10%+",
        req_txt: "Terraform >= 10%",
        desc: "Planetary climate engineering expands population caps",
    },
    BreakthroughMeta {
        id: "alchemy2",
        bit: BP_ALCHEMY2,
        icon: "⚗️",
        name: "Advanced Alchemy",
        req_txt: "ENE 10 (or MA + ENE 5)",
        desc: "2x surplus resource-to-mineral transmutation",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FleetClass {
    Col,
    Cruiser,
    Battleship,
    Bomber,
    Freight,
}

impl FleetClass {
    pub const fn speed_mod(self) -> f32 {
        match self {
            FleetClass::Col => 1.00,
            FleetClass::Cruiser => 1.28,
            FleetClass::Battleship => 0.90,
            FleetClass::Bomber => 0.94,
            FleetClass::Freight => 1.08,
        }
    }

    pub const fn is_combat(self) -> bool {
        matches!(
            self,
            FleetClass::Cruiser | FleetClass::Battleship | FleetClass::Bomber
        )
    }

    /// Annual maintenance cost in Resources (Res/yr) for a persistent fleet of this class.
    pub const fn upkeep(self) -> f32 {
        match self {
            FleetClass::Col => 0.8,
            FleetClass::Freight => 1.2,
            FleetClass::Cruiser => 2.5,
            FleetClass::Bomber => 3.0,
            FleetClass::Battleship => 5.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimConfig {
    pub galaxy_size: GalaxySize,
    pub galaxy_shape: GalaxyShape,
    pub planets: usize,
    pub wormhole_pairs: usize,
    pub crust_richness: f32,
    pub gate_cost: f32,
    pub intercept_rad: f32,
    pub scanner_range: f32,
    pub salvage_rate: f32,
    pub aggro_bias: f32,
    pub sprawl_penalty: f32,
    pub col_ship_cost: f32,
    pub bomber_pen: f32,
    pub fac_cost_res: f32,
    pub fac_cost_germ: f32,
    pub fleet_speed: f32,
    pub growth: f32,
    pub op_limit: f32,
    pub ignore_racial_traits: bool,
    #[serde(default)]
    pub scenario_modifiers: ScenarioModifiers,
}

impl Default for SimConfig {
    fn default() -> Self {
        let size = GalaxySize::Small;
        let (planets, _, _) = size.specs();
        Self {
            galaxy_size: size,
            galaxy_shape: GalaxyShape::Spiral,
            planets,
            wormhole_pairs: 2,
            crust_richness: 800.0,
            gate_cost: 45.0,
            intercept_rad: 22.0,
            scanner_range: 115.0,
            salvage_rate: 0.65,
            aggro_bias: 0.65,
            sprawl_penalty: 0.22,
            col_ship_cost: 85.0,
            bomber_pen: 0.65,
            fac_cost_res: 10.0,
            fac_cost_germ: 4.0,
            fleet_speed: 32.0,
            growth: 0.12,
            op_limit: 10.0,
            ignore_racial_traits: false,
            scenario_modifiers: ScenarioModifiers::default(),
        }
    }
}
