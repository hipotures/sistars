use serde::{Deserialize, Serialize};

use crate::config::{FleetClass, NUM_EMPIRES, NUM_TECH_FIELDS};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Doctrine {
    pub name: String,
    pub color: String,
    pub fac_share: f32,
    pub ship_share: f32,
    pub tech_share: f32,
    pub col_pop_req: f32,
    pub dist_exp: f32,
    pub qual_exp: f32,
    pub cruiser_share: f32,
    pub bship_share: f32,
    pub bomber_share: f32,
    pub intercept_bias: f32,
    pub tech_alloc: [f32; NUM_TECH_FIELDS],
    pub kills: u32,
    pub gates: u32,
    pub last_fit: i32,
    pub win_rate: u32,
}

pub fn default_doctrines() -> [Doctrine; NUM_EMPIRES] {
    [
        Doctrine {
            name: "🟢 Industry".to_string(),
            color: "#4ade80".to_string(),
            fac_share: 0.52,
            ship_share: 0.22,
            tech_share: 0.14,
            col_pop_req: 0.35,
            dist_exp: 1.4,
            qual_exp: 1.7,
            cruiser_share: 0.30,
            bship_share: 0.45,
            bomber_share: 0.25,
            intercept_bias: 0.45,
            tech_alloc: [0.22, 0.14, 0.14, 0.24, 0.10, 0.16],
            kills: 0,
            gates: 0,
            last_fit: 0,
            win_rate: 33,
        },
        Doctrine {
            name: "🔵 Fleet".to_string(),
            color: "#38bdf8".to_string(),
            fac_share: 0.46,
            ship_share: 0.28,
            tech_share: 0.12,
            col_pop_req: 0.28,
            dist_exp: 1.9,
            qual_exp: 1.2,
            cruiser_share: 0.50,
            bship_share: 0.30,
            bomber_share: 0.20,
            intercept_bias: 0.75,
            tech_alloc: [0.14, 0.18, 0.26, 0.16, 0.16, 0.10],
            kills: 0,
            gates: 0,
            last_fit: 0,
            win_rate: 33,
        },
        Doctrine {
            name: "🟡 Science".to_string(),
            color: "#facc15".to_string(),
            fac_share: 0.48,
            ship_share: 0.22,
            tech_share: 0.20,
            col_pop_req: 0.34,
            dist_exp: 1.5,
            qual_exp: 1.5,
            cruiser_share: 0.30,
            bship_share: 0.25,
            bomber_share: 0.45,
            intercept_bias: 0.55,
            tech_alloc: [0.24, 0.22, 0.16, 0.12, 0.14, 0.12],
            kills: 0,
            gates: 0,
            last_fit: 0,
            win_rate: 34,
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fleet {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub from: usize,
    pub target: usize,
    pub emp: u8, // 1..=3
    #[serde(rename = "type")]
    pub fleet_type: FleetClass,
    pub min_type: u8, // 0=Iron, 1=Bora, 2=Germ
    pub power: f32,
    pub max_power: f32,
    pub hunting: bool,
    pub used_wh: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Wormhole {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DebrisField {
    pub x: f32,
    pub y: f32,
    pub iron: f32,
    pub bora: f32,
    pub germ: f32,
    pub ttl: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombatFx {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub mx: f32,
    pub my: f32,
    pub ttl: u8,
    pub col: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JumpFx {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub ttl: u8,
    pub col: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimEvent {
    pub year: f32,
    pub text: String,
    pub color: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct EmpireStats {
    pub planets: u32,
    pub factories: f32,
    pub res: f32,
    pub gates: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanetView {
    pub id: usize,
    pub x: f32,
    pub y: f32,
    pub owner: u8,
    pub has_gate: bool,
    pub hab: f32,
    pub pop: f32,
    pub max_pop: f32,
    pub mines: f32,
    pub factories: f32,
    pub act_fac: f32,
    pub max_buildings: f32,
    pub iron_surf: f32,
    pub iron_crust: f32,
    pub bora_surf: f32,
    pub bora_crust: f32,
    pub germ_surf: f32,
    pub germ_crust: f32,
    pub res: f32,
    pub defense: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimSnapshot {
    pub year: f32,
    pub ticks: u64,
    pub map_seed: u32,
    pub dna_generation: u32,
    pub stats: [EmpireStats; NUM_EMPIRES],
    pub tech_lv: [[f32; NUM_TECH_FIELDS]; NUM_EMPIRES],
    pub perks: [u8; NUM_EMPIRES],
    pub doctrines: [Doctrine; NUM_EMPIRES],
    pub planets: Vec<PlanetView>,
    pub fleets: Vec<Fleet>,
    pub wormholes: Vec<Wormhole>,
    pub debris_fields: Vec<DebrisField>,
    pub new_combat_fx: Vec<CombatFx>,
    pub new_jump_fx: Vec<JumpFx>,
    pub power_history: Vec<[f32; NUM_EMPIRES]>,
    pub events: Vec<SimEvent>,
    pub space_battles_count: u32,
    pub invasions_count: u32,
    pub total_salvaged: u32,
    pub gate_jumps_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TournamentResult {
    pub num_games: u32,
    pub wins: [u32; NUM_EMPIRES],
    pub win_rates: [u32; NUM_EMPIRES],
    pub total_res: [f32; NUM_EMPIRES],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoEvolutionResult {
    pub dna_generation: u32,
    pub tournament: TournamentResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoBalanceResult {
    pub ind_weight: f32,
    pub exp_weight: f32,
    pub tech_weight: f32,
    pub tournament: TournamentResult,
}
