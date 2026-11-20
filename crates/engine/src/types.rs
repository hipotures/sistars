use serde::{Deserialize, Serialize};

use crate::config::{FleetClass, NUM_TECH_FIELDS};
use crate::tech::{CatalogEntry, LrtMeta, PrtMeta};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Doctrine {
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
    #[serde(default)]
    pub kills: u32,
    #[serde(default)]
    pub gates: u32,
    #[serde(default)]
    pub last_fit: i32,
    #[serde(default)]
    pub win_rate: u32,
}

impl Default for Doctrine {
    fn default() -> Self {
        Self {
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
            win_rate: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RaceDef {
    pub id: String,
    pub name: String,
    pub color: String,
    pub prt: String,
    #[serde(default)]
    pub lrt_mask: u32,
    #[serde(default)]
    pub is_custom: bool,
    #[serde(default)]
    pub doctrine: Doctrine,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmpireLoadoutView {
    pub unlocked_count: u16,
    pub unlocked_bits: [u32; 8],
    pub engine: String,
    pub weapon: String,
    pub bomb: String,
    pub armor: String,
    pub shield: String,
    pub scanner: String,
    pub defense: String,
    pub terraform: String,
    pub stargate: String,
    pub cruiser_hull: String,
    pub battleship_hull: String,
    pub bomber_hull: String,
    pub range_col: f32,
    pub range_cruiser: f32,
    pub range_battleship: f32,
    pub range_bomber: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmpireSummaryView {
    pub id: String,
    pub name: String,
    pub color: String,
    pub prt: String,
    pub lrt_mask: u32,
    pub is_custom: bool,
    pub stats: EmpireStats,
    pub tech_lv: [f32; NUM_TECH_FIELDS],
    pub perks: u8,
    pub doctrine: Doctrine,
    pub loadout: EmpireLoadoutView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fleet {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub from: usize,
    pub target: usize,
    pub emp: u8, // 1..=empires.len()
    #[serde(rename = "type")]
    pub fleet_type: FleetClass,
    pub min_type: u8,
    pub power: f32,
    pub max_power: f32,
    pub hunting: bool,
    pub used_wh: bool,
    pub steer_x: f32,
    pub steer_y: f32,
    pub wh_route: i8,
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
pub struct HudSnapshot {
    pub year: f32,
    pub ticks: u64,
    pub map_seed: u32,
    pub world_w: f32,
    pub world_h: f32,
    pub dna_generation: u32,
    pub empires: Vec<EmpireSummaryView>,
    pub wormholes: Vec<Wormhole>,
    pub new_combat_fx: Vec<CombatFx>,
    pub new_jump_fx: Vec<JumpFx>,
    pub events: Vec<SimEvent>,
    pub planet_count: usize,
    pub fleet_count: usize,
    pub space_battles_count: u32,
    pub invasions_count: u32,
    pub total_salvaged: u32,
    pub gate_jumps_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportsSnapshot {
    pub planets: Vec<PlanetView>,
    pub fleets: Vec<Fleet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimSnapshot {
    pub year: f32,
    pub ticks: u64,
    pub map_seed: u32,
    pub world_w: f32,
    pub world_h: f32,
    pub dna_generation: u32,
    pub empires: Vec<EmpireSummaryView>,
    pub planets: Vec<PlanetView>,
    pub fleets: Vec<Fleet>,
    pub wormholes: Vec<Wormhole>,
    pub debris_fields: Vec<DebrisField>,
    pub new_combat_fx: Vec<CombatFx>,
    pub new_jump_fx: Vec<JumpFx>,
    pub power_history: Vec<Vec<f32>>,
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
    pub wins: Vec<u32>,
    pub win_rates: Vec<u32>,
    pub total_res: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoEvolutionResult {
    pub dna_generation: u32,
    pub tournament: TournamentResult,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TechCatalogView {
    pub entries: Vec<CatalogEntry>,
    pub prts: Vec<PrtMeta>,
    pub lrts: Vec<LrtMeta>,
    pub classic_races: Vec<RaceDef>,
}
