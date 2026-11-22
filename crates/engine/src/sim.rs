use crate::config::{
    points_for_tech_level, tech_level_from_points, FleetClass, SimConfig, BREAKTHROUGHS,
    BP_ALCHEMY2, DT, MAX_DEBRIS_FIELDS, MAX_EMPIRES, MAX_EVENT_LOG, MAX_POWER_HISTORY,
    NUM_TECH_FIELDS, SHIP_COST_SCALE,
};
use crate::galaxy::{generate_galaxy, GalaxyShape, GalaxySize};
use crate::governor::{
    run_empire_governor, sector_for_xy, EmpireView, GovernorAction, OwnPlanetState, VisibleWormhole,
};
use crate::system_gen::{generate_star_system, PrimaryWorldInput};
use crate::tech::{
    classic_races, starting_tech_for_traits, EmpireTechCache, HullSpec, LRT_ARM, LRT_BET, LRT_CE,
    LRT_GR, LRT_IFE, LRT_ISB, LRT_LSP, LRT_MA, LRT_OBRM, LRT_RS, LRT_TT, LRT_UR,
};
use crate::types::{
    round1, round2, AgentDebrisBrief, AgentEmpireSummary, AgentEnemyFleetBrief, AgentObservation,
    AgentPolicyView, AgentStargateBrief, AgentWormholeBrief, CombatFx, CommandFeedback,
    DebrisField, DetectedEnemyFleet, DipMessage, DiplomaticStance, EmpireStats, EmpireSummaryView,
    EnemyPlanetBrief, ExplorationOrder, Fleet, GalaxyOverview, HubKind, HudEvent, HudSnapshot,
    JumpFx, KnownEmpireSummary, LiveAuditor, NamedMinerals, NamedTechLevels, OwnFleetsByClass,
    OwnFleetsSummary, OwnHubBrief, OwnSectorFleetBrief, PendingHubOrderBrief, PlanetIntel,
    PlanetView, RaceDef, ReportsSnapshot, SandboxPlanet, SectorMode, SectorSummary, SimAnomaly,
    SimEvent, SimSnapshot, StarSystemView, StepProfile, StrategicCommand, VictoryConfig,
    VictoryGoalView, VisibleDebrisBrief, Wormhole, WreckEntry, INTEL_FLAG_EXPLORED,
    INTEL_FLAG_HAS_STARGATE, INTEL_FLAG_PEN_SCANNED,
};

#[cfg(not(target_arch = "wasm32"))]
#[inline]
fn perf_now_ms() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = performance, js_name = now)]
    fn wasm_perf_now_ms() -> f64;
}

#[cfg(target_arch = "wasm32")]
#[inline]
fn perf_now_ms() -> f64 {
    wasm_perf_now_ms()
}

const MAX_PLANET_NEIGHBORS: usize = 96;
const AI_DECISION_STRIDE: usize = 6;
pub const MAX_TRACE_EVENTS: usize = 2_048;
pub const PLANET_RENDER_STRIDE: usize = 7;
pub const FLEET_RENDER_STRIDE: usize = 7;
pub const DEBRIS_RENDER_STRIDE: usize = 3;

const MAX_GRID_CELLS: usize = 4_096;

/// Compact 16-byte spatially-sorted grid entry for cache-linear neighborhood queries
/// without random memory indirections into the full `Fleet`/`DebrisField` vectors.
#[derive(Debug, Clone, Copy, Default)]
pub struct GridEntry {
    pub x: f32,
    pub y: f32,
    pub idx: u32,
    pub tag: u8,   // emp_u8 for fleets, 0 for debris/planets
    pub flags: u8, // bit 0: is_combat, bit 1: is_prey_class (Bomber/Freight/Col)
}

/// Zero-allocation flat CSR spatial grid (`cell_offsets` + `sorted_entries`) reused across ticks.
#[derive(Debug, Clone, Default)]
pub struct UniformGrid {
    cols: usize,
    rows: usize,
    inv_cell: f32,
    cell_offsets: Vec<u32>,
    sorted_entries: Vec<GridEntry>,
    write_cursor: Vec<u32>,
    entity_cells: Vec<u32>,
    staged_entries: Vec<GridEntry>,
}

impl UniformGrid {
    pub fn build<F>(
        &mut self,
        world_w: f32,
        world_h: f32,
        cell_size: f32,
        entity_count: usize,
        entry_fn: F,
    ) where
        F: Fn(usize) -> GridEntry,
    {
        if entity_count == 0 {
            self.cols = 0;
            self.rows = 0;
            self.cell_offsets.clear();
            self.sorted_entries.clear();
            return;
        }
        // Adapt grid cell budget to entity_count so sparse worlds stay inside L1 cache (<16 KB)
        let cell_budget = (entity_count * 2).clamp(64, MAX_GRID_CELLS);
        let min_cell_for_cap = ((world_w * world_h) / cell_budget as f32).sqrt();
        let cell = cell_size.max(min_cell_for_cap).max(16.0);
        self.inv_cell = 1.0 / cell;
        self.cols = ((world_w * self.inv_cell).ceil() as usize).max(1);
        self.rows = ((world_h * self.inv_cell).ceil() as usize).max(1);
        let total_cells = self.cols * self.rows;

        self.cell_offsets.clear();
        self.cell_offsets.resize(total_cells + 1, 0);
        self.entity_cells.clear();
        self.entity_cells.reserve(entity_count);
        self.staged_entries.clear();
        self.staged_entries.reserve(entity_count);

        let cols = self.cols;
        let max_cx = (cols - 1) as isize;
        let max_cy = (self.rows - 1) as isize;
        let inv_cell = self.inv_cell;

        // Pass 1: stage compact entries once and count entities per cell
        for idx in 0..entity_count {
            let e = entry_fn(idx);
            let cx = ((e.x * inv_cell) as isize).clamp(0, max_cx) as usize;
            let cy = ((e.y * inv_cell) as isize).clamp(0, max_cy) as usize;
            let cell = cy * cols + cx;
            self.staged_entries.push(e);
            self.entity_cells.push(cell as u32);
            self.cell_offsets[cell + 1] += 1;
        }

        // Pass 2: prefix sum -> cell_offsets[c]..cell_offsets[c+1]
        let mut running = 0u32;
        for c in 0..total_cells {
            running += self.cell_offsets[c + 1];
            self.cell_offsets[c + 1] = running;
        }

        // Pass 3: scatter staged entries into contiguous cell buckets
        self.write_cursor.clear();
        self.write_cursor
            .extend_from_slice(&self.cell_offsets[..total_cells]);
        self.sorted_entries
            .resize(entity_count, GridEntry::default());
        for (&cell_u32, &entry) in self.entity_cells.iter().zip(self.staged_entries.iter()) {
            let cell = cell_u32 as usize;
            let dst = self.write_cursor[cell] as usize;
            self.sorted_entries[dst] = entry;
            self.write_cursor[cell] = (dst + 1) as u32;
        }
    }

    #[inline]
    pub fn for_each_in_radius<F: FnMut(GridEntry)>(&self, x: f32, y: f32, radius: f32, mut f: F) {
        if self.cols == 0 || self.rows == 0 {
            return;
        }
        let min_x = (((x - radius) * self.inv_cell).floor() as isize)
            .clamp(0, (self.cols - 1) as isize) as usize;
        let max_x = (((x + radius) * self.inv_cell).floor() as isize)
            .clamp(0, (self.cols - 1) as isize) as usize;
        let min_y = (((y - radius) * self.inv_cell).floor() as isize)
            .clamp(0, (self.rows - 1) as isize) as usize;
        let max_y = (((y + radius) * self.inv_cell).floor() as isize)
            .clamp(0, (self.rows - 1) as isize) as usize;

        for cy in min_y..=max_y {
            let row_off = cy * self.cols;
            let start = self.cell_offsets[row_off + min_x] as usize;
            let end = self.cell_offsets[row_off + max_x + 1] as usize;
            for &entry in &self.sorted_entries[start..end] {
                f(entry);
            }
        }
    }
}

pub fn starbase_tier_name(tier: u8) -> &'static str {
    match tier {
        1 => "Orbital Fort",
        2 => "Space Dock",
        3 => "Space Station",
        4 => "Ultra Station",
        5 => "Death Star",
        _ => "None",
    }
}

#[inline]
fn starbase_base_hp(tier: u8) -> f32 {
    match tier {
        1 => 100.0,
        2 => 250.0,
        3 => 500.0,
        4 => 1000.0,
        5 => 1500.0,
        _ => 0.0,
    }
}

#[inline]
pub fn starbase_dock_mult(tier: u8) -> f32 {
    match tier {
        0 => 0.75,
        1 => 1.00,
        2 => 1.25,
        3 => 1.45,
        4 => 1.65,
        _ => 1.90,
    }
}

#[inline]
pub fn starbase_repair_rate(tier: u8) -> f32 {
    match tier {
        0 => 0.03,
        1 => 0.10,
        2 => 0.14,
        3 => 0.18,
        4 => 0.22,
        _ => 0.28,
    }
}

#[inline]
fn starbase_build_cost(tier: u8) -> (f32, f32, f32, f32) {
    // (iron, bora, germ, res_buf)
    match tier {
        1 => (18.0, 5.0, 22.0, 45.0),
        2 => (28.0, 10.0, 32.0, 90.0),
        3 => (65.0, 40.0, 110.0, 200.0),
        4 => (90.0, 60.0, 160.0, 320.0),
        5 => (120.0, 80.0, 210.0, 450.0),
        _ => (0.0, 0.0, 0.0, 0.0),
    }
}

#[inline]
fn starbase_upkeep(tier: u8) -> f32 {
    match tier {
        1 => 0.8,
        2 => 1.5,
        3 => 2.5,
        4 => 4.0,
        5 => 6.0,
        _ => 0.0,
    }
}

/// Per-empire precomputed tick constants to avoid string comparisons and redundant math in inner loops.
#[derive(Debug, Clone, Copy, Default)]
struct EmpireTickCache {
    terra_bonus: f32,
    lv5_pop_mult: f32,
    mining_mult: f32,
    fac_efficiency: f32,
    sprawl: f32,
    research_mult: f32,
    alch_mult: f32,
    def_cap: f32,
    def_surplus_rate: f32,
    has_stargate_tech: bool,
    max_gates: usize,
    min_fac_for_gate: f32,
    gate_iron: f32,
    gate_germ: f32,
    gate_res: f32,
    min_hub_sq: f32,
    effective_ship_cost: f32,
    bio_growth_mult: f32,
    mine_share: f32,
    fac_share: f32,
    ship_share: f32,
    tech_share: f32,
    col_pop_req: f32,
    tech_alloc: [f32; NUM_TECH_FIELDS],
    scan_r: f32,
    scan_pen: f32,
    gate_range_sq: f32,
    cargo_cap: f32,
    freight_range: f32,
    col_range: f32,
    col_i: f32,
    col_g: f32,
    base_step_dist: f32,
    salvage_mult: f32,
    cruiser_scan_max: f32,
    tachyon_red: f32,
    intercept_bias: f32,
    prt_ss: bool,
    prt_is: bool,
}

#[derive(Debug, Clone)]
pub struct EmpireState {
    pub race: RaceDef,
    pub tech_pts: [f32; NUM_TECH_FIELDS],
    pub tech_lv: [f32; NUM_TECH_FIELDS],
    pub tech_int_lv: [i32; NUM_TECH_FIELDS],
    pub unlocked_bp: u8,
    pub sprawl_mult: f32,
    pub res_total: f32,
    pub last_stats: EmpireStats,
    pub tech_cache: EmpireTechCache,
    pub sector_directives: Vec<SectorMode>,
    pub pending_hub_orders: Vec<(usize, HubKind)>,
    pub active_exploration_orders: Vec<ExplorationOrder>,
    pub inbox: Vec<DipMessage>,
    pub recent_alerts: Vec<String>,
    pub contact_mask: u16,
}

impl EmpireState {
    pub fn from_race(race: RaceDef, ignore_traits: bool) -> Self {
        let start_lv = starting_tech_for_traits(&race.prt, race.lrt_mask);
        let mut tech_pts = [0.0f32; NUM_TECH_FIELDS];
        let mut tech_lv = [0.0f32; NUM_TECH_FIELDS];
        for f in 0..NUM_TECH_FIELDS {
            tech_pts[f] = points_for_tech_level(start_lv[f]);
            tech_lv[f] = start_lv[f] as f32;
        }
        let tech_cache =
            EmpireTechCache::resolve(&start_lv, &race.prt, race.lrt_mask, ignore_traits);
        let unlocked_bp = tech_cache.perk_mask;
        Self {
            race,
            tech_pts,
            tech_lv,
            tech_int_lv: start_lv,
            unlocked_bp,
            sprawl_mult: 1.0,
            res_total: 25.0,
            last_stats: EmpireStats {
                planets: 1,
                factories: 16.0,
                res: 25.0,
                gates: 0,
                ..EmpireStats::default()
            },
            tech_cache,
            sector_directives: vec![SectorMode::Balanced; 36],
            pending_hub_orders: Vec::new(),
            active_exploration_orders: Vec::new(),
            inbox: Vec::new(),
            recent_alerts: Vec::new(),
            contact_mask: 0,
        }
    }

    pub fn push_alert(&mut self, text: String) {
        self.recent_alerts.push(text);
        if self.recent_alerts.len() > 10 {
            self.recent_alerts.remove(0);
        }
    }

    pub fn hull_for_class(&self, class: FleetClass) -> &HullSpec {
        match class {
            FleetClass::Col => &self.tech_cache.hull_col,
            FleetClass::Freight => &self.tech_cache.hull_freight,
            FleetClass::Cruiser => &self.tech_cache.hull_cruiser,
            FleetClass::Battleship => &self.tech_cache.hull_battleship,
            FleetClass::Bomber => &self.tech_cache.hull_bomber,
        }
    }

    pub fn ship_range(&self, class: FleetClass) -> f32 {
        match class {
            FleetClass::Col => self.tech_cache.range_col,
            FleetClass::Freight => self.tech_cache.range_freight,
            FleetClass::Cruiser => self.tech_cache.range_cruiser,
            FleetClass::Battleship => self.tech_cache.range_battleship,
            FleetClass::Bomber => self.tech_cache.range_bomber,
        }
    }

    /// Normalized mineral costs (iron, bora, germ) preserving exact craig-stars hull ratios.
    pub fn ship_mineral_cost(&self, class: FleetClass) -> (f32, f32, f32) {
        let h = self.hull_for_class(class);
        let scale = if class == FleetClass::Col {
            SHIP_COST_SCALE * 0.85
        } else {
            SHIP_COST_SCALE
        };
        (
            (h.iron * scale).max(4.0),
            (h.bora * scale).max(2.0),
            (h.germ * scale).max(4.0),
        )
    }
}

#[derive(Debug, Clone)]
pub struct Simulation {
    pub cfg: SimConfig,
    pub empires: Vec<EmpireState>,
    pub dna_generation: u32,

    pub year: f32,
    pub ticks: u64,
    pub current_map_seed: u32,
    pub rng_seed: u32,
    pub world_w: f32,
    pub world_h: f32,

    // SoA planet buffers
    pub pos_x: Vec<f32>,
    pub pos_y: Vec<f32>,
    pub pop: Vec<f32>,
    pub hab: Vec<f32>,
    pub mines: Vec<f32>,
    pub factories: Vec<f32>,
    pub min_surf: Vec<f32>,
    pub min_crust: Vec<f32>,
    pub ship_prod_buf: Vec<f32>,
    pub defense: Vec<f32>,
    pub last_res: Vec<f32>,
    pub act_fac: Vec<f32>,
    pub owner: Vec<u8>,
    pub has_gate: Vec<u8>,
    pub starbase_tier: Vec<u8>, // 0 = none, 1 = Orbital Fort .. 5 = Death Star
    pub starbase_hp: Vec<f32>,
    pub starbase_max_hp: Vec<f32>,
    pub claim_mask: Vec<u16>,
    pub explored_mask: Vec<u16>,
    /// Per-empire intelligence table of size `num_empires * num_planets` (`e * n + p`).
    pub intel_planets: Vec<PlanetIntel>,
    /// Per-wormhole bitmask (`1 << e`) of empires that have discovered endpoint A (x1, y1).
    pub discovered_wh_a: Vec<u16>,
    /// Per-wormhole bitmask (`1 << e`) of empires that have discovered endpoint B (x2, y2).
    pub discovered_wh_b: Vec<u16>,

    // Precomputed flat CSR sorted nearest-neighbor lists per planet & wormhole endpoints
    neighbor_offsets: Vec<u32>,
    neighbor_data: Vec<(u32, f32)>,
    wh_near_a: Vec<Vec<(u32, f32)>>,
    wh_near_b: Vec<Vec<(u32, f32)>>,

    // Reusable spatial grids & per-tick scratch buffers (zero per-tick allocations)
    planet_grid: UniformGrid,
    fleet_grid: UniformGrid,
    debris_grid: UniformGrid,
    wh_grid: UniformGrid,
    ecm_grid: UniformGrid,
    max_active_ecm_range: f32,
    active_ecm_jammers: Vec<(f32, f32, f32, f32, u8)>,
    ecm_cache_valid: bool,
    scan_scratch: Vec<(usize, f32, f32, f32)>,
    gate_lists: Vec<Vec<usize>>,
    p_counts: Vec<u32>,
    pub tick_stats: Vec<EmpireStats>,
    combat_candidates: Vec<usize>,

    // Flat packed render buffers for zero-copy WASM -> JS canvas drawing
    pub planet_render_buf: Vec<f32>,
    pub fleet_render_buf: Vec<f32>,
    pub debris_render_buf: Vec<f32>,

    // Dynamic world entities
    pub fleets: Vec<Fleet>,
    pub wormholes: Vec<Wormhole>,
    pub debris_fields: Vec<DebrisField>,
    pub pending_combat_fx: Vec<CombatFx>,
    pub pending_jump_fx: Vec<JumpFx>,
    pub power_history: Vec<Vec<f32>>,
    pub log_events: Vec<HudEvent>,

    // Diagnostic Sandbox, Event Trace & Live Auditor
    pub ai_enabled: bool,
    pub trace_enabled: bool,
    pub events: Vec<SimEvent>,
    pub anomalies: Vec<SimAnomaly>,
    pub auditor: LiveAuditor,

    // Diplomacy & Victory Conditions
    pub relations: [[DiplomaticStance; MAX_EMPIRES]; MAX_EMPIRES],
    pub victory_cfg: VictoryConfig,
    pub winner: Option<usize>,

    // Telemetry
    pub space_battles_count: u32,
    pub invasions_count: u32,
    pub total_salvaged: u32,
    pub gate_jumps_count: u32,
    next_fleet_id: u32,

    // Reusable zero-alloc orbital garrison lookup buffers (rebuilt once per tick)
    orbit_warship: Vec<i32>,
    orbit_freight: Vec<i32>,
    orbit_count: Vec<u16>,

    // Optional per-phase micro-profiler
    pub profile_enabled: bool,
    pub last_profile: StepProfile,
}

impl Default for Simulation {
    fn default() -> Self {
        let mut sim = Self::new(SimConfig::default(), classic_races());
        sim.init(12345, false);
        sim
    }
}

impl Simulation {
    pub fn new(cfg: SimConfig, races: Vec<RaceDef>) -> Self {
        let valid_races = if races.len() < 2 {
            classic_races()
        } else {
            races.into_iter().take(MAX_EMPIRES).collect()
        };
        let num_emp = valid_races.len();
        let empires = valid_races
            .into_iter()
            .map(|r| EmpireState::from_race(r, cfg.ignore_racial_traits))
            .collect();

        let (_, world_w, world_h) = cfg.galaxy_size.specs();
        Self {
            cfg,
            empires,
            dna_generation: 0,
            year: 2400.0,
            ticks: 0,
            current_map_seed: 1,
            rng_seed: 12345,
            world_w,
            world_h,
            pos_x: Vec::new(),
            pos_y: Vec::new(),
            pop: Vec::new(),
            hab: Vec::new(),
            mines: Vec::new(),
            factories: Vec::new(),
            min_surf: Vec::new(),
            min_crust: Vec::new(),
            ship_prod_buf: Vec::new(),
            defense: Vec::new(),
            last_res: Vec::new(),
            act_fac: Vec::new(),
            owner: Vec::new(),
            has_gate: Vec::new(),
            starbase_tier: Vec::new(),
            starbase_hp: Vec::new(),
            starbase_max_hp: Vec::new(),
            claim_mask: Vec::new(),
            explored_mask: Vec::new(),
            intel_planets: Vec::new(),
            discovered_wh_a: Vec::new(),
            discovered_wh_b: Vec::new(),
            neighbor_offsets: Vec::new(),
            neighbor_data: Vec::new(),
            wh_near_a: Vec::new(),
            wh_near_b: Vec::new(),
            planet_grid: UniformGrid::default(),
            fleet_grid: UniformGrid::default(),
            debris_grid: UniformGrid::default(),
            wh_grid: UniformGrid::default(),
            ecm_grid: UniformGrid::default(),
            max_active_ecm_range: 0.0,
            active_ecm_jammers: Vec::new(),
            ecm_cache_valid: false,
            scan_scratch: Vec::with_capacity(128),
            gate_lists: vec![Vec::new(); num_emp],
            p_counts: vec![0; num_emp],
            tick_stats: vec![EmpireStats::default(); num_emp],
            combat_candidates: Vec::with_capacity(64),
            planet_render_buf: Vec::new(),
            fleet_render_buf: Vec::new(),
            debris_render_buf: Vec::new(),
            fleets: Vec::new(),
            wormholes: Vec::new(),
            debris_fields: Vec::new(),
            pending_combat_fx: Vec::new(),
            pending_jump_fx: Vec::new(),
            power_history: Vec::new(),
            log_events: Vec::new(),
            ai_enabled: true,
            trace_enabled: false,
            events: Vec::new(),
            anomalies: Vec::new(),
            auditor: LiveAuditor::default(),
            relations: Self::default_relations(),
            victory_cfg: VictoryConfig::default(),
            winner: None,
            space_battles_count: 0,
            invasions_count: 0,
            total_salvaged: 0,
            gate_jumps_count: 0,
            next_fleet_id: 1,
            orbit_warship: Vec::new(),
            orbit_freight: Vec::new(),
            orbit_count: Vec::new(),
            profile_enabled: false,
            last_profile: StepProfile::default(),
        }
    }

    pub fn set_races(&mut self, races: Vec<RaceDef>, seed: u32) -> bool {
        if races.len() < 2 || races.len() > MAX_EMPIRES {
            return false;
        }
        let num_emp = races.len();
        self.empires = races
            .into_iter()
            .map(|r| EmpireState::from_race(r, self.cfg.ignore_racial_traits))
            .collect();
        self.gate_lists = vec![Vec::new(); num_emp];
        self.p_counts = vec![0; num_emp];
        self.tick_stats = vec![EmpireStats::default(); num_emp];
        self.dna_generation = 0;
        self.init(seed, false);
        true
    }

    pub fn refresh_all_tech_caches(&mut self) {
        let ignore = self.cfg.ignore_racial_traits;
        for emp in &mut self.empires {
            emp.tech_cache = EmpireTechCache::resolve(
                &emp.tech_int_lv,
                &emp.race.prt,
                emp.race.lrt_mask,
                ignore,
            );
        }
    }

    #[inline]
    pub fn next_rand(&mut self) -> f32 {
        self.rng_seed = self
            .rng_seed
            .wrapping_mul(1664525)
            .wrapping_add(1013904223);
        (self.rng_seed as f64 / 4294967296.0) as f32
    }

    pub fn push_event(&mut self, text: String, color: &str) {
        self.log_events.push(HudEvent {
            year: self.year,
            text,
            color: color.to_string(),
        });
        while self.log_events.len() > MAX_EVENT_LOG {
            self.log_events.remove(0);
        }
    }

    pub fn default_relations() -> [[DiplomaticStance; MAX_EMPIRES]; MAX_EMPIRES] {
        let mut rel = [[DiplomaticStance::War; MAX_EMPIRES]; MAX_EMPIRES];
        for i in 0..MAX_EMPIRES {
            rel[i][i] = DiplomaticStance::Alliance;
        }
        rel
    }

    pub fn set_relation(&mut self, emp_a: usize, emp_b: usize, stance: DiplomaticStance) {
        if emp_a < MAX_EMPIRES && emp_b < MAX_EMPIRES {
            self.relations[emp_a][emp_b] = stance;
            self.relations[emp_b][emp_a] = stance;
        }
    }

    #[inline]
    pub fn get_relation(&self, emp_a: usize, emp_b: usize) -> DiplomaticStance {
        if emp_a < MAX_EMPIRES && emp_b < MAX_EMPIRES {
            self.relations[emp_a][emp_b]
        } else {
            DiplomaticStance::War
        }
    }

    #[inline]
    pub fn are_hostile(&self, emp_a_u8: u8, emp_b_u8: u8) -> bool {
        if emp_a_u8 == 0 || emp_b_u8 == 0 || emp_a_u8 == emp_b_u8 {
            return false;
        }
        let a = (emp_a_u8 - 1) as usize;
        let b = (emp_b_u8 - 1) as usize;
        if a >= MAX_EMPIRES || b >= MAX_EMPIRES {
            return false;
        }
        self.relations[a][b] == DiplomaticStance::War
    }

    /// Records a structured `SimEvent` when `trace_enabled` is true, immediately runs `LiveAuditor`,
    /// and maintains `self.events` as a bounded ring buffer (`MAX_TRACE_EVENTS`) so RAM never grows unboundedly.
    #[inline]
    pub fn record_trace_event(&mut self, ev: SimEvent) {
        if !self.trace_enabled {
            return;
        }
        let valid = self
            .auditor
            .observe_event(self.ticks, self.year, &ev, &mut self.anomalies);
        if self.auditor.prune_valid_events && valid {
            return;
        }
        if self.events.len() >= MAX_TRACE_EVENTS {
            let drop_n = self.events.len() + 1 - MAX_TRACE_EVENTS;
            self.events.drain(0..drop_n);
        }
        self.events.push(ev);
    }

    /// Drains all buffered trace events from `self.events`, freeing memory on the fly.
    pub fn drain_events(&mut self) -> Vec<SimEvent> {
        std::mem::take(&mut self.events)
    }

    /// Serializes any anomalies captured by `LiveAuditor` along with audit counters to JSON.
    pub fn export_anomalies_json(&self) -> String {
        let payload = serde_json::json!({
            "year": self.year,
            "ticks": self.ticks,
            "auditedEventsCount": self.auditor.audited_events_count,
            "closedTransactionsCount": self.auditor.closed_transactions_count,
            "openSalvageTransactions": self.auditor.pending_salvage_loads.len(),
            "anomaliesCount": self.anomalies.len(),
            "anomalies": self.anomalies,
        });
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
    }

    /// Creates a deterministic, minimal sandbox universe with custom planets, fleets, and debris fields.
    /// Enables `trace_enabled` by default so tests and diagnostic runners immediately capture `SimEvent`s and `LiveAuditor` checks.
    pub fn new_sandbox(
        mut cfg: SimConfig,
        races: Vec<RaceDef>,
        planets: Vec<SandboxPlanet>,
        mut fleets: Vec<Fleet>,
        debris_fields: Vec<DebrisField>,
        ai_enabled: bool,
    ) -> Self {
        let valid_races = if races.is_empty() {
            classic_races().into_iter().take(2).collect()
        } else {
            races.into_iter().take(MAX_EMPIRES).collect()
        };
        cfg.planets = planets.len();
        let mut sim = Self::new(cfg, valid_races);
        let n = planets.len();
        let num_emp = sim.empires.len();

        sim.ai_enabled = ai_enabled;
        sim.trace_enabled = true;
        sim.events.clear();
        sim.anomalies.clear();
        sim.auditor = LiveAuditor::default();
        sim.relations = Self::default_relations();
        sim.winner = None;

        sim.pos_x.resize(n, 0.0);
        sim.pos_y.resize(n, 0.0);
        sim.pop.resize(n, 0.0);
        sim.hab.resize(n, 1.0);
        sim.mines.resize(n, 0.0);
        sim.factories.resize(n, 0.0);
        sim.min_surf.resize(n * 3, 0.0);
        sim.min_crust.resize(n * 3, 0.0);
        sim.ship_prod_buf.resize(n, 0.0);
        sim.defense.resize(n, 0.0);
        sim.last_res.resize(n, 0.0);
        sim.act_fac.resize(n, 0.0);
        sim.owner.resize(n, 0);
        sim.has_gate.resize(n, 0);
        sim.starbase_tier.resize(n, 0);
        sim.starbase_hp.resize(n, 0.0);
        sim.starbase_max_hp.resize(n, 0.0);
        sim.claim_mask.resize(n, 0);
        sim.explored_mask.resize(n, 0);
        sim.intel_planets
            .resize(num_emp * n, PlanetIntel::default());

        for (i, p) in planets.iter().enumerate() {
            sim.pos_x[i] = p.x;
            sim.pos_y[i] = p.y;
            sim.owner[i] = p.owner;
            sim.hab[i] = p.hab.clamp(0.05, 1.15);
            sim.pop[i] = p.pop.max(0.0);
            sim.mines[i] = p.mines.max(0.0);
            sim.factories[i] = p.factories.max(0.0);
            sim.defense[i] = p.defense.max(0.0);
            sim.min_surf[i * 3] = p.iron_surf.max(0.0);
            sim.min_surf[i * 3 + 1] = p.bora_surf.max(0.0);
            sim.min_surf[i * 3 + 2] = p.germ_surf.max(0.0);
            sim.min_crust[i * 3] = p.iron_crust.max(0.0);
            sim.min_crust[i * 3 + 1] = p.bora_crust.max(0.0);
            sim.min_crust[i * 3 + 2] = p.germ_crust.max(0.0);
            sim.ship_prod_buf[i] = p.ship_prod_buf.max(0.0);
            sim.has_gate[i] = if p.has_gate { 1 } else { 0 };
            sim.starbase_tier[i] = p.starbase_tier;
            let sb_hp = p
                .starbase_hp
                .unwrap_or_else(|| starbase_base_hp(p.starbase_tier));
            sim.starbase_hp[i] = sb_hp;
            sim.starbase_max_hp[i] = sb_hp.max(starbase_base_hp(p.starbase_tier));
        }

        if n > 0 {
            sim.build_planet_neighbors();
        }

        let mut max_id = 0u32;
        for (idx, fl) in fleets.iter_mut().enumerate() {
            if fl.id == 0 {
                fl.id = (idx as u32) + 1;
            }
            max_id = max_id.max(fl.id);
            if fl.ship_count == 0 {
                fl.ship_count = 1;
            }
            if fl.upkeep <= 0.0 {
                fl.upkeep = fl.fleet_type.upkeep() * (fl.ship_count as f32);
            }
            if fl.fleet_type == FleetClass::Freight {
                if fl.max_power <= 0.0 {
                    let e = ((fl.emp.max(1) - 1) as usize).min(num_emp.saturating_sub(1));
                    fl.max_power = (sim.empires[e].tech_cache.hull_freight.cargo * 0.75)
                        .clamp(45.0, 220.0);
                }
                if fl.power <= 0.0 {
                    fl.power = 12.0;
                }
            } else {
                if fl.power <= 0.0 {
                    fl.power = fl.max_power.max(40.0);
                }
                if fl.max_power < fl.power {
                    fl.max_power = fl.power;
                }
            }
            if fl.orbiting && fl.target < n {
                fl.x = sim.pos_x[fl.target];
                fl.y = sim.pos_y[fl.target];
                fl.steer_x = fl.x;
                fl.steer_y = fl.y;
            } else if fl.steer_x == 0.0 && fl.steer_y == 0.0 && fl.target < n {
                fl.steer_x = sim.pos_x[fl.target];
                fl.steer_y = sim.pos_y[fl.target];
            }
        }
        sim.next_fleet_id = max_id + 1;
        sim.fleets = fleets;
        sim.debris_fields = debris_fields;
        sim.orbit_warship.resize(n, -1);
        sim.orbit_freight.resize(n, -1);
        sim.orbit_count.resize(n, 0);

        for e in 0..num_emp {
            let emp_u8 = (e + 1) as u8;
            let mut p_cnt = 0u32;
            let mut f_sum = 0.0f32;
            for i in 0..n {
                if sim.owner[i] == emp_u8 {
                    p_cnt += 1;
                    f_sum += sim.factories[i];
                }
            }
            sim.empires[e].last_stats.planets = p_cnt;
            sim.empires[e].last_stats.factories = f_sum;
            sim.empires[e].res_total = (f_sum + 25.0).max(25.0);
            sim.empires[e].last_stats.res = sim.empires[e].res_total;
        }

        sim.discovered_wh_a.resize(sim.wormholes.len(), 0);
        sim.discovered_wh_b.resize(sim.wormholes.len(), 0);
        sim.refresh_initial_fow();

        sim
    }

