use serde::{Deserialize, Serialize};

use crate::config::{FleetClass, NUM_TECH_FIELDS};
use crate::tech::{CatalogEntry, LrtMeta, PrtMeta};

fn default_one() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

fn default_debris_ttl() -> f32 {
    12.0
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioModifiers {
    #[serde(default = "default_one")]
    pub scanner_range_mult: f32,
    #[serde(default = "default_one")]
    pub ew_strength_mult: f32,
    #[serde(default = "default_one")]
    pub tech_speed_mult: f32,
    #[serde(default = "default_one")]
    pub growth_rate_mult: f32,
    #[serde(default)]
    pub core_richness_bias: f32,
    #[serde(default)]
    pub fallen_empire: bool,
    #[serde(default = "default_true")]
    pub allow_stargates: bool,
    #[serde(default = "default_debris_ttl")]
    pub debris_ttl: f32,
    #[serde(default = "default_true")]
    pub auto_salvage: bool,
    #[serde(default = "default_true")]
    pub allow_alchemy: bool,
}

impl Default for ScenarioModifiers {
    fn default() -> Self {
        Self {
            scanner_range_mult: 1.0,
            ew_strength_mult: 1.0,
            tech_speed_mult: 1.0,
            growth_rate_mult: 1.0,
            core_richness_bias: 0.0,
            fallen_empire: false,
            allow_stargates: true,
            debris_ttl: 12.0,
            auto_salvage: true,
            allow_alchemy: true,
        }
    }
}

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
    pub scanner_range: f32,
    pub cloak_armed_pct: f32,
    pub cloak_unarmed_pct: f32,
    pub tachyon_reduction_pct: f32,
    pub torpedo_jamming_pct: f32,
    pub ecm_jammer_range: f32,
    pub ecm_jammer_strength: f32,
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
    pub wh_route: i16,
    #[serde(default)]
    pub salvage_mode: u8, // 0 = normal, 1 = salvage outbound/scooping, 2 = salvage returning home
    #[serde(default)]
    pub cargo_iron: f32,
    #[serde(default)]
    pub cargo_bora: f32,
    #[serde(default)]
    pub cargo_germ: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Wormhole {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WreckEntry {
    pub emp: u8,
    pub ship_type: FleetClass,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebrisField {
    pub x: f32,
    pub y: f32,
    pub iron: f32,
    pub bora: f32,
    pub germ: f32,
    pub ttl: f32,
    #[serde(default)]
    pub wrecks: Vec<WreckEntry>,
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
    pub has_ecm_jammer: bool,
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

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepProfile {
    pub econ_ms: f64,
    pub ai_ms: f64,
    pub move_ms: f64,
    pub ew_scan_ms: f64,
    pub combat_ms: f64,
    pub total_ms: f64,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_profile: Option<StepProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportsSnapshot {
    pub planets: Vec<PlanetView>,
    pub fleets: Vec<Fleet>,
    pub debris_fields: Vec<DebrisField>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyType {
    Terrestrial,
    Oceanic,
    Desert,
    Ice,
    Volcanic,
    Rock,
    GasGiant,
    IceGiant,
    Dwarf,
    AsteroidBelt,
    KuiperBelt,
    Comet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StarKind {
    MainSequence,
    BrownDwarf,
    WhiteDwarf,
    Pulsar,
    BlackHole,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StarView {
    pub star_kind: StarKind,
    pub spectral_class: String,
    pub temp_k: f32,
    pub mass_solar: f32,
    pub radius_solar: f32,
    pub luminosity_solar: f32,
    pub color_rgb: [f32; 3],
    pub seed: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanetLiveStateView {
    pub owner: u8,
    pub owner_color: String,
    pub pop_norm: f32,
    pub has_starbase: bool,
    pub has_stargate: bool,
    pub has_jammer: bool,
    pub mineral_tint: [f32; 3],
    pub grav_g: f32,
    pub rad_mr: f32,
    pub terra_pct: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoonView {
    pub name: String,
    pub orbit_radius_planet: f32,
    pub orbit_angle_rad: f32,
    pub orbit_speed: f32,
    pub radius_earth: f32,
    pub body_type: BodyType,
    pub temp_k: f32,
    pub water_pct: f32,
    pub ice_pct: f32,
    pub atmo_density: f32,
    pub atmo_color_rgb: [f32; 3],
    pub cloud_pct: f32,
    pub seed: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemBodyView {
    pub name: String,
    pub orbit_au: f32,
    pub orbit_angle_rad: f32,
    pub radius_earth: f32,
    pub body_type: BodyType,
    pub temp_k: f32,
    pub water_pct: f32,
    pub ice_pct: f32,
    pub atmo_density: f32,
    pub atmo_color_rgb: [f32; 3],
    pub cloud_pct: f32,
    pub has_rings: bool,
    pub ring_inner: f32,
    pub ring_outer: f32,
    pub ring_color_rgb: [f32; 3],
    pub moons_count: u8,
    pub moons: Vec<MoonView>,
    pub seed: u32,
    pub live_state: Option<PlanetLiveStateView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StarSystemView {
    pub planet_id: usize,
    pub system_seed: u32,
    pub star: StarView,
    pub companion_star: Option<StarView>,
    pub primary_index: usize,
    pub bodies: Vec<SystemBodyView>,
}
