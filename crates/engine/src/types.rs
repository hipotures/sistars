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

fn default_ship_count() -> u16 {
    1
}

fn default_mine_share() -> f32 {
    0.12
}

fn default_surplus_def_share() -> f32 {
    0.20
}

fn default_surplus_ship_share() -> f32 {
    0.50
}

fn default_surplus_alch_share() -> f32 {
    0.30
}

fn default_max_upkeep_ratio() -> f32 {
    0.55
}

fn default_min_shipyard_pop_ratio() -> f32 {
    0.32
}

fn default_freight_export_threshold() -> f32 {
    95.0
}

fn default_freight_import_threshold() -> f32 {
    20.0
}

fn default_colonize_vs_attack_bias() -> f32 {
    0.65
}

fn default_min_colonize_hab() -> f32 {
    0.15
}

fn default_mineral_weights() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

fn default_min_force_ratio() -> f32 {
    1.25
}

fn default_min_garrison_count() -> u16 {
    1
}

pub const INTEL_FLAG_EXPLORED: u16 = 1 << 0;
pub const INTEL_FLAG_PEN_SCANNED: u16 = 1 << 1;
pub const INTEL_FLAG_HAS_STARGATE: u16 = 1 << 2;

/// Compact per-empire intelligence record for a single star system (~28 bytes).
/// Frozen at `last_seen_tick` whenever the planet leaves the empire's active scanner coverage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanetIntel {
    pub last_seen_tick: u32,
    pub owner: u8,
    pub starbase_tier: u8,
    pub flags: u16,
    pub pop_est: f32,
    pub def_est: f32,
    pub fac_est: f32,
    pub hab_est: f32,
    pub crust_sum_est: f32,
}

impl PlanetIntel {
    #[inline]
    pub const fn is_explored(self) -> bool {
        (self.flags & INTEL_FLAG_EXPLORED) != 0
    }

    #[inline]
    pub const fn is_pen_scanned(self) -> bool {
        (self.flags & INTEL_FLAG_PEN_SCANNED) != 0
    }