    pub fn set_galaxy_options(&mut self, size: &str, shape: &str) -> bool {
        let mut changed = false;
        if let Some(sz) = GalaxySize::from_str_code(size) {
            self.cfg.galaxy_size = sz;
            let (planets, _, _) = sz.specs();
            self.cfg.planets = planets;
            changed = true;
        }
        if let Some(sh) = GalaxyShape::from_str_code(shape) {
            self.cfg.galaxy_shape = sh;
            changed = true;
        }
        changed
    }

    #[inline]
    fn planet_neighbors_slice(&self, i: usize) -> &[(u32, f32)] {
        let start = self.neighbor_offsets[i] as usize;
        let end = self.neighbor_offsets[i + 1] as usize;
        &self.neighbor_data[start..end]
    }

    pub fn build_planet_neighbors(&mut self) {
        let n = self.pos_x.len();
        self.neighbor_offsets.clear();
        self.neighbor_offsets.reserve(n + 1);
        self.neighbor_data.clear();

        // Adaptive neighbor cap + O(N) UniformGrid lookup so 100,000 planets initialize in ~60 ms!
        let max_k = if n >= 5_000 { 28 } else { MAX_PLANET_NEIGHBORS };
        self.neighbor_data.reserve(n * max_k.min(n));
        let search_rad = if n >= 20_000 {
            320.0
        } else if n >= 5_000 {
            420.0
        } else {
            950.0
        };

        {
            let xs = &self.pos_x;
            let ys = &self.pos_y;
            self.planet_grid
                .build(self.world_w, self.world_h, 110.0, n, |i| GridEntry {
                    x: xs[i],
                    y: ys[i],
                    idx: i as u32,
                    tag: 0,
                    flags: 0,
                });
        }

        let mut grid = UniformGrid::default();
        if n > 1200 {
            let xs = &self.pos_x;
            let ys = &self.pos_y;
            grid.build(self.world_w, self.world_h, search_rad * 0.5, n, |i| {
                GridEntry {
                    x: xs[i],
                    y: ys[i],
                    idx: i as u32,
                    tag: 0,
                    flags: 0,
                }
            });
        }

        let mut scratch: Vec<(u32, f32)> = Vec::with_capacity(128);
        self.neighbor_offsets.push(0);
        for i in 0..n {
            scratch.clear();
            let xi = self.pos_x[i];
            let yi = self.pos_y[i];
            let xs = &self.pos_x;
            let ys = &self.pos_y;

            if n <= 1200 {
                for j in 0..n {
                    if i == j {
                        continue;
                    }
                    let dx = xs[j] - xi;
                    let dy = ys[j] - yi;
                    scratch.push((j as u32, dx * dx + dy * dy));
                }
            } else {
                grid.for_each_in_radius(xi, yi, search_rad, |entry| {
                    let j = entry.idx as usize;
                    if i == j {
                        return;
                    }
                    let dx = entry.x - xi;
                    let dy = entry.y - yi;
                    scratch.push((entry.idx, dx * dx + dy * dy));
                });
            }

            let keep = max_k.min(scratch.len());
            if keep < scratch.len() {
                scratch.select_nth_unstable_by(keep, |a, b| a.1.total_cmp(&b.1));
                scratch.truncate(keep);
            }
            scratch.sort_unstable_by(|a, b| a.1.total_cmp(&b.1));
            for &(j, d_sq) in &scratch {
                self.neighbor_data.push((j, d_sq.sqrt()));
            }
            self.neighbor_offsets.push(self.neighbor_data.len() as u32);
        }

        // Precompute closest 20 planets to each wormhole endpoint (grid-accelerated for 100-500 wormholes)
        self.wh_near_a.clear();
        self.wh_near_b.clear();
        let wh_k = 20usize.min(n);
        if n > 1200 {
            let wh_search_rad = search_rad;
            for wh in &self.wormholes {
                for (ex, ey, dest) in [
                    (wh.x1, wh.y1, &mut self.wh_near_a),
                    (wh.x2, wh.y2, &mut self.wh_near_b),
                ] {
                    scratch.clear();
                    grid.for_each_in_radius(ex, ey, wh_search_rad, |entry| {
                        let dx = entry.x - ex;
                        let dy = entry.y - ey;
                        scratch.push((entry.idx, dx * dx + dy * dy));
                    });
                    if scratch.len() < wh_k {
                        scratch.clear();
                        for j in 0..n {
                            let dx = self.pos_x[j] - ex;
                            let dy = self.pos_y[j] - ey;
                            scratch.push((j as u32, dx * dx + dy * dy));
                        }
                    }
                    if wh_k < scratch.len() {
                        scratch.select_nth_unstable_by(wh_k, |a, b| a.1.total_cmp(&b.1));
                        scratch.truncate(wh_k);
                    }
                    scratch.sort_unstable_by(|a, b| a.1.total_cmp(&b.1));
                    dest.push(scratch.iter().map(|&(j, d_sq)| (j, d_sq.sqrt())).collect());
                }
            }
        } else {
            for wh in &self.wormholes {
                for (ex, ey, dest) in [
                    (wh.x1, wh.y1, &mut self.wh_near_a),
                    (wh.x2, wh.y2, &mut self.wh_near_b),
                ] {
                    scratch.clear();
                    for j in 0..n {
                        let dx = self.pos_x[j] - ex;
                        let dy = self.pos_y[j] - ey;
                        scratch.push((j as u32, dx * dx + dy * dy));
                    }
                    if wh_k < scratch.len() {
                        scratch.select_nth_unstable_by(wh_k, |a, b| a.1.total_cmp(&b.1));
                        scratch.truncate(wh_k);
                    }
                    scratch.sort_unstable_by(|a, b| a.1.total_cmp(&b.1));
                    dest.push(scratch.iter().map(|&(j, d_sq)| (j, d_sq.sqrt())).collect());
                }
            }
        }

        // Build static CSR UniformGrid of all wormhole endpoints for O(1) AI relay & fleet jump lookups
        let wh_endpoints = self.wormholes.len() * 2;
        if wh_endpoints > 0 {
            let wormholes = &self.wormholes;
            self.wh_grid.build(self.world_w, self.world_h, 96.0, wh_endpoints, |idx| {
                let w = idx / 2;
                let dir = idx % 2;
                let wh = wormholes[w];
                let (x, y) = if dir == 0 {
                    (wh.x1, wh.y1)
                } else {
                    (wh.x2, wh.y2)
                };
                GridEntry {
                    x,
                    y,
                    idx: idx as u32,
                    tag: 0,
                    flags: 0,
                }
            });
        } else {
            self.wh_grid.build(self.world_w, self.world_h, 96.0, 0, |_| GridEntry::default());
        }
    }

    pub fn init(&mut self, seed: u32, silent: bool) {
        self.current_map_seed = seed;
        self.rng_seed = seed;
        let num_emp = self.empires.len();

        let generated = generate_galaxy(
            self.cfg.galaxy_size,
            self.cfg.galaxy_shape,
            num_emp,
            self.cfg.crust_richness,
            self.cfg.wormhole_pairs,
            &mut self.rng_seed,
        );
        let n = generated.pos_x.len();
        self.cfg.planets = n;
        self.world_w = generated.world_w;
        self.world_h = generated.world_h;
        self.pos_x = generated.pos_x;
        self.pos_y = generated.pos_y;
        self.hab = generated.hab;
        self.min_crust = generated.min_crust;
        self.wormholes = generated.wormholes;

        // Scenario modifier: Core Richness Bias (rich galactic core, sparse outer rim)
        let core_bias = self.cfg.scenario_modifiers.core_richness_bias.clamp(0.0, 1.5);
        if core_bias > 0.001 {
            let cx = self.world_w * 0.5;
            let cy = self.world_h * 0.5;
            let inv_rx = 1.0 / (cx.max(1.0));
            let inv_ry = 1.0 / (cy.max(1.0));
            for i in 0..n {
                let nx = (self.pos_x[i] - cx) * inv_rx;
                let ny = (self.pos_y[i] - cy) * inv_ry;
                let r_norm = (nx * nx + ny * ny).sqrt().clamp(0.0, 1.2);
                let center_factor = (1.0 - r_norm).clamp(0.0, 1.0);
                let crust_mult = (1.0 + core_bias * (center_factor * 1.45 - 0.65)).clamp(0.22, 2.5);
                let base = i * 3;
                self.min_crust[base] *= crust_mult;
                self.min_crust[base + 1] *= crust_mult;
                self.min_crust[base + 2] *= crust_mult;
                let hab_shift = core_bias * (center_factor * 0.35 - 0.18);
                self.hab[i] = (self.hab[i] + hab_shift).clamp(0.18, 1.0);
            }
        }

        self.pop.clear();
        self.pop.resize(n, 0.0);
        self.mines.clear();
        self.mines.resize(n, 0.0);
        self.factories.clear();
        self.factories.resize(n, 0.0);
        self.min_surf.clear();
        self.min_surf.resize(n * 3, 0.0);
        self.ship_prod_buf.clear();
        self.ship_prod_buf.resize(n, 0.0);
        self.defense.clear();
        self.defense.resize(n, 0.0);
        self.last_res.clear();
        self.last_res.resize(n, 0.0);
        self.act_fac.clear();
        self.act_fac.resize(n, 0.0);
        self.owner.clear();
        self.owner.resize(n, 0);
        self.has_gate.clear();
        self.has_gate.resize(n, 0);
        self.starbase_tier.clear();
        self.starbase_tier.resize(n, 0);
        self.starbase_hp.clear();
        self.starbase_hp.resize(n, 0.0);
        self.starbase_max_hp.clear();
        self.starbase_max_hp.resize(n, 0.0);
        self.claim_mask.clear();
        self.claim_mask.resize(n, 0);
        self.explored_mask.clear();
        self.explored_mask.resize(n, 0);
        self.intel_planets.clear();
        self.intel_planets
            .resize(num_emp * n, PlanetIntel::default());
        self.discovered_wh_a.clear();
        self.discovered_wh_a.resize(self.wormholes.len(), 0);
        self.discovered_wh_b.clear();
        self.discovered_wh_b.resize(self.wormholes.len(), 0);

        self.build_planet_neighbors();

        let ignore = self.cfg.ignore_racial_traits;
        for e in 0..num_emp {
            self.hab[e] = 1.0;

            let lrt_mask = self.empires[e].race.lrt_mask;
            let prt = self.empires[e].race.prt.clone();
            let start_pop = if lrt_mask & LRT_LSP != 0 { 30.0 } else { 42.0 };
            self.pop[e] = start_pop;
            self.mines[e] = 18.0;
            self.factories[e] = 16.0;
            for m in 0..3 {
                self.min_surf[e * 3 + m] = 160.0;
                self.min_crust[e * 3 + m] = self.cfg.crust_richness * 1.6;
            }
            self.defense[e] = if prt == "PRT_IS" { 55.0 } else { 40.0 };
            self.owner[e] = (e + 1) as u8;
            // Homeworlds start with a Space Station (tier 3), matching Stars!
            self.starbase_tier[e] = 3;
            let sb_hp = starbase_base_hp(3);
            self.starbase_hp[e] = sb_hp;
            self.starbase_max_hp[e] = sb_hp;

            let emp = &mut self.empires[e];
            emp.race.doctrine.kills = 0;
            emp.race.doctrine.gates = 0;
            let start_lv = starting_tech_for_traits(&prt, lrt_mask);
            for f in 0..NUM_TECH_FIELDS {
                emp.tech_pts[f] = points_for_tech_level(start_lv[f]);
                emp.tech_lv[f] = start_lv[f] as f32;
                emp.tech_int_lv[f] = start_lv[f];
            }
            emp.tech_cache = EmpireTechCache::resolve(&start_lv, &prt, lrt_mask, ignore);
            emp.unlocked_bp = emp.tech_cache.perk_mask;
            emp.sprawl_mult = 1.0;
            emp.res_total = 25.0;
            emp.sector_directives.fill(SectorMode::Balanced);
            emp.pending_hub_orders.clear();
            emp.active_exploration_orders.clear();
            emp.inbox.clear();
            emp.recent_alerts.clear();
            emp.contact_mask = 0;
            emp.last_stats = EmpireStats {
                planets: 1,
                factories: 16.0,
                res: 25.0,
                gates: 0,
                ..EmpireStats::default()
            };
        }

        // Scenario modifier: Fallen Empire (1 deterministic empire starts with 11-12 planets, Starbases, +4 tech)
        if self.cfg.scenario_modifiers.fallen_empire && num_emp > 0 {
            let fe = (seed as usize) % num_emp;
            let prt = self.empires[fe].race.prt.clone();
            let lrt_mask = self.empires[fe].race.lrt_mask;
            let mut boosted_lv = self.empires[fe].tech_int_lv;
            for f in 0..NUM_TECH_FIELDS {
                boosted_lv[f] = (boosted_lv[f] + 4).min(crate::config::MAX_TECH_LEVEL as i32);
                self.empires[fe].tech_int_lv[f] = boosted_lv[f];
                self.empires[fe].tech_lv[f] = boosted_lv[f] as f32;
                self.empires[fe].tech_pts[f] = points_for_tech_level(boosted_lv[f]);
            }
            self.empires[fe].tech_cache =
                EmpireTechCache::resolve(&boosted_lv, &prt, lrt_mask, ignore);
            self.empires[fe].unlocked_bp = self.empires[fe].tech_cache.perk_mask;
            self.defense[fe] = self.defense[fe].max(72.0);
            self.pop[fe] = 95.0;
            self.factories[fe] = 52.0;
            self.mines[fe] = 44.0;

            let fe_owner = (fe + 1) as u8;
            let extra_target = 10usize + ((seed as usize) % 2); // 10 or 11 extras -> 11 or 12 total
            let nb_start = self.neighbor_offsets[fe] as usize;
            let nb_end = self.neighbor_offsets[fe + 1] as usize;
            let mut claimed = Vec::with_capacity(extra_target);
            for &(j_u32, _) in &self.neighbor_data[nb_start..nb_end] {
                if claimed.len() >= extra_target {
                    break;
                }
                let j = j_u32 as usize;
                if self.owner[j] == 0 {
                    claimed.push(j);
                }
            }
            for &j in &claimed {
                self.owner[j] = fe_owner;
                self.hab[j] = self.hab[j].max(0.65);
                self.pop[j] = 68.0;
                self.mines[j] = 32.0;
                self.factories[j] = 36.0;
                self.defense[j] = 65.0; // Starbase-class planetary defense
                self.starbase_tier[j] = 3;
                let fe_sb_hp = starbase_base_hp(3) * 1.25;
                self.starbase_hp[j] = fe_sb_hp;
                self.starbase_max_hp[j] = fe_sb_hp;
                let b = j * 3;
                self.min_surf[b] = 140.0;
                self.min_surf[b + 1] = 140.0;
                self.min_surf[b + 2] = 140.0;
            }
            let fe_planets = (1 + claimed.len()) as u32;
            let fe_fac = self.factories[fe] + claimed.iter().map(|&j| self.factories[j]).sum::<f32>();
            self.empires[fe].last_stats.planets = fe_planets;
            self.empires[fe].last_stats.factories = fe_fac;
            self.empires[fe].last_stats.res = fe_fac * 1.15;
            self.empires[fe].res_total = fe_fac * 1.15;
        }

        self.fleets.clear();
        self.orbit_warship.clear();
        self.orbit_warship.resize(n, -1);
        self.orbit_freight.clear();
        self.orbit_freight.resize(n, -1);
        self.orbit_count.clear();
        self.orbit_count.resize(n, 0);
        self.debris_fields.clear();
        self.pending_combat_fx.clear();
        self.pending_jump_fx.clear();
        self.power_history.clear();
        self.year = 2400.0;
        self.ticks = 0;
        self.space_battles_count = 0;
        self.invasions_count = 0;
        self.total_salvaged = 0;
        self.gate_jumps_count = 0;
        self.next_fleet_id = 1;
        self.winner = None;
        self.relations = Self::default_relations();
        self.events.clear();
        self.anomalies.clear();
        self.auditor = LiveAuditor::default();

        // Initial planetary scanner reveal around each empire's owned worlds
        self.refresh_initial_fow();

        if !silent {
            self.log_events.clear();
            self.push_event(
                format!(
                    "New {:?} {:?} galaxy #{} ({} systems, {:.0}×{:.0}ly, {} empires).",
                    self.cfg.galaxy_size,
                    self.cfg.galaxy_shape,
                    seed % 1000,
                    n,
                    self.world_w,
                    self.world_h,
                    num_emp
                ),
                "#38bdf8",
            );
        }
    }

