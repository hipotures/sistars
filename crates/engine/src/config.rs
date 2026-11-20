use serde::{Deserialize, Serialize};

pub const WORLD_W: f32 = 1000.0;
pub const WORLD_H: f32 = 560.0;
pub const DT: f32 = 0.05;
pub const NUM_EMPIRES: usize = 3;
pub const NUM_TECH_FIELDS: usize = 6;
pub const MAX_POWER_HISTORY: usize = 300;
pub const MAX_DEBRIS_FIELDS: usize = 45;
pub const MAX_EVENT_LOG: usize = 5;

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
        desc: "Shields & Alchemy",
        color: "#38bdf8",
    },
    TechFieldMeta {
        index: 1,
        key: "WEP",
        name: "Weapons",
        desc: "Fleet Firepower",
        color: "#f87171",
    },
    TechFieldMeta {
        index: 2,
        key: "PRO",
        name: "Propulsion",
        desc: "Warp Speed",
        color: "#a78bfa",
    },
    TechFieldMeta {
        index: 3,
        key: "CON",
        name: "Construction",
        desc: "Cheaper Hulls",
        color: "#fbbf24",
    },
    TechFieldMeta {
        index: 4,
        key: "ELE",
        name: "Electronics",
        desc: "Scanners & Sprawl",
        color: "#22d3ee",
    },
    TechFieldMeta {
        index: 5,
        key: "BIO",
        name: "Biotechnology",
        desc: "Growth & Habitability",
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
    pub req: [(usize, f32); 2],
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
        req: [(4, 3.0), (0, 2.0)],
        req_txt: "ELE 3 + ENE 2",
        desc: "+50% Cruiser intercept pursuit range",
    },
    BreakthroughMeta {
        id: "warp10",
        bit: BP_WARP10,
        icon: "🚀",
        name: "Warp 10 Drive",
        req: [(2, 4.0), (0, 3.0)],
        req_txt: "PRO 4 + ENE 3",
        desc: "+35% speed for all fleets",
    },
    BreakthroughMeta {
        id: "stargate",
        bit: BP_STARGATE,
        icon: "🌀",
        name: "Stargate Network",
        req: [(2, 5.0), (3, 4.0)],
        req_txt: "PRO 5 + CON 4",
        desc: "Unlocks Stargate construction",
    },
    BreakthroughMeta {
        id: "phaseDef",
        bit: BP_PHASE_DEF,
        icon: "🛡️",
        name: "Phase Shields",
        req: [(0, 5.0), (1, 4.0)],
        req_txt: "ENE 5 + WEP 4",
        desc: "+35% planetary defense & +20% attack",
    },
    BreakthroughMeta {
        id: "terra",
        bit: BP_TERRA,
        icon: "🧬",
        name: "Terraforming",
        req: [(5, 5.0), (3, 3.0)],
        req_txt: "BIO 5 + CON 3",
        desc: "+20% population cap on planets",
    },
    BreakthroughMeta {
        id: "alchemy2",
        bit: BP_ALCHEMY2,
        icon: "⚗️",
        name: "Alchemy II",
        req: [(0, 6.0), (5, 4.0)],
        req_txt: "ENE 6 + BIO 4",
        desc: "2x mineral transmutation efficiency",
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

#[derive(Debug, Clone, Copy)]
pub struct ShipDesign {
    pub label: &'static str,
    pub iron: f32,
    pub bora: f32,
    pub germ: f32,
    pub speed_mod: f32,
}

impl FleetClass {
    pub const fn design(self) -> ShipDesign {
        match self {
            FleetClass::Col => ShipDesign {
                label: "● Colonizer",
                iron: 12.0,
                bora: 4.0,
                germ: 8.0,
                speed_mod: 1.00,
            },
            FleetClass::Cruiser => ShipDesign {
                label: "▲ Cruiser",
                iron: 10.0,
                bora: 6.0,
                germ: 8.0,
                speed_mod: 1.32,
            },
            FleetClass::Battleship => ShipDesign {
                label: "◆ Battleship",
                iron: 24.0,
                bora: 10.0,
                germ: 6.0,
                speed_mod: 0.90,
            },
            FleetClass::Bomber => ShipDesign {
                label: "★ Bomber",
                iron: 8.0,
                bora: 20.0,
                germ: 12.0,
                speed_mod: 0.94,
            },
            FleetClass::Freight => ShipDesign {
                label: "■ Freighter",
                iron: 0.0,
                bora: 0.0,
                germ: 0.0,
                speed_mod: 1.10,
            },
        }
    }

    pub const fn is_combat(self) -> bool {
        matches!(
            self,
            FleetClass::Cruiser | FleetClass::Battleship | FleetClass::Bomber
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimConfig {
    pub planets: usize,
    pub wormhole_pairs: usize,
    pub crust_richness: f32,
    pub gate_cost: f32,
    pub intercept_rad: f32,
    pub scanner_range: f32,
    pub salvage_rate: f32,
    pub ind_weight: f32,
    pub exp_weight: f32,
    pub tech_weight: f32,
    pub aggro_bias: f32,
    pub sprawl_penalty: f32,
    pub col_ship_cost: f32,
    pub bomber_pen: f32,
    pub fac_cost_res: f32,
    pub fac_cost_germ: f32,
    pub fleet_speed: f32,
    pub growth: f32,
    pub op_limit: f32,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            planets: 80,
            wormhole_pairs: 2,
            crust_richness: 800.0,
            gate_cost: 45.0,
            intercept_rad: 22.0,
            scanner_range: 115.0,
            salvage_rate: 0.65,
            ind_weight: 1.05,
            exp_weight: 1.05,
            tech_weight: 1.15,
            aggro_bias: 0.65,
            sprawl_penalty: 0.22,
            col_ship_cost: 90.0,
            bomber_pen: 0.65,
            fac_cost_res: 10.0,
            fac_cost_germ: 4.0,
            fleet_speed: 36.0,
            growth: 0.12,
            op_limit: 10.0,
        }
    }
}