    #[inline]
    pub const fn has_stargate(self) -> bool {
        (self.flags & INTEL_FLAG_HAS_STARGATE) != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SectorMode {
    Expand,
    Fortify,
    Attack,
    Ignore,
}

impl Default for SectorMode {
    fn default() -> Self {
        SectorMode::Expand
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HubKind {
    Stargate,
    Starbase,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DipMessage {
    pub tick: u64,
    pub year: f32,
    pub from_emp: u8,
    pub to_emp: u8,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiplomaticStance {
    War,
    Neutral,
    Ceasefire,
    Alliance,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VictoryConfig {
    pub enabled: bool,
    pub planet_share_threshold: f32,
    pub res_share_threshold: f32,
    pub min_planets_for_share: u32,
    pub elimination_victory: bool,
    pub max_year: Option<f32>,
}

impl Default for VictoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            planet_share_threshold: 0.60,
            res_share_threshold: 0.65,
            min_planets_for_share: 2,
            elimination_victory: true,
            max_year: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxPlanet {
    pub x: f32,
    pub y: f32,
    pub owner: u8,
    pub hab: f32,
    pub pop: f32,
    pub mines: f32,
    pub factories: f32,
    pub defense: f32,
    pub iron_surf: f32,
    pub bora_surf: f32,
    pub germ_surf: f32,
    pub iron_crust: f32,
    pub bora_crust: f32,
    pub germ_crust: f32,
    pub ship_prod_buf: f32,
    pub has_gate: bool,
    pub starbase_tier: u8,
    pub starbase_hp: Option<f32>,
}

impl Default for SandboxPlanet {
    fn default() -> Self {
        Self {
            x: 100.0,
            y: 100.0,
            owner: 0,
            hab: 1.0,
            pop: 40.0,
            mines: 15.0,
            factories: 15.0,
            defense: 10.0,
            iron_surf: 100.0,
            bora_surf: 100.0,
            germ_surf: 100.0,
            iron_crust: 600.0,
            bora_crust: 600.0,
            germ_crust: 600.0,
            ship_prod_buf: 0.0,
            has_gate: false,
            starbase_tier: 0,
            starbase_hp: None,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Doctrine {
    pub fac_share: f32,
    pub ship_share: f32,
    pub tech_share: f32,
    #[serde(default = "default_mine_share")]
    pub mine_share: f32,
    pub col_pop_req: f32,
    #[serde(default = "default_min_shipyard_pop_ratio")]
    pub min_shipyard_pop_ratio: f32,
    pub dist_exp: f32,
    pub qual_exp: f32,
    pub cruiser_share: f32,
    pub bship_share: f32,
    pub bomber_share: f32,
    pub intercept_bias: f32,
    pub tech_alloc: [f32; NUM_TECH_FIELDS],
    #[serde(default = "default_surplus_def_share")]
    pub surplus_def_share: f32,
    #[serde(default = "default_surplus_ship_share")]
    pub surplus_ship_share: f32,
    #[serde(default = "default_surplus_alch_share")]
    pub surplus_alch_share: f32,
    #[serde(default = "default_max_upkeep_ratio")]
    pub max_upkeep_ratio: f32,
    #[serde(default = "default_freight_export_threshold")]
    pub freight_export_threshold: f32,
    #[serde(default = "default_freight_import_threshold")]
    pub freight_import_threshold: f32,
    #[serde(default = "default_colonize_vs_attack_bias")]
    pub colonize_vs_attack_bias: f32,
    #[serde(default = "default_min_colonize_hab")]
    pub min_colonize_hab: f32,
    #[serde(default = "default_mineral_weights")]
    pub mineral_weights: [f32; 3],
    #[serde(default = "default_min_force_ratio")]
    pub min_force_ratio: f32,
    #[serde(default = "default_min_garrison_count")]
    pub min_garrison_count: u16,
    #[serde(default)]
    pub kills: u32,
    #[serde(default)]
    pub gates: u32,
    #[serde(default)]
    pub last_fit: i32,
    #[serde(default)]
    pub win_rate: u32,
}

pub type EmpirePolicy = Doctrine;

impl Default for Doctrine {
    fn default() -> Self {
        Self {
            fac_share: 0.48,
            ship_share: 0.24,
            tech_share: 0.16,
            mine_share: 0.12,
            col_pop_req: 0.32,
            min_shipyard_pop_ratio: 0.32,
            dist_exp: 1.5,
            qual_exp: 1.5,
            cruiser_share: 0.40,
            bship_share: 0.35,
            bomber_share: 0.25,
            intercept_bias: 0.55,
            tech_alloc: [0.17, 0.17, 0.17, 0.17, 0.16, 0.16],
            surplus_def_share: 0.20,
            surplus_ship_share: 0.50,
            surplus_alch_share: 0.30,
            max_upkeep_ratio: 0.55,
            freight_export_threshold: 95.0,
            freight_import_threshold: 20.0,
            colonize_vs_attack_bias: 0.65,
            min_colonize_hab: 0.15,
            mineral_weights: [1.0, 1.0, 1.0],
            min_force_ratio: 1.25,
            min_garrison_count: 1,
            kills: 0,
            gates: 0,
            last_fit: 0,
            win_rate: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StrategicCommand {
    SetPolicy(EmpirePolicy),
    SetDiplomacy {
        target_emp: usize,
        stance: DiplomaticStance,
    },
    SetSectorDirective {
        sector_id: u16,
        mode: SectorMode,
    },
    BuildHub {
        planet_idx: usize,
        kind: HubKind,
    },
    SendDipMessage {
        to_emp: usize,
        text: String,
    },
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
    #[serde(default)]
    pub orbiting: bool,
    #[serde(default)]
    pub upkeep: f32,
    #[serde(default = "default_ship_count")]
    pub ship_count: u16,
}

impl Default for Fleet {
    fn default() -> Self {
        Self {
            id: 0,
            x: 0.0,
            y: 0.0,
            from: 0,
            target: 0,
            emp: 1,
            fleet_type: FleetClass::Cruiser,
            min_type: 0,
            power: 40.0,
            max_power: 40.0,
            hunting: false,
            used_wh: false,
            steer_x: 0.0,
            steer_y: 0.0,
            wh_route: -1,
            salvage_mode: 0,
            cargo_iron: 0.0,
            cargo_bora: 0.0,
            cargo_germ: 0.0,
            orbiting: false,
            upkeep: FleetClass::Cruiser.upkeep(),
            ship_count: 1,
        }
    }
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
pub struct HudEvent {
    pub year: f32,
    pub text: String,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SimEvent {
    SalvageLoaded {
        fleet_id: u32,
        debris_before: f32,
        loaded: f32,
        debris_after: f32,
        cargo_cap: f32,
    },
    CargoUnloaded {
        fleet_id: u32,
        planet_idx: usize,
        unloaded: f32,
        planet_surf_before: f32,
        planet_surf_after: f32,
    },
    SpaceBattle {
        winner_emp: u8,
        loser_emp: u8,
        orbital_defense: bool,
        planet_idx: Option<usize>,
    },
    StarbaseAssault {
        planet_idx: usize,
        sb_hp_before: f32,
        sb_hp_after: f32,
        attacker_survived: bool,
    },
    PlanetAssault {
        planet_idx: usize,
        captured: bool,
        def_before: f32,
        def_after: f32,
    },
    EconomyTick {
        planet_idx: usize,
        res_gen: f32,
        spent_tech: f32,
        spent_mines: f32,
        spent_fac: f32,
        spent_def: f32,
        spent_ship: f32,
        spent_alchemy: f32,
    },
    VictoryTriggered {
        winner_emp: u8,
        reason: String,
        year: f32,
    },
}

impl std::fmt::Display for SimEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SimEvent::SalvageLoaded {
                fleet_id,
                debris_before,
                loaded,
                debris_after,
                cargo_cap,
            } => write!(
                f,
                "[SalvageLoaded] fleet=#{fleet_id} cap={cargo_cap:.1}kT | debris {debris_before:.1}kT -> loaded {loaded:.1}kT -> remaining {debris_after:.1}kT"
            ),
            SimEvent::CargoUnloaded {
                fleet_id,
                planet_idx,
                unloaded,
                planet_surf_before,
                planet_surf_after,
            } => write!(
                f,
                "[CargoUnloaded] fleet=#{fleet_id} planet=#{planet_idx} | unloaded={unloaded:.1}kT (surf {planet_surf_before:.1}kT -> {planet_surf_after:.1}kT)"
            ),
            SimEvent::SpaceBattle {
                winner_emp,
                loser_emp,
                orbital_defense,
                planet_idx,
            } => write!(
                f,
                "[SpaceBattle] winner=Emp{winner_emp} loser=Emp{loser_emp} orbital_defense={orbital_defense} planet={planet_idx:?}"
            ),
            SimEvent::StarbaseAssault {
                planet_idx,
                sb_hp_before,
                sb_hp_after,
                attacker_survived,
            } => write!(
                f,
                "[StarbaseAssault] planet=#{planet_idx} sb_hp={sb_hp_before:.1}->{sb_hp_after:.1} attacker_survived={attacker_survived}"
            ),
            SimEvent::PlanetAssault {
                planet_idx,
                captured,
                def_before,
                def_after,
            } => write!(
                f,
                "[PlanetAssault] planet=#{planet_idx} captured={captured} def={def_before:.1}->{def_after:.1}"
            ),
            SimEvent::EconomyTick {
                planet_idx,
                res_gen,
                spent_tech,
                spent_mines,
                spent_fac,
                spent_def,
                spent_ship,
                spent_alchemy,
            } => write!(
                f,
                "[EconomyTick] planet=#{planet_idx} res_gen={res_gen:.3} | tech={spent_tech:.3} mines={spent_mines:.3} fac={spent_fac:.3} def={spent_def:.3} ship={spent_ship:.3} alch={spent_alchemy:.3}"
            ),
            SimEvent::VictoryTriggered {
                winner_emp,
                reason,
                year,
            } => write!(
                f,
                "[VictoryTriggered] winner=Emp{winner_emp} reason={reason} year={year:.2}"
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimAnomaly {
    pub tick: u64,
    pub year: f32,
    pub kind: String,
    pub message: String,
    pub event: Option<SimEvent>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveAuditor {
    pub audited_events_count: u64,
    pub closed_transactions_count: u64,
    pub pending_salvage_loads: std::collections::HashMap<u32, (f32, f32)>,
    pub prune_valid_events: bool,
}

impl LiveAuditor {
    pub fn observe_event(
        &mut self,
        tick: u64,
        year: f32,
        ev: &SimEvent,
        anomalies: &mut Vec<SimAnomaly>,
    ) -> bool {
        self.audited_events_count += 1;
        let mut valid = true;
        let mut push_anom = |kind: &str, message: String| {
            valid = false;
            if anomalies.len() < 1024 {
                anomalies.push(SimAnomaly {
                    tick,
                    year,
                    kind: kind.to_string(),
                    message,
                    event: Some(ev.clone()),
                });
            }
        };

        match ev {
            SimEvent::SalvageLoaded {
                fleet_id,
                debris_before,
                loaded,
                debris_after,
                cargo_cap,
            } => {
                if !debris_before.is_finite()
                    || !loaded.is_finite()
                    || !debris_after.is_finite()
                    || !cargo_cap.is_finite()
                    || *loaded < -1e-4
                    || *debris_after < -1e-4
                {
                    push_anom(
                        "SalvageInvalidValues",
                        format!("Non-finite or negative values on fleet #{fleet_id}"),
                    );
                }
                if *loaded > *cargo_cap + 1e-3 {
                    push_anom(
                        "SalvageCargoCapExceeded",
                        format!(
                            "Fleet #{fleet_id} loaded {loaded:.3} kT exceeding cargo_cap {cargo_cap:.3} kT"
                        ),
                    );
                }
                if (*debris_before - *loaded - *debris_after).abs() > 1e-2 {
                    push_anom(
                        "SalvageMassConservationViolation",
                        format!(
                            "Fleet #{fleet_id} debris_before ({debris_before:.3}) - loaded ({loaded:.3}) != debris_after ({debris_after:.3})"
                        ),
                    );
                }
                let entry = self
                    .pending_salvage_loads
                    .entry(*fleet_id)
                    .or_insert((0.0, *cargo_cap));
                entry.0 += *loaded;
                entry.1 = *cargo_cap;
                if entry.0 > *cargo_cap + 1e-3 {
                    let acc = entry.0;
                    push_anom(
                        "SalvageCumulativeCargoCapExceeded",
                        format!(
                            "Fleet #{fleet_id} cumulative salvage {acc:.3} kT exceeds cargo_cap {cargo_cap:.3} kT"
                        ),
                    );
                }
            }
            SimEvent::CargoUnloaded {
                fleet_id,
                planet_idx,
                unloaded,
                planet_surf_before,
                planet_surf_after,
            } => {
                self.closed_transactions_count += 1;
                if (*planet_surf_after - *planet_surf_before - *unloaded).abs() > 1e-2 {
                    push_anom(
                        "CargoUnloadMassMismatch",
                        format!(
                            "Fleet #{fleet_id} at planet #{planet_idx}: surf_after ({planet_surf_after:.3}) - surf_before ({planet_surf_before:.3}) != unloaded ({unloaded:.3})"
                        ),
                    );
                }
                if let Some((expected_loaded, cap)) = self.pending_salvage_loads.remove(fleet_id) {
                    if (*unloaded - expected_loaded).abs() > 1e-2 {
                        push_anom(
                            "SalvageUnloadMismatch",
                            format!(
                                "Fleet #{fleet_id} unloaded {unloaded:.3} kT at planet #{planet_idx} but loaded {expected_loaded:.3} kT from debris"
                            ),
                        );
                    }
                    if *unloaded > cap + 1e-3 {
                        push_anom(
                            "SalvageUnloadExceedsCap",
                            format!(
                                "Fleet #{fleet_id} unloaded {unloaded:.3} kT > cargo_cap {cap:.3} kT"
                            ),
                        );
                    }
                }
            }
            SimEvent::EconomyTick {
                planet_idx,
                res_gen,
                spent_tech,
                spent_mines,
                spent_fac,
                spent_def,
                spent_ship,
                spent_alchemy,
            } => {
                self.closed_transactions_count += 1;
                let total_spent = spent_tech
                    + spent_mines
                    + spent_fac
                    + spent_def
                    + spent_ship
                    + spent_alchemy;
                if !res_gen.is_finite()
                    || !total_spent.is_finite()
                    || *res_gen < -1e-4
                    || *spent_tech < -1e-4
                    || *spent_mines < -1e-4
                    || *spent_fac < -1e-4
                    || *spent_def < -1e-4
                    || *spent_ship < -1e-4
                    || *spent_alchemy < -1e-4
                {
                    push_anom(
                        "EconomyInvalidSpend",
                        format!("Negative or NaN expenditure on planet #{planet_idx}"),
                    );
                }
                if total_spent > *res_gen + 1e-3 {
                    push_anom(
                        "EconomyBudgetOverSpend",
                        format!(
                            "Planet #{planet_idx} spent {total_spent:.4} > res_gen {res_gen:.4}"
                        ),
                    );
                }
            }
            SimEvent::StarbaseAssault {
                planet_idx,
                sb_hp_before,
                sb_hp_after,
                attacker_survived,
            } => {
                self.closed_transactions_count += 1;
                if *sb_hp_after > *sb_hp_before + 1e-3 || *sb_hp_after < -1e-4 {
                    push_anom(
                        "StarbaseInvalidHpDelta",
                        format!(
                            "Planet #{planet_idx} starbase HP went from {sb_hp_before:.2} to {sb_hp_after:.2}"
                        ),
                    );
                }
                if *sb_hp_after > 0.0 && *attacker_survived {
                    push_anom(
                        "StarbaseBypassedWhileAlive",
                        format!(
                            "Attacker survived at planet #{planet_idx} while Starbase still had {sb_hp_after:.2} HP"
                        ),
                    );
                }
            }
            SimEvent::PlanetAssault {
                planet_idx,
                def_before,
                def_after,
                ..
            } => {
                self.closed_transactions_count += 1;
                if *def_after > *def_before + 1e-3 || *def_after < -1e-4 {
                    push_anom(
                        "PlanetAssaultInvalidDefDelta",
                        format!(
                            "Planet #{planet_idx} defense went from {def_before:.2} to {def_after:.2}"
                        ),
                    );
                }
            }
            SimEvent::SpaceBattle { .. } | SimEvent::VictoryTriggered { .. } => {
                self.closed_transactions_count += 1;
            }
        }
        valid
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmpireStats {
    pub planets: u32,
    pub factories: f32,
    pub res: f32,
    pub gates: u32,
    #[serde(default)]
    pub fleet_upkeep: f32,
    #[serde(default)]
    pub fleet_count: u32,
    #[serde(default)]
    pub orbiting_count: u32,
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
    #[serde(default)]
    pub orbiting_fleets: u16,
    #[serde(default)]
    pub starbase_tier: u8,
    #[serde(default)]
    pub starbase_name: String,
    #[serde(default)]
    pub starbase_hp: f32,
    #[serde(default)]
    pub starbase_max_hp: f32,
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
    pub events: Vec<HudEvent>,
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
    pub events: Vec<HudEvent>,
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

// ---------------------------------------------------------------------------
// Compressed Fog-of-War Intelligence Observation for External AI / LLM Agents
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEmpireSummary {
    pub empire_id: u8,
    pub name: String,
    pub prt: String,
    pub year: f32,
    pub tick: u64,
    pub res_per_year: f32,
    pub surf_iron: f32,
    pub surf_bora: f32,
    pub surf_germ: f32,
    pub upkeep_ratio: f32,
    pub planet_count: u32,
    pub fleet_count: u32,
    pub orbiting_fleet_count: u32,
    pub tech_levels: [f32; NUM_TECH_FIELDS],
    pub policy: EmpirePolicy,
    pub winner_emp: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectorSummary {
    pub sector_id: u16,
    pub row: u8,
    pub col: u8,
    pub own_planets: u16,
    pub own_res_per_year: f32,
    pub own_factories: f32,
    pub unexplored_stars: u16,
    pub neutral_explored_planets: u16,
    pub foreign_planets_by_emp: Vec<(u8, u16)>,
    pub avg_intel_age_years: f32,
    pub detected_enemy_fleet_power: f32,
    pub directive: SectorMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnPlanetBrief {
    pub planet_idx: usize,
    pub sector_id: u16,
    pub x: f32,
    pub y: f32,
    pub pop: f32,
    pub factories: f32,
    pub defense: f32,
    pub starbase_tier: u8,
    pub has_gate: bool,
    pub ship_prod_buf: f32,
    pub min_surf: [f32; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownStargateBrief {
    pub planet_idx: usize,
    pub owner: u8,
    pub sector_id: u16,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredWormholeBrief {
    pub wormhole_id: usize,
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedEnemyFleet {
    pub fleet_id: u32,
    pub emp: u8,
    pub fleet_type: FleetClass,
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub power: f32,
    pub ship_count: u16,
    pub orbiting: bool,
    pub sector_id: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleDebrisBrief {
    pub debris_idx: usize,
    pub x: f32,
    pub y: f32,
    pub iron: f32,
    pub bora: f32,
    pub germ: f32,
    pub ttl: f32,
    pub sector_id: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignPlanetIntelBrief {
    pub planet_idx: usize,
    pub sector_id: u16,
    pub x: f32,
    pub y: f32,
    pub owner: u8,
    pub starbase_tier: u8,
    pub has_stargate: bool,
    pub last_seen_tick: u32,
    pub pen_scanned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pop_est: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub def_est: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fac_est: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hab_est: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crust_sum_est: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentKeyPoints {
    pub top_own_planets: Vec<OwnPlanetBrief>,
    pub known_foreign_planets: Vec<ForeignPlanetIntelBrief>,
    pub known_stargates: Vec<KnownStargateBrief>,
    pub discovered_wormholes: Vec<DiscoveredWormholeBrief>,
    pub detected_enemy_fleets: Vec<DetectedEnemyFleet>,
    pub visible_debris: Vec<VisibleDebrisBrief>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownEmpireSummary {
    pub emp_id: u8,
    pub name: String,
    pub stance: DiplomaticStance,
    pub known_planets_count: u32,
    pub estimated_factories: f32,
    pub estimated_pop: f32,
    pub detected_fleet_power: f32,
    pub inbox: Vec<DipMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentObservation {
    pub empire: AgentEmpireSummary,
    pub sectors: Vec<SectorSummary>,
    pub key_points: AgentKeyPoints,
    pub known_empires: Vec<KnownEmpireSummary>,
    pub recent_alerts: Vec<String>,
}