    pub fn step_tick(&mut self, silent: bool) -> &[EmpireStats] {
        let prof = self.profile_enabled;
        let t_step_start = if prof { perf_now_ms() } else { 0.0 };
        let mut prof_ai_ms = 0.0f64;
        let mut prof_planet_scan_ms = 0.0f64;
        let mut prof_fleet_scan_ms = 0.0f64;

        let n = self.owner.len();
        let num_emp = self.empires.len();
        if self.tick_stats.len() != num_emp {
            self.tick_stats.resize(num_emp, EmpireStats::default());
        }
        self.tick_stats.fill(EmpireStats::default());

        self.p_counts.fill(0);
        for gl in &mut self.gate_lists {
            gl.clear();
        }
        let mut neutral_planets_count = 0usize;
        for i in 0..n {
            let emp = self.owner[i];
            if emp == 0 {
                neutral_planets_count += 1;
            } else {
                let e = (emp - 1) as usize;
                if e < num_emp {
                    self.p_counts[e] += 1;
                    if self.has_gate[i] != 0 {
                        self.gate_lists[e].push(i);
                        self.tick_stats[e].gates += 1;
                    }
                }
            }
        }

        let mods = self.cfg.scenario_modifiers;
        let scan_mult = mods.scanner_range_mult.clamp(0.2, 3.0);
        let ew_mult = mods.ew_strength_mult.clamp(0.2, 3.0);
        let tech_mult = mods.tech_speed_mult.clamp(0.2, 5.0);
        let growth_mult = mods.growth_rate_mult.clamp(0.2, 5.0);

        let mut total_galactic_res = 1.0f32;
        let mut leader_e = 0usize;
        let mut max_ecm_r = 0.0f32;
        let mut emp_cache = [EmpireTickCache::default(); MAX_EMPIRES];
        for e in 0..num_emp {
            if self.empires[e].tech_cache.ecm_jammer_strength > 0.0 {
                max_ecm_r = max_ecm_r.max(self.empires[e].tech_cache.ecm_jammer_range);
            }
            let excess = self.p_counts[e].saturating_sub(10) as f32;
            let ele_discount = 1.0 / (1.0 + self.empires[e].tech_lv[4] * 0.05);
            let prt = self.empires[e].race.prt.as_str();
            let prt_it = prt == "PRT_IT";
            let prt_he = prt == "PRT_HE";
            let prt_ca = prt == "PRT_CA";
            let prt_is = prt == "PRT_IS";
            let prt_sd = prt == "PRT_SD";
            let prt_pp = prt == "PRT_PP";
            let prt_wm = prt == "PRT_WM";
            let prt_ar = prt == "PRT_AR";
            let prt_joat = prt == "PRT_JOAT";
            let prt_ss = prt == "PRT_SS";

            let prt_sprawl = if prt_it { 0.72 } else { 1.0 };
            let sprawl_factor = self.cfg.sprawl_penalty * prt_sprawl * ele_discount;
            let sprawl = 1.0 / (1.0 + excess * (sprawl_factor / 18.0));
            self.empires[e].sprawl_mult = sprawl;
            self.empires[e].race.doctrine.gates = self.tick_stats[e].gates;

            total_galactic_res += self.empires[e].res_total;
            if self.empires[e].res_total > self.empires[leader_e].res_total {
                leader_e = e;
            }

            let emp = &self.empires[e];
            let lrt = emp.race.lrt_mask;
            let lv = emp.tech_lv;
            let tc = &emp.tech_cache;
            let doc = &emp.race.doctrine;

            let terra_bonus = (tc.terraform_ability / 100.0)
                * if lrt & LRT_TT != 0 { 1.30 } else { 1.0 }
                * if prt_ca { 1.25 } else { 1.0 };
            let prt_pop_mod = if prt_he {
                0.72
            } else if prt_joat {
                1.18
            } else {
                1.0
            } * if lrt & LRT_OBRM != 0 { 1.08 } else { 1.0 };
            let mining_mult = (1.0 + (tc.mining_rate - 2.0).max(0.0) * 0.025)
                * if lrt & LRT_ARM != 0 { 1.18 } else { 1.0 }
                * if prt_ar { 1.15 } else { 1.0 };
            let fac_efficiency = if prt_is {
                1.12
            } else if prt_ar {
                1.15
            } else {
                1.0
            };
            let research_mult = (1.0
                + if lrt & LRT_BET != 0 { 0.15 } else { 0.0 }
                + if lrt & LRT_GR != 0 { 0.12 } else { 0.0 })
                * tech_mult;
            let prt_alch = if prt_pp { 1.35 } else { 1.0 };
            let lrt_alch = if lrt & LRT_MA != 0 { 1.8 } else { 1.0 };
            let perk_alch = if tc.perk_mask & BP_ALCHEMY2 != 0 {
                1.6
            } else {
                1.0
            };
            let alch_base = 22.0 / (1.0 + lv[0] * 0.05);
            let prt_def_mult = if prt_is || prt_sd { 1.28 } else { 1.0 }
                * if lrt & LRT_ISB != 0 { 1.15 } else { 1.0 };
            let gate_cap_cost = if prt_it {
                self.cfg.gate_cost
            } else {
                self.cfg.gate_cost * 1.4
            };
            let min_hub_dist = if prt_it { 95.0 } else { 155.0 };
            let con_discount = 1.0 / (1.0 + lv[3] * 0.035);
            let trait_ship_discount = if prt_wm { 0.90 } else { 1.0 }
                * if lrt & LRT_CE != 0 { 0.88 } else { 1.0 };
            let gate_range = tc.stargate_max_range.max(250.0);
            let (col_i_raw, _, col_g_raw) = emp.ship_mineral_cost(FleetClass::Col);
            let col_mult = self.cfg.col_cost_mult.clamp(0.5, 5.0);
            let col_i = col_i_raw * col_mult;
            let col_g = col_g_raw * col_mult;
            let warp_mult = (tc.engine_warp / 6.0).clamp(0.75, 2.0);
            let ife_mult = if lrt & LRT_IFE != 0 { 1.10 } else { 1.0 };

            let raw_f = doc.fac_share.max(0.0);
            let raw_s = doc.ship_share.max(0.0);
            let raw_t = doc.tech_share.max(0.0);
            let sum_fst = raw_f + raw_s + raw_t;
            let (fac_share, ship_share, tech_share, mine_share) = if sum_fst > 0.95 {
                let sc = 0.95 / sum_fst;
                (raw_f * sc, raw_s * sc, raw_t * sc, 0.05)
            } else {
                (raw_f, raw_s, raw_t, 1.0 - sum_fst)
            };

            emp_cache[e] = EmpireTickCache {
                terra_bonus,
                lv5_pop_mult: 170.0 * (1.0 + lv[5] * 0.015) * prt_pop_mod,
                mining_mult: 0.55 * mining_mult * DT,
                fac_efficiency,
                sprawl,
                research_mult,
                alch_mult: if mods.allow_alchemy {
                    (0.45 * prt_alch * lrt_alch * perk_alch) / alch_base
                } else {
                    0.0
                },
                def_cap: 65.0 * (0.7 + tc.defense_coverage * 0.45) * prt_def_mult,
                def_surplus_rate: if prt_is { 0.22 } else { 0.12 },
                has_stargate_tech: mods.allow_stargates && tc.stargate_name.is_some(),
                max_gates: if prt_it {
                    (self.p_counts[e] as usize / 4).max(2)
                } else {
                    (self.p_counts[e] as usize / 8).max(2)
                },
                min_fac_for_gate: if prt_it { 40.0 } else { 58.0 },
                gate_iron: tc.stargate_cost_iron.min(gate_cap_cost),
                gate_germ: tc.stargate_cost_germ.min(gate_cap_cost),
                gate_res: if prt_it {
                    self.cfg.gate_cost * 1.2
                } else {
                    self.cfg.gate_cost * 1.8
                },
                min_hub_sq: min_hub_dist * min_hub_dist,
                effective_ship_cost: self.cfg.col_ship_cost * con_discount * trait_ship_discount,
                bio_growth_mult: self.cfg.growth
                    * growth_mult
                    * (1.0 + lv[5] * 0.025)
                    * if prt_he { 1.80 } else { 1.0 }
                    * DT,
                mine_share,
                fac_share,
                ship_share,
                tech_share,
                col_pop_req: doc.col_pop_req,
                tech_alloc: doc.tech_alloc,
                scan_r: (if prt_ss {
                    tc.scanner_range * 1.25
                } else {
                    tc.scanner_range
                }) * scan_mult,
                scan_pen: (if prt_ss {
                    tc.scanner_pen * 1.25
                } else {
                    tc.scanner_pen
                }) * scan_mult,
                gate_range_sq: gate_range * gate_range,
                cargo_cap: (tc.hull_freight.cargo * 0.75).clamp(45.0, 220.0),
                freight_range: emp.ship_range(FleetClass::Freight),
                col_range: emp.ship_range(FleetClass::Col),
                col_i,
                col_g,
                base_step_dist: self.cfg.fleet_speed * warp_mult * ife_mult * DT,
                salvage_mult: if lrt & LRT_UR != 0 { 1.30 } else { 1.0 },
                cruiser_scan_max: ((self.cfg.scanner_range * 0.65 + tc.scanner_range * 0.45)
                    * if prt_ss { 1.25 } else { 1.0 }
                    * scan_mult)
                .min(240.0),
                tachyon_red: tc.tachyon_cloak_reduction,
                intercept_bias: doc.intercept_bias,
                prt_ss,
                prt_is,
            };
        }
        self.max_active_ecm_range = max_ecm_r;
        self.active_ecm_jammers.clear();
        if max_ecm_r > 0.0 {
            for i in 0..n {
                let p_owner = self.owner[i];
                if p_owner != 0 && (self.has_gate[i] != 0 || self.defense[i] >= 55.0) {
                    let pe = (p_owner - 1) as usize;
                    let ptc = &self.empires[pe].tech_cache;
                    if ptc.ecm_jammer_strength > 0.0 && ptc.ecm_jammer_range > 0.0 {
                        self.active_ecm_jammers.push((
                            self.pos_x[i],
                            self.pos_y[i],
                            ptc.ecm_jammer_range,
                            ptc.ecm_jammer_strength,
                            p_owner,
                        ));
                    }
                }
            }
        }
        {
            let jammers = &self.active_ecm_jammers;
            let cell = max_ecm_r.max(90.0);
            self.ecm_grid
                .build(self.world_w, self.world_h, cell, jammers.len(), |idx| {
                    let &(jx, jy, _, _, owner_u8) = &jammers[idx];
                    GridEntry {
                        x: jx,
                        y: jy,
                        idx: idx as u32,
                        tag: owner_u8,
                        flags: 0,
                    }
                });
        }
        self.ecm_cache_valid = true;
        let _ = (neutral_planets_count, total_galactic_res, leader_e);

        let tick_usize = self.ticks as usize;
        let inv_fac_cost_res = 1.0 / self.cfg.fac_cost_res;
        let inv_fac_cost_germ = 1.0 / self.cfg.fac_cost_germ;
        let _ = 1.0 / (self.cfg.crust_richness * 3.0);

        // Build debris spatial grid once per tick for O(1) AI salvage dispatch, multi-stop scooping & auto-salvage
        let any_debris = !self.debris_fields.is_empty();
        if any_debris {
            let debris = &self.debris_fields;
            let deb_cell = (self.cfg.intercept_rad * 4.0).max(90.0);
            self.debris_grid
                .build(self.world_w, self.world_h, deb_cell, debris.len(), |idx| {
                    let deb = &debris[idx];
                    GridEntry {
                        x: deb.x,
                        y: deb.y,
                        idx: idx as u32,
                        tag: 0,
                        flags: 0,
                    }
                });
        }

        // Compute per-empire fleet upkeep, scrap surplus orbiting ships on severe deficit, and build O(1) orbital lookup
        let mut emp_upkeep = [0.0f32; MAX_EMPIRES];
        let mut emp_fleet_count = [0u32; MAX_EMPIRES];
        let mut emp_orbiting_count = [0u32; MAX_EMPIRES];
        for fl in &self.fleets {
            let e = (fl.emp - 1) as usize;
            if e < num_emp {
                emp_upkeep[e] += fl.upkeep;
                emp_fleet_count[e] += 1;
                if fl.orbiting {
                    emp_orbiting_count[e] += 1;
                }
            }
        }
        // Add orbital Starbase upkeep to each empire's maintenance budget
        for i in 0..n {
            let sb_tier = self.starbase_tier[i];
            if sb_tier > 0 && self.owner[i] > 0 {
                let e = (self.owner[i] - 1) as usize;
                if e < num_emp {
                    emp_upkeep[e] += starbase_upkeep(sb_tier);
                }
            }
        }
        let mut emp_upkeep_ratio = [0.0f32; MAX_EMPIRES];
        for e in 0..num_emp {
            emp_upkeep_ratio[e] = emp_upkeep[e] / self.empires[e].res_total.max(1.0);
        }

        // Over-budget automatic decommissioning: every 5 ticks, empires spending >55% of GDP on upkeep
        // scrap their most damaged or surplus orbiting ship at a friendly planet, recovering 50% of hull minerals.
        if self.ai_enabled && self.ticks % 5 == 0 {
            for e in 0..num_emp {
                if emp_upkeep_ratio[e] <= 0.55 || emp_orbiting_count[e] == 0 {
                    continue;
                }
                let emp_u8 = (e + 1) as u8;
                let mut scrap_idx: Option<usize> = None;
                let mut worst_score = f32::MAX;
                for (f_idx, fl) in self.fleets.iter().enumerate() {
                    if !fl.orbiting || fl.emp != emp_u8 {
                        continue;
                    }
                    let p = fl.target;
                    if p >= n || self.owner[p] != emp_u8 {
                        continue;
                    }
                    let hp_frac = if fl.max_power > 0.0 {
                        (fl.power / fl.max_power).clamp(0.0, 1.0)
                    } else {
                        0.5
                    };
                    // Prefer scrapping damaged warships or excess freighters first
                    let score = hp_frac + if fl.fleet_type == FleetClass::Freight { -0.15 } else { 0.0 };
                    if score < worst_score {
                        worst_score = score;
                        scrap_idx = Some(f_idx);
                    }
                }
                if let Some(s_idx) = scrap_idx {
                    let scrapped = self.fleets.swap_remove(s_idx);
                    let p = scrapped.target;
                    let (ci, cb, cg) = self.empires[e].ship_mineral_cost(scrapped.fleet_type);
                    let mult = (scrapped.ship_count.max(1) as f32) * 0.5;
                    let b = p * 3;
                    self.min_surf[b] += ci * mult;
                    self.min_surf[b + 1] += cb * mult;
                    self.min_surf[b + 2] += cg * mult;
                    emp_upkeep[e] = (emp_upkeep[e] - scrapped.upkeep).max(0.0);
                    emp_fleet_count[e] = emp_fleet_count[e].saturating_sub(1);
                    emp_orbiting_count[e] = emp_orbiting_count[e].saturating_sub(1);
                    emp_upkeep_ratio[e] = emp_upkeep[e] / self.empires[e].res_total.max(1.0);
                }
            }
        }

        for e in 0..num_emp {
            self.tick_stats[e].fleet_upkeep = emp_upkeep[e];
            self.tick_stats[e].fleet_count = emp_fleet_count[e];
            self.tick_stats[e].orbiting_count = emp_orbiting_count[e];
        }

        if self.orbit_warship.len() != n {
            self.orbit_warship.resize(n, -1);
            self.orbit_freight.resize(n, -1);
            self.orbit_count.resize(n, 0);
        }
        self.orbit_warship.fill(-1);
        self.orbit_freight.fill(-1);
        self.orbit_count.fill(0);
        for (f_idx, fl) in self.fleets.iter().enumerate() {
            if !fl.orbiting {
                continue;
            }
            let p = fl.target;
            if p >= n {
                continue;
            }
            self.orbit_count[p] = self.orbit_count[p].saturating_add(fl.ship_count.max(1));
            if fl.emp == self.owner[p] {
                if fl.fleet_type == FleetClass::Freight && self.orbit_freight[p] < 0 {
                    self.orbit_freight[p] = f_idx as i32;
                } else if fl.fleet_type.is_combat()
                    && fl.power >= fl.max_power * 0.70
                    && (self.orbit_warship[p] < 0
                        || fl.power > self.fleets[self.orbit_warship[p] as usize].power)
                {
                    self.orbit_warship[p] = f_idx as i32;
                }
            }
        }

        // 1. Planetary economy, production, and time-sliced neighbor-list AI decisions
        for i in 0..n {
            let emp_u8 = self.owner[i];
            if emp_u8 == 0 {
                continue;
            }
            let e = (emp_u8 - 1) as usize;
            if e >= num_emp {
                continue;
            }
            let base_idx = i * 3;
            let ec = &emp_cache[e];

            let eff_hab = (self.hab[i] + ec.terra_bonus).min(1.15);
            let max_pop = eff_hab * ec.lv5_pop_mult;
            let max_buildings = (max_pop * 0.1) * self.cfg.op_limit;
            let max_operable = (self.pop[i] * 0.1) * self.cfg.op_limit;

            let total_crust_here = self.min_crust[base_idx]
                + self.min_crust[base_idx + 1]
                + self.min_crust[base_idx + 2];
            let act_mines = if total_crust_here > 0.3 {
                self.mines[i].min(max_operable)
            } else {
                0.0
            };
            let act_fac = self.factories[i].min(max_operable);
            self.act_fac[i] = act_fac;

            let mine_cap = act_mines * ec.mining_mult;
            for m in 0..3 {
                let c = self.min_crust[base_idx + m];
                if c > 0.0 {
                    let extr = c.min(mine_cap);
                    self.min_crust[base_idx + m] = c - extr;
                    self.min_surf[base_idx + m] += extr;
                }
            }

            let res_rate = (self.pop[i] * 0.32 + act_fac * ec.fac_efficiency) * ec.sprawl;
            let net_res_mult = (1.0 - emp_upkeep_ratio[e]).clamp(0.15, 1.0);
            let res_gen = res_rate * net_res_mult * DT;
            self.last_res[i] = res_rate;

            self.tick_stats[e].planets += 1;
            self.tick_stats[e].factories += self.factories[i];
            self.tick_stats[e].res += res_rate;

            // 1a. Research allocation
            let spent_tech = res_gen * ec.tech_share;
            let research_gen = spent_tech * ec.research_mult;
            let tech_pts = &mut self.empires[e].tech_pts;
            for f in 0..NUM_TECH_FIELDS {
                tech_pts[f] += research_gen * ec.tech_alloc[f];
            }

            // 1b. Mine construction (5.0 Res per mine; unspent mine budget flows into surplus_res)
            let mine_budget = res_gen * ec.mine_share;
            let room_for_mines = if total_crust_here > 1.0 {
                (max_buildings - self.mines[i]).max(0.0)
            } else {
                0.0
            };
            let built_mines = room_for_mines.min(mine_budget * 0.2);
            self.mines[i] += built_mines;
            let spent_mines = (built_mines * 5.0).min(mine_budget);
            let mine_surplus = (mine_budget - spent_mines).max(0.0);

            // 1c. Factory construction (unspent factory budget flows into surplus_res)
            let room_for_fac = (max_buildings - self.factories[i]).max(0.0);
            let fac_budget = res_gen * ec.fac_share;
            let built_fac = room_for_fac
                .min(fac_budget * inv_fac_cost_res)
                .min(self.min_surf[base_idx + 2] * inv_fac_cost_germ);
            self.factories[i] += built_fac;
            self.min_surf[base_idx + 2] -= built_fac * self.cfg.fac_cost_germ;
            let spent_fac = (built_fac * self.cfg.fac_cost_res).min(fac_budget);

            let base_ship_budget = res_gen * ec.ship_share;
            let can_build_ships = self.pop[i] > max_pop * ec.col_pop_req;
            let mut surplus_res = (fac_budget - spent_fac).max(0.0)
                + mine_surplus
                + if can_build_ships { 0.0 } else { base_ship_budget };

            // 1d. Sequential surplus_res spending (no double/triple spending!):
            // Step 1: Planetary defense from surplus_res
            let mut spent_def = 0.0f32;
            if self.defense[i] < ec.def_cap && self.min_surf[base_idx + 1] > 2.0 && surplus_res > 0.0
            {
                let d_add = (ec.def_cap - self.defense[i])
                    .min(surplus_res * ec.def_surplus_rate)
                    .min(self.min_surf[base_idx + 1] * 0.5);
                self.defense[i] += d_add;
                self.min_surf[base_idx + 1] -= d_add * 0.15;
                spent_def = d_add;
                surplus_res = (surplus_res - spent_def).max(0.0);
            }

            // Step 2: Shipyard production buffer (base ship share + up to 50% of remaining surplus_res)
            let spent_ship = if can_build_ships {
                let ship_from_surplus = surplus_res * 0.5;
                surplus_res = (surplus_res - ship_from_surplus).max(0.0);
                let total_ship_res = base_ship_budget + ship_from_surplus;
                let dock_mult = starbase_dock_mult(self.starbase_tier[i]);
                self.ship_prod_buf[i] += total_ship_res * dock_mult;
                total_ship_res
            } else {
                0.0
            };

            // Step 3: Mineral Alchemy consumes remaining surplus_res
            let mut spent_alchemy = 0.0f32;
            if surplus_res > 0.0 && ec.alch_mult > 0.0 {
                spent_alchemy = surplus_res;
                let transmuted = spent_alchemy * ec.alch_mult;
                let mut min_m = 0usize;
                if self.min_surf[base_idx + 1] < self.min_surf[base_idx + min_m] {
                    min_m = 1;
                }
                if self.min_surf[base_idx + 2] < self.min_surf[base_idx + min_m] {
                    min_m = 2;
                }
                self.min_surf[base_idx + min_m] += transmuted;
            }

            if self.trace_enabled {
                self.record_trace_event(SimEvent::EconomyTick {
                    planet_idx: i,
                    res_gen,
                    spent_tech,
                    spent_mines,
                    spent_fac,
                    spent_def,
                    spent_ship,
                    spent_alchemy,
                });
            }

            // Starbase self-repair
            let cur_sb = self.starbase_tier[i];
            if cur_sb > 0 && self.starbase_hp[i] < self.starbase_max_hp[i] {
                self.starbase_hp[i] = (self.starbase_hp[i] + self.starbase_max_hp[i] * 0.08 * DT)
                    .min(self.starbase_max_hp[i]);
            }

            // Update own planet intel & run planetary scanner sweep (Fog of War discovery)
            self.update_own_planet_intel(e, i);
            let do_scan =
                n <= 32 || self.trace_enabled || (i + tick_usize) % AI_DECISION_STRIDE == 0;
            if do_scan {
                let t_scan = if prof { perf_now_ms() } else { 0.0 };
                self.scan_area_for_empire(
                    self.pos_x[i],
                    self.pos_y[i],
                    e,
                    ec.scan_r,
                    ec.scan_pen,
                    Some(i),
                );
                if prof {
                    prof_planet_scan_ms += perf_now_ms() - t_scan;
                }
            }

            let startup_factor = if self.pop[i] < 15.0 {
                0.4 + 0.6 * (self.pop[i] / 15.0)
            } else {
                1.0
            };
            let growth_delta = self.pop[i]
                * ec.bio_growth_mult
                * startup_factor
                * (1.0 - self.pop[i] / max_pop);
            self.pop[i] = (self.pop[i] + growth_delta).max(1.0).min(max_pop);
        }

        // 1e. Execute isolated Empire Governor (`governor.rs`) using strictly Fog-of-War-filtered `EmpireView`
        if self.ai_enabled {
            let t_ai = if prof { perf_now_ms() } else { 0.0 };
            self.run_all_governors(&emp_cache, &emp_upkeep_ratio, silent);
            if prof {
                prof_ai_ms += perf_now_ms() - t_ai;
            }
        }

        // 2. Update 6 tech levels (0..=26) & recompute EmpireTechCache ONLY on integer level-up
        let ignore_traits = self.cfg.ignore_racial_traits;
        for e in 0..num_emp {
            let mut leveled_up = false;
            for f in 0..NUM_TECH_FIELDS {
                let new_lv = tech_level_from_points(self.empires[e].tech_pts[f]);
                self.empires[e].tech_lv[f] = new_lv;
                let new_int = new_lv.floor() as i32;
                if new_int > self.empires[e].tech_int_lv[f] {
                    self.empires[e].tech_int_lv[f] = new_int;
                    leveled_up = true;
                }
            }
            let st = self.tick_stats[e];
            self.empires[e].res_total = st.res;
            self.empires[e].last_stats = st;
            self.empires[e].race.doctrine.last_fit = st.res.round() as i32;

            if leveled_up {
                let prev_bship = self.empires[e].tech_cache.hull_battleship.name.clone();
                let new_cache = EmpireTechCache::resolve(
                    &self.empires[e].tech_int_lv,
                    &self.empires[e].race.prt,
                    self.empires[e].race.lrt_mask,
                    ignore_traits,
                );
                let new_perks = new_cache.perk_mask;
                let new_bship = new_cache.hull_battleship.name.clone();
                self.empires[e].tech_cache = new_cache;

                if !silent {
                    for bp in &BREAKTHROUGHS {
                        if (new_perks & bp.bit != 0) && (self.empires[e].unlocked_bp & bp.bit == 0)
                        {
                            self.empires[e].unlocked_bp |= bp.bit;
                            let msg = format!(
                                "{} unlocked {} {}!",
                                self.empires[e].race.name, bp.icon, bp.name
                            );
                            let col = self.empires[e].race.color.clone();
                            self.push_event(msg, &col);
                        }
                    }
                    if new_bship != prev_bship {
                        let msg = format!(
                            "{} commissioned new capital hull: {}!",
                            self.empires[e].race.name, new_bship
                        );
                        let col = self.empires[e].race.color.clone();
                        self.push_event(msg, &col);
                    }
                }
            }
        }

        // 3. Populate flat CSR spatial grids for O(1) scanner pursuit & debris salvage
        let t_after_econ = if prof { perf_now_ms() } else { 0.0 };
        let hit_rad = self.cfg.intercept_rad;
        let hit_rad_sq = hit_rad * hit_rad;
        let salvage_rad = hit_rad * 1.4;
        let salvage_rad_sq = salvage_rad * salvage_rad;
        // Larger cell size for Cruiser scan (radius <= 180) keeps row scan to 4-5 rows
        let scan_grid_cell = (hit_rad * 4.0).max(90.0);

        {
            let fleets = &self.fleets;
            self.fleet_grid
                .build(self.world_w, self.world_h, scan_grid_cell, fleets.len(), |idx| {
                    let fl = &fleets[idx];
                    let flags = (if fl.fleet_type.is_combat() { 1 } else { 0 })
                        | (if matches!(
                            fl.fleet_type,
                            FleetClass::Bomber | FleetClass::Freight | FleetClass::Col
                        ) {
                            2
                        } else {
                            0
                        });
                    GridEntry {
                        x: fl.x,
                        y: fl.y,
                        idx: idx as u32,
                        tag: fl.emp,
                        flags,
                    }
                });
        }

        let has_auto_salvage = mods.auto_salvage && any_debris;

        // 4. Fleet movement, wormholes, grid-accelerated interception pursuit, salvage, arrival
        let mut f_idx = self.fleets.len();
        while f_idx > 0 {
            f_idx -= 1;
            if self.fleets[f_idx].power <= 0.0 {
                continue;
            }
            let emp_u8 = self.fleets[f_idx].emp;
            let e = (emp_u8 - 1) as usize;
            let ec = &emp_cache[e];

            // Orbiting garrison / docked fleet maintenance (co-located with host star system)
            if self.fleets[f_idx].orbiting {
                let p = self.fleets[f_idx].target;
                if p >= n || self.owner[p] != emp_u8 {
                    if p < n {
                        let ox = self.pos_x[p];
                        let oy = self.pos_y[p];
                        self.scan_area_for_empire(
                            ox,
                            oy,
                            e,
                            ec.scan_r * 0.65,
                            ec.scan_pen * 0.65,
                            Some(p),
                        );
                    }
                    self.fleets[f_idx].orbiting = false;
                } else {
                    let ox = self.pos_x[p];
                    let oy = self.pos_y[p];
                    self.fleets[f_idx].x = ox;
                    self.fleets[f_idx].y = oy;
                    self.fleets[f_idx].steer_x = ox;
                    self.fleets[f_idx].steer_y = oy;
                    // Docked combat ships repair faster with an orbital Starbase drydock (3%/yr -> up to 28%/yr)
                    if self.fleets[f_idx].fleet_type.is_combat()
                        && emp_upkeep_ratio[e] <= 0.55
                        && self.fleets[f_idx].power < self.fleets[f_idx].max_power
                    {
                        let r_rate = starbase_repair_rate(self.starbase_tier[p]);
                        let rep = self.fleets[f_idx].max_power * r_rate * DT;
                        self.fleets[f_idx].power =
                            (self.fleets[f_idx].power + rep).min(self.fleets[f_idx].max_power);
                    }
                    continue;
                }
            }

            // Passive shipboard scanner sweep along flight path
            if ((self.fleets[f_idx].id as usize) + tick_usize) % AI_DECISION_STRIDE == 0
                || n <= 32
                || self.trace_enabled
            {
                let fx = self.fleets[f_idx].x;
                let fy = self.fleets[f_idx].y;
                let t_p = self.fleets[f_idx].target;
                let orb_p = if t_p < n {
                    let odx = self.pos_x[t_p] - fx;
                    let ody = self.pos_y[t_p] - fy;
                    if odx * odx + ody * ody <= 16.0 {
                        Some(t_p)
                    } else {
                        None
                    }
                } else {
                    None
                };
                self.scan_area_for_empire(
                    fx,
                    fy,
                    e,
                    ec.scan_r * 0.65,
                    ec.scan_pen * 0.65,
                    orb_p,
                );
            }

            let salvage_mode = self.fleets[f_idx].salvage_mode;

            // Dedicated Salvage Freighter state machine (1 = outbound/multi-stop scoop, 2 = returning home)
            if salvage_mode == 1 {
                let fx = self.fleets[f_idx].x;
                let fy = self.fleets[f_idx].y;
                let sx = self.fleets[f_idx].steer_x;
                let sy = self.fleets[f_idx].steer_y;
                let step_dist = ec.base_step_dist * FleetClass::Freight.speed_mod();
                let dx = sx - fx;
                let dy = sy - fy;
                let dist_sq = dx * dx + dy * dy;
                let scoop_r = step_dist.max(10.0);

                if dist_sq <= scoop_r * scoop_r {
                    // Scoop up to remaining cargo capacity proportionally from debris fields at this site
                    let cap = if self.fleets[f_idx].max_power > 0.0 {
                        self.fleets[f_idx].max_power
                    } else {
                        ec.cargo_cap
                    };
                    let fleet_id = self.fleets[f_idx].id;
                    let mut carried = self.fleets[f_idx].cargo_iron
                        + self.fleets[f_idx].cargo_bora
                        + self.fleets[f_idx].cargo_germ;
                    let mut add_i = 0.0f32;
                    let mut add_b = 0.0f32;
                    let mut add_g = 0.0f32;
                    let mut trace_loads: Vec<(f32, f32, f32)> = Vec::new();
                    let trace_on = self.trace_enabled;

                    {
                        let debris_slice = &mut self.debris_fields;
                        self.debris_grid
                            .for_each_in_radius(sx, sy, 14.0, |entry| {
                                let rem = (cap - carried).max(0.0);
                                if rem <= 1e-4 {
                                    return;
                                }
                                let d_idx = entry.idx as usize;
                                if d_idx >= debris_slice.len() {
                                    return;
                                }
                                let deb = &mut debris_slice[d_idx];
                                if deb.ttl <= 0.0 {
                                    return;
                                }
                                let ddx = deb.x - sx;
                                let ddy = deb.y - sy;
                                if ddx * ddx + ddy * ddy > 196.0 {
                                    return;
                                }
                                let avail = deb.iron + deb.bora + deb.germ;
                                if avail <= 1e-4 {
                                    deb.iron = 0.0;
                                    deb.bora = 0.0;
                                    deb.germ = 0.0;
                                    deb.ttl = 0.0;
                                    return;
                                }
                                let take = avail.min(rem);
                                let frac = (take / avail).clamp(0.0, 1.0);
                                let ti = deb.iron * frac;
                                let tb = deb.bora * frac;
                                let tg = deb.germ * frac;
                                deb.iron = (deb.iron - ti).max(0.0);
                                deb.bora = (deb.bora - tb).max(0.0);
                                deb.germ = (deb.germ - tg).max(0.0);
                                let loaded_here = ti + tb + tg;
                                let mut rem_debris = deb.iron + deb.bora + deb.germ;
                                if rem_debris <= 1e-4 {
                                    deb.iron = 0.0;
                                    deb.bora = 0.0;
                                    deb.germ = 0.0;
                                    deb.ttl = 0.0;
                                    rem_debris = 0.0;
                                }
                                if trace_on {
                                    trace_loads.push((avail, loaded_here, rem_debris));
                                }
                                add_i += ti;
                                add_b += tb;
                                add_g += tg;
                                carried += loaded_here;
                            });
                    }

                    for (debris_before, loaded, debris_after) in trace_loads {
                        self.record_trace_event(SimEvent::SalvageLoaded {
                            fleet_id,
                            debris_before,
                            loaded,
                            debris_after,
                            cargo_cap: cap,
                        });
                    }

                    self.fleets[f_idx].cargo_iron += add_i;
                    self.fleets[f_idx].cargo_bora += add_b;
                    self.fleets[f_idx].cargo_germ += add_g;
                    let total_cargo = self.fleets[f_idx].cargo_iron
                        + self.fleets[f_idx].cargo_bora
                        + self.fleets[f_idx].cargo_germ;
                    self.fleets[f_idx].power = total_cargo.max(12.0);

                    // Multi-stop chaining: if hold still has room (>= 15 kT) and another wreck is within scanner range & fuel range
                    let rem_after = (cap - total_cargo).max(0.0);
                    let mut next_stop: Option<(f32, f32)> = None;
                    if rem_after >= 15.0 {
                        let home_idx = self.fleets[f_idx].from;
                        let hx = self.pos_x[home_idx];
                        let hy = self.pos_y[home_idx];
                        let max_home_sq = (ec.freight_range * 0.95) * (ec.freight_range * 0.95);
                        let local_scan_r = (ec.scan_r * 0.65).min(85.0);
                        let mut best_d_sq = local_scan_r * local_scan_r;
                        let debris_ref = &self.debris_fields;
                        self.debris_grid
                            .for_each_in_radius(sx, sy, local_scan_r, |entry| {
                                let d_idx = entry.idx as usize;
                                if d_idx >= debris_ref.len() {
                                    return;
                                }
                                let deb = &debris_ref[d_idx];
                                if deb.ttl <= 0.0 || deb.iron + deb.bora + deb.germ < 5.0 {
                                    return;
                                }
                                let hdx = entry.x - hx;
                                let hdy = entry.y - hy;
                                if hdx * hdx + hdy * hdy > max_home_sq {
                                    return;
                                }
                                let ndx = entry.x - sx;
                                let ndy = entry.y - sy;
                                let nd_sq = ndx * ndx + ndy * ndy;
                                if nd_sq > 9.0 && nd_sq < best_d_sq {
                                    best_d_sq = nd_sq;
                                    next_stop = Some((entry.x, entry.y));
                                }
                            });
                    }

                    if let Some((nx, ny)) = next_stop {
                        self.fleets[f_idx].steer_x = nx;
                        self.fleets[f_idx].steer_y = ny;
                    } else {
                        // Return home with salvaged minerals
                        let home_idx = self.fleets[f_idx].from;
                        self.fleets[f_idx].salvage_mode = 2;
                        self.fleets[f_idx].target = home_idx;
                        self.fleets[f_idx].steer_x = self.pos_x[home_idx];
                        self.fleets[f_idx].steer_y = self.pos_y[home_idx];
                    }
                } else if dist_sq > 0.01 {
                    let dist = dist_sq.sqrt();
                    let move_dist = step_dist.min(dist);
                    let inv_d = move_dist / dist;
                    self.fleets[f_idx].x += dx * inv_d;
                    self.fleets[f_idx].y += dy * inv_d;
                }
                continue;
            }

            let mut target_idx = self.fleets[f_idx].target;
            let mut tx = self.pos_x[target_idx];
            let mut ty = self.pos_y[target_idx];
            self.fleets[f_idx].hunting = false;

            let wh_route = self.fleets[f_idx].wh_route;
            let mut heading_to_wh = false;
            if wh_route >= 0 && !self.fleets[f_idx].used_wh {
                let w = (wh_route / 2) as usize;
                let dir = wh_route % 2;
                if let Some(&wh) = self.wormholes.get(w) {
                    let (ex, ey, ox, oy) = if dir == 0 {
                        (wh.x1, wh.y1, wh.x2, wh.y2)
                    } else {
                        (wh.x2, wh.y2, wh.x1, wh.y1)
                    };
                    let fx = self.fleets[f_idx].x;
                    let fy = self.fleets[f_idx].y;
                    let d_sq = (ex - fx) * (ex - fx) + (ey - fy) * (ey - fy);
                    if d_sq <= 784.0 {
                        if !silent && self.pending_jump_fx.len() < 25 {
                            self.pending_jump_fx.push(JumpFx {
                                x1: fx,
                                y1: fy,
                                x2: ox,
                                y2: oy,
                                ttl: 12,
                                col: "#d946ef".to_string(),
                            });
                        }
                        self.fleets[f_idx].x = ox;
                        self.fleets[f_idx].y = oy;
                        self.fleets[f_idx].used_wh = true;
                        self.fleets[f_idx].wh_route = -1;
                        self.gate_jumps_count += 1;
                        if w < self.discovered_wh_a.len() {
                            self.discovered_wh_a[w] |= 1u16 << e;
                            self.discovered_wh_b[w] |= 1u16 << e;
                        }
                        self.empires[e]
                            .active_exploration_orders
                            .retain(|o| !matches!(o, ExplorationOrder::ProbeWormhole { wormhole_id } if *wormhole_id == w));
                        self.scan_area_for_empire(
                            ox,
                            oy,
                            e,
                            ec.scan_r.max(85.0),
                            ec.scan_pen.max(40.0),
                            None,
                        );

                        // If a Colonizer jumped and its target was already taken by an enemy,
                        // pick another free explored neutral near the exit or return home through the wormhole!
                        let target_intel = self.intel_planets[e * n + target_idx];
                        if self.fleets[f_idx].fleet_type == FleetClass::Col
                            && target_intel.is_explored()
                            && target_intel.owner != 0
                            && target_intel.owner != emp_u8
                        {
                            let near_exit = if dir == 0 {
                                &self.wh_near_b[w]
                            } else {
                                &self.wh_near_a[w]
                            };
                            let mut alt: Option<usize> = None;
                            for &(cand_u32, _) in near_exit {
                                let cand = cand_u32 as usize;
                                let c_intel = self.intel_planets[e * n + cand];
                                if c_intel.is_explored() && c_intel.owner == 0 {
                                    alt = Some(cand);
                                    break;
                                }
                            }
                            self.claim_mask[target_idx] &= !(1u16 << e);
                            if let Some(new_t) = alt {
                                target_idx = new_t;
                                self.fleets[f_idx].target = new_t;
                                self.claim_mask[new_t] |= 1u16 << e;
                            } else {
                                // No free neutral on the other side: safely return through the same wormhole
                                let home = self.fleets[f_idx].from;
                                target_idx = home;
                                self.fleets[f_idx].target = home;
                                self.fleets[f_idx].x = ex;
                                self.fleets[f_idx].y = ey;
                            }
                            tx = self.pos_x[target_idx];
                            ty = self.pos_y[target_idx];
                        }
                    } else {
                        tx = ex;
                        ty = ey;
                        heading_to_wh = true;
                    }
                } else {
                    self.fleets[f_idx].wh_route = -1;
                }
            } else if !self.fleets[f_idx].used_wh && !self.wormholes.is_empty() {
                let fx = self.fleets[f_idx].x;
                let fy = self.fleets[f_idx].y;
                let mut jump_dest: Option<(f32, f32)> = None;
                let wormholes_ref = &self.wormholes;
                self.wh_grid.for_each_in_radius(fx, fy, 28.0, |entry| {
                    if jump_dest.is_some() {
                        return;
                    }
                    let d_sq = (entry.x - fx) * (entry.x - fx) + (entry.y - fy) * (entry.y - fy);
                    if d_sq <= 784.0 {
                        let w = (entry.idx / 2) as usize;
                        let dir = entry.idx % 2;
                        let wh = wormholes_ref[w];
                        let (ox, oy) = if dir == 0 {
                            (wh.x2, wh.y2)
                        } else {
                            (wh.x1, wh.y1)
                        };
                        let direct_dist = (tx - fx).hypot(ty - fy);
                        if (tx - ox).hypot(ty - oy) < direct_dist * 0.78 {
                            jump_dest = Some((ox, oy));
                        }
                    }
                });
                if let Some((ox, oy)) = jump_dest {
                    if !silent && self.pending_jump_fx.len() < 25 {
                        self.pending_jump_fx.push(JumpFx {
                            x1: fx,
                            y1: fy,
                            x2: ox,
                            y2: oy,
                            ttl: 12,
                            col: "#d946ef".to_string(),
                        });
                    }
                    self.fleets[f_idx].x = ox;
                    self.fleets[f_idx].y = oy;
                    self.fleets[f_idx].used_wh = true;
                    self.gate_jumps_count += 1;
                }
            }

            // Cruiser interception using flat CSR UniformGrid:
            // Strictly checks real EW detection (`d_sq <= eff_scan * eff_scan` with Cloak, Tachyon, and spatial planetary ECM)
            // BEFORE reading `other.power`!
            let ec = &emp_cache[e];
            let intercept_roll =
                ((self.fleets[f_idx].id.wrapping_mul(2654435761)) >> 24) as f32 / 256.0;
            if !heading_to_wh
                && self.fleets[f_idx].fleet_type == FleetClass::Cruiser
                && intercept_roll < ec.intercept_bias
            {
                let t_ew = if prof { perf_now_ms() } else { 0.0 };
                let tachyon_red = ec.tachyon_red;
                let scan_max = ec.cruiser_scan_max;
                let fx = self.fleets[f_idx].x;
                let fy = self.fleets[f_idx].y;
                let f_pow = self.fleets[f_idx].power;
                let mut best_prey: Option<(f32, f32)> = None;
                let mut best_prey_dist_sq = scan_max * scan_max;
                let fleets_ref = &self.fleets;
                let empires_ref = &self.empires;
                let relations_ref = &self.relations;
                let sim_ref: &Simulation = &*self;

                self.fleet_grid.for_each_in_radius(fx, fy, scan_max, |entry| {
                    if entry.tag == 0
                        || entry.tag == emp_u8
                        || relations_ref[e][(entry.tag - 1) as usize] != DiplomaticStance::War
                    {
                        return;
                    }
                    let k = entry.idx as usize;
                    if k >= fleets_ref.len() || k == f_idx {
                        return;
                    }
                    let dx = entry.x - fx;
                    let dy = entry.y - fy;
                    let d_sq = dx * dx + dy * dy;
                    if d_sq >= best_prey_dist_sq {
                        return;
                    }
                    // Step 1: Verify real EW detection (Cloak vs Tachyon + spatial planetary ECM around target coordinates)
                    // BEFORE reading `other.power`!
                    let other_e = (entry.tag - 1) as usize;
                    let other_tc = &empires_ref[other_e].tech_cache;
                    let target_cloak = (if (entry.flags & 1) != 0 {
                        other_tc.cloak_armed_pct
                    } else {
                        other_tc.cloak_unarmed_pct
                    } * ew_mult)
                        .clamp(0.0, 0.96);
                    let ecm_pen = sim_ref.local_ecm_jamming_at(entry.x, entry.y, emp_u8, ew_mult);
                    let eff_scan = effective_detection_range(
                        scan_max,
                        target_cloak,
                        tachyon_red,
                        ecm_pen,
                    );
                    if d_sq > eff_scan * eff_scan {
                        return;
                    }
                    // Step 2: Only after detection passes, evaluate whether the detected fleet is a valid prey target
                    let other = &fleets_ref[k];
                    if (entry.flags & 2) != 0 || other.power < f_pow * 0.9 {
                        best_prey_dist_sq = d_sq;
                        best_prey = Some((entry.x, entry.y));
                    }
                });

                if let Some((px, py)) = best_prey {
                    tx = px;
                    ty = py;
                    self.fleets[f_idx].hunting = true;
                }
                if prof {
                    prof_fleet_scan_ms += perf_now_ms() - t_ew;
                }
            }
            self.fleets[f_idx].steer_x = tx;
            self.fleets[f_idx].steer_y = ty;

            let fx = self.fleets[f_idx].x;
            let fy = self.fleets[f_idx].y;
            let _ = (has_auto_salvage, salvage_rad, salvage_rad_sq);

            let step_dist = ec.base_step_dist * self.fleets[f_idx].fleet_type.speed_mod();

            let tdx = self.pos_x[target_idx] - fx;
            let tdy = self.pos_y[target_idx] - fy;
            if !heading_to_wh && tdx * tdx + tdy * tdy <= step_dist * step_dist {
                self.claim_mask[target_idx] &= !(1u16 << e);
                // Arriving fleet's shipboard scanner performs an orbital penetrating scan of the target system & nearby space
                self.scan_area_for_empire(
                    self.pos_x[target_idx],
                    self.pos_y[target_idx],
                    e,
                    ec.scan_r * 0.65,
                    ec.scan_pen * 0.65,
                    Some(target_idx),
                );

                let fl_type = self.fleets[f_idx].fleet_type;
                let fl_emp = self.fleets[f_idx].emp;
                let fl_id = self.fleets[f_idx].id;
                if fl_type == FleetClass::Freight {
                    if self.owner[target_idx] == fl_emp {
                        let b = target_idx * 3;
                        let planet_surf_before =
                            self.min_surf[b] + self.min_surf[b + 1] + self.min_surf[b + 2];
                        let unloaded = if self.fleets[f_idx].salvage_mode == 2 {
                            // Unload exact physical cargo carried in hold (never multiply by salvage_mult on unload!)
                            let di = self.fleets[f_idx].cargo_iron;
                            let db = self.fleets[f_idx].cargo_bora;
                            let dg = self.fleets[f_idx].cargo_germ;
                            self.min_surf[b] += di;
                            self.min_surf[b + 1] += db;
                            self.min_surf[b + 2] += dg;
                            let tot = di + db + dg;
                            self.total_salvaged += tot.round() as u32;
                            tot
                        } else {
                            let mt = self.fleets[f_idx].min_type as usize;
                            let cargo = self.fleets[f_idx].power;
                            self.min_surf[b + mt] += cargo;
                            cargo
                        };
                        let planet_surf_after =
                            self.min_surf[b] + self.min_surf[b + 1] + self.min_surf[b + 2];
                        if self.trace_enabled {
                            self.record_trace_event(SimEvent::CargoUnloaded {
                                fleet_id: fl_id,
                                planet_idx: target_idx,
                                unloaded,
                                planet_surf_before,
                                planet_surf_after,
                            });
                        }
                        // Dock freighter in orbit at friendly destination for reuse (at most 1 docked freighter per planet)
                        let has_existing_freight = self.fleets.iter().enumerate().any(|(idx, f)| {
                            idx != f_idx
                                && f.orbiting
                                && f.target == target_idx
                                && f.emp == fl_emp
                                && f.fleet_type == FleetClass::Freight
                        });
                        if has_existing_freight {
                            let home = self.fleets[f_idx].from;
                            let home_has_freight = home < n
                                && self.fleets.iter().enumerate().any(|(idx, f)| {
                                    idx != f_idx
                                        && f.orbiting
                                        && f.target == home
                                        && f.emp == fl_emp
                                        && f.fleet_type == FleetClass::Freight
                                });
                            if home != target_idx
                                && home < n
                                && self.owner[home] == fl_emp
                                && !home_has_freight
                            {
                                let hx = self.pos_x[home];
                                let hy = self.pos_y[home];
                                let fl = &mut self.fleets[f_idx];
                                fl.orbiting = true;
                                fl.from = home;
                                fl.target = home;
                                fl.x = hx;
                                fl.y = hy;
                                fl.steer_x = hx;
                                fl.steer_y = hy;
                                fl.salvage_mode = 0;
                                fl.cargo_iron = 0.0;
                                fl.cargo_bora = 0.0;
                                fl.cargo_germ = 0.0;
                                fl.power = 12.0;
                                fl.max_power = fl.max_power.max(ec.cargo_cap);
                                fl.wh_route = -1;
                                fl.used_wh = false;
                            } else {
                                // Recycle redundant freighter hull minerals at destination
                                let (hi, hb, hg) =
                                    self.empires[e].ship_mineral_cost(FleetClass::Freight);
                                self.min_surf[b] += hi * 0.4;
                                self.min_surf[b + 1] += hb * 0.4;
                                self.min_surf[b + 2] += hg * 0.4;
                                self.fleets.swap_remove(f_idx);
                            }
                        } else {
                            let ox = self.pos_x[target_idx];
                            let oy = self.pos_y[target_idx];
                            let fl = &mut self.fleets[f_idx];
                            fl.orbiting = true;
                            fl.from = target_idx;
                            fl.target = target_idx;
                            fl.x = ox;
                            fl.y = oy;
                            fl.steer_x = ox;
                            fl.steer_y = oy;
                            fl.salvage_mode = 0;
                            fl.cargo_iron = 0.0;
                            fl.cargo_bora = 0.0;
                            fl.cargo_germ = 0.0;
                            fl.power = 12.0;
                            fl.max_power = fl.max_power.max(ec.cargo_cap);
                            fl.wh_route = -1;
                            fl.used_wh = false;
                        }
                    } else {
                        // Destination lost before arrival: redirect home if still friendly, else remove
                        let home = self.fleets[f_idx].from;
                        if home != target_idx && home < n && self.owner[home] == fl_emp {
                            self.fleets[f_idx].target = home;
                            self.fleets[f_idx].steer_x = self.pos_x[home];
                            self.fleets[f_idx].steer_y = self.pos_y[home];
                            self.fleets[f_idx].wh_route = -1;
                        } else {
                            self.fleets.swap_remove(f_idx);
                        }
                    }
                } else if fl_type == FleetClass::Col {
                    // Colonizer dismantles into starter colony infrastructure (as in Stars!)
                    let fl = self.fleets.swap_remove(f_idx);
                    if self.owner[target_idx] == 0 {
                        self.owner[target_idx] = fl.emp;
                        self.pop[target_idx] = 9.0;
                        self.mines[target_idx] = 3.0;
                        self.factories[target_idx] = 2.0;
                        self.min_surf[target_idx * 3] = 15.0;
                        self.min_surf[target_idx * 3 + 1] = 15.0;
                        self.min_surf[target_idx * 3 + 2] = 15.0;
                        self.defense[target_idx] = 8.0;
                        self.update_own_planet_intel(e, target_idx);
                    } else if self.owner[target_idx] == fl.emp {
                        self.pop[target_idx] =
                            (self.hab[target_idx] * 170.0).min(self.pop[target_idx] + 7.0);
                        self.update_own_planet_intel(e, target_idx);
                    }
                } else if self.owner[target_idx] == fl_emp
                    || !self.are_hostile(fl_emp, self.owner[target_idx])
                {
                    // Warship arrives at friendly, neutral, or non-hostile (Neutral/Ceasefire/Alliance) world
                    if self.owner[target_idx] == fl_emp {
                        let merge_idx = self.fleets.iter().enumerate().position(|(idx, f)| {
                            idx != f_idx
                                && f.orbiting
                                && f.target == target_idx
                                && f.emp == fl_emp
                                && f.fleet_type == fl_type
                        });
                        if let Some(m_idx) = merge_idx {
                            let add_pow = self.fleets[f_idx].power;
                            let add_max = self.fleets[f_idx].max_power;
                            let add_cnt = self.fleets[f_idx].ship_count.max(1);
                            self.fleets[m_idx].power += add_pow;
                            self.fleets[m_idx].max_power += add_max;
                            self.fleets[m_idx].ship_count =
                                self.fleets[m_idx].ship_count.saturating_add(add_cnt);
                            self.fleets[m_idx].upkeep =
                                fl_type.upkeep() * (self.fleets[m_idx].ship_count as f32);
                            self.fleets.swap_remove(f_idx);
                        } else {
                            let ox = self.pos_x[target_idx];
                            let oy = self.pos_y[target_idx];
                            let fl = &mut self.fleets[f_idx];
                            fl.orbiting = true;
                            fl.from = target_idx;
                            fl.target = target_idx;
                            fl.x = ox;
                            fl.y = oy;
                            fl.steer_x = ox;
                            fl.steer_y = oy;
                            fl.hunting = false;
                            fl.wh_route = -1;
                        }
                    } else {
                        // Close-range orbital survey on neutral / non-hostile arrival (unlocks pen-scan intel!)
                        let ox = self.pos_x[target_idx];
                        let oy = self.pos_y[target_idx];
                        self.scan_area_for_empire(
                            ox,
                            oy,
                            e,
                            ec.scan_r.max(70.0),
                            ec.scan_pen.max(35.0),
                            Some(target_idx),
                        );
                        let (scols, srows) = self.sector_grid_dims();
                        let arrived_sec =
                            sector_for_xy(ox, oy, self.world_w, self.world_h, scols, srows);
                        self.empires[e]
                            .active_exploration_orders
                            .retain(|o| !matches!(o, ExplorationOrder::ScoutSector { sector_id } if *sector_id == arrived_sec));

                        let home = self.fleets[f_idx].from;
                        if home != target_idx && home < n && self.owner[home] == fl_emp {
                            self.fleets[f_idx].target = home;
                            self.fleets[f_idx].steer_x = self.pos_x[home];
                            self.fleets[f_idx].steer_y = self.pos_y[home];
                            self.fleets[f_idx].wh_route = -1;
                        } else {
                            let fl = &mut self.fleets[f_idx];
                            fl.x = ox;
                            fl.y = oy;
                            fl.steer_x = ox;
                            fl.steer_y = oy;
                        }
                    }
                } else {
                    // Warship planetary assault sequence:
                    // Step 1: Must first defeat any hostile combat fleets stationed in orbit (Orbital Garrison)
                    // Step 2: Must then defeat any orbital Starbase
                    // Step 3: Finally assaults planetary ground defenses
                    let def_e = (self.owner[target_idx] - 1) as usize;
                    let sx = self.pos_x[target_idx];
                    let sy = self.pos_y[target_idx];

                    // Step 1: Orbital Garrison interception
                    let mut garrison_indices: Vec<usize> = Vec::new();
                    for (g_idx, g_fl) in self.fleets.iter().enumerate() {
                        if g_idx != f_idx
                            && g_fl.orbiting
                            && g_fl.target == target_idx
                            && g_fl.power > 0.0
                            && g_fl.fleet_type.is_combat()
                            && self.are_hostile(fl_emp, g_fl.emp)
                        {
                            garrison_indices.push(g_idx);
                        }
                    }

                    for g_idx in garrison_indices {
                        if self.fleets[f_idx].power <= 0.0 {
                            break;
                        }
                        if self.fleets[g_idx].power <= 0.0 {
                            continue;
                        }
                        self.resolve_fleet_pair_combat(
                            f_idx,
                            g_idx,
                            &emp_cache,
                            ew_mult,
                            true,
                            Some(target_idx),
                            silent,
                        );
                    }

                    if self.fleets[f_idx].power <= 0.0 {
                        // Attacker was destroyed by the orbiting garrison before reaching the Starbase or planet!
                        continue;
                    }

                    let mut fl_power = self.fleets[f_idx].power;

                    // Step 2: Orbital Starbase engagement
                    let sb_tier = self.starbase_tier[target_idx];
                    if sb_tier > 0 && self.starbase_hp[target_idx] > 0.0 {
                        let sb_hp_before = self.starbase_hp[target_idx];
                        let sb_combat_power = sb_hp_before * 0.22;
                        let dmg_to_sb = fl_power * 1.15;
                        let sb_hp_after = (sb_hp_before - dmg_to_sb).max(0.0);
                        self.starbase_hp[target_idx] = sb_hp_after;
                        let dmg_to_fl = sb_combat_power * 0.65;
                        fl_power -= dmg_to_fl;
                        self.fleets[f_idx].power = fl_power;

                        if sb_hp_after <= 0.0 {
                            // Starbase destroyed! Drop wreckage into debris field
                            let (si, sb, sg, _) = starbase_build_cost(sb_tier);
                            let rate = self.cfg.salvage_rate;
                            if rate > 0.0 && (si + sb + sg) * rate > 0.5 {
                                let ttl = self.cfg.scenario_modifiers.debris_ttl.max(1.0);
                                self.debris_fields.push(DebrisField {
                                    x: sx,
                                    y: sy,
                                    iron: si * rate,
                                    bora: sb * rate,
                                    germ: sg * rate,
                                    ttl,
                                    wrecks: vec![WreckEntry {
                                        emp: (def_e + 1) as u8,
                                        ship_type: FleetClass::Battleship,
                                        count: 1,
                                    }],
                                });
                            }
                            self.starbase_tier[target_idx] = 0;
                            self.starbase_max_hp[target_idx] = 0.0;
                            if !silent && sb_tier >= 3 {
                                let msg = format!(
                                    "{} destroyed {} {} at #{}!",
                                    self.empires[e].race.name,
                                    self.empires[def_e].race.name,
                                    starbase_tier_name(sb_tier),
                                    target_idx + 1
                                );
                                let col = self.empires[e].race.color.clone();
                                self.push_event(msg, &col);
                            }
                        }

                        let attacker_survived = sb_hp_after <= 0.0 && fl_power > 0.0;
                        if self.trace_enabled {
                            self.record_trace_event(SimEvent::StarbaseAssault {
                                planet_idx: target_idx,
                                sb_hp_before,
                                sb_hp_after,
                                attacker_survived,
                            });
                        }

                        if !attacker_survived {
                            let dead = self.fleets.swap_remove(f_idx);
                            self.spawn_salvage_debris(sx, sy, &dead);
                            continue;
                        }
                    }

                    // Step 3: Planetary Ground Defense assault
                    let def_before = self.defense[target_idx];
                    let mut bunker_bonus: f32 = if emp_cache[def_e].prt_is {
                        1.30
                    } else {
                        1.0
                    };
                    let mut effective_def = def_before;
                    if fl_type == FleetClass::Bomber {
                        let pen = (self.cfg.bomber_pen
                            + (self.empires[e].tech_cache.bomb_power * 0.02).min(0.22))
                        .min(0.88);
                        effective_def *= 1.0 - pen;
                        bunker_bonus = (bunker_bonus * 0.75).max(1.0);
                    }
                    let def_threshold =
                        (20.0 + effective_def + self.pop[target_idx] * 0.35) * bunker_bonus;
                    let captured = fl_power > def_threshold;
                    if captured {
                        self.owner[target_idx] = fl_emp;
                        self.pop[target_idx] = (self.pop[target_idx] * 0.65).max(10.0);
                        let struct_surv = if fl_type == FleetClass::Bomber {
                            if self.empires[e].tech_cache.bomb_smart {
                                0.85
                            } else {
                                0.68
                            }
                        } else {
                            0.82
                        };
                        self.factories[target_idx] *= struct_surv;
                        self.defense[target_idx] = (def_before * 0.5).min(20.0);
                        let def_after = self.defense[target_idx];
                        self.invasions_count += 1;
                        self.update_own_planet_intel(e, target_idx);
                        self.push_empire_alert(
                            e,
                            format!(
                                "Captured planet #{} from {}",
                                target_idx + 1,
                                self.empires[def_e].race.name
                            ),
                        );
                        self.push_empire_alert(
                            def_e,
                            format!(
                                "Lost planet #{} to {}",
                                target_idx + 1,
                                self.empires[e].race.name
                            ),
                        );
                        if !silent && self.has_gate[target_idx] != 0 {
                            let msg = format!(
                                "{} captured Stargate hub #{}!",
                                self.empires[e].race.name,
                                target_idx + 1
                            );
                            let col = self.empires[e].race.color.clone();
                            self.push_event(msg, &col);
                        }
                        if self.trace_enabled {
                            self.record_trace_event(SimEvent::PlanetAssault {
                                planet_idx: target_idx,
                                captured: true,
                                def_before,
                                def_after,
                            });
                        }
                        // Victorious warship takes assault damage and stations in orbit around captured planet
                        let assault_dmg = (def_threshold * 0.35).min(fl_power * 0.72);
                        let fl = &mut self.fleets[f_idx];
                        fl.power = (fl.power - assault_dmg).max(fl.max_power * 0.15);
                        fl.orbiting = true;
                        fl.from = target_idx;
                        fl.target = target_idx;
                        fl.x = sx;
                        fl.y = sy;
                        fl.steer_x = sx;
                        fl.steer_y = sy;
                        fl.hunting = false;
                        fl.wh_route = -1;
                    } else {
                        let dmg = if fl_type == FleetClass::Bomber {
                            fl_power * 0.85
                        } else {
                            (fl_power / bunker_bonus) * 0.45
                        };
                        self.defense[target_idx] =
                            (def_before - dmg).max(4.0).min(def_before * 0.9);
                        let def_after = self.defense[target_idx];
                        if fl_type == FleetClass::Bomber {
                            self.factories[target_idx] =
                                (self.factories[target_idx] * 0.92).max(2.0);
                        }
                        if self.trace_enabled {
                            self.record_trace_event(SimEvent::PlanetAssault {
                                planet_idx: target_idx,
                                captured: false,
                                def_before,
                                def_after,
                            });
                        }
                        // Repelled attacker is destroyed in planetary fire and leaves battle wreckage
                        let dead = self.fleets.swap_remove(f_idx);
                        self.spawn_salvage_debris(sx, sy, &dead);
                    }
                }
            } else {
                let dx = tx - fx;
                let dy = ty - fy;
                let dist_sq = dx * dx + dy * dy;
                if dist_sq > 0.01 {
                    let dist = dist_sq.sqrt();
                    let move_dist = step_dist.min(dist);
                    let inv_d = move_dist / dist;
                    self.fleets[f_idx].x += dx * inv_d;
                    self.fleets[f_idx].y += dy * inv_d;
                }
            }
        }

        // 5. Deep space combat using flat CSR UniformGrid (O(F) average, cache-linear 16-byte entries!)
        let t_after_move = if prof { perf_now_ms() } else { 0.0 };
        let fl_len = self.fleets.len();
        if fl_len > 1 {
            let combat_cell = hit_rad.max(20.0);
            {
                let fleets = &self.fleets;
                self.fleet_grid
                    .build(self.world_w, self.world_h, combat_cell, fl_len, |idx| {
                        let fl = &fleets[idx];
                        GridEntry {
                            x: fl.x,
                            y: fl.y,
                            idx: idx as u32,
                            tag: fl.emp,
                            flags: if fl.fleet_type.is_combat() { 1 } else { 0 },
                        }
                    });
            }

            for i in 0..fl_len {
                if self.fleets[i].power <= 0.0 {
                    continue;
                }
                let ax = self.fleets[i].x;
                let ay = self.fleets[i].y;
                let emp_a = self.fleets[i].emp;
                let type_a = self.fleets[i].fleet_type;
                let a_c = type_a.is_combat();

                self.combat_candidates.clear();
                {
                    let candidates = &mut self.combat_candidates;
                    let i_u32 = i as u32;
                    let relations_ref = &self.relations;
                    let ea = (emp_a - 1) as usize;
                    self.fleet_grid.for_each_in_radius(ax, ay, hit_rad, |entry| {
                        if entry.idx <= i_u32
                            || entry.tag == 0
                            || entry.tag == emp_a
                            || relations_ref[ea][(entry.tag - 1) as usize] != DiplomaticStance::War
                        {
                            return;
                        }
                        if !a_c && (entry.flags & 1) == 0 {
                            return;
                        }
                        let dx = ax - entry.x;
                        let dy = ay - entry.y;
                        if dx * dx + dy * dy <= hit_rad_sq {
                            candidates.push(entry.idx as usize);
                        }
                    });
                }

                for c_idx in 0..self.combat_candidates.len() {
                    let j = self.combat_candidates[c_idx];
                    if self.fleets[i].power <= 0.0 {
                        break;
                    }
                    if self.fleets[j].power <= 0.0 {
                        continue;
                    }
                    let is_orbital = self.fleets[i].orbiting || self.fleets[j].orbiting;
                    let orb_planet = if self.fleets[i].orbiting {
                        Some(self.fleets[i].target)
                    } else if self.fleets[j].orbiting {
                        Some(self.fleets[j].target)
                    } else {
                        None
                    };
                    let i_survived = self.resolve_fleet_pair_combat(
                        i,
                        j,
                        &emp_cache,
                        ew_mult,
                        is_orbital,
                        orb_planet,
                        silent,
                    );
                    if !i_survived {
                        break;
                    }
                }
            }
        }

        // 6. Remove destroyed fleets (O(1) swap_remove) & decay debris TTL
        let mut i = self.fleets.len();
        while i > 0 {
            i -= 1;
            if self.fleets[i].power <= 0.0 {
                let target = self.fleets[i].target;
                let e = (self.fleets[i].emp - 1) as usize;
                if target < self.claim_mask.len() {
                    self.claim_mask[target] &= !(1u16 << e);
                }
                self.fleets.swap_remove(i);
            }
        }

        let mut i = self.debris_fields.len();
        while i > 0 {
            i -= 1;
            self.debris_fields[i].ttl -= DT;
            let total = self.debris_fields[i].iron
                + self.debris_fields[i].bora
                + self.debris_fields[i].germ;
            if self.debris_fields[i].ttl <= 0.0 || total <= 1e-4 {
                self.debris_fields.swap_remove(i);
            }
        }

        self.year += DT;
        self.ticks += 1;

        self.check_victory_conditions();
        if self.trace_enabled {
            self.audit_tick_invariants();
        }

        if !silent && self.ticks % 5 == 0 {
            self.power_history
                .push(self.tick_stats.iter().map(|s| s.res).collect());
            if self.power_history.len() > MAX_POWER_HISTORY {
                self.power_history.remove(0);
            }
        }

        if prof {
            let t_end = perf_now_ms();
            let econ_block = t_after_econ - t_step_start;
            let move_block = t_after_move - t_after_econ;
            let combat_ms = (t_end - t_after_move).max(0.0);
            let total_ms = (t_end - t_step_start).max(0.0);
            let econ_ms = (econ_block - prof_ai_ms - prof_planet_scan_ms).max(0.0);
            let move_ms = (move_block - prof_fleet_scan_ms).max(0.0);
            let ew_scan_ms = (prof_planet_scan_ms + prof_fleet_scan_ms).max(0.0);
            self.last_profile = StepProfile {
                econ_ms,
                ai_ms: prof_ai_ms,
                move_ms,
                ew_scan_ms,
                combat_ms,
                total_ms,
            };
        }

        self.ecm_cache_valid = false;
        &self.tick_stats
    }

    #[inline]
    pub fn planet_has_ecm_jammer(&self, i: usize) -> bool {
        let emp_u8 = self.owner[i];
        if emp_u8 == 0 {
            return false;
        }
        let e = (emp_u8 - 1) as usize;
        let tc = &self.empires[e].tech_cache;
        tc.ecm_jammer_strength > 0.0
            && tc.ecm_jammer_range > 0.0
            && (self.has_gate[i] != 0 || self.defense[i] >= 55.0)
    }

    #[inline]
    pub fn effective_scanner_range(&self, e: usize) -> f32 {
        let base = self.empires[e].tech_cache.scanner_range
            * self.cfg.scenario_modifiers.scanner_range_mult.clamp(0.2, 3.0);
        if self.empires[e].race.prt == "PRT_SS" {
            base * 1.25
        } else {
            base
        }
    }

    #[inline]
    pub fn effective_pen_scanner_range(&self, e: usize) -> f32 {
        let base = self.empires[e].tech_cache.scanner_pen
            * self.cfg.scenario_modifiers.scanner_range_mult.clamp(0.2, 3.0);
        if self.empires[e].race.prt == "PRT_SS" {
            base * 1.25
        } else {
            base
        }
    }

    #[inline]
    pub fn planet_intel(&self, emp_idx: usize, planet_idx: usize) -> PlanetIntel {
        let n = self.owner.len();
        if emp_idx >= self.empires.len() || planet_idx >= n {
            return PlanetIntel::default();
        }
        self.intel_planets[emp_idx * n + planet_idx]
    }

    #[inline]
    pub fn is_wormhole_discovered(&self, emp_idx: usize, wh_idx: usize) -> bool {
        if emp_idx >= self.empires.len() || wh_idx >= self.discovered_wh_a.len() {
            return false;
        }
        let bit = 1u16 << emp_idx;
        ((self.discovered_wh_a[wh_idx] | self.discovered_wh_b[wh_idx]) & bit) != 0
    }

    #[inline]
    pub fn is_wormhole_end_a_discovered(&self, emp_idx: usize, wh_idx: usize) -> bool {
        if emp_idx >= self.empires.len() || wh_idx >= self.discovered_wh_a.len() {
            return false;
        }
        (self.discovered_wh_a[wh_idx] & (1u16 << emp_idx)) != 0
    }

    #[inline]
    pub fn is_wormhole_end_b_discovered(&self, emp_idx: usize, wh_idx: usize) -> bool {
        if emp_idx >= self.empires.len() || wh_idx >= self.discovered_wh_b.len() {
            return false;
        }
        (self.discovered_wh_b[wh_idx] & (1u16 << emp_idx)) != 0
    }

    pub fn push_empire_alert(&mut self, emp_idx: usize, msg: String) {
        if let Some(emp) = self.empires.get_mut(emp_idx) {
            if emp.recent_alerts.len() >= 12 {
                emp.recent_alerts.remove(0);
            }
            emp.recent_alerts.push(msg);
        }
    }

    /// Updates `intel_planets` for an owned planet `i` (`self.owner[i] == (e + 1) as u8`).
    #[inline]
    pub fn update_own_planet_intel(&mut self, e: usize, i: usize) {
        let n = self.owner.len();
        if e >= self.empires.len() || i >= n {
            return;
        }
        let emp_u8 = (e + 1) as u8;
        let bit = 1u16 << e;
        self.explored_mask[i] |= bit;
        let b = i * 3;
        let crust_sum = self.min_crust[b] + self.min_crust[b + 1] + self.min_crust[b + 2];
        let mut flags = INTEL_FLAG_EXPLORED | INTEL_FLAG_PEN_SCANNED;
        if self.has_gate[i] != 0 {
            flags |= INTEL_FLAG_HAS_STARGATE;
        }
        self.intel_planets[e * n + i] = PlanetIntel {
            last_seen_tick: self.ticks as u32,
            owner: emp_u8,
            starbase_tier: self.starbase_tier[i],
            flags,
            pop_est: self.pop[i],
            def_est: self.defense[i],
            fac_est: self.factories[i],
            hab_est: self.hab[i],
            crust_sum_est: crust_sum,
        };
    }

    /// Computes spatial enemy planetary ECM jamming penalty (`0.0..=0.85`) at coordinates `(x, y)`
    /// against `observer_emp_u8`, querying `self.active_ecm_jammers` or `self.planet_grid` (never reading private fleet trajectory fields).
    pub fn local_ecm_jamming_at(
        &self,
        x: f32,
        y: f32,
        observer_emp_u8: u8,
        ew_mult: f32,
    ) -> f32 {
        if ew_mult <= 0.0 {
            return 0.0;
        }
        if self.ecm_cache_valid {
            if self.active_ecm_jammers.is_empty() || self.max_active_ecm_range <= 0.0 {
                return 0.0;
            }
            let mut ecm_pen = 0.0f32;
            let jammers = &self.active_ecm_jammers;
            self.ecm_grid
                .for_each_in_radius(x, y, self.max_active_ecm_range, |entry| {
                    if entry.tag != observer_emp_u8 {
                        let &(jx, jy, jam_r, jam_str, _) = &jammers[entry.idx as usize];
                        let jdx = x - jx;
                        let jdy = y - jy;
                        let jdist_sq = jdx * jdx + jdy * jdy;
                        if jdist_sq < jam_r * jam_r {
                            let falloff = 1.0 - jdist_sq.sqrt() / jam_r;
                            ecm_pen = ecm_pen.max((jam_str * falloff * ew_mult).min(0.85));
                        }
                    }
                });
            return ecm_pen;
        }

        let max_jam_r = self
            .empires
            .iter()
            .filter(|emp| emp.tech_cache.ecm_jammer_strength > 0.0)
            .map(|emp| emp.tech_cache.ecm_jammer_range)
            .fold(0.0f32, f32::max);
        if max_jam_r <= 0.0 {
            return 0.0;
        }
        let mut ecm_pen = 0.0f32;
        let owner_ref = &self.owner;
        let empires_ref = &self.empires;
        let has_gate_ref = &self.has_gate;
        let defense_ref = &self.defense;
        let pos_x_ref = &self.pos_x;
        let pos_y_ref = &self.pos_y;

        self.planet_grid.for_each_in_radius(x, y, max_jam_r, |entry| {
            let p_idx = entry.idx as usize;
            let p_owner = owner_ref[p_idx];
            if p_owner != 0 && p_owner != observer_emp_u8 {
                let pe = (p_owner - 1) as usize;
                let ptc = &empires_ref[pe].tech_cache;
                let jam_r = ptc.ecm_jammer_range;
                if ptc.ecm_jammer_strength > 0.0
                    && jam_r > 0.0
                    && (has_gate_ref[p_idx] != 0 || defense_ref[p_idx] >= 55.0)
                {
                    let jdx = x - pos_x_ref[p_idx];
                    let jdy = y - pos_y_ref[p_idx];
                    let jdist_sq = jdx * jdx + jdy * jdy;
                    if jdist_sq < jam_r * jam_r {
                        let falloff = 1.0 - jdist_sq.sqrt() / jam_r;
                        ecm_pen =
                            ecm_pen.max((ptc.ecm_jammer_strength * falloff * ew_mult).min(0.85));
                    }
                }
            }
        });
        ecm_pen
    }

    /// Performs a spatial scanner sweep around `(cx, cy)` for empire `e` using `self.planet_grid` (and sorted neighbor fast-path when untruncated) and `self.wh_grid`.
    /// - Non-penetrating `non_pen_radius` (suppressed by enemy planetary ECM) updates ONLY `last_seen_tick`, `owner`, `starbase_tier`, and `INTEL_FLAG_EXPLORED`.
    /// - Penetrating `pen_radius` (suppressed by enemy planetary ECM) OR direct `orbital_planet` presence sets `INTEL_FLAG_PEN_SCANNED`
    ///   and updates `hab_est`, `crust_sum_est`, `pop_est`, `fac_est`, `def_est`, and `INTEL_FLAG_HAS_STARGATE`.
    /// - Also marks any wormhole endpoints within `non_pen_radius` as discovered in `discovered_wormholes`.
    pub fn scan_area_for_empire(
        &mut self,
        cx: f32,
        cy: f32,
        e: usize,
        non_pen_radius: f32,
        pen_radius: f32,
        orbital_planet: Option<usize>,
    ) {
        let n = self.owner.len();
        if e >= self.empires.len() || n == 0 {
            return;
        }
        let emp_u8 = (e + 1) as u8;
        let bit = 1u16 << e;
        let tick_u32 = self.ticks as u32;
        let ew_mult = self.cfg.scenario_modifiers.ew_strength_mult.clamp(0.0, 2.5);

        // 1. Direct orbital presence always grants full penetrating scan of the orbited planet
        if let Some(orb_idx) = orbital_planet {
            if orb_idx < n {
                self.explored_mask[orb_idx] |= bit;
                let p_owner = self.owner[orb_idx];
                if p_owner != 0 && p_owner != emp_u8 {
                    self.empires[e].contact_mask |= 1u16 << (p_owner - 1);
                }
                let b = orb_idx * 3;
                let crust_sum =
                    self.min_crust[b] + self.min_crust[b + 1] + self.min_crust[b + 2];
                let mut flags = INTEL_FLAG_EXPLORED | INTEL_FLAG_PEN_SCANNED;
                if self.has_gate[orb_idx] != 0 {
                    flags |= INTEL_FLAG_HAS_STARGATE;
                }
                self.intel_planets[e * n + orb_idx] = PlanetIntel {
                    last_seen_tick: tick_u32,
                    owner: p_owner,
                    starbase_tier: self.starbase_tier[orb_idx],
                    flags,
                    pop_est: self.pop[orb_idx],
                    def_est: self.defense[orb_idx],
                    fac_est: self.factories[orb_idx],
                    hab_est: self.hab[orb_idx],
                    crust_sum_est: crust_sum,
                };
            }
        }

        let max_r = non_pen_radius.max(pen_radius);
        if max_r <= 0.0 {
            return;
        }

        // 2. Discover wormhole endpoints within scanner radius (strictly per-endpoint, no leaking the far end!)
        if !self.wormholes.is_empty() && non_pen_radius > 0.0 {
            let r_sq = non_pen_radius * non_pen_radius;
            let disc_a = &mut self.discovered_wh_a;
            let disc_b = &mut self.discovered_wh_b;
            self.wh_grid.for_each_in_radius(cx, cy, non_pen_radius, |entry| {
                let dx = entry.x - cx;
                let dy = entry.y - cy;
                if dx * dx + dy * dy <= r_sq {
                    let w = (entry.idx / 2) as usize;
                    let is_b = (entry.idx & 1) == 1;
                    if w < disc_a.len() {
                        if is_b {
                            disc_b[w] |= bit;
                        } else {
                            disc_a[w] |= bit;
                        }
                    }
                }
            });
        }

        // 3. Collect all planets within max_r into reusable `self.scan_scratch` (zero heap allocation!)
        //    Fast path: if centered on `orb_idx` and its sorted neighbor slice reaches >= max_r without hitting the truncation cap,
        //    use the sorted slice directly; otherwise query the spatial `planet_grid` so long-range scanners never suffer truncation!
        self.scan_scratch.clear();
        let max_r_sq = max_r * max_r;
        let mut used_sorted_neighbors = false;
        if let Some(center_idx) = orbital_planet {
            if center_idx < n
                && (self.pos_x[center_idx] - cx).abs() < 1e-3
                && (self.pos_y[center_idx] - cy).abs() < 1e-3
            {
                let start = self.neighbor_offsets[center_idx] as usize;
                let end = self.neighbor_offsets[center_idx + 1] as usize;
                let max_k = if n >= 5_000 { 28 } else { MAX_PLANET_NEIGHBORS };
                let untruncated =
                    (end - start) < max_k || (end > start && self.neighbor_data[end - 1].1 >= max_r);
                if untruncated {
                    used_sorted_neighbors = true;
                    for &(j_u32, dist) in &self.neighbor_data[start..end] {
                        if dist > max_r {
                            break;
                        }
                        let j = j_u32 as usize;
                        self.scan_scratch
                            .push((j, self.pos_x[j], self.pos_y[j], dist * dist));
                    }
                }
            }
        }
        if !used_sorted_neighbors {
            let scratch = &mut self.scan_scratch;
            self.planet_grid.for_each_in_radius(cx, cy, max_r, |entry| {
                let j = entry.idx as usize;
                if Some(j) == orbital_planet {
                    return;
                }
                let dx = entry.x - cx;
                let dy = entry.y - cy;
                let d_sq = dx * dx + dy * dy;
                if d_sq <= max_r_sq {
                    scratch.push((j, entry.x, entry.y, d_sq));
                }
            });
        }

        let pen_r_sq = pen_radius * pen_radius;
        let guaranteed_non_pen_sq = (non_pen_radius * 0.15) * (non_pen_radius * 0.15);
        let guaranteed_pen_sq = (pen_radius * 0.15) * (pen_radius * 0.15);
        for idx in 0..self.scan_scratch.len() {
            let (j, px, py, d_sq) = self.scan_scratch[idx];
            let p_owner = self.owner[j];
            if p_owner == emp_u8 {
                continue;
            }
            let cur_intel = self.intel_planets[e * n + j];
            if cur_intel.is_explored()
                && cur_intel.last_seen_tick == tick_u32
                && (pen_radius <= 0.0 || d_sq > pen_r_sq || cur_intel.is_pen_scanned())
            {
                continue;
            }
            let (in_non_pen, in_pen) = if d_sq <= guaranteed_non_pen_sq
                && (pen_radius <= 0.0 || d_sq <= guaranteed_pen_sq)
            {
                (true, pen_radius > 0.0)
            } else {
                let ecm_pen = self.local_ecm_jamming_at(px, py, emp_u8, ew_mult);
                let eff_non_pen = non_pen_radius * (1.0 - ecm_pen);
                let eff_pen = pen_radius * (1.0 - ecm_pen);
                (
                    d_sq <= eff_non_pen * eff_non_pen,
                    eff_pen > 0.0 && d_sq <= eff_pen * eff_pen,
                )
            };

            if in_non_pen || in_pen {
                self.explored_mask[j] |= bit;
                if p_owner != 0 {
                    self.empires[e].contact_mask |= 1u16 << (p_owner - 1);
                }
                let intel = &mut self.intel_planets[e * n + j];
                intel.last_seen_tick = tick_u32;
                intel.owner = p_owner;
                intel.starbase_tier = self.starbase_tier[j];
                intel.flags |= INTEL_FLAG_EXPLORED;

                if in_pen {
                    let b = j * 3;
                    let crust_sum =
                        self.min_crust[b] + self.min_crust[b + 1] + self.min_crust[b + 2];
                    intel.flags |= INTEL_FLAG_PEN_SCANNED;
                    if self.has_gate[j] != 0 {
                        intel.flags |= INTEL_FLAG_HAS_STARGATE;
                    } else {
                        intel.flags &= !INTEL_FLAG_HAS_STARGATE;
                    }
                    intel.pop_est = self.pop[j];
                    intel.def_est = self.defense[j];
                    intel.fac_est = self.factories[j];
                    intel.hab_est = self.hab[j];
                    intel.crust_sum_est = crust_sum;
                }
            }
        }
    }

    /// Initializes per-empire `intel_planets`, `explored_mask`, and `discovered_wormholes`
    /// strictly from each empire's owned planets and active fleets.
    pub fn refresh_initial_fow(&mut self) {
        let n = self.owner.len();
        let num_emp = self.empires.len();
        self.explored_mask.fill(0);
        self.intel_planets.clear();
        self.intel_planets
            .resize(num_emp * n, PlanetIntel::default());
        self.discovered_wh_a.clear();
        self.discovered_wh_a.resize(self.wormholes.len(), 0);
        self.discovered_wh_b.clear();
        self.discovered_wh_b.resize(self.wormholes.len(), 0);
        self.max_active_ecm_range = self
            .empires
            .iter()
            .map(|emp| emp.tech_cache.ecm_jammer_range)
            .fold(0.0f32, f32::max);

        for i in 0..n {
            let o = self.owner[i];
            if o > 0 {
                let e = (o - 1) as usize;
                if e < num_emp {
                    let scan_r = self.effective_scanner_range(e);
                    let scan_pen = self.effective_pen_scanner_range(e);
                    self.update_own_planet_intel(e, i);
                    self.scan_area_for_empire(
                        self.pos_x[i],
                        self.pos_y[i],
                        e,
                        scan_r,
                        scan_pen,
                        Some(i),
                    );
                }
            }
        }

        let fleets_snapshot: Vec<(usize, f32, f32, usize, bool)> = self
            .fleets
            .iter()
            .filter(|fl| fl.emp > 0 && fl.power > 0.0)
            .map(|fl| ((fl.emp - 1) as usize, fl.x, fl.y, fl.target, fl.orbiting))
            .collect();

        for (e, fx, fy, target, orbiting) in fleets_snapshot {
            if e >= num_emp {
                continue;
            }
            let scan_r = self.effective_scanner_range(e) * 0.65;
            let scan_pen = self.effective_pen_scanner_range(e) * 0.65;
            let orb = if orbiting && target < n {
                Some(target)
            } else if target < n {
                let dx = self.pos_x[target] - fx;
                let dy = self.pos_y[target] - fy;
                if dx * dx + dy * dy <= 16.0 {
                    Some(target)
                } else {
                    None
                }
            } else {
                None
            };
            self.scan_area_for_empire(fx, fy, e, scan_r, scan_pen, orb);
        }
    }

    /// Returns all foreign fleets currently detected by empire `e`'s planetary or shipboard scanners,
    /// accounting for target Cloak, observer Tachyon Detector reduction, and spatial enemy planetary ECM.
    pub fn compute_detected_enemy_fleets(
        &self,
        e: usize,
        sector_cols: usize,
        sector_rows: usize,
    ) -> Vec<DetectedEnemyFleet> {
        if e >= self.empires.len() {
            return Vec::new();
        }
        let emp_u8 = (e + 1) as u8;
        let scan_r = self.effective_scanner_range(e);
        let tc = &self.empires[e].tech_cache;
        let scan_mult = self.cfg.scenario_modifiers.scanner_range_mult.clamp(0.2, 3.0);
        let prt_ss = self.empires[e].race.prt == "PRT_SS";
        let cruiser_scan_max = ((self.cfg.scanner_range * 0.65 + tc.scanner_range * 0.45)
            * if prt_ss { 1.25 } else { 1.0 }
            * scan_mult)
            .min(240.0);
        let tachyon_red = tc.tachyon_cloak_reduction;
        let ew_mult = self.cfg.scenario_modifiers.ew_strength_mult.clamp(0.0, 2.5);

        let own_fleets: Vec<(f32, f32, f32)> = self
            .fleets
            .iter()
            .filter(|fl| fl.emp == emp_u8 && fl.power > 0.0)
            .map(|fl| {
                let base_s = if fl.fleet_type == FleetClass::Cruiser {
                    cruiser_scan_max
                } else {
                    scan_r * 0.65
                };
                (fl.x, fl.y, base_s)
            })
            .collect();

        let mut detected = Vec::new();
        let owner_ref = &self.owner;

        for fl in &self.fleets {
            if fl.emp == 0 || fl.emp == emp_u8 || fl.power <= 0.0 {
                continue;
            }
            let other_e = (fl.emp - 1) as usize;
            if other_e >= self.empires.len() {
                continue;
            }
            let other_tc = &self.empires[other_e].tech_cache;
            let target_cloak = (if fl.fleet_type.is_combat() {
                other_tc.cloak_armed_pct
            } else {
                other_tc.cloak_unarmed_pct
            } * ew_mult)
                .clamp(0.0, 0.96);
            let ecm_pen = self.local_ecm_jamming_at(fl.x, fl.y, emp_u8, ew_mult);
            let eff_planet_scan =
                effective_detection_range(scan_r, target_cloak, tachyon_red, ecm_pen);

            let mut is_seen = false;
            if eff_planet_scan > 0.0 {
                let r_sq = eff_planet_scan * eff_planet_scan;
                self.planet_grid
                    .for_each_in_radius(fl.x, fl.y, eff_planet_scan, |entry| {
                        if is_seen {
                            return;
                        }
                        let p_idx = entry.idx as usize;
                        if owner_ref[p_idx] == emp_u8 {
                            let dx = entry.x - fl.x;
                            let dy = entry.y - fl.y;
                            if dx * dx + dy * dy <= r_sq {
                                is_seen = true;
                            }
                        }
                    });
            }

            if !is_seen {
                for &(ox, oy, base_ship_scan) in &own_fleets {
                    let eff_ship = effective_detection_range(
                        base_ship_scan,
                        target_cloak,
                        tachyon_red,
                        ecm_pen,
                    );
                    let dx = ox - fl.x;
                    let dy = oy - fl.y;
                    if dx * dx + dy * dy <= eff_ship * eff_ship {
                        is_seen = true;
                        break;
                    }
                }
            }

            if is_seen {
                let (vx, vy) = if fl.orbiting {
                    (0.0, 0.0)
                } else {
                    let dx = fl.steer_x - fl.x;
                    let dy = fl.steer_y - fl.y;
                    let len = dx.hypot(dy);
                    let spd = self.cfg.fleet_speed * fl.fleet_type.speed_mod();
                    if len > 1e-4 {
                        (dx / len * spd, dy / len * spd)
                    } else {
                        (0.0, 0.0)
                    }
                };
                let sec_id = sector_for_xy(
                    fl.x,
                    fl.y,
                    self.world_w,
                    self.world_h,
                    sector_cols,
                    sector_rows,
                );
                detected.push(DetectedEnemyFleet {
                    fleet_id: fl.id,
                    emp: fl.emp,
                    fleet_type: fl.fleet_type,
                    x: fl.x,
                    y: fl.y,
                    vx,
                    vy,
                    power: fl.power,
                    ship_count: fl.ship_count,
                    orbiting: fl.orbiting,
                    sector_id: sec_id,
                });
            }
        }
        detected
    }

    /// Returns all active `DebrisField`s currently inside empire `e`'s active planetary or shipboard scanner range.
    pub fn compute_visible_debris(&self, e: usize) -> Vec<VisibleDebrisBrief> {
        if e >= self.empires.len() || self.debris_fields.is_empty() {
            return Vec::new();
        }
        let emp_u8 = (e + 1) as u8;
        let scan_r = self.effective_scanner_range(e);
        let ship_scan_r = scan_r * 0.65;
        let ew_mult = self.cfg.scenario_modifiers.ew_strength_mult.clamp(0.0, 2.5);
        let (sector_cols, sector_rows) = self.sector_grid_dims();

        let own_fleets: Vec<(f32, f32)> = self
            .fleets
            .iter()
            .filter(|fl| fl.emp == emp_u8 && fl.power > 0.0)
            .map(|fl| (fl.x, fl.y))
            .collect();

        let owner_ref = &self.owner;
        let mut visible = Vec::new();

        for (d_idx, deb) in self.debris_fields.iter().enumerate() {
            if deb.ttl <= 0.0 || (deb.iron + deb.bora + deb.germ) <= 0.5 {
                continue;
            }
            let ecm_pen = self.local_ecm_jamming_at(deb.x, deb.y, emp_u8, ew_mult);
            let eff_planet_r = scan_r * (1.0 - ecm_pen);
            let eff_ship_r = ship_scan_r * (1.0 - ecm_pen);

            let mut is_seen = false;
            if eff_planet_r > 0.0 {
                let r_sq = eff_planet_r * eff_planet_r;
                self.planet_grid
                    .for_each_in_radius(deb.x, deb.y, eff_planet_r, |entry| {
                        if is_seen {
                            return;
                        }
                        if owner_ref[entry.idx as usize] == emp_u8 {
                            let dx = entry.x - deb.x;
                            let dy = entry.y - deb.y;
                            if dx * dx + dy * dy <= r_sq {
                                is_seen = true;
                            }
                        }
                    });
            }

            if !is_seen && eff_ship_r > 0.0 {
                let r_sq = eff_ship_r * eff_ship_r;
                for &(ox, oy) in &own_fleets {
                    let dx = ox - deb.x;
                    let dy = oy - deb.y;
                    if dx * dx + dy * dy <= r_sq {
                        is_seen = true;
                        break;
                    }
                }
            }

            if is_seen {
                let sec_id = sector_for_xy(
                    deb.x,
                    deb.y,
                    self.world_w,
                    self.world_h,
                    sector_cols,
                    sector_rows,
                );
                visible.push(VisibleDebrisBrief {
                    debris_idx: d_idx,
                    x: deb.x,
                    y: deb.y,
                    iron: deb.iron,
                    bora: deb.bora,
                    germ: deb.germ,
                    ttl: deb.ttl,
                    sector_id: sec_id,
                });
            }
        }
        visible
    }

    fn sector_grid_dims(&self) -> (usize, usize) {
        if self.owner.len() > 1200 {
            (6, 6)
        } else {
            (4, 4)
        }
    }

    /// Builds a Fog-of-War-filtered `EmpireView` for each empire and executes `governor::run_empire_governor`.
    fn run_all_governors(
        &mut self,
        emp_cache: &[EmpireTickCache; MAX_EMPIRES],
        emp_upkeep_ratio: &[f32; MAX_EMPIRES],
        silent: bool,
    ) {
        let n = self.owner.len();
        let num_emp = self.empires.len();
        let tick_usize = self.ticks as usize;
        let (sector_cols, sector_rows) = self.sector_grid_dims();

        // Bucket owned planets per empire in a single O(N) pass
        let mut own_planets_by_emp: Vec<Vec<OwnPlanetState>> =
            (0..num_emp).map(|_| Vec::new()).collect();
        for i in 0..n {
            let emp_u8 = self.owner[i];
            if emp_u8 == 0 {
                continue;
            }
            let e = (emp_u8 - 1) as usize;
            if e >= num_emp {
                continue;
            }
            let ec = &emp_cache[e];
            let max_pop = ((self.hab[i] + ec.terra_bonus).min(1.15)) * ec.lv5_pop_mult;
            let ow = self.orbit_warship[i];
            let (ow_pow, ow_cls) = if ow >= 0 && (ow as usize) < self.fleets.len() {
                let fl = &self.fleets[ow as usize];
                (fl.power, Some(fl.fleet_type))
            } else {
                (0.0, None)
            };
            let b = i * 3;
            let ai_turn = (i + tick_usize) % AI_DECISION_STRIDE == 0 || n <= 32;
            own_planets_by_emp[e].push(OwnPlanetState {
                planet_idx: i,
                x: self.pos_x[i],
                y: self.pos_y[i],
                pop: self.pop[i],
                max_pop,
                factories: self.factories[i],
                defense: self.defense[i],
                ship_prod_buf: self.ship_prod_buf[i],
                min_surf: [self.min_surf[b], self.min_surf[b + 1], self.min_surf[b + 2]],
                has_gate: self.has_gate[i] != 0,
                starbase_tier: self.starbase_tier[i],
                orbit_freight: self.orbit_freight[i],
                orbit_warship: ow,
                orbit_warship_power: ow_pow,
                orbit_warship_class: ow_cls,
                orbit_count: self.orbit_count[i],
                ai_turn,
            });
        }

        let unit_powers: Vec<(f32, f32, f32, f32)> = (0..num_emp)
            .map(|e| {
                (
                    self.compute_unit_power(e, FleetClass::Cruiser),
                    self.compute_unit_power(e, FleetClass::Battleship),
                    self.compute_unit_power(e, FleetClass::Bomber),
                    self.compute_unit_power(e, FleetClass::Col),
                )
            })
            .collect();

        for e in 0..num_emp {
            if own_planets_by_emp[e].is_empty() {
                continue;
            }
            let any_turn = own_planets_by_emp[e].iter().any(|op| op.ai_turn)
                || !self.empires[e].pending_hub_orders.is_empty();
            if !any_turn {
                continue;
            }
            let emp_u8 = (e + 1) as u8;
            let ec = &emp_cache[e];

            // Fast O(F_orbiting) check for enemy fleets orbiting currently-scanned planets
            let cur_tick_u32 = self.ticks as u32;
            let stride_u32 = AI_DECISION_STRIDE as u32;
            let mut visible_enemy_fleets = Vec::new();
            for fl in &self.fleets {
                if fl.orbiting && fl.emp != 0 && fl.emp != emp_u8 && fl.power > 0.0 && fl.target < n
                {
                    let intel = &self.intel_planets[e * n + fl.target];
                    if intel.is_explored()
                        && cur_tick_u32.saturating_sub(intel.last_seen_tick) <= stride_u32
                    {
                        visible_enemy_fleets.push(DetectedEnemyFleet {
                            fleet_id: fl.id,
                            emp: fl.emp,
                            fleet_type: fl.fleet_type,
                            x: fl.x,
                            y: fl.y,
                            vx: 0.0,
                            vy: 0.0,
                            power: fl.power,
                            ship_count: fl.ship_count,
                            orbiting: true,
                            sector_id: 0,
                        });
                    }
                }
            }
            let visible_debris = if self.debris_fields.is_empty() {
                Vec::new()
            } else {
                self.compute_visible_debris(e)
            };

            let mut visible_wormholes = Vec::new();
            for (w, &wh) in self.wormholes.iter().enumerate() {
                let bit = 1u16 << e;
                let has_a = (self.discovered_wh_a.get(w).copied().unwrap_or(0) & bit) != 0;
                let has_b = (self.discovered_wh_b.get(w).copied().unwrap_or(0) & bit) != 0;
                if has_a || has_b {
                    visible_wormholes.push(VisibleWormhole {
                        wormhole_id: w,
                        wh,
                        has_a,
                        has_b,
                    });
                }
            }

            let actions = {
                let emp = &self.empires[e];
                let tc = &emp.tech_cache;
                let (up_cr, up_bs, up_bm, up_col) = unit_powers[e];
                let view = EmpireView {
                    emp_idx: e,
                    emp_u8,
                    ticks: self.ticks,
                    year: self.year,
                    world_w: self.world_w,
                    world_h: self.world_h,
                    cfg: &self.cfg,
                    policy: &emp.race.doctrine,
                    upkeep_ratio: emp_upkeep_ratio[e],
                    total_active_fleets: self.fleets.len(),
                    has_stargate_tech: ec.has_stargate_tech,
                    max_gates: ec.max_gates,
                    min_fac_for_gate: ec.min_fac_for_gate,
                    gate_iron: ec.gate_iron,
                    gate_germ: ec.gate_germ,
                    gate_res: ec.gate_res,
                    min_hub_sq: ec.min_hub_sq,
                    gate_range_sq: ec.gate_range_sq,
                    best_starbase_tier: tc.best_starbase_tier,
                    effective_ship_cost: ec.effective_ship_cost,
                    cargo_cap: ec.cargo_cap,
                    freight_range: ec.freight_range,
                    col_range: ec.col_range,
                    col_i: ec.col_i,
                    col_g: ec.col_g,
                    salvage_mult: ec.salvage_mult,
                    bomb_power: tc.bomb_power,
                    unit_power_cruiser: up_cr,
                    unit_power_bship: up_bs,
                    unit_power_bomber: up_bm,
                    unit_power_col: up_col,
                    range_cruiser: emp.ship_range(FleetClass::Cruiser),
                    range_bship: emp.ship_range(FleetClass::Battleship),
                    range_bomber: emp.ship_range(FleetClass::Bomber),
                    cost_cruiser: emp.ship_mineral_cost(FleetClass::Cruiser),
                    cost_bship: emp.ship_mineral_cost(FleetClass::Battleship),
                    cost_bomber: emp.ship_mineral_cost(FleetClass::Bomber),
                    cost_freight: emp.ship_mineral_cost(FleetClass::Freight),
                    sector_cols,
                    sector_rows,
                    sector_directives: &emp.sector_directives,
                    relations: &self.relations[e],
                    planet_pos_x: &self.pos_x,
                    planet_pos_y: &self.pos_y,
                    neighbor_offsets: &self.neighbor_offsets,
                    neighbor_data: &self.neighbor_data,
                    intel_planets: &self.intel_planets[e * n..(e + 1) * n],
                    claim_mask: &self.claim_mask,
                    own_planets: &own_planets_by_emp[e],
                    own_planet_min_surf: &self.min_surf,
                    own_gates: &self.gate_lists[e],
                    visible_wormholes: &visible_wormholes,
                    wh_near_a: &self.wh_near_a,
                    wh_near_b: &self.wh_near_b,
                    visible_enemy_fleets: &visible_enemy_fleets,
                    visible_debris: &visible_debris,
                    pending_hub_orders: &emp.pending_hub_orders,
                    active_exploration_orders: &emp.active_exploration_orders,
                };
                let mut local_rng = self.rng_seed;
                let acts = run_empire_governor(&view, &mut local_rng);
                self.rng_seed = local_rng;
                acts
            };

            self.apply_governor_actions(e, actions, &emp_cache[e], silent);
        }
    }

    fn apply_governor_actions(
        &mut self,
        e: usize,
        actions: Vec<GovernorAction>,
        ec: &EmpireTickCache,
        silent: bool,
    ) {
        let n = self.owner.len();
        let emp_u8 = (e + 1) as u8;
        let my_bit = 1u16 << e;

        for action in actions {
            match action {
                GovernorAction::BuildStargate { planet_idx } => {
                    if planet_idx >= n
                        || self.owner[planet_idx] != emp_u8
                        || self.has_gate[planet_idx] != 0
                    {
                        continue;
                    }
                    let b = planet_idx * 3;
                    if self.ship_prod_buf[planet_idx] < ec.gate_res
                        || self.min_surf[b] < ec.gate_iron
                        || self.min_surf[b + 2] < ec.gate_germ
                    {
                        continue;
                    }
                    self.ship_prod_buf[planet_idx] -= ec.gate_res;
                    self.min_surf[b] -= ec.gate_iron;
                    self.min_surf[b + 2] -= ec.gate_germ;
                    self.factories[planet_idx] =
                        (self.factories[planet_idx] - ec.gate_res * 0.25).max(10.0);
                    self.has_gate[planet_idx] = 1;
                    self.gate_lists[e].push(planet_idx);
                    self.intel_planets[e * n + planet_idx].flags |= INTEL_FLAG_HAS_STARGATE;
                    self.empires[e]
                        .pending_hub_orders
                        .retain(|&(p, k)| !(p == planet_idx && k == HubKind::Stargate));
                    if !silent {
                        let msg = format!(
                            "{} built a Stargate at #{}!",
                            self.empires[e].race.name,
                            planet_idx + 1
                        );
                        let col = self.empires[e].race.color.clone();
                        self.push_event(msg, &col);
                    }
                }
                GovernorAction::BuildStarbase {
                    planet_idx,
                    target_tier,
                } => {
                    if planet_idx >= n || self.owner[planet_idx] != emp_u8 {
                        continue;
                    }
                    let cur_sb = self.starbase_tier[planet_idx];
                    if target_tier <= cur_sb {
                        continue;
                    }
                    let (ti, tb, tg, tr) = starbase_build_cost(target_tier);
                    let (pi, pb, pg, pr) = starbase_build_cost(cur_sb);
                    let (ci, cb, cg, cr) = (
                        (ti - pi * 0.5).max(0.0),
                        (tb - pb * 0.5).max(0.0),
                        (tg - pg * 0.5).max(0.0),
                        (tr - pr * 0.5).max(0.0),
                    );
                    let b = planet_idx * 3;
                    if self.ship_prod_buf[planet_idx] < cr
                        || self.min_surf[b] < ci
                        || self.min_surf[b + 1] < cb
                        || self.min_surf[b + 2] < cg
                    {
                        continue;
                    }
                    self.ship_prod_buf[planet_idx] -= cr;
                    self.min_surf[b] -= ci;
                    self.min_surf[b + 1] -= cb;
                    self.min_surf[b + 2] -= cg;
                    self.factories[planet_idx] =
                        (self.factories[planet_idx] - (cr * 0.08).min(6.0)).max(8.0);
                    let new_max_hp = starbase_base_hp(target_tier);
                    self.starbase_tier[planet_idx] = target_tier;
                    self.starbase_hp[planet_idx] = new_max_hp;
                    self.starbase_max_hp[planet_idx] = new_max_hp;
                    self.intel_planets[e * n + planet_idx].starbase_tier = target_tier;
                    self.empires[e]
                        .pending_hub_orders
                        .retain(|&(p, k)| !(p == planet_idx && k == HubKind::Starbase));
                    if !silent && target_tier >= 3 {
                        let msg = format!(
                            "{} commissioned {} at #{}!",
                            self.empires[e].race.name,
                            starbase_tier_name(target_tier),
                            planet_idx + 1
                        );
                        let col = self.empires[e].race.color.clone();
                        self.push_event(msg, &col);
                    }
                }
                GovernorAction::DispatchFreighter {
                    from,
                    target,
                    min_type,
                    cargo,
                    docked_fleet_idx,
                } => {
                    if from >= n || target >= n || self.owner[from] != emp_u8 {
                        continue;
                    }
                    let m = (min_type as usize).min(2);
                    let b = from * 3;
                    if let Some(f_idx) = docked_fleet_idx {
                        if f_idx < self.fleets.len()
                            && self.fleets[f_idx].orbiting
                            && self.fleets[f_idx].emp == emp_u8
                        {
                            let actual_cargo = cargo.min(self.min_surf[b + m]).max(1.0);
                            self.min_surf[b + m] -= actual_cargo;
                            let tx = self.pos_x[target];
                            let ty = self.pos_y[target];
                            let fl = &mut self.fleets[f_idx];
                            fl.orbiting = false;
                            fl.from = from;
                            fl.target = target;
                            fl.min_type = m as u8;
                            fl.power = actual_cargo;
                            fl.max_power = fl.max_power.max(actual_cargo);
                            fl.steer_x = tx;
                            fl.steer_y = ty;
                            fl.salvage_mode = 0;
                            self.orbit_freight[from] = -1;
                            self.orbit_count[from] = self.orbit_count[from].saturating_sub(1);
                        }
                    } else {
                        let (hi, hb, hg) = self.empires[e].ship_mineral_cost(FleetClass::Freight);
                        let (ci, cb, cg) = (hi * 0.5, hb * 0.5, hg * 0.5);
                        if self.ship_prod_buf[from] >= ec.effective_ship_cost * 0.35
                            && self.min_surf[b] >= ci
                            && self.min_surf[b + 1] >= cb
                            && self.min_surf[b + 2] >= cg
                        {
                            self.ship_prod_buf[from] -= ec.effective_ship_cost * 0.35;
                            self.min_surf[b] -= ci;
                            self.min_surf[b + 1] -= cb;
                            self.min_surf[b + 2] -= cg;
                            let actual_cargo = cargo.min(self.min_surf[b + m]).max(1.0);
                            self.min_surf[b + m] = (self.min_surf[b + m] - actual_cargo).max(0.0);
                            self.spawn_fleet(
                                from,
                                target,
                                emp_u8,
                                FleetClass::Freight,
                                actual_cargo,
                                1,
                                m as u8,
                                -1,
                                silent,
                            );
                        }
                    }
                }
                GovernorAction::DispatchSalvage {
                    from,
                    dest_x,
                    dest_y,
                    docked_fleet_idx,
                } => {
                    if from >= n || self.owner[from] != emp_u8 {
                        continue;
                    }
                    let b = from * 3;
                    if let Some(f_idx) = docked_fleet_idx {
                        if f_idx < self.fleets.len()
                            && self.fleets[f_idx].orbiting
                            && self.fleets[f_idx].emp == emp_u8
                        {
                            let fl = &mut self.fleets[f_idx];
                            fl.orbiting = false;
                            fl.from = from;
                            fl.target = from;
                            fl.steer_x = dest_x;
                            fl.steer_y = dest_y;
                            fl.salvage_mode = 1;
                            fl.cargo_iron = 0.0;
                            fl.cargo_bora = 0.0;
                            fl.cargo_germ = 0.0;
                            fl.power = 12.0;
                            fl.max_power = ec.cargo_cap;
                            self.orbit_freight[from] = -1;
                            self.orbit_count[from] = self.orbit_count[from].saturating_sub(1);
                        }
                    } else {
                        let (hi, hb, hg) = self.empires[e].ship_mineral_cost(FleetClass::Freight);
                        let (ci, cb, cg) = (hi * 0.5, hb * 0.5, hg * 0.5);
                        if self.ship_prod_buf[from] >= ec.effective_ship_cost * 0.35
                            && self.min_surf[b] >= ci
                            && self.min_surf[b + 1] >= cb
                            && self.min_surf[b + 2] >= cg
                        {
                            self.ship_prod_buf[from] -= ec.effective_ship_cost * 0.35;
                            self.min_surf[b] -= ci;
                            self.min_surf[b + 1] -= cb;
                            self.min_surf[b + 2] -= cg;
                            let id = self.next_fleet_id;
                            self.next_fleet_id += 1;
                            self.fleets.push(Fleet {
                                id,
                                x: self.pos_x[from],
                                y: self.pos_y[from],
                                from,
                                target: from,
                                emp: emp_u8,
                                fleet_type: FleetClass::Freight,
                                min_type: 0,
                                power: 12.0,
                                max_power: ec.cargo_cap,
                                hunting: false,
                                used_wh: false,
                                steer_x: dest_x,
                                steer_y: dest_y,
                                wh_route: -1,
                                salvage_mode: 1,
                                cargo_iron: 0.0,
                                cargo_bora: 0.0,
                                cargo_germ: 0.0,
                                orbiting: false,
                                upkeep: FleetClass::Freight.upkeep(),
                                ship_count: 1,
                            });
                        }
                    }
                }
                GovernorAction::LaunchFleet {
                    from,
                    target,
                    class,
                    ship_count,
                    wh_route,
                    docked_fleet_idx,
                } => {
                    if from >= n || target >= n || self.owner[from] != emp_u8 {
                        continue;
                    }
                    let b = from * 3;
                    if class == FleetClass::Col {
                        let col_res_cost =
                            ec.effective_ship_cost * self.cfg.col_cost_mult.clamp(0.5, 5.0);
                        if self.ship_prod_buf[from] < col_res_cost
                            || self.min_surf[b] < ec.col_i
                            || self.min_surf[b + 2] < ec.col_g
                        {
                            continue;
                        }
                        self.ship_prod_buf[from] -= col_res_cost;
                        self.min_surf[b] -= ec.col_i;
                        self.min_surf[b + 2] -= ec.col_g;
                        self.pop[from] = (self.pop[from] - 10.0).max(5.0);
                        self.claim_mask[target] |= my_bit;
                        let pow = self.compute_fleet_power(e, FleetClass::Col, 1);
                        self.spawn_fleet(
                            from,
                            target,
                            emp_u8,
                            FleetClass::Col,
                            pow,
                            1,
                            0,
                            wh_route,
                            silent,
                        );
                    } else if let Some(f_idx) = docked_fleet_idx {
                        if f_idx >= self.fleets.len()
                            || !self.fleets[f_idx].orbiting
                            || self.fleets[f_idx].emp != emp_u8
                        {
                            continue;
                        }
                        // Optionally reinforce the docked fleet with newly built ships before launching
                        let fl_class = self.fleets[f_idx].fleet_type;
                        if ship_count > 0 && fl_class == class {
                            let (ci_u, cb_u, cg_u) = self.empires[e].ship_mineral_cost(class);
                            let max_res = (self.ship_prod_buf[from]
                                / ec.effective_ship_cost.max(1.0))
                            .floor() as u16;
                            let max_i = (self.min_surf[b] / ci_u.max(0.1)).floor() as u16;
                            let max_b = (self.min_surf[b + 1] / cb_u.max(0.1)).floor() as u16;
                            let max_g = (self.min_surf[b + 2] / cg_u.max(0.1)).floor() as u16;
                            let add_cnt =
                                ship_count.min(max_res).min(max_i).min(max_b).min(max_g);
                            if add_cnt > 0 {
                                let cnt_f = add_cnt as f32;
                                self.ship_prod_buf[from] -= ec.effective_ship_cost * cnt_f;
                                self.min_surf[b] -= ci_u * cnt_f;
                                self.min_surf[b + 1] -= cb_u * cnt_f;
                                self.min_surf[b + 2] -= cg_u * cnt_f;
                                let add_pow = self.compute_fleet_power(e, class, add_cnt);
                                let fl = &mut self.fleets[f_idx];
                                fl.power += add_pow;
                                fl.max_power += add_pow;
                                fl.ship_count = fl.ship_count.saturating_add(add_cnt);
                                fl.upkeep = class.upkeep() * (fl.ship_count as f32);
                            }
                        }
                        self.claim_mask[target] |= my_bit;
                        let mut start_x = self.pos_x[from];
                        let mut start_y = self.pos_y[from];
                        let mut jumped_gate = false;
                        if wh_route < 0 && self.has_gate[from] != 0 && self.gate_lists[e].len() > 1
                        {
                            let direct_thresh = (self.pos_x[target] - start_x)
                                .hypot(self.pos_y[target] - start_y)
                                * 0.72;
                            for &(cand_u32, dist_to_target) in self.planet_neighbors_slice(target) {
                                if dist_to_target >= direct_thresh {
                                    break;
                                }
                                let g_idx = cand_u32 as usize;
                                if g_idx == from
                                    || self.owner[g_idx] != emp_u8
                                    || self.has_gate[g_idx] == 0
                                {
                                    continue;
                                }
                                let jdx = self.pos_x[g_idx] - start_x;
                                let jdy = self.pos_y[g_idx] - start_y;
                                if jdx * jdx + jdy * jdy <= ec.gate_range_sq {
                                    if !silent && self.pending_jump_fx.len() < 25 {
                                        self.pending_jump_fx.push(JumpFx {
                                            x1: start_x,
                                            y1: start_y,
                                            x2: self.pos_x[g_idx],
                                            y2: self.pos_y[g_idx],
                                            ttl: 12,
                                            col: "#22d3ee".to_string(),
                                        });
                                    }
                                    start_x = self.pos_x[g_idx];
                                    start_y = self.pos_y[g_idx];
                                    jumped_gate = true;
                                    self.gate_jumps_count += 1;
                                    break;
                                }
                            }
                        }
                        let (init_steer_x, init_steer_y) = if !jumped_gate && wh_route >= 0 {
                            let w = (wh_route / 2) as usize;
                            let dir = wh_route % 2;
                            if let Some(&wh) = self.wormholes.get(w) {
                                if dir == 0 {
                                    (wh.x1, wh.y1)
                                } else {
                                    (wh.x2, wh.y2)
                                }
                            } else {
                                (self.pos_x[target], self.pos_y[target])
                            }
                        } else {
                            (self.pos_x[target], self.pos_y[target])
                        };
                        let fl = &mut self.fleets[f_idx];
                        fl.orbiting = false;
                        fl.x = start_x;
                        fl.y = start_y;
                        fl.from = from;
                        fl.target = target;
                        fl.used_wh = jumped_gate;
                        fl.wh_route = if jumped_gate { -1 } else { wh_route };
                        fl.steer_x = init_steer_x;
                        fl.steer_y = init_steer_y;
                        self.orbit_warship[from] = -1;
                        self.orbit_count[from] = self.orbit_count[from].saturating_sub(1);
                    } else {
                        let (ci_u, cb_u, cg_u) = self.empires[e].ship_mineral_cost(class);
                        let max_res = (self.ship_prod_buf[from]
                            / ec.effective_ship_cost.max(1.0))
                        .floor() as u16;
                        let max_i = (self.min_surf[b] / ci_u.max(0.1)).floor() as u16;
                        let max_b = (self.min_surf[b + 1] / cb_u.max(0.1)).floor() as u16;
                        let max_g = (self.min_surf[b + 2] / cg_u.max(0.1)).floor() as u16;
                        let cnt = ship_count.min(max_res).min(max_i).min(max_b).min(max_g);
                        if cnt == 0 {
                            continue;
                        }
                        let cnt_f = cnt as f32;
                        self.ship_prod_buf[from] -= ec.effective_ship_cost * cnt_f;
                        self.min_surf[b] -= ci_u * cnt_f;
                        self.min_surf[b + 1] -= cb_u * cnt_f;
                        self.min_surf[b + 2] -= cg_u * cnt_f;
                        self.claim_mask[target] |= my_bit;
                        let pow = self.compute_fleet_power(e, class, cnt);
                        self.spawn_fleet(
                            from, target, emp_u8, class, pow, cnt, 0, wh_route, silent,
                        );
                    }
                }
                GovernorAction::BuildGarrison {
                    planet_idx,
                    class,
                    ship_count,
                } => {
                    if planet_idx >= n || self.owner[planet_idx] != emp_u8 {
                        continue;
                    }
                    let b = planet_idx * 3;
                    let (ci_u, cb_u, cg_u) = self.empires[e].ship_mineral_cost(class);
                    let max_res = (self.ship_prod_buf[planet_idx]
                        / ec.effective_ship_cost.max(1.0))
                    .floor() as u16;
                    let max_i = (self.min_surf[b] / ci_u.max(0.1)).floor() as u16;
                    let max_b = (self.min_surf[b + 1] / cb_u.max(0.1)).floor() as u16;
                    let max_g = (self.min_surf[b + 2] / cg_u.max(0.1)).floor() as u16;
                    let cnt = ship_count.min(max_res).min(max_i).min(max_b).min(max_g);
                    if cnt == 0 {
                        continue;
                    }
                    let cnt_f = cnt as f32;
                    self.ship_prod_buf[planet_idx] -= ec.effective_ship_cost * cnt_f;
                    self.min_surf[b] -= ci_u * cnt_f;
                    self.min_surf[b + 1] -= cb_u * cnt_f;
                    self.min_surf[b + 2] -= cg_u * cnt_f;
                    let g_pow = self.compute_fleet_power(e, class, cnt);

                    // Merge into an existing orbiting warship fleet of the same class at `planet_idx`
                    // so task forces can accumulate enough ships to overcome fortified enemy Starbases.
                    if let Some(existing) = self.fleets.iter_mut().find(|fl| {
                        fl.orbiting
                            && fl.emp == emp_u8
                            && fl.target == planet_idx
                            && fl.fleet_type == class
                    }) {
                        existing.power += g_pow;
                        existing.max_power += g_pow;
                        existing.ship_count = existing.ship_count.saturating_add(cnt);
                        existing.upkeep = class.upkeep() * (existing.ship_count as f32);
                        self.orbit_count[planet_idx] =
                            self.orbit_count[planet_idx].saturating_add(cnt);
                    } else {
                        let id = self.next_fleet_id;
                        self.next_fleet_id += 1;
                        let px = self.pos_x[planet_idx];
                        let py = self.pos_y[planet_idx];
                        self.fleets.push(Fleet {
                            id,
                            x: px,
                            y: py,
                            from: planet_idx,
                            target: planet_idx,
                            emp: emp_u8,
                            fleet_type: class,
                            min_type: 0,
                            power: g_pow,
                            max_power: g_pow,
                            hunting: false,
                            used_wh: false,
                            steer_x: px,
                            steer_y: py,
                            wh_route: -1,
                            salvage_mode: 0,
                            cargo_iron: 0.0,
                            cargo_bora: 0.0,
                            cargo_germ: 0.0,
                            orbiting: true,
                            upkeep: class.upkeep() * cnt_f,
                            ship_count: cnt,
                        });
                        self.orbit_count[planet_idx] =
                            self.orbit_count[planet_idx].saturating_add(cnt);
                    }
                }
            }
        }
    }

    fn resolve_target_empire_idx(&self, caller_idx: usize, raw_target: usize) -> Option<usize> {
        let num_emp = self.empires.len();
        if raw_target < num_emp && raw_target != caller_idx {
            Some(raw_target)
        } else if raw_target >= 1 && (raw_target - 1) < num_emp {
            Some(raw_target - 1)
        } else {
            None
        }
    }

    /// Returns `(min_fac_for_gate, min_hub_dist, max_gates, gate_res, gate_iron, gate_germ)` for `empire_idx`.
    pub fn stargate_requirements(&self, empire_idx: usize) -> (f32, f32, usize, f32, f32, f32) {
        let emp = &self.empires[empire_idx];
        let prt_it = !self.cfg.ignore_racial_traits && emp.race.prt == "PRT_IT";
        let min_fac = if prt_it { 34.0 } else { 58.0 };
        let min_hub_dist = if prt_it { 95.0 } else { 155.0 };
        let max_gates = if prt_it { 12 } else { 6 };
        let gate_cap_cost = if prt_it {
            self.cfg.gate_cost
        } else {
            self.cfg.gate_cost * 1.4
        };
        let gate_res = gate_cap_cost * 1.1;
        let gate_iron = gate_cap_cost * 0.7;
        let gate_germ = gate_cap_cost * 0.7;
        (min_fac, min_hub_dist, max_gates, gate_res, gate_iron, gate_germ)
    }

    /// Processes a queue of high-level strategic commands (`StrategicCommand`) for `empire_idx`
    /// and returns deterministic per-command validation & execution feedback.
    pub fn submit_commands(
        &mut self,
        empire_idx: usize,
        cmds: Vec<StrategicCommand>,
    ) -> Vec<CommandFeedback> {
        let mut feedback = Vec::with_capacity(cmds.len());
        if empire_idx >= self.empires.len() {
            return feedback;
        }
        let emp_u8 = (empire_idx + 1) as u8;
        let n = self.owner.len();
        let (sector_cols, sector_rows) = self.sector_grid_dims();
        let num_sectors = sector_cols * sector_rows;

        for cmd in cmds {
            match cmd {
                StrategicCommand::SetPolicy(patch) => {
                    self.empires[empire_idx].race.doctrine.apply_patch(&patch);
                    feedback.push(CommandFeedback {
                        command: "setPolicy".to_string(),
                        status: "applied".to_string(),
                        reason: None,
                    });
                }
                StrategicCommand::SetDiplomacy {
                    target_emp,
                    stance,
                } => {
                    if let Some(target_idx) =
                        self.resolve_target_empire_idx(empire_idx, target_emp)
                    {
                        self.set_relation(empire_idx, target_idx, stance);
                        self.empires[empire_idx].contact_mask |= 1u16 << target_idx;
                        self.empires[target_idx].contact_mask |= 1u16 << empire_idx;
                        let sender_name = self.empires[empire_idx].race.name.clone();
                        self.push_empire_alert(
                            target_idx,
                            format!("{sender_name} changed diplomatic stance to {stance:?}"),
                        );
                        feedback.push(CommandFeedback {
                            command: format!("setDiplomacy(targetEmp={target_emp}, {stance:?})"),
                            status: "applied".to_string(),
                            reason: None,
                        });
                    } else {
                        feedback.push(CommandFeedback {
                            command: format!("setDiplomacy(targetEmp={target_emp})"),
                            status: "rejected".to_string(),
                            reason: Some(format!("Invalid target empire ID {target_emp}")),
                        });
                    }
                }
                StrategicCommand::SetSectorDirective { sector_id, mode } => {
                    let s = sector_id as usize;
                    if s < num_sectors.max(64) {
                        if s >= self.empires[empire_idx].sector_directives.len() {
                            self.empires[empire_idx]
                                .sector_directives
                                .resize(s + 1, SectorMode::Balanced);
                        }
                        self.empires[empire_idx].sector_directives[s] = mode;
                        feedback.push(CommandFeedback {
                            command: format!("setSectorDirective(sectorId={sector_id}, {mode:?})"),
                            status: "applied".to_string(),
                            reason: None,
                        });
                    } else {
                        feedback.push(CommandFeedback {
                            command: format!("setSectorDirective(sectorId={sector_id})"),
                            status: "rejected".to_string(),
                            reason: Some(format!("Sector {sector_id} out of grid bounds")),
                        });
                    }
                }
                StrategicCommand::BuildHub { planet_idx, kind } => {
                    let cmd_label = format!("buildHub(planetId={planet_idx}, {kind:?})");
                    if planet_idx >= n || self.owner[planet_idx] != emp_u8 {
                        feedback.push(CommandFeedback {
                            command: cmd_label,
                            status: "rejected".to_string(),
                            reason: Some(format!(
                                "Planet {planet_idx} does not exist or is not owned by your empire"
                            )),
                        });
                        continue;
                    }
                    match kind {
                        HubKind::Stargate => {
                            let (min_fac, min_hub_dist, max_gates, gate_res, gate_iron, gate_germ) =
                                self.stargate_requirements(empire_idx);
                            let mut reasons: Vec<String> = Vec::new();
                            if !self.cfg.scenario_modifiers.allow_stargates {
                                reasons.push("Stargates are disabled in this scenario".to_string());
                            }
                            if self.empires[empire_idx].tech_cache.stargate_max_range <= 0.0 {
                                reasons.push("Stargate technology not yet unlocked".to_string());
                            }
                            if self.has_gate[planet_idx] != 0 {
                                reasons.push(format!("Planet {planet_idx} already has a Stargate"));
                            }
                            if self.gate_lists[empire_idx].len() >= max_gates {
                                reasons.push(format!(
                                    "Empire reached Stargate limit ({}/{})",
                                    self.gate_lists[empire_idx].len(),
                                    max_gates
                                ));
                            }
                            let fac_here = self.factories[planet_idx];
                            if fac_here + 0.01 < min_fac {
                                reasons.push(format!(
                                    "Planet {planet_idx} has {:.1} factories (requires >= {:.1})",
                                    fac_here, min_fac
                                ));
                            }
                            let px = self.pos_x[planet_idx];
                            let py = self.pos_y[planet_idx];
                            for &g in &self.gate_lists[empire_idx] {
                                let dist = (self.pos_x[g] - px).hypot(self.pos_y[g] - py);
                                if dist < min_hub_dist {
                                    reasons.push(format!(
                                        "Planet {planet_idx} is {:.0} ly from existing Stargate on planet {g} (min {:.0} ly)",
                                        dist, min_hub_dist
                                    ));
                                    break;
                                }
                            }
                            if !reasons.is_empty() {
                                feedback.push(CommandFeedback {
                                    command: cmd_label,
                                    status: "rejected".to_string(),
                                    reason: Some(reasons.join("; ")),
                                });
                                continue;
                            }
                            if !self.empires[empire_idx]
                                .pending_hub_orders
                                .contains(&(planet_idx, kind))
                            {
                                self.empires[empire_idx]
                                    .pending_hub_orders
                                    .push((planet_idx, kind));
                            }
                            let b = planet_idx * 3;
                            let miss_r = (gate_res - self.ship_prod_buf[planet_idx]).max(0.0);
                            let miss_i = (gate_iron - self.min_surf[b]).max(0.0);
                            let miss_g = (gate_germ - self.min_surf[b + 2]).max(0.0);
                            let reason = if miss_r > 0.1 || miss_i > 0.1 || miss_g > 0.1 {
                                Some(format!(
                                    "Queued awaiting resources (missing {:.1} shipProdBuf, {:.1} iron, {:.1} germ)",
                                    miss_r, miss_i, miss_g
                                ))
                            } else {
                                Some("Queued — resources ready, will build on next tick".to_string())
                            };
                            feedback.push(CommandFeedback {
                                command: cmd_label,
                                status: "queued".to_string(),
                                reason,
                            });
                        }
                        HubKind::Starbase => {
                            let target_tier =
                                self.empires[empire_idx].tech_cache.best_starbase_tier.max(1);
                            let cur_sb = self.starbase_tier[planet_idx];
                            if cur_sb >= target_tier {
                                feedback.push(CommandFeedback {
                                    command: cmd_label,
                                    status: "rejected".to_string(),
                                    reason: Some(format!(
                                        "Planet {planet_idx} already has Starbase tier {cur_sb} (max unlocked tier {target_tier})"
                                    )),
                                });
                                continue;
                            }
                            if !self.empires[empire_idx]
                                .pending_hub_orders
                                .contains(&(planet_idx, kind))
                            {
                                self.empires[empire_idx]
                                    .pending_hub_orders
                                    .push((planet_idx, kind));
                            }
                            let (ti, tb, tg, tr) = starbase_build_cost(target_tier);
                            let (pi, pb, pg, pr) = starbase_build_cost(cur_sb);
                            let (ci, cb, cg, cr) = (
                                (ti - pi * 0.5).max(0.0),
                                (tb - pb * 0.5).max(0.0),
                                (tg - pg * 0.5).max(0.0),
                                (tr - pr * 0.5).max(0.0),
                            );
                            let b = planet_idx * 3;
                            let miss_r = (cr - self.ship_prod_buf[planet_idx]).max(0.0);
                            let miss_i = (ci - self.min_surf[b]).max(0.0);
                            let miss_b = (cb - self.min_surf[b + 1]).max(0.0);
                            let miss_g = (cg - self.min_surf[b + 2]).max(0.0);
                            let reason = if miss_r > 0.1 || miss_i > 0.1 || miss_b > 0.1 || miss_g > 0.1 {
                                Some(format!(
                                    "Queued for Starbase T{target_tier} (missing {:.1} res, {:.1} iron, {:.1} bora, {:.1} germ)",
                                    miss_r, miss_i, miss_b, miss_g
                                ))
                            } else {
                                Some(format!("Queued for Starbase T{target_tier} — will build on next tick"))
                            };
                            feedback.push(CommandFeedback {
                                command: cmd_label,
                                status: "queued".to_string(),
                                reason,
                            });
                        }
                    }
                }
                StrategicCommand::SendDipMessage { to_emp, text } => {
                    if let Some(target_idx) = self.resolve_target_empire_idx(empire_idx, to_emp) {
                        self.empires[empire_idx].contact_mask |= 1u16 << target_idx;
                        self.empires[target_idx].contact_mask |= 1u16 << empire_idx;
                        let msg = DipMessage {
                            tick: self.ticks,
                            year: self.year,
                            from_emp: (empire_idx + 1) as u8,
                            to_emp: (target_idx + 1) as u8,
                            text: text.clone(),
                        };
                        if self.empires[target_idx].inbox.len() >= 16 {
                            self.empires[target_idx].inbox.remove(0);
                        }
                        self.empires[target_idx].inbox.push(msg);
                        let sender_name = self.empires[empire_idx].race.name.clone();
                        self.push_empire_alert(
                            target_idx,
                            format!("Diplomatic transmission from {sender_name}: {text}"),
                        );
                        feedback.push(CommandFeedback {
                            command: format!("sendDipMessage(toEmp={to_emp})"),
                            status: "applied".to_string(),
                            reason: None,
                        });
                    } else {
                        feedback.push(CommandFeedback {
                            command: format!("sendDipMessage(toEmp={to_emp})"),
                            status: "rejected".to_string(),
                            reason: Some(format!("Invalid target empire ID {to_emp}")),
                        });
                    }
                }
                StrategicCommand::ScoutSector { sector_id } => {
                    let cmd_label = format!("scoutSector(sectorId={sector_id})");
                    if (sector_id as usize) >= num_sectors {
                        feedback.push(CommandFeedback {
                            command: cmd_label,
                            status: "rejected".to_string(),
                            reason: Some(format!(
                                "Sector {sector_id} out of range (0..{num_sectors})"
                            )),
                        });
                    } else {
                        let order = ExplorationOrder::ScoutSector { sector_id };
                        if !self.empires[empire_idx]
                            .active_exploration_orders
                            .contains(&order)
                        {
                            self.empires[empire_idx]
                                .active_exploration_orders
                                .push(order);
                        }
                        feedback.push(CommandFeedback {
                            command: cmd_label,
                            status: "queued".to_string(),
                            reason: Some(format!(
                                "Priority scout mission queued for Sector #{sector_id}"
                            )),
                        });
                    }
                }
                StrategicCommand::ProbeWormhole { wormhole_id } => {
                    let cmd_label = format!("probeWormhole(wormholeId={wormhole_id})");
                    if wormhole_id >= self.wormholes.len()
                        || !self.is_wormhole_discovered(empire_idx, wormhole_id)
                    {
                        feedback.push(CommandFeedback {
                            command: cmd_label,
                            status: "rejected".to_string(),
                            reason: Some(format!(
                                "Wormhole #{wormhole_id} is unknown or undiscovered by your empire"
                            )),
                        });
                    } else if self.is_wormhole_end_a_discovered(empire_idx, wormhole_id)
                        && self.is_wormhole_end_b_discovered(empire_idx, wormhole_id)
                    {
                        feedback.push(CommandFeedback {
                            command: cmd_label,
                            status: "applied".to_string(),
                            reason: Some(format!(
                                "Both endpoints of Wormhole #{wormhole_id} are already mapped"
                            )),
                        });
                    } else {
                        let order = ExplorationOrder::ProbeWormhole { wormhole_id };
                        if !self.empires[empire_idx]
                            .active_exploration_orders
                            .contains(&order)
                        {
                            self.empires[empire_idx]
                                .active_exploration_orders
                                .push(order);
                        }
                        feedback.push(CommandFeedback {
                            command: cmd_label,
                            status: "queued".to_string(),
                            reason: Some(format!(
                                "Priority wormhole transit probe queued for Wormhole #{wormhole_id}"
                            )),
                        });
                    }
                }
            }
        }
        feedback
    }

    /// Generates a compact, strictly Fog-of-War-isolated, topology-only observation report for `empire_idx`.
    pub fn take_agent_observation(&self, empire_idx: usize) -> AgentObservation {
        let num_emp = self.empires.len();
        let e = empire_idx.min(num_emp.saturating_sub(1));
        let emp_u8 = (e + 1) as u8;
        let n = self.owner.len();
        let (sector_cols, sector_rows) = self.sector_grid_dims();
        let num_sectors = sector_cols * sector_rows;

        let mut sectors: Vec<SectorSummary> = (0..num_sectors)
            .map(|s| SectorSummary {
                sector_id: s as u16,
                row: (s / sector_cols) as u8,
                col: (s % sector_cols) as u8,
                directive: self.empires[e]
                    .sector_directives
                    .get(s)
                    .copied()
                    .unwrap_or(SectorMode::Balanced),
                unexplored_stars: 0,
                own_planets: 0,
                own_res_per_year: 0.0,
                neutral_known: 0,
                neutral_pen_scanned: 0,
                avg_neutral_hab: None,
                enemy_planets_by_emp: Vec::new(),
                detected_enemy_fleet_power: 0.0,
                avg_intel_age_years: 0.0,
            })
            .collect();

        let mut sector_intel_age_sum = vec![0.0f32; num_sectors];
        let mut sector_intel_count = vec![0u32; num_sectors];
        let mut sector_neutral_hab_sum = vec![0.0f32; num_sectors];
        let mut sector_neutral_hab_cnt = vec![0u32; num_sectors];

        let mut own_planets_count = 0usize;
        let mut explored_foreign_or_neutral = 0usize;
        let mut unexplored_planets_count = 0usize;
        let mut neutral_known_planets = 0usize;
        let mut enemy_known_planets = 0usize;

        let mut own_res_sum = 0.0f32;
        let mut own_surf_iron = 0.0f32;
        let mut own_surf_bora = 0.0f32;
        let mut own_surf_germ = 0.0f32;
        let mut own_coords: Vec<(f32, f32)> = Vec::new();
        let mut own_hubs: Vec<OwnHubBrief> = Vec::new();
        let mut stargates: Vec<AgentStargateBrief> = Vec::new();
        let mut enemy_planets_raw: Vec<(u32, EnemyPlanetBrief)> = Vec::new();

        let emp_ref = &self.empires[e];
        let ignore_traits = self.cfg.ignore_racial_traits;
        let prt_pop_mod = if ignore_traits {
            1.0
        } else if emp_ref.race.prt == "PRT_HE" {
            0.5
        } else if emp_ref.race.prt == "PRT_JOAT" {
            1.2
        } else {
            1.0
        };
        let lrt_mask = if ignore_traits { 0 } else { emp_ref.race.lrt_mask };
        let terra_bonus = (emp_ref.tech_cache.terraform_ability / 100.0)
            * if lrt_mask & LRT_TT != 0 { 1.30 } else { 1.0 }
            * if !ignore_traits && emp_ref.race.prt == "PRT_CA" {
                1.25
            } else {
                1.0
            };
        let lv5_pop_mult = 170.0 * (1.0 + emp_ref.tech_lv[5] * 0.015) * prt_pop_mod;

        let intel_slice = &self.intel_planets[e * n..(e + 1) * n];
        for i in 0..n {
            let px = self.pos_x[i];
            let py = self.pos_y[i];
            let sec_id =
                sector_for_xy(px, py, self.world_w, self.world_h, sector_cols, sector_rows)
                    as usize;
            if self.owner[i] == emp_u8 {
                own_planets_count += 1;
                own_coords.push((px, py));
                let p_res = if self.last_res[i] > 0.0 {
                    self.last_res[i]
                } else {
                    self.pop[i] * 0.32 + self.factories[i]
                };
                own_res_sum += p_res;
                let b = i * 3;
                own_surf_iron += self.min_surf[b];
                own_surf_bora += self.min_surf[b + 1];
                own_surf_germ += self.min_surf[b + 2];

                sectors[sec_id].own_planets += 1;
                sectors[sec_id].own_res_per_year += p_res;

                let eff_hab = (self.hab[i] + terra_bonus).min(1.15);
                let max_pop_here = eff_hab * lv5_pop_mult;
                own_hubs.push(OwnHubBrief {
                    planet_id: i,
                    sector_id: sec_id as u16,
                    pop: round1(self.pop[i]),
                    max_pop: round1(max_pop_here),
                    factories: round1(self.factories[i]),
                    defense: round1(self.defense[i]),
                    starbase_tier: self.starbase_tier[i],
                    has_gate: self.has_gate[i] != 0,
                    ship_prod_buf: round1(self.ship_prod_buf[i]),
                    minerals: NamedMinerals::from_array([
                        self.min_surf[b],
                        self.min_surf[b + 1],
                        self.min_surf[b + 2],
                    ])
                    .round1(),
                });
                if self.has_gate[i] != 0 {
                    stargates.push(AgentStargateBrief {
                        planet_id: i,
                        owner_emp: emp_u8,
                        sector_id: sec_id as u16,
                    });
                }
            } else {
                let intel = &intel_slice[i];
                if !intel.is_explored() {
                    unexplored_planets_count += 1;
                    sectors[sec_id].unexplored_stars += 1;
                } else {
                    explored_foreign_or_neutral += 1;
                    let age_ticks = (self.ticks as u32).saturating_sub(intel.last_seen_tick);
                    let age_years = (age_ticks as f32) * DT;
                    sector_intel_age_sum[sec_id] += age_years;
                    sector_intel_count[sec_id] += 1;
                    let pen = intel.is_pen_scanned();

                    if intel.owner == 0 {
                        neutral_known_planets += 1;
                        sectors[sec_id].neutral_known += 1;
                        if pen {
                            sectors[sec_id].neutral_pen_scanned += 1;
                            sector_neutral_hab_sum[sec_id] += intel.hab_est;
                            sector_neutral_hab_cnt[sec_id] += 1;
                        }
                    } else if intel.owner != emp_u8 {
                        enemy_known_planets += 1;
                        if let Some(pair) = sectors[sec_id]
                            .enemy_planets_by_emp
                            .iter_mut()
                            .find(|(o, _)| *o == intel.owner)
                        {
                            pair.1 += 1;
                        } else {
                            sectors[sec_id].enemy_planets_by_emp.push((intel.owner, 1));
                        }
                        if intel.has_stargate() {
                            stargates.push(AgentStargateBrief {
                                planet_id: i,
                                owner_emp: intel.owner,
                                sector_id: sec_id as u16,
                            });
                        }
                        enemy_planets_raw.push((
                            intel.last_seen_tick,
                            EnemyPlanetBrief {
                                planet_id: i,
                                sector_id: sec_id as u16,
                                owner_emp: intel.owner,
                                starbase_tier: intel.starbase_tier,
                                has_stargate: intel.has_stargate(),
                                intel_age_years: round1(age_years),
                                pen_scanned: pen,
                                pop_est: if pen { Some(round1(intel.pop_est)) } else { None },
                                fac_est: if pen { Some(round1(intel.fac_est)) } else { None },
                                def_est: if pen { Some(round1(intel.def_est)) } else { None },
                            },
                        ));
                    }
                }
            }
        }

        for s in 0..num_sectors {
            sectors[s].own_res_per_year = round1(sectors[s].own_res_per_year);
            if sector_intel_count[s] > 0 {
                sectors[s].avg_intel_age_years =
                    round1(sector_intel_age_sum[s] / (sector_intel_count[s] as f32));
            }
            if sector_neutral_hab_cnt[s] > 0 {
                sectors[s].avg_neutral_hab =
                    Some(round2(sector_neutral_hab_sum[s] / (sector_neutral_hab_cnt[s] as f32)));
            }
            sectors[s].enemy_planets_by_emp.sort_by_key(|&(o, _)| o);
        }

        // Top 10 own hubs (prioritizing production, starbases, gates)
        own_hubs.sort_by(|a, b| {
            let sa = a.factories + a.pop * 0.3 + (a.starbase_tier as f32) * 15.0 + if a.has_gate { 20.0 } else { 0.0 };
            let sb = b.factories + b.pop * 0.3 + (b.starbase_tier as f32) * 15.0 + if b.has_gate { 20.0 } else { 0.0 };
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
        own_hubs.truncate(10);
        stargates.truncate(12);

        // Most recent strictly foreign enemy planets (max 12)
        enemy_planets_raw.sort_by(|a, b| b.0.cmp(&a.0));
        let enemy_planets: Vec<EnemyPlanetBrief> = enemy_planets_raw
            .into_iter()
            .take(12)
            .map(|(_, brief)| brief)
            .collect();

        let mut own_fleets_count = 0u32;
        let mut own_orbiting_count = 0u32;
        let mut own_upkeep = 0.0f32;
        let mut by_class = OwnFleetsByClass::default();
        let mut sec_orbit_cnt = vec![0u16; num_sectors];
        let mut sec_orbit_pow = vec![0.0f32; num_sectors];
        let mut sec_transit_cnt = vec![0u16; num_sectors];
        let mut sec_mission_col = vec![0u16; num_sectors];
        let mut sec_mission_atk = vec![0u16; num_sectors];
        let mut sec_mission_scout = vec![0u16; num_sectors];
        let mut sec_mission_freight = vec![0u16; num_sectors];

        for fl in &self.fleets {
            if fl.emp == emp_u8 && fl.power > 0.0 {
                own_fleets_count += 1;
                own_upkeep += fl.upkeep;
                match fl.fleet_type {
                    FleetClass::Col => by_class.colony = by_class.colony.saturating_add(1),
                    FleetClass::Freight => by_class.freighter = by_class.freighter.saturating_add(1),
                    FleetClass::Cruiser => by_class.cruiser = by_class.cruiser.saturating_add(1),
                    FleetClass::Battleship => by_class.battleship = by_class.battleship.saturating_add(1),
                    FleetClass::Bomber => by_class.bomber = by_class.bomber.saturating_add(1),
                }
                let fsec = sector_for_xy(
                    fl.x,
                    fl.y,
                    self.world_w,
                    self.world_h,
                    sector_cols,
                    sector_rows,
                ) as usize;
                if fsec < num_sectors {
                    if fl.orbiting {
                        own_orbiting_count += 1;
                        sec_orbit_cnt[fsec] = sec_orbit_cnt[fsec].saturating_add(1);
                        sec_orbit_pow[fsec] += fl.power;
                    } else {
                        sec_transit_cnt[fsec] = sec_transit_cnt[fsec].saturating_add(1);
                        if fl.fleet_type == FleetClass::Col {
                            sec_mission_col[fsec] += 1;
                        } else if fl.fleet_type == FleetClass::Freight {
                            sec_mission_freight[fsec] += 1;
                        } else if fl.target < n && self.owner[fl.target] != 0 && self.owner[fl.target] != emp_u8 {
                            sec_mission_atk[fsec] += 1;
                        } else {
                            sec_mission_scout[fsec] += 1;
                        }
                    }
                } else if fl.orbiting {
                    own_orbiting_count += 1;
                }
            }
        }

        let mut by_sector_fleets: Vec<OwnSectorFleetBrief> = Vec::new();
        for s in 0..num_sectors {
            if sec_orbit_cnt[s] == 0 && sec_transit_cnt[s] == 0 {
                continue;
            }
            let mut parts: Vec<String> = Vec::new();
            if sec_orbit_cnt[s] > 0 {
                parts.push(format!("{} garrison/idle", sec_orbit_cnt[s]));
            }
            if sec_mission_col[s] > 0 {
                parts.push(format!("{} colonizing", sec_mission_col[s]));
            }
            if sec_mission_scout[s] > 0 {
                parts.push(format!("{} scouting/transit", sec_mission_scout[s]));
            }
            if sec_mission_atk[s] > 0 {
                parts.push(format!("{} attacking", sec_mission_atk[s]));
            }
            if sec_mission_freight[s] > 0 {
                parts.push(format!("{} logistics", sec_mission_freight[s]));
            }
            by_sector_fleets.push(OwnSectorFleetBrief {
                sector_id: s as u16,
                orbiting_count: sec_orbit_cnt[s],
                orbiting_power: round1(sec_orbit_pow[s]),
                in_transit_count: sec_transit_cnt[s],
                active_mission_summary: parts.join(", "),
            });
        }

        let own_fleets_summary = OwnFleetsSummary {
            total: own_fleets_count,
            orbiting: own_orbiting_count,
            in_transit: own_fleets_count.saturating_sub(own_orbiting_count),
            by_class,
            by_sector: by_sector_fleets,
        };

        // Build pendingHubOrders status for AgentObservation
        let (_, _, _, gate_res, gate_iron, gate_germ) = self.stargate_requirements(e);
        let mut pending_hub_orders: Vec<PendingHubOrderBrief> = Vec::new();
        for &(pid, kind) in &self.empires[e].pending_hub_orders {
            if pid >= n || self.owner[pid] != emp_u8 {
                continue;
            }
            let b = pid * 3;
            let (miss_r, miss_i, miss_b, miss_g) = match kind {
                HubKind::Stargate => (
                    (gate_res - self.ship_prod_buf[pid]).max(0.0),
                    (gate_iron - self.min_surf[b]).max(0.0),
                    0.0,
                    (gate_germ - self.min_surf[b + 2]).max(0.0),
                ),
                HubKind::Starbase => {
                    let target_tier = self.empires[e].tech_cache.best_starbase_tier.max(1);
                    let cur_sb = self.starbase_tier[pid];
                    let (ti, tb, tg, tr) = starbase_build_cost(target_tier);
                    let (pi, pb, pg, pr) = starbase_build_cost(cur_sb);
                    (
                        ((tr - pr * 0.5).max(0.0) - self.ship_prod_buf[pid]).max(0.0),
                        ((ti - pi * 0.5).max(0.0) - self.min_surf[b]).max(0.0),
                        ((tb - pb * 0.5).max(0.0) - self.min_surf[b + 1]).max(0.0),
                        ((tg - pg * 0.5).max(0.0) - self.min_surf[b + 2]).max(0.0),
                    )
                }
            };
            let has_missing_min = miss_i > 0.1 || miss_b > 0.1 || miss_g > 0.1;
            let status = if miss_r <= 0.1 && !has_missing_min {
                "ready".to_string()
            } else {
                "awaiting_resources".to_string()
            };
            pending_hub_orders.push(PendingHubOrderBrief {
                planet_id: pid,
                kind,
                status,
                missing_res: round1(miss_r),
                missing_minerals: if has_missing_min {
                    Some(NamedMinerals::from_array([miss_i, miss_b, miss_g]).round1())
                } else {
                    None
                },
            });
        }

        let mut raw_enemy_fleets =
            self.compute_detected_enemy_fleets(e, sector_cols, sector_rows);
        for ef in &raw_enemy_fleets {
            if (ef.sector_id as usize) < sectors.len() {
                sectors[ef.sector_id as usize].detected_enemy_fleet_power += ef.power;
            }
        }
        for sec in &mut sectors {
            sec.detected_enemy_fleet_power = round1(sec.detected_enemy_fleet_power);
        }

        // Omit completely empty sectors
        sectors.retain(|s| {
            s.unexplored_stars > 0
                || s.own_planets > 0
                || s.neutral_known > 0
                || !s.enemy_planets_by_emp.is_empty()
                || s.detected_enemy_fleet_power > 0.0
        });

        raw_enemy_fleets.sort_by(|a, b| {
            b.power
                .partial_cmp(&a.power)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let detected_enemy_fleets: Vec<AgentEnemyFleetBrief> = raw_enemy_fleets
            .iter()
            .take(16)
            .map(|ef| {
                let nearest_sq = own_coords
                    .iter()
                    .map(|&(ox, oy)| {
                        let dx = ef.x - ox;
                        let dy = ef.y - oy;
                        dx * dx + dy * dy
                    })
                    .fold(f32::INFINITY, f32::min);
                let dist_ly = if nearest_sq.is_finite() {
                    nearest_sq.sqrt().round() as u32
                } else {
                    0
                };
                AgentEnemyFleetBrief {
                    fleet_id: ef.fleet_id,
                    owner_emp: ef.emp,
                    fleet_type: ef.fleet_type,
                    sector_id: ef.sector_id,
                    power: round1(ef.power),
                    ship_count: ef.ship_count,
                    orbiting: ef.orbiting,
                    dist_from_nearest_own_ly: dist_ly,
                }
            })
            .collect();

        let mut raw_debris = self.compute_visible_debris(e);
        raw_debris.sort_by(|a, b| {
            (b.iron + b.bora + b.germ)
                .partial_cmp(&(a.iron + a.bora + a.germ))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let visible_debris: Vec<AgentDebrisBrief> = raw_debris
            .into_iter()
            .take(12)
            .map(|d| AgentDebrisBrief {
                debris_idx: d.debris_idx,
                sector_id: d.sector_id,
                minerals: NamedMinerals::from_array([d.iron, d.bora, d.germ]).round1(),
                ttl: round1(d.ttl),
            })
            .collect();

        // Topology-only wormholes: strictly hide far endpoint (`to_sector: None`) until discovered
        let bit = 1u16 << e;
        let mut wormholes = Vec::new();
        for (w, &wh) in self.wormholes.iter().enumerate() {
            let has_a = (self.discovered_wh_a.get(w).copied().unwrap_or(0) & bit) != 0;
            let has_b = (self.discovered_wh_b.get(w).copied().unwrap_or(0) & bit) != 0;
            if !has_a && !has_b {
                continue;
            }
            let sec_a = sector_for_xy(wh.x1, wh.y1, self.world_w, self.world_h, sector_cols, sector_rows);
            let sec_b = sector_for_xy(wh.x2, wh.y2, self.world_w, self.world_h, sector_cols, sector_rows);
            let (from_sector, to_sector) = match (has_a, has_b) {
                (true, true) => (sec_a, Some(sec_b)),
                (true, false) => (sec_a, None),
                (false, true) => (sec_b, None),
                (false, false) => unreachable!(),
            };
            wormholes.push(AgentWormholeBrief {
                wormhole_id: w,
                from_sector,
                to_sector,
            });
        }
        wormholes.truncate(12);

        // Build known_empires strictly from PlanetIntel, detected fleets, and diplomatic contact/inbox
        let mut known_empires = Vec::new();
        for other_e in 0..num_emp {
            if other_e == e {
                continue;
            }
            let other_u8 = (other_e + 1) as u8;
            let mut known_p = 0u32;
            let mut est_fac = 0.0f32;
            let mut est_pop = 0.0f32;
            for intel in intel_slice {
                if intel.is_explored() && intel.owner == other_u8 {
                    known_p += 1;
                    if intel.is_pen_scanned() {
                        est_fac += intel.fac_est;
                        est_pop += intel.pop_est;
                    } else {
                        est_fac += 15.0;
                        est_pop += 30.0;
                    }
                }
            }
            let spotted_fl_pow: f32 = raw_enemy_fleets
                .iter()
                .filter(|ef| ef.emp == other_u8)
                .map(|ef| ef.power)
                .sum();
            let inbox_from_other: Vec<DipMessage> = self.empires[e]
                .inbox
                .iter()
                .filter(|m| m.from_emp == other_u8)
                .cloned()
                .collect();
            let has_contact = (self.empires[e].contact_mask & (1u16 << other_e)) != 0
                || known_p > 0
                || spotted_fl_pow > 0.0
                || !inbox_from_other.is_empty();

            if has_contact {
                known_empires.push(KnownEmpireSummary {
                    emp_id: other_u8,
                    name: self.empires[other_e].race.name.clone(),
                    stance: self.relations[e][other_e],
                    known_planets_count: known_p,
                    estimated_factories: round1(est_fac),
                    estimated_pop: round1(est_pop),
                    detected_fleet_power: round1(spotted_fl_pow),
                    inbox: inbox_from_other,
                });
            }
        }

        let emp = &self.empires[e];
        AgentObservation {
            galaxy_overview: GalaxyOverview {
                total_planets: n,
                grid_rows: sector_rows as u16,
                grid_cols: sector_cols as u16,
                explored_planets: own_planets_count + explored_foreign_or_neutral,
                unexplored_planets: unexplored_planets_count,
                own_planets: own_planets_count,
                neutral_known_planets,
                enemy_known_planets,
                victory_goal: VictoryGoalView {
                    planet_dominance_pct: round1(self.victory_cfg.planet_share_threshold * 100.0),
                    resource_dominance_pct: round1(self.victory_cfg.res_share_threshold * 100.0),
                    max_year: round1(self.victory_cfg.max_year.unwrap_or(0.0)),
                },
            },
            empire: AgentEmpireSummary {
                empire_id: emp_u8,
                name: emp.race.name.clone(),
                prt: emp.race.prt.clone(),
                year: round1(self.year),
                tick: self.ticks,
                res_per_year: round1(own_res_sum),
                minerals: NamedMinerals::from_array([own_surf_iron, own_surf_bora, own_surf_germ])
                    .round1(),
                upkeep_ratio: round2(own_upkeep / own_res_sum.max(1.0)),
                planet_count: own_planets_count as u32,
                fleet_count: own_fleets_count,
                orbiting_fleet_count: own_orbiting_count,
                tech_levels: NamedTechLevels::from_array(emp.tech_lv).round1(),
                policy: AgentPolicyView::from(&emp.race.doctrine),
                winner_emp: self.winner.map(|w| (w + 1) as u8),
            },
            sectors,
            own_hubs,
            own_fleets_summary,
            pending_hub_orders,
            active_exploration_orders: emp.active_exploration_orders.clone(),
            enemy_planets,
            wormholes,
            stargates,
            detected_enemy_fleets,
            visible_debris,
            known_empires,
            recent_alerts: emp.recent_alerts.clone(),
        }
    }

    pub fn take_agent_observation_json(&self, empire_idx: usize) -> String {
        let obs = self.take_agent_observation(empire_idx);
        serde_json::to_string(&obs).unwrap_or_else(|_| "{}".to_string())
    }

    /// Resolves a battle between `fleets[i]` and `fleets[j]`. Returns `true` if `fleets[i]` won/survived.
    #[allow(clippy::too_many_arguments)]
    fn resolve_fleet_pair_combat(
        &mut self,
        i: usize,
        j: usize,
        emp_cache: &[EmpireTickCache; MAX_EMPIRES],
        ew_mult: f32,
        orbital_defense: bool,
        planet_idx: Option<usize>,
        silent: bool,
    ) -> bool {
        let ax = self.fleets[i].x;
        let ay = self.fleets[i].y;
        let bx = self.fleets[j].x;
        let by = self.fleets[j].y;
        let emp_a = self.fleets[i].emp;
        let emp_b = self.fleets[j].emp;
        let type_a = self.fleets[i].fleet_type;
        let type_b = self.fleets[j].fleet_type;
        let a_c = type_a.is_combat();
        let b_c = type_b.is_combat();

        self.space_battles_count += 1;

        let mut mod_a = if !b_c { 3.5 } else { 1.0 };
        let mut mod_b = if !a_c { 3.5 } else { 1.0 };
        if type_a == FleetClass::Cruiser && type_b == FleetClass::Bomber {
            mod_a = 1.9;
            mod_b = 0.55;
        }
        if type_b == FleetClass::Cruiser && type_a == FleetClass::Bomber {
            mod_b = 1.9;
            mod_a = 0.55;
        }
        if type_a == FleetClass::Battleship && type_b == FleetClass::Cruiser {
            mod_a = 1.45;
            mod_b = 0.72;
        }
        if type_b == FleetClass::Battleship && type_a == FleetClass::Cruiser {
            mod_b = 1.45;
            mod_a = 0.72;
        }

        let ea = (emp_a - 1) as usize;
        let eb = (emp_b - 1) as usize;
        if emp_cache[ea].prt_ss {
            mod_a *= 1.12;
        }
        if emp_cache[eb].prt_ss {
            mod_b *= 1.12;
        }

        mod_a = apply_torpedo_jamming(
            mod_a,
            self.empires[ea].tech_cache.weapon_is_torpedo,
            self.empires[eb].tech_cache.torpedo_jamming * ew_mult,
        );
        mod_b = apply_torpedo_jamming(
            mod_b,
            self.empires[eb].tech_cache.weapon_is_torpedo,
            self.empires[ea].tech_cache.torpedo_jamming * ew_mult,
        );

        let eff_a = (if a_c { self.fleets[i].power } else { 8.0 }) * mod_a;
        let eff_b = (if b_c { self.fleets[j].power } else { 8.0 }) * mod_b;
        let mx = (ax + bx) * 0.5;
        let my = (ay + by) * 0.5;

        if !silent && self.pending_combat_fx.len() < 35 {
            let winner_e = if eff_a >= eff_b { ea } else { eb };
            self.pending_combat_fx.push(CombatFx {
                x1: ax,
                y1: ay,
                x2: bx,
                y2: by,
                mx,
                my,
                ttl: 10,
                col: self.empires[winner_e].race.color.clone(),
            });
        }

        if eff_a >= eff_b {
            self.fleets[i].power *= 1.0 - ((eff_b / eff_a) * 0.65).min(0.85);
            self.fleets[j].power = 0.0;
            self.empires[ea].race.doctrine.kills += 1;
            let dead = self.fleets[j].clone();
            self.spawn_salvage_debris(mx, my, &dead);
            if self.trace_enabled {
                self.record_trace_event(SimEvent::SpaceBattle {
                    winner_emp: emp_a,
                    loser_emp: emp_b,
                    orbital_defense,
                    planet_idx,
                });
            }
            true
        } else {
            self.fleets[j].power *= 1.0 - ((eff_a / eff_b) * 0.65).min(0.85);
            self.fleets[i].power = 0.0;
            self.empires[eb].race.doctrine.kills += 1;
            let dead = self.fleets[i].clone();
            self.spawn_salvage_debris(mx, my, &dead);
            if self.trace_enabled {
                self.record_trace_event(SimEvent::SpaceBattle {
                    winner_emp: emp_b,
                    loser_emp: emp_a,
                    orbital_defense,
                    planet_idx,
                });
            }
            false
        }
    }

    fn check_victory_conditions(&mut self) {
        if !self.victory_cfg.enabled || self.winner.is_some() {
            return;
        }
        let num_emp = self.empires.len();
        if num_emp == 0 {
            return;
        }
        let total_planets = self.owner.len();
        if total_planets == 0 {
            return;
        }

        let mut emp_planets = [0u32; MAX_EMPIRES];
        let mut inhabited = 0u32;
        for &o in &self.owner {
            if o > 0 {
                let e = (o - 1) as usize;
                if e < num_emp {
                    emp_planets[e] += 1;
                    inhabited += 1;
                }
            }
        }

        // 1. Elimination Victory: only one empire has any planets and no other empire has active fleets
        if self.victory_cfg.elimination_victory && num_emp >= 2 && inhabited > 0 {
            let mut alive_count = 0usize;
            let mut last_alive = 0usize;
            for e in 0..num_emp {
                let emp_u8 = (e + 1) as u8;
                let has_fleets = self.fleets.iter().any(|fl| fl.emp == emp_u8 && fl.power > 0.0);
                if emp_planets[e] > 0 || has_fleets {
                    alive_count += 1;
                    last_alive = e;
                }
            }
            if alive_count == 1 {
                self.winner = Some(last_alive);
                self.record_trace_event(SimEvent::VictoryTriggered {
                    winner_emp: (last_alive + 1) as u8,
                    reason: "EliminationVictory".to_string(),
                    year: self.year,
                });
                return;
            }
        }

        // 2. Planet / Resource Dominance Victory
        // Evaluated against total_planets on small sandbox maps, or against inhabited planets once >= 50% of galaxy is colonized
        let min_req = self.victory_cfg.min_planets_for_share.max(1);
        let p_thresh = self.victory_cfg.planet_share_threshold.clamp(0.50, 1.0);
        let r_thresh = self.victory_cfg.res_share_threshold.clamp(0.50, 1.0);
        let total_res: f32 = self.tick_stats.iter().map(|s| s.res).sum();
        let galaxy_settled = (inhabited as usize) * 2 >= total_planets || total_planets <= 12;

        if galaxy_settled && inhabited >= min_req {
            let denom_planets = if total_planets <= 12 {
                (total_planets as f32).max(1.0)
            } else {
                (inhabited as f32).max(1.0)
            };
            for e in 0..num_emp {
                if emp_planets[e] < min_req {
                    continue;
                }
                let p_share = emp_planets[e] as f32 / denom_planets;
                if p_share >= p_thresh {
                    self.winner = Some(e);
                    self.record_trace_event(SimEvent::VictoryTriggered {
                        winner_emp: (e + 1) as u8,
                        reason: format!("PlanetDominance({:.0}%)", p_share * 100.0),
                        year: self.year,
                    });
                    return;
                }
                if total_res > 1.0 {
                    let r_share = self.tick_stats[e].res / total_res;
                    if r_share >= r_thresh && emp_planets[e] * 2 >= inhabited {
                        self.winner = Some(e);
                        self.record_trace_event(SimEvent::VictoryTriggered {
                            winner_emp: (e + 1) as u8,
                            reason: format!("ResourceDominance({:.0}%)", r_share * 100.0),
                            year: self.year,
                        });
                        return;
                    }
                }
            }
        }

        // 3. Optional Max Year Time Limit Victory
        if let Some(max_yr) = self.victory_cfg.max_year {
            if self.year >= max_yr {
                let mut best_e = 0usize;
                for e in 1..num_emp {
                    if self.tick_stats[e].res > self.tick_stats[best_e].res
                        || (self.tick_stats[e].res == self.tick_stats[best_e].res
                            && emp_planets[e] > emp_planets[best_e])
                    {
                        best_e = e;
                    }
                }
                self.winner = Some(best_e);
                self.record_trace_event(SimEvent::VictoryTriggered {
                    winner_emp: (best_e + 1) as u8,
                    reason: format!("MaxYearReached({max_yr:.0})"),
                    year: self.year,
                });
            }
        }
    }

    fn audit_tick_invariants(&mut self) {
        let n = self.owner.len();
        for i in 0..n {
            let b = i * 3;
            let vals = [
                self.pop[i],
                self.mines[i],
                self.factories[i],
                self.defense[i],
                self.min_surf[b],
                self.min_surf[b + 1],
                self.min_surf[b + 2],
                self.min_crust[b],
                self.min_crust[b + 1],
                self.min_crust[b + 2],
                self.starbase_hp[i],
            ];
            if vals.iter().any(|&v| !v.is_finite() || v < -1e-4) && self.anomalies.len() < 1024 {
                self.anomalies.push(SimAnomaly {
                    tick: self.ticks,
                    year: self.year,
                    kind: "PlanetStateInvariantViolation".to_string(),
                    message: format!("Planet #{i} has NaN or negative state values: {vals:?}"),
                    event: None,
                });
            }
        }

        for fl in &self.fleets {
            if !fl.x.is_finite() || !fl.y.is_finite() || !fl.power.is_finite() || fl.power < -1e-4 {
                if self.anomalies.len() < 1024 {
                    self.anomalies.push(SimAnomaly {
                        tick: self.ticks,
                        year: self.year,
                        kind: "FleetStateInvariantViolation".to_string(),
                        message: format!(
                            "Fleet #{} has invalid pos/power: ({}, {}, pow={})",
                            fl.id, fl.x, fl.y, fl.power
                        ),
                        event: None,
                    });
                }
            }
            if fl.fleet_type == FleetClass::Freight {
                let cargo = fl.cargo_iron + fl.cargo_bora + fl.cargo_germ;
                let cap = fl.max_power.max(45.0);
                if cargo > cap + 1e-3 && self.anomalies.len() < 1024 {
                    self.anomalies.push(SimAnomaly {
                        tick: self.ticks,
                        year: self.year,
                        kind: "FreighterOverCapInFlight".to_string(),
                        message: format!(
                            "Freighter #{} carries {cargo:.2} kT > cap {cap:.2} kT",
                            fl.id
                        ),
                        event: None,
                    });
                }
            }
        }

        // Prune any destroyed salvage freighters from pending_salvage_loads so open transactions never leak
        if !self.auditor.pending_salvage_loads.is_empty() {
            let fleets_ref = &self.fleets;
            self.auditor
                .pending_salvage_loads
                .retain(|&fid, _| fleets_ref.iter().any(|f| f.id == fid));
        }
    }

    /// Computes unit combat power for a single ship (`ship_count == 1`) of `class` for empire `e`,
    /// strictly determined by unlocked hull, weapons, armor, shields, and racial traits (no free scaling from planetary factory count).
    pub fn compute_unit_power(&self, e: usize, class: FleetClass) -> f32 {
        let emp = &self.empires[e];
        let tc = &emp.tech_cache;
        let hull = emp.hull_for_class(class);

        let wep_scale = (1.0 + (tc.weapon_power / 40.0).sqrt() * 0.80)
            * if emp.race.prt == "PRT_WM" { 1.12 } else { 1.0 };
        let shield_mult = if emp.race.lrt_mask & LRT_RS != 0 {
            1.25
        } else {
            1.0
        };
        let dur_scale =
            1.0 + ((hull.armor + tc.armor_val + tc.shield_val * shield_mult) / 400.0).sqrt() * 0.45;
        let class_mod = match class {
            FleetClass::Battleship => 1.32,
            FleetClass::Bomber => 1.10 + (tc.bomb_power * 0.03).min(0.45),
            FleetClass::Cruiser => 1.05,
            _ => 0.8,
        };
        40.0 * wep_scale * dur_scale * class_mod
    }

    /// Computes total combat power for a fleet of `ship_count` ships, paid for proportionally in resources and minerals.
    pub fn compute_fleet_power(&self, e: usize, class: FleetClass, ship_count: u16) -> f32 {
        self.compute_unit_power(e, class) * (ship_count.max(1) as f32)
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_fleet(
        &mut self,
        from_idx: usize,
        target_idx: usize,
        emp_u8: u8,
        fleet_type: FleetClass,
        power: f32,
        ship_count: u16,
        min_type: u8,
        wh_route: i16,
        silent: bool,
    ) {
        let e = (emp_u8 - 1) as usize;
        let mut start_x = self.pos_x[from_idx];
        let mut start_y = self.pos_y[from_idx];
        let mut jumped_gate = false;
        let gate_range = self.empires[e].tech_cache.stargate_max_range.max(250.0);

        if wh_route < 0 && self.has_gate[from_idx] != 0 && self.gate_lists[e].len() > 1 {
            // Find the closest friendly Stargate to the target via target's sorted neighbor list (O(1))
            let direct_thresh =
                (self.pos_x[target_idx] - start_x).hypot(self.pos_y[target_idx] - start_y) * 0.72;
            let gate_range_sq = gate_range * gate_range;
            let mut best_gate: Option<usize> = None;
            for &(cand_u32, dist_to_target) in self.planet_neighbors_slice(target_idx) {
                if dist_to_target >= direct_thresh {
                    break;
                }
                let g_idx = cand_u32 as usize;
                if g_idx == from_idx || self.owner[g_idx] != emp_u8 || self.has_gate[g_idx] == 0 {
                    continue;
                }
                let jdx = self.pos_x[g_idx] - start_x;
                let jdy = self.pos_y[g_idx] - start_y;
                if jdx * jdx + jdy * jdy <= gate_range_sq {
                    best_gate = Some(g_idx);
                    break;
                }
            }
            if let Some(g_idx) = best_gate {
                if !silent && self.pending_jump_fx.len() < 25 {
                    self.pending_jump_fx.push(JumpFx {
                        x1: start_x,
                        y1: start_y,
                        x2: self.pos_x[g_idx],
                        y2: self.pos_y[g_idx],
                        ttl: 12,
                        col: "#22d3ee".to_string(),
                    });
                }
                start_x = self.pos_x[g_idx];
                start_y = self.pos_y[g_idx];
                jumped_gate = true;
                self.gate_jumps_count += 1;
            }
        }

        let (init_steer_x, init_steer_y) = if wh_route >= 0 {
            let w = (wh_route / 2) as usize;
            let dir = wh_route % 2;
            if let Some(&wh) = self.wormholes.get(w) {
                if dir == 0 {
                    (wh.x1, wh.y1)
                } else {
                    (wh.x2, wh.y2)
                }
            } else {
                (self.pos_x[target_idx], self.pos_y[target_idx])
            }
        } else {
            (self.pos_x[target_idx], self.pos_y[target_idx])
        };

        let id = self.next_fleet_id;
        self.next_fleet_id += 1;
        self.fleets.push(Fleet {
            id,
            x: start_x,
            y: start_y,
            from: from_idx,
            target: target_idx,
            emp: emp_u8,
            fleet_type,
            min_type,
            power,
            max_power: power,
            hunting: false,
            used_wh: jumped_gate,
            steer_x: init_steer_x,
            steer_y: init_steer_y,
            wh_route: if jumped_gate { -1 } else { wh_route },
            salvage_mode: 0,
            cargo_iron: 0.0,
            cargo_bora: 0.0,
            cargo_germ: 0.0,
            orbiting: false,
            upkeep: fleet_type.upkeep() * (ship_count.max(1) as f32),
            ship_count: ship_count.max(1),
        });
    }

    fn spawn_salvage_debris(&mut self, x: f32, y: f32, dead_fleet: &Fleet) {
        if self.cfg.salvage_rate <= 0.0 {
            return;
        }
        let ttl = self.cfg.scenario_modifiers.debris_ttl.max(1.0);
        let count_u32 = dead_fleet.ship_count.max(1) as u32;
        let count_f = count_u32 as f32;
        let wreck = WreckEntry {
            emp: dead_fleet.emp,
            ship_type: dead_fleet.fleet_type,
            count: count_u32,
        };
        let (iron, bora, germ) = if dead_fleet.fleet_type == FleetClass::Freight {
            let e = (dead_fleet.emp - 1) as usize;
            let (hi, hb, hg) = self.empires[e].ship_mineral_cost(FleetClass::Freight);
            if dead_fleet.salvage_mode != 0 {
                (
                    (dead_fleet.cargo_iron + hi * 0.5 * count_f) * self.cfg.salvage_rate,
                    (dead_fleet.cargo_bora + hb * 0.5 * count_f) * self.cfg.salvage_rate,
                    (dead_fleet.cargo_germ + hg * 0.5 * count_f) * self.cfg.salvage_rate,
                )
            } else {
                let mut m = [0.0f32; 3];
                m[dead_fleet.min_type as usize] = dead_fleet.max_power * self.cfg.salvage_rate;
                (m[0], m[1], m[2])
            }
        } else {
            let e = (dead_fleet.emp - 1) as usize;
            let (ci, cb, cg) = self.empires[e].ship_mineral_cost(dead_fleet.fleet_type);
            (
                ci * count_f * self.cfg.salvage_rate,
                cb * count_f * self.cfg.salvage_rate,
                cg * count_f * self.cfg.salvage_rate,
            )
        };
        if iron + bora + germ <= 0.5 {
            return;
        }

        // Merge nearby battle wreckage within 10 ly into a single debris field
        let persistent_mode = !self.cfg.scenario_modifiers.auto_salvage || ttl > 50.0;
        if persistent_mode {
            const MERGE_RAD_SQ: f32 = 10.0 * 10.0;
            for existing in &mut self.debris_fields {
                if existing.ttl <= 0.0 || (existing.iron + existing.bora + existing.germ) <= 0.5 {
                    continue;
                }
                let dx = existing.x - x;
                let dy = existing.y - y;
                if dx * dx + dy * dy <= MERGE_RAD_SQ {
                    existing.iron += iron;
                    existing.bora += bora;
                    existing.germ += germ;
                    existing.ttl = existing.ttl.max(ttl);
                    if let Some(entry) = existing
                        .wrecks
                        .iter_mut()
                        .find(|w| w.emp == wreck.emp && w.ship_type == wreck.ship_type)
                    {
                        entry.count = entry.count.saturating_add(1);
                    } else {
                        existing.wrecks.push(wreck);
                    }
                    return;
                }
            }
        }

        let max_fields = if persistent_mode { 2000 } else { MAX_DEBRIS_FIELDS };
        if self.debris_fields.len() >= max_fields {
            self.debris_fields.swap_remove(0);
        }
        self.debris_fields.push(DebrisField {
            x,
            y,
            iron,
            bora,
            germ,
            ttl,
            wrecks: vec![wreck],
        });
    }

    pub fn update_param(&mut self, key: &str, value: f32) -> bool {
        let mut needs_reset = false;
        match key {
            "planets" => {
                self.cfg.planets = (value.round() as usize).clamp(30, 100_000);
                needs_reset = true;
            }
            "wormholePairs" => {
                self.cfg.wormhole_pairs = (value.round() as usize).clamp(0, 500);
                needs_reset = true;
            }
            "crustRichness" => {
                self.cfg.crust_richness = value.clamp(300.0, 2500.0);
                needs_reset = true;
            }
            "gateCost" => self.cfg.gate_cost = value.clamp(15.0, 120.0),
            "colCostMult" => self.cfg.col_cost_mult = (value / 100.0).clamp(0.5, 5.0),
            "interceptRad" => self.cfg.intercept_rad = value.clamp(8.0, 50.0),
            "salvageRate" => self.cfg.salvage_rate = (value / 100.0).clamp(0.0, 1.0),
            "aggroBias" => self.cfg.aggro_bias = (value / 100.0).clamp(0.0, 1.0),
            "sprawlPenalty" => self.cfg.sprawl_penalty = (value / 100.0).clamp(0.0, 0.5),
            "ignoreRacialTraits" => {
                self.cfg.ignore_racial_traits = value >= 0.5;
                self.refresh_all_tech_caches();
            }
            _ => {}
        }
        needs_reset
    }

    /// Packs hot render data into flat f32 buffers for zero-copy WASM -> JS views.
    pub fn fill_render_buffers(&mut self) {
        let n = self.owner.len();
        self.planet_render_buf.clear();
        self.planet_render_buf.reserve(n * PLANET_RENDER_STRIDE);
        for i in 0..n {
            let b = i * 3;
            let crust_sum = self.min_crust[b] + self.min_crust[b + 1] + self.min_crust[b + 2];
            let p_flags = (if self.has_gate[i] != 0 { 1 } else { 0 })
                | (if self.planet_has_ecm_jammer(i) { 2 } else { 0 })
                | (if self.orbit_count.get(i).copied().unwrap_or(0) > 0 { 4 } else { 0 });
            self.planet_render_buf.push(self.pos_x[i]);
            self.planet_render_buf.push(self.pos_y[i]);
            self.planet_render_buf.push(self.owner[i] as f32);
            self.planet_render_buf.push(self.factories[i]);
            self.planet_render_buf.push(self.hab[i]);
            self.planet_render_buf.push(p_flags as f32);
            self.planet_render_buf.push(crust_sum);
        }

        self.fleet_render_buf.clear();
        self.fleet_render_buf
            .reserve(self.fleets.len() * FLEET_RENDER_STRIDE);
        for fl in &self.fleets {
            // Orbiting/docked fleets are represented by the host star's garrison pip & System View list
            if fl.orbiting {
                continue;
            }
            let type_code = match fl.fleet_type {
                FleetClass::Col => 0.0,
                FleetClass::Cruiser => 1.0,
                FleetClass::Battleship => 2.0,
                FleetClass::Bomber => 3.0,
                FleetClass::Freight => 4.0,
            };
            let flags = (if fl.hunting { 1 } else { 0 })
                | (if fl.used_wh { 2 } else { 0 })
                | (if fl.orbiting { 4 } else { 0 });
            self.fleet_render_buf.push(fl.x);
            self.fleet_render_buf.push(fl.y);
            self.fleet_render_buf.push(fl.steer_x);
            self.fleet_render_buf.push(fl.steer_y);
            self.fleet_render_buf.push(fl.emp as f32);
            self.fleet_render_buf.push(type_code);
            self.fleet_render_buf.push(flags as f32);
        }

        self.debris_render_buf.clear();
        self.debris_render_buf
            .reserve(self.debris_fields.len() * DEBRIS_RENDER_STRIDE);
        for deb in &self.debris_fields {
            self.debris_render_buf.push(deb.x);
            self.debris_render_buf.push(deb.y);
            self.debris_render_buf.push(deb.iron + deb.bora + deb.germ);
        }
    }

    pub fn orbiting_fleets_at(&self, planet_idx: usize) -> Vec<Fleet> {
        self.fleets
            .iter()
            .filter(|fl| fl.orbiting && fl.target == planet_idx)
            .cloned()
            .collect()
    }

    pub fn planet_view(&self, i: usize) -> Option<PlanetView> {
        if i >= self.owner.len() {
            return None;
        }
        let b = i * 3;
        let emp_u8 = self.owner[i];
        let max_pop = if emp_u8 > 0 {
            let e = (emp_u8 - 1) as usize;
            let emp = &self.empires[e];
            let terra = (emp.tech_cache.terraform_ability / 100.0)
                * if emp.race.lrt_mask & LRT_TT != 0 {
                    1.30
                } else {
                    1.0
                };
            let eff_hab = (self.hab[i] + terra).min(1.15);
            let prt_pop = match emp.race.prt.as_str() {
                "PRT_HE" => 0.72,
                "PRT_JOAT" => 1.18,
                _ => 1.0,
            };
            eff_hab * 170.0 * (1.0 + emp.tech_lv[5] * 0.015) * prt_pop
        } else {
            self.hab[i] * 170.0
        };

        Some(PlanetView {
            id: i + 1,
            x: self.pos_x[i],
            y: self.pos_y[i],
            owner: emp_u8,
            has_gate: self.has_gate[i] != 0,
            has_ecm_jammer: self.planet_has_ecm_jammer(i),
            hab: self.hab[i],
            pop: self.pop[i],
            max_pop,
            mines: self.mines[i],
            factories: self.factories[i],
            act_fac: self.act_fac[i],
            max_buildings: (self.hab[i] * 170.0 / 10.0) * self.cfg.op_limit,
            iron_surf: self.min_surf[b],
            iron_crust: self.min_crust[b],
            bora_surf: self.min_surf[b + 1],
            bora_crust: self.min_crust[b + 1],
            germ_surf: self.min_surf[b + 2],
            germ_crust: self.min_crust[b + 2],
            res: self.last_res[i],
            defense: self.defense[i],
            orbiting_fleets: self.orbit_count.get(i).copied().unwrap_or(0),
            starbase_tier: self.starbase_tier.get(i).copied().unwrap_or(0),
            starbase_name: starbase_tier_name(self.starbase_tier.get(i).copied().unwrap_or(0))
                .to_string(),
            starbase_hp: self.starbase_hp.get(i).copied().unwrap_or(0.0),
            starbase_max_hp: self.starbase_max_hp.get(i).copied().unwrap_or(0.0),
        })
    }

    /// Lazily generates the deterministic star system anchored to planet `i` (< 0.5 ms, 0 RAM stored).
    pub fn get_star_system(&self, i: usize) -> Option<StarSystemView> {
        if i >= self.owner.len() {
            return None;
        }
        let b = i * 3;
        let emp_u8 = self.owner[i];
        let (owner_color, terra_bonus, max_pop) = if emp_u8 > 0 {
            let e = (emp_u8 - 1) as usize;
            let emp = &self.empires[e];
            let terra = (emp.tech_cache.terraform_ability / 100.0)
                * if emp.race.lrt_mask & LRT_TT != 0 { 1.30 } else { 1.0 }
                * if emp.race.prt == "PRT_CA" { 1.25 } else { 1.0 };
            let eff_hab = (self.hab[i] + terra).min(1.15);
            let prt_pop = match emp.race.prt.as_str() {
                "PRT_HE" => 0.72,
                "PRT_JOAT" => 1.18,
                _ => 1.0,
            };
            let mp = eff_hab * 170.0 * (1.0 + emp.tech_lv[5] * 0.015) * prt_pop;
            (emp.race.color.clone(), terra, mp.max(1.0))
        } else {
            ("#94a3b8".to_string(), 0.0, (self.hab[i] * 170.0).max(1.0))
        };

        let input = PrimaryWorldInput {
            map_seed: self.current_map_seed,
            planet_id: i + 1,
            base_hab: self.hab[i],
            terra_bonus,
            iron_crust: self.min_crust[b],
            bora_crust: self.min_crust[b + 1],
            germ_crust: self.min_crust[b + 2],
            owner: emp_u8,
            owner_color,
            pop_norm: if emp_u8 > 0 {
                (self.pop[i] / max_pop).clamp(0.0, 1.0)
            } else {
                0.0
            },
            has_starbase: emp_u8 > 0 && self.starbase_tier.get(i).copied().unwrap_or(0) > 0,
            has_stargate: self.has_gate[i] != 0,
            has_jammer: self.planet_has_ecm_jammer(i),
        };
        Some(generate_star_system(&input))
    }

    pub fn take_hud_snapshot(&mut self) -> HudSnapshot {
        self.fill_render_buffers();
        let empires = self
            .empires
            .iter()
            .map(|emp| EmpireSummaryView {
                id: emp.race.id.clone(),
                name: emp.race.name.clone(),
                color: emp.race.color.clone(),
                prt: emp.race.prt.clone(),
                lrt_mask: emp.race.lrt_mask,
                is_custom: emp.race.is_custom,
                stats: emp.last_stats,
                tech_lv: emp.tech_lv,
                perks: emp.tech_cache.perk_mask,
                doctrine: emp.race.doctrine.clone(),
                loadout: emp.tech_cache.to_loadout_view(),
            })
            .collect();

        let new_combat_fx = std::mem::take(&mut self.pending_combat_fx);
        let new_jump_fx = std::mem::take(&mut self.pending_jump_fx);

        HudSnapshot {
            year: self.year,
            ticks: self.ticks,
            map_seed: self.current_map_seed,
            world_w: self.world_w,
            world_h: self.world_h,
            dna_generation: self.dna_generation,
            empires,
            wormholes: self.wormholes.clone(),
            new_combat_fx,
            new_jump_fx,
            events: self.log_events.clone(),
            planet_count: self.owner.len(),
            fleet_count: self.fleets.len(),
            space_battles_count: self.space_battles_count,
            invasions_count: self.invasions_count,
            total_salvaged: self.total_salvaged,
            gate_jumps_count: self.gate_jumps_count,
            step_profile: if self.profile_enabled {
                Some(self.last_profile)
            } else {
                None
            },
        }
    }

    pub fn take_reports_snapshot(&self) -> ReportsSnapshot {
        let planets = (0..self.owner.len())
            .filter_map(|i| self.planet_view(i))
            .collect();
        ReportsSnapshot {
            planets,
            fleets: self.fleets.clone(),
            debris_fields: self.debris_fields.clone(),
        }
    }

    pub fn take_snapshot(&mut self) -> SimSnapshot {
        let hud = self.take_hud_snapshot();
        let reports = self.take_reports_snapshot();
        SimSnapshot {
            year: hud.year,
            ticks: hud.ticks,
            map_seed: hud.map_seed,
            world_w: hud.world_w,
            world_h: hud.world_h,
            dna_generation: hud.dna_generation,
            empires: hud.empires,
            planets: reports.planets,
            fleets: reports.fleets,
            wormholes: hud.wormholes,
            debris_fields: self.debris_fields.clone(),
            new_combat_fx: hud.new_combat_fx,
            new_jump_fx: hud.new_jump_fx,
            power_history: self.power_history.clone(),
            events: hud.events,
            space_battles_count: hud.space_battles_count,
            invasions_count: hud.invasions_count,
            total_salvaged: hud.total_salvaged,
            gate_jumps_count: hud.gate_jumps_count,
        }
    }

    pub fn export_state_json(&self) -> String {
        let races: Vec<&RaceDef> = self.empires.iter().map(|e| &e.race).collect();
        let payload = serde_json::json!({
            "year": (self.year * 10.0).round() / 10.0,
            "dnaGeneration": self.dna_generation,
            "config": self.cfg,
            "races": races,
        });
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Effective target cloak after observer's Tachyon Detector reduction.
#[inline]
pub fn effective_cloak_pct(target_cloak_pct: f32, observer_tachyon_reduction: f32) -> f32 {
    target_cloak_pct.clamp(0.0, 0.98) * (1.0 - observer_tachyon_reduction.clamp(0.0, 0.95))
}

/// Effective fleet detection / interception range accounting for target cloak,
/// observer Tachyon Detector reduction, and local enemy ECM jamming penalty.
#[inline]
pub fn effective_detection_range(
    base_scan_range: f32,
    target_cloak_pct: f32,
    observer_tachyon_reduction: f32,
    local_ecm_penalty: f32,
) -> f32 {
    let eff_cloak = effective_cloak_pct(target_cloak_pct, observer_tachyon_reduction);
    base_scan_range * (1.0 - eff_cloak) * (1.0 - local_ecm_penalty.clamp(0.0, 0.85))
}

/// Combat firepower modifier after defender's torpedo jamming deflection.
#[inline]
pub fn apply_torpedo_jamming(
    attacker_mod: f32,
    attacker_uses_torpedoes: bool,
    defender_torpedo_jamming: f32,
) -> f32 {
    let jam = defender_torpedo_jamming.clamp(0.0, 0.75);
    let weight = if attacker_uses_torpedoes { 0.65 } else { 0.20 };
    attacker_mod * (1.0 - jam * weight)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn electronic_warfare_detection_and_torpedo_jamming() {
        // 75% cloak without Tachyon cuts 160 ly scan range down to 40 ly
        let uncountered = effective_detection_range(160.0, 0.75, 0.0, 0.0);
        assert!((uncountered - 40.0).abs() < 1e-3);

        // 50% Tachyon reduction cuts effective cloak to 37.5%, restoring range to 100 ly
        let countered = effective_detection_range(160.0, 0.75, 0.50, 0.0);
        assert!((countered - 100.0).abs() < 1e-3);
        assert!(countered > uncountered);

        // Enemy ECM field further suppresses detection range
        let jammed = effective_detection_range(160.0, 0.75, 0.50, 0.40);
        assert!((jammed - 60.0).abs() < 1e-3);
        assert!(jammed < countered);

        // Torpedo jamming strongly reduces torpedo-armed attacker damage
        let base_mod = 1.0;
        let vs_torp = apply_torpedo_jamming(base_mod, true, 0.50);
        let vs_beam = apply_torpedo_jamming(base_mod, false, 0.50);
        assert!(vs_torp < vs_beam);
        assert!((vs_torp - 0.675).abs() < 1e-4);
    }

    #[test]
    fn deterministic_six_empire_simulation() {
        let mut a = Simulation::new(SimConfig::default(), classic_races());
        let mut b = Simulation::new(SimConfig::default(), classic_races());
        a.init(80000, true);
        b.init(80000, true);

        for _ in 0..400 {
            a.step_tick(true);
            b.step_tick(true);
        }
        let sa = &a.tick_stats;
        let sb = &b.tick_stats;

        assert_eq!(sa.len(), 6);
        for e in 0..6 {
            assert_eq!(sa[e].planets, sb[e].planets);
            assert!((sa[e].res - sb[e].res).abs() < 1e-4);
        }
        assert_eq!(a.space_battles_count, b.space_battles_count);
    }

    #[test]
    fn supports_twelve_custom_empires_and_tech_progression() {
        let mut roster = classic_races();
        for i in 0..6 {
            let mut r = roster[i].clone();
            r.id = format!("custom_{i}");
            r.name = format!("Custom {i}");
            r.is_custom = true;
            roster.push(r);
        }
        let mut sim = Simulation::new(SimConfig::default(), roster);
        sim.init(99001, false);
        assert_eq!(sim.empires.len(), 12);

        for _ in 0..1200 {
            sim.step_tick(false);
        }

        let max_lv = sim
            .empires
            .iter()
            .flat_map(|e| e.tech_int_lv)
            .max()
            .unwrap_or(0);
        let total_planets: u32 = sim.empires.iter().map(|e| e.last_stats.planets).sum();
        assert!(total_planets > 40, "expected expansion, got {total_planets}");
        assert!(max_lv >= 8, "expected tech progression >= 8, got {max_lv}");
    }

    #[test]
    fn wormhole_only_mode_with_300_wormholes_and_disabled_stargates() {
        let mut cfg = SimConfig::default();
        cfg.galaxy_size = GalaxySize::Large;
        cfg.wormhole_pairs = 300;
        cfg.scenario_modifiers.allow_stargates = false;
        let mut sim = Simulation::new(cfg, classic_races());
        sim.init(77777, true);
        assert_eq!(sim.wormholes.len(), 300);

        for _ in 0..300 {
            sim.step_tick(true);
        }

        let total_gates: u32 = sim.empires.iter().map(|e| e.last_stats.gates).sum();
        assert_eq!(total_gates, 0, "expected 0 stargates when allow_stargates=false");
        assert!(sim.gate_jumps_count > 0, "expected fleets to use wormholes");
    }

    #[test]
    fn scenario_core_richness_bias_is_deterministic_and_enriches_core() {
        let mut cfg_std = SimConfig::default();
        cfg_std.galaxy_size = GalaxySize::Large;
        let mut sim_std = Simulation::new(cfg_std, classic_races());
        sim_std.init(42424, true);

        let mut cfg_core = SimConfig::default();
        cfg_core.galaxy_size = GalaxySize::Large;
        cfg_core.scenario_modifiers.core_richness_bias = 0.85;
        let mut sim_a = Simulation::new(cfg_core.clone(), classic_races());
        let mut sim_b = Simulation::new(cfg_core, classic_races());
        sim_a.init(42424, true);
        sim_b.init(42424, true);

        // Determinism check
        assert_eq!(sim_a.hab, sim_b.hab);
        assert_eq!(sim_a.min_crust, sim_b.min_crust);

        // Compare average core crust vs rim crust
        let cx = sim_a.world_w * 0.5;
        let cy = sim_a.world_h * 0.5;
        let mut core_sum = 0.0f32;
        let mut core_cnt = 0u32;
        let mut rim_sum = 0.0f32;
        let mut rim_cnt = 0u32;
        for i in 6..sim_a.pos_x.len() {
            let r = ((sim_a.pos_x[i] - cx) / cx).hypot((sim_a.pos_y[i] - cy) / cy);
            let crust = sim_a.min_crust[i * 3] + sim_a.min_crust[i * 3 + 1] + sim_a.min_crust[i * 3 + 2];
            if r < 0.35 {
                core_sum += crust;
                core_cnt += 1;
            } else if r > 0.72 {
                rim_sum += crust;
                rim_cnt += 1;
            }
        }
        assert!(core_cnt > 0 && rim_cnt > 0);
        let core_avg = core_sum / core_cnt as f32;
        let rim_avg = rim_sum / rim_cnt as f32;
        assert!(
            core_avg > rim_avg * 1.75,
            "expected core ({core_avg}) to be much richer than rim ({rim_avg})"
        );
    }

    #[test]
    fn scenario_fallen_empire_starts_with_advantage_and_is_deterministic() {
        let mut cfg = SimConfig::default();
        cfg.galaxy_size = GalaxySize::Large;
        cfg.scenario_modifiers.fallen_empire = true;
        let mut a = Simulation::new(cfg.clone(), classic_races());
        let mut b = Simulation::new(cfg, classic_races());
        a.init(55123, true);
        b.init(55123, true);

        let fe = 55123usize % a.empires.len();
        assert!(
            a.empires[fe].last_stats.planets >= 11 && a.empires[fe].last_stats.planets <= 12,
            "expected 11-12 starting planets for Fallen Empire, got {}",
            a.empires[fe].last_stats.planets
        );
        // Other empires start with 1 planet
        for (idx, emp) in a.empires.iter().enumerate() {
            if idx != fe {
                assert_eq!(emp.last_stats.planets, 1);
            }
        }
        // Fallen Empire starts with +4 tech bonus over baseline
        assert!(a.empires[fe].tech_int_lv[0] >= 4);
        // Determinism
        assert_eq!(a.owner, b.owner);
        assert_eq!(a.empires[fe].tech_int_lv, b.empires[fe].tech_int_lv);
    }
}
