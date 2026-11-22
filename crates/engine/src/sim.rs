use crate::config::{
    points_for_tech_level, tech_level_from_points, FleetClass, SimConfig, BREAKTHROUGHS,
    BP_ALCHEMY2, DT, MAX_DEBRIS_FIELDS, MAX_EMPIRES, MAX_EVENT_LOG, MAX_POWER_HISTORY,
    NUM_TECH_FIELDS, SHIP_COST_SCALE,
};
use crate::galaxy::{generate_galaxy, GalaxyShape, GalaxySize};
use crate::system_gen::{generate_star_system, PrimaryWorldInput};
use crate::tech::{
    classic_races, starting_tech_for_traits, EmpireTechCache, HullSpec, LRT_ARM, LRT_BET, LRT_CE,
    LRT_GR, LRT_IFE, LRT_ISB, LRT_LSP, LRT_MA, LRT_OBRM, LRT_RS, LRT_TT, LRT_UR,
};
use crate::types::{
    CombatFx, DebrisField, EmpireStats, EmpireSummaryView, Fleet, HudSnapshot, JumpFx, PlanetView,
    RaceDef, ReportsSnapshot, SimEvent, SimSnapshot, StarSystemView, StepProfile, Wormhole,
    WreckEntry,
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
    min_hub_sq: f32,
    effective_ship_cost: f32,
    bio_growth_mult: f32,
    mine_share: f32,
    fac_share: f32,
    ship_share: f32,
    tech_share: f32,
    col_pop_req: f32,
    dist_exp: f32,
    qual_exp: f32,
    cruiser_share: f32,
    bship_share: f32,
    tech_alloc: [f32; NUM_TECH_FIELDS],
    scan_r: f32,
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

    // Precomputed flat CSR sorted nearest-neighbor lists per planet & wormhole endpoints
    neighbor_offsets: Vec<u32>,
    neighbor_data: Vec<(u32, f32)>,
    wh_near_a: Vec<Vec<(u32, f32)>>,
    wh_near_b: Vec<Vec<(u32, f32)>>,

    // Reusable spatial grids & per-tick scratch buffers (zero per-tick allocations)
    fleet_grid: UniformGrid,
    debris_grid: UniformGrid,
    wh_grid: UniformGrid,
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
    pub events: Vec<SimEvent>,

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
            neighbor_offsets: Vec::new(),
            neighbor_data: Vec::new(),
            wh_near_a: Vec::new(),
            wh_near_b: Vec::new(),
            fleet_grid: UniformGrid::default(),
            debris_grid: UniformGrid::default(),
            wh_grid: UniformGrid::default(),
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
            events: Vec::new(),
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
        self.events.push(SimEvent {
            year: self.year,
            text,
            color: color.to_string(),
        });
        while self.events.len() > MAX_EVENT_LOG {
            self.events.remove(0);
        }
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

    fn build_planet_neighbors(&mut self) {
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

            let scan_r = self.effective_scanner_range(fe);
            for &j in &claimed {
                self.reveal_around_planet(j, fe, scan_r);
            }
        }

        // Initial planetary scanner reveal around each empire's homeworld
        for e in 0..num_emp {
            let scan_r = self.effective_scanner_range(e);
            self.reveal_around_planet(e, e, scan_r);
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

        if !silent {
            self.events.clear();
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
        let mut emp_cache = [EmpireTickCache::default(); MAX_EMPIRES];
        for e in 0..num_emp {
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
                min_hub_sq: min_hub_dist * min_hub_dist,
                effective_ship_cost: self.cfg.col_ship_cost * con_discount * trait_ship_discount,
                bio_growth_mult: self.cfg.growth
                    * growth_mult
                    * (1.0 + lv[5] * 0.025)
                    * if prt_he { 1.80 } else { 1.0 }
                    * DT,
                mine_share: (1.0 - doc.fac_share - doc.ship_share - doc.tech_share).max(0.05),
                fac_share: doc.fac_share,
                ship_share: doc.ship_share,
                tech_share: doc.tech_share,
                col_pop_req: doc.col_pop_req,
                dist_exp: doc.dist_exp,
                qual_exp: doc.qual_exp,
                cruiser_share: doc.cruiser_share,
                bship_share: doc.bship_share,
                tech_alloc: doc.tech_alloc,
                scan_r: (if prt_ss {
                    tc.scanner_range * 1.25
                } else {
                    tc.scanner_range
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

        let tick_usize = self.ticks as usize;
        let inv_fac_cost_res = 1.0 / self.cfg.fac_cost_res;
        let inv_fac_cost_germ = 1.0 / self.cfg.fac_cost_germ;
        let inv_crust_norm = 1.0 / (self.cfg.crust_richness * 3.0);

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
        if self.ticks % 5 == 0 {
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
                    let b = p * 3;
                    self.min_surf[b] += ci * 0.5;
                    self.min_surf[b + 1] += cb * 0.5;
                    self.min_surf[b + 2] += cg * 0.5;
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
            self.orbit_count[p] = self.orbit_count[p].saturating_add(1);
            if fl.emp == self.owner[p] {
                if fl.fleet_type == FleetClass::Freight && self.orbit_freight[p] < 0 {
                    self.orbit_freight[p] = f_idx as i32;
                } else if fl.fleet_type.is_combat()
                    && fl.power >= fl.max_power * 0.70
                    && self.orbit_warship[p] < 0
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

            let research_gen = res_gen * ec.tech_share * ec.research_mult;
            let tech_pts = &mut self.empires[e].tech_pts;
            for f in 0..NUM_TECH_FIELDS {
                tech_pts[f] += research_gen * ec.tech_alloc[f];
            }

            let room_for_fac = (max_buildings - self.factories[i]).max(0.0);
            let fac_budget = res_gen * ec.fac_share;
            let built_fac = room_for_fac
                .min(fac_budget * inv_fac_cost_res)
                .min(self.min_surf[base_idx + 2] * inv_fac_cost_germ);
            self.factories[i] += built_fac;
            self.min_surf[base_idx + 2] -= built_fac * self.cfg.fac_cost_germ;

            let surplus_res = (fac_budget - built_fac * self.cfg.fac_cost_res).max(0.0);

            if surplus_res > 0.0 && ec.alch_mult > 0.0 {
                let transmuted = surplus_res * ec.alch_mult;
                let mut min_m = 0usize;
                if self.min_surf[base_idx + 1] < self.min_surf[base_idx + min_m] {
                    min_m = 1;
                }
                if self.min_surf[base_idx + 2] < self.min_surf[base_idx + min_m] {
                    min_m = 2;
                }
                self.min_surf[base_idx + min_m] += transmuted;
            }

            // Strategic Stargate hubs (capped per empire + minimum hub spacing; PRT_IT builds denser networks)
            if ec.has_stargate_tech
                && self.has_gate[i] == 0
                && self.gate_lists[e].len() < ec.max_gates
                && self.factories[i] >= ec.min_fac_for_gate
            {
                if self.min_surf[base_idx] >= ec.gate_iron
                    && self.min_surf[base_idx + 2] >= ec.gate_germ
                    && self.next_rand() < 0.06
                {
                    let xi = self.pos_x[i];
                    let yi = self.pos_y[i];
                    let min_hub_sq = ec.min_hub_sq;
                    let too_close = self.gate_lists[e].iter().any(|&g| {
                        let dx = self.pos_x[g] - xi;
                        let dy = self.pos_y[g] - yi;
                        dx * dx + dy * dy < min_hub_sq
                    });
                    if !too_close {
                        self.min_surf[base_idx] -= ec.gate_iron;
                        self.min_surf[base_idx + 2] -= ec.gate_germ;
                        self.has_gate[i] = 1;
                        self.gate_lists[e].push(i);
                        if !silent && self.gate_lists[e].len() == 1 {
                            let g_name = self.empires[e]
                                .tech_cache
                                .stargate_name
                                .clone()
                                .unwrap_or_else(|| "Stargate".to_string());
                            let msg = format!(
                                "{} built its first 🌀 {} (#{})!",
                                self.empires[e].race.name,
                                g_name,
                                i + 1
                            );
                            let col = self.empires[e].race.color.clone();
                            self.push_event(msg, &col);
                        }
                    }
                }
            }

            // Planetary defense
            if self.defense[i] < ec.def_cap && self.min_surf[base_idx + 1] > 2.0 {
                let d_add = (ec.def_cap - self.defense[i])
                    .min(surplus_res * ec.def_surplus_rate)
                    .min(self.min_surf[base_idx + 1] * 0.5);
                self.defense[i] += d_add;
                self.min_surf[base_idx + 1] -= d_add * 0.15;
            }

            // Accumulate shipyard buffer every tick (boosted by orbital Starbase dock tier)
            if self.pop[i] > max_pop * ec.col_pop_req {
                let dock_mult = starbase_dock_mult(self.starbase_tier[i]);
                self.ship_prod_buf[i] += (res_gen * ec.ship_share + surplus_res * 0.5) * dock_mult;
            }

            // Starbase self-repair & autonomous construction/upgrade
            let cur_sb = self.starbase_tier[i];
            if cur_sb > 0 && self.starbase_hp[i] < self.starbase_max_hp[i] {
                self.starbase_hp[i] = (self.starbase_hp[i] + self.starbase_max_hp[i] * 0.08 * DT)
                    .min(self.starbase_max_hp[i]);
            }
            let best_sb = self.empires[e].tech_cache.best_starbase_tier;
            if cur_sb < best_sb
                && self.pop[i] > 28.0
                && self.factories[i] >= 14.0
                && emp_upkeep_ratio[e] < 0.40
                && self.next_rand() < 0.04
            {
                let target_tier = if cur_sb == 0 {
                    // Start with Space Station on major worlds, Orbital Fort/Space Dock on smaller colonies
                    if self.factories[i] >= 36.0 && best_sb >= 3 {
                        3
                    } else if best_sb >= 2 && self.factories[i] >= 22.0 {
                        2
                    } else {
                        1
                    }
                } else {
                    best_sb
                };
                if target_tier > cur_sb {
                    let (ti, tb, tg, tr) = starbase_build_cost(target_tier);
                    // Upgrading refunds 50% of previous starbase hull cost (matching craig-stars starbaseHullRefundFactor)
                    let (pi, pb, pg, pr) = starbase_build_cost(cur_sb);
                    let ci = (ti - pi * 0.5).max(0.0);
                    let cb = (tb - pb * 0.5).max(0.0);
                    let cg = (tg - pg * 0.5).max(0.0);
                    let cr = (tr - pr * 0.5).max(0.0);
                    if self.ship_prod_buf[i] >= cr
                        && self.min_surf[base_idx] >= ci
                        && self.min_surf[base_idx + 1] >= cb
                        && self.min_surf[base_idx + 2] >= cg
                    {
                        self.ship_prod_buf[i] -= cr;
                        self.min_surf[base_idx] -= ci;
                        self.min_surf[base_idx + 1] -= cb;
                        self.min_surf[base_idx + 2] -= cg;
                        let tc = &self.empires[e].tech_cache;
                        let tech_hp_mult = 1.0 + (tc.armor_val + tc.shield_val) * 0.0008;
                        let new_max_hp = starbase_base_hp(target_tier) * tech_hp_mult;
                        self.starbase_tier[i] = target_tier;
                        self.starbase_max_hp[i] = new_max_hp;
                        self.starbase_hp[i] = new_max_hp;
                    }
                }
            }

            // Time-sliced AI target search (freighters, colonizers, warships) using precomputed CSR neighbor lists
            let ai_turn = (i + tick_usize) % AI_DECISION_STRIDE == 0;
            if ai_turn {
                let t_scan = if prof { perf_now_ms() } else { 0.0 };
                // Planetary scanner sweep (Fog of War discovery)
                self.reveal_around_planet(i, e, ec.scan_r);
                let t_ai = if prof {
                    let now = perf_now_ms();
                    prof_planet_scan_ms += now - t_scan;
                    now
                } else {
                    0.0
                };

                // Freighter logistics (scaled probability for stride)
                if self.next_rand() < 0.25 {
                    let cargo_cap = ec.cargo_cap;
                    let freight_range = ec.freight_range;
                    let nb_start = self.neighbor_offsets[i] as usize;
                    let nb_end = self.neighbor_offsets[i + 1] as usize;
                    for m in 0..3 {
                        if self.min_surf[base_idx + m] > 95.0 {
                            let mut needy: Option<usize> = None;
                            let mut lowest_val = 20.0f32;
                            for &(j_u32, dist) in &self.neighbor_data[nb_start..nb_end] {
                                if dist > freight_range {
                                    break;
                                }
                                let j = j_u32 as usize;
                                if self.owner[j] == emp_u8 && self.min_surf[j * 3 + m] < lowest_val
                                {
                                    lowest_val = self.min_surf[j * 3 + m];
                                    needy = Some(j);
                                }
                            }
                            if let Some(target_idx) = needy {
                                let cargo = cargo_cap.min(self.min_surf[base_idx + m] * 0.45);
                                let docked_idx = self.orbit_freight[i];
                                if docked_idx >= 0 {
                                    let fi = docked_idx as usize;
                                    self.orbit_freight[i] = -1;
                                    self.min_surf[base_idx + m] -= cargo;
                                    let fl = &mut self.fleets[fi];
                                    fl.orbiting = false;
                                    fl.from = i;
                                    fl.target = target_idx;
                                    fl.min_type = m as u8;
                                    fl.power = cargo;
                                    fl.max_power = cargo_cap;
                                    fl.salvage_mode = 0;
                                    fl.cargo_iron = 0.0;
                                    fl.cargo_bora = 0.0;
                                    fl.cargo_germ = 0.0;
                                    fl.steer_x = self.pos_x[target_idx];
                                    fl.steer_y = self.pos_y[target_idx];
                                    fl.wh_route = -1;
                                    fl.used_wh = false;
                                    break;
                                } else if emp_upkeep_ratio[e] < 0.45
                                    && self.ship_prod_buf[i] >= ec.effective_ship_cost * 0.35
                                    && self.fleets.len() < 4000
                                {
                                    let (hi, hb, hg) =
                                        self.empires[e].ship_mineral_cost(FleetClass::Freight);
                                    let (ci, cb, cg) = (hi * 0.5, hb * 0.5, hg * 0.5);
                                    if self.min_surf[base_idx] >= ci
                                        && self.min_surf[base_idx + 1] >= cb
                                        && self.min_surf[base_idx + 2] >= cg
                                    {
                                        self.ship_prod_buf[i] -= ec.effective_ship_cost * 0.35;
                                        self.min_surf[base_idx] -= ci;
                                        self.min_surf[base_idx + 1] -= cb;
                                        self.min_surf[base_idx + 2] -= cg;
                                        let actual_cargo =
                                            cargo.min(self.min_surf[base_idx + m]).max(1.0);
                                        self.min_surf[base_idx + m] =
                                            (self.min_surf[base_idx + m] - actual_cargo).max(0.0);
                                        self.spawn_fleet(
                                            i,
                                            target_idx,
                                            emp_u8,
                                            FleetClass::Freight,
                                            actual_cargo,
                                            m as u8,
                                            -1,
                                            silent,
                                        );
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }

                // Dedicated Salvage Freighter dispatch (flies to rich debris, scoops up to cargo_cap, chains nearby wrecks, returns home)
                if any_debris
                    && self.pop[i] > 20.0
                    && self.next_rand() < 0.06
                    && self.fleets.len() < 4000
                {
                    let xi = self.pos_x[i];
                    let yi = self.pos_y[i];
                    let f_range = ec.freight_range * 0.92;
                    let f_range_sq = f_range * f_range;
                    let min_worth = if ec.salvage_mult > 1.1 { 15.0 } else { 25.0 };
                    let mut best_deb: Option<(f32, f32)> = None;
                    let mut best_deb_score = 0.0f32;
                    let debris_ref = &self.debris_fields;
                    self.debris_grid
                        .for_each_in_radius(xi, yi, f_range, |entry| {
                            let d_idx = entry.idx as usize;
                            if d_idx >= debris_ref.len() {
                                return;
                            }
                            let deb = &debris_ref[d_idx];
                            if deb.ttl <= 0.5 {
                                return;
                            }
                            let tot = deb.iron + deb.bora + deb.germ;
                            if tot < min_worth {
                                return;
                            }
                            let dx = entry.x - xi;
                            let dy = entry.y - yi;
                            let d_sq = dx * dx + dy * dy;
                            if d_sq > f_range_sq {
                                return;
                            }
                            let score = tot / (d_sq.sqrt() + 35.0);
                            if score > best_deb_score {
                                best_deb_score = score;
                                best_deb = Some((entry.x, entry.y));
                            }
                        });
                    if let Some((dx, dy)) = best_deb {
                        let docked_idx = self.orbit_freight[i];
                        if docked_idx >= 0 {
                            let fi = docked_idx as usize;
                            self.orbit_freight[i] = -1;
                            let fl = &mut self.fleets[fi];
                            fl.orbiting = false;
                            fl.from = i;
                            fl.target = i;
                            fl.min_type = 0;
                            fl.power = 12.0;
                            fl.max_power = ec.cargo_cap;
                            fl.hunting = false;
                            fl.used_wh = false;
                            fl.steer_x = dx;
                            fl.steer_y = dy;
                            fl.wh_route = -1;
                            fl.salvage_mode = 1;
                            fl.cargo_iron = 0.0;
                            fl.cargo_bora = 0.0;
                            fl.cargo_germ = 0.0;
                        } else if emp_upkeep_ratio[e] < 0.45
                            && self.ship_prod_buf[i] >= ec.effective_ship_cost * 0.35
                        {
                            let (hi, hb, hg) =
                                self.empires[e].ship_mineral_cost(FleetClass::Freight);
                            let (ci, cb, cg) = (hi * 0.5, hb * 0.5, hg * 0.5);
                            if self.min_surf[base_idx] >= ci
                                && self.min_surf[base_idx + 1] >= cb
                                && self.min_surf[base_idx + 2] >= cg
                            {
                                self.ship_prod_buf[i] -= ec.effective_ship_cost * 0.35;
                                self.min_surf[base_idx] -= ci;
                                self.min_surf[base_idx + 1] -= cb;
                                self.min_surf[base_idx + 2] -= cg;
                                let id = self.next_fleet_id;
                                self.next_fleet_id += 1;
                                self.fleets.push(Fleet {
                                    id,
                                    x: xi,
                                    y: yi,
                                    from: i,
                                    target: i,
                                    emp: emp_u8,
                                    fleet_type: FleetClass::Freight,
                                    min_type: 0,
                                    // Non-zero hull HP so outbound empty salvage freighters survive the end-of-tick death sweep
                                    power: 12.0,
                                    max_power: ec.cargo_cap,
                                    hunting: false,
                                    used_wh: false,
                                    steer_x: dx,
                                    steer_y: dy,
                                    wh_route: -1,
                                    salvage_mode: 1,
                                    cargo_iron: 0.0,
                                    cargo_bora: 0.0,
                                    cargo_germ: 0.0,
                                    orbiting: false,
                                    upkeep: FleetClass::Freight.upkeep(),
                                });
                            }
                        }
                    }
                }

                // Shipyard launch decision
                let effective_ship_cost = ec.effective_ship_cost;

                if self.pop[i] > max_pop * ec.col_pop_req
                    && self.ship_prod_buf[i] >= effective_ship_cost
                {
                    let mut best_target: Option<usize> = None;
                    let mut best_score = -1.0f32;
                    let mut best_wh_route: i16 = -1;
                    let mut mission = FleetClass::Col;
                    let my_bit = 1u16 << e;
                    let dist_exp = ec.dist_exp;
                    let qual_exp = ec.qual_exp;
                    let nb_start = self.neighbor_offsets[i] as usize;
                    let nb_end = self.neighbor_offsets[i + 1] as usize;

                    let col_range = ec.col_range;
                    let col_res_cost = effective_ship_cost * self.cfg.col_cost_mult.clamp(0.5, 5.0);
                    if neutral_planets_count > 0
                        && self.ship_prod_buf[i] >= col_res_cost
                        && self.min_surf[base_idx] >= ec.col_i
                        && self.min_surf[base_idx + 2] >= ec.col_g
                    {
                        for &(j_u32, dist) in &self.neighbor_data[nb_start..nb_end] {
                            if dist > col_range {
                                break;
                            }
                            let j = j_u32 as usize;
                            if self.owner[j] == 0 && (self.claim_mask[j] & my_bit) == 0 {
                                let d = dist.max(25.0);
                                let known = (self.explored_mask[j] & my_bit) != 0;
                                let score = if known {
                                    let sum_crust = self.min_crust[j * 3]
                                        + self.min_crust[j * 3 + 1]
                                        + self.min_crust[j * 3 + 2];
                                    let h = self.hab[j];
                                    let quality = (h * h.sqrt()) * (0.4 + sum_crust * inv_crust_norm);
                                    quality.powf(qual_exp) / (d / 180.0).powf(dist_exp)
                                } else {
                                    // Uncharted system: empire doesn't know hab/minerals yet, can scout if needed
                                    0.22 / (d / 180.0).powf(dist_exp)
                                };
                                if score > best_score {
                                    best_score = score;
                                    best_target = Some(j);
                                    mission = FleetClass::Col;
                                }
                            }
                        }
                        // Check a bounded sample of friendly Stargate relays (O(1) even with 500+ gates)
                        if best_target.is_none()
                            && self.has_gate[i] != 0
                            && self.gate_lists[e].len() > 1
                        {
                            let xi = self.pos_x[i];
                            let yi = self.pos_y[i];
                            let gate_range_sq = ec.gate_range_sq;
                            let gl_len = self.gate_lists[e].len();
                            let sample_count = gl_len.min(12);
                            let stride = (gl_len / sample_count).max(1);
                            let offset = (i + tick_usize) % gl_len;
                            for s in 0..sample_count {
                                let g = self.gate_lists[e][(offset + s * stride) % gl_len];
                                if g == i {
                                    continue;
                                }
                                let dx = self.pos_x[g] - xi;
                                let dy = self.pos_y[g] - yi;
                                if dx * dx + dy * dy > gate_range_sq {
                                    continue;
                                }
                                let g_start = self.neighbor_offsets[g] as usize;
                                let g_end = (self.neighbor_offsets[g + 1] as usize).min(g_start + 16);
                                for &(j_u32, dist) in &self.neighbor_data[g_start..g_end] {
                                    if dist > col_range {
                                        break;
                                    }
                                    let j = j_u32 as usize;
                                    if self.owner[j] == 0 && (self.claim_mask[j] & my_bit) == 0 {
                                        let d = dist.max(25.0);
                                        let sum_crust = self.min_crust[j * 3]
                                            + self.min_crust[j * 3 + 1]
                                            + self.min_crust[j * 3 + 2];
                                        let h = self.hab[j];
                                        let quality = (h * h.sqrt()) * (0.4 + sum_crust * inv_crust_norm);
                                        let score =
                                            quality.powf(qual_exp) / (d / 180.0).powf(dist_exp);
                                        if score > best_score {
                                            best_score = score;
                                            best_target = Some(j);
                                            best_wh_route = -1;
                                            mission = FleetClass::Col;
                                        }
                                    }
                                }
                            }
                        }

                        // Check natural Wormhole relays via static CSR wh_grid (O(1) even with 500 wormholes!)
                        if !self.wormholes.is_empty() {
                            let xi = self.pos_x[i];
                            let yi = self.pos_y[i];
                            let max_entry = col_range * 0.88;
                            let max_entry_sq = max_entry * max_entry;
                            let wh_near_a = &self.wh_near_a;
                            let wh_near_b = &self.wh_near_b;
                            let owner_ref = &self.owner;
                            let claim_ref = &self.claim_mask;
                            let min_crust_ref = &self.min_crust;
                            let hab_ref = &self.hab;
                            let mut checked_wh = 0u32;
                            self.wh_grid.for_each_in_radius(xi, yi, max_entry, |entry| {
                                if checked_wh >= 16 {
                                    return;
                                }
                                let dx = entry.x - xi;
                                let dy = entry.y - yi;
                                let d_entry_sq = dx * dx + dy * dy;
                                if d_entry_sq >= max_entry_sq {
                                    return;
                                }
                                checked_wh += 1;
                                let d_entry = d_entry_sq.sqrt();
                                let rem_range = col_range - d_entry;
                                let w = (entry.idx / 2) as usize;
                                let dir = (entry.idx % 2) as u8;
                                let near_exit = if dir == 0 {
                                    &wh_near_b[w]
                                } else {
                                    &wh_near_a[w]
                                };
                                for &(j_u32, d_exit) in near_exit {
                                    if d_exit > rem_range {
                                        break;
                                    }
                                    let j = j_u32 as usize;
                                    if owner_ref[j] == 0 && (claim_ref[j] & my_bit) == 0 {
                                        let d = (d_entry + d_exit).max(25.0);
                                        let sum_crust = min_crust_ref[j * 3]
                                            + min_crust_ref[j * 3 + 1]
                                            + min_crust_ref[j * 3 + 2];
                                        let h = hab_ref[j];
                                        let quality =
                                            (h * h.sqrt()) * (0.4 + sum_crust * inv_crust_norm);
                                        let score = (quality.powf(qual_exp)
                                            / (d / 180.0).powf(dist_exp))
                                            * 1.08;
                                        if score > best_score {
                                            best_score = score;
                                            best_target = Some(j);
                                            best_wh_route = entry.idx as i16;
                                            mission = FleetClass::Col;
                                        }
                                    }
                                }
                            });
                        }
                    }

                    if best_target.is_none() && self.pop[i] > 28.0 {
                        let r_roll = self.next_rand();
                        let try_order = if r_roll < ec.cruiser_share {
                            [
                                FleetClass::Cruiser,
                                FleetClass::Battleship,
                                FleetClass::Bomber,
                            ]
                        } else if r_roll < ec.cruiser_share + ec.bship_share {
                            [
                                FleetClass::Battleship,
                                FleetClass::Bomber,
                                FleetClass::Cruiser,
                            ]
                        } else {
                            [
                                FleetClass::Bomber,
                                FleetClass::Cruiser,
                                FleetClass::Battleship,
                            ]
                        };

                        let mut chosen_class: Option<FleetClass> = None;
                        for c_class in try_order {
                            let (ci, cb, cg) = self.empires[e].ship_mineral_cost(c_class);
                            if self.min_surf[base_idx] >= ci
                                && self.min_surf[base_idx + 1] >= cb
                                && self.min_surf[base_idx + 2] >= cg
                            {
                                chosen_class = Some(c_class);
                                break;
                            }
                        }

                        if let Some(c_class) = chosen_class {
                            let my_attack_est =
                                self.compute_fleet_power(e, c_class, self.factories[i]);
                            let ship_range = self.empires[e].ship_range(c_class);
                            let bomb_pen = if c_class == FleetClass::Bomber {
                                (self.cfg.bomber_pen
                                    + (self.empires[e].tech_cache.bomb_power * 0.02).min(0.22))
                                .min(0.88)
                            } else {
                                0.0
                            };

                            for &(j_u32, dist) in &self.neighbor_data[nb_start..nb_end] {
                                if dist > ship_range {
                                    break;
                                }
                                let j = j_u32 as usize;
                                let target_owner = self.owner[j];
                                if target_owner != 0
                                    && target_owner != emp_u8
                                    && (self.explored_mask[j] & my_bit) != 0
                                    && (self.claim_mask[j] & my_bit) == 0
                                {
                                    let target_e = (target_owner - 1) as usize;
                                    let d = dist.max(30.0);
                                    let eff_def = self.defense[j] * (1.0 - bomb_pen);
                                    let enemy_def = 20.0 + eff_def + self.pop[j] * 0.35;
                                    let target_share =
                                        self.empires[target_e].res_total / total_galactic_res;
                                    let leader_aggro_bonus =
                                        if target_e == leader_e && target_share > 0.30 {
                                            1.0 + (target_share - 0.25) * 4.0 * self.cfg.aggro_bias
                                        } else {
                                            1.0
                                        };

                                    if my_attack_est * leader_aggro_bonus > enemy_def * 0.72 {
                                        let score = ((self.factories[j] + 15.0)
                                            * (my_attack_est / enemy_def)
                                            * leader_aggro_bonus)
                                            / (d / 180.0).powf(1.35);
                                        if score > best_score {
                                            best_score = score;
                                            best_target = Some(j);
                                            mission = c_class;
                                        }
                                    }
                                }
                            }

                            // Check a bounded sample of friendly Stargate relays (O(1) even with 500+ gates)
                            if best_target.is_none()
                                && self.has_gate[i] != 0
                                && self.gate_lists[e].len() > 1
                            {
                                let xi = self.pos_x[i];
                                let yi = self.pos_y[i];
                                let gate_range_sq = ec.gate_range_sq;
                                let gl_len = self.gate_lists[e].len();
                                let sample_count = gl_len.min(12);
                                let stride = (gl_len / sample_count).max(1);
                                let offset = (i + tick_usize) % gl_len;
                                for s in 0..sample_count {
                                    let g = self.gate_lists[e][(offset + s * stride) % gl_len];
                                    if g == i {
                                        continue;
                                    }
                                    let dx = self.pos_x[g] - xi;
                                    let dy = self.pos_y[g] - yi;
                                    if dx * dx + dy * dy > gate_range_sq {
                                        continue;
                                    }
                                    let g_start = self.neighbor_offsets[g] as usize;
                                    let g_end =
                                        (self.neighbor_offsets[g + 1] as usize).min(g_start + 16);
                                    for &(j_u32, dist) in &self.neighbor_data[g_start..g_end] {
                                        if dist > ship_range {
                                            break;
                                        }
                                        let j = j_u32 as usize;
                                        let target_owner = self.owner[j];
                                        if target_owner != 0
                                            && target_owner != emp_u8
                                            && (self.explored_mask[j] & my_bit) != 0
                                            && (self.claim_mask[j] & my_bit) == 0
                                        {
                                            let target_e = (target_owner - 1) as usize;
                                            let d = dist.max(30.0);
                                            let eff_def = self.defense[j] * (1.0 - bomb_pen);
                                            let enemy_def = 20.0 + eff_def + self.pop[j] * 0.35;
                                            let target_share = self.empires[target_e].res_total
                                                / total_galactic_res;
                                            let leader_aggro_bonus =
                                                if target_e == leader_e && target_share > 0.30 {
                                                    1.0 + (target_share - 0.25)
                                                        * 4.0
                                                        * self.cfg.aggro_bias
                                                } else {
                                                    1.0
                                                };
                                            if my_attack_est * leader_aggro_bonus > enemy_def * 0.72
                                            {
                                                let score = ((self.factories[j] + 15.0)
                                                    * (my_attack_est / enemy_def)
                                                    * leader_aggro_bonus)
                                                    / (d / 180.0).powf(1.35);
                                                if score > best_score {
                                                    best_score = score;
                                                    best_target = Some(j);
                                                    best_wh_route = -1;
                                                    mission = c_class;
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Check natural Wormhole invasion relays via static CSR wh_grid (O(1) even with 500 wormholes!)
                            if !self.wormholes.is_empty() {
                                let xi = self.pos_x[i];
                                let yi = self.pos_y[i];
                                let max_entry = ship_range * 0.88;
                                let max_entry_sq = max_entry * max_entry;
                                let wh_near_a = &self.wh_near_a;
                                let wh_near_b = &self.wh_near_b;
                                let owner_ref = &self.owner;
                                let explored_ref = &self.explored_mask;
                                let claim_ref = &self.claim_mask;
                                let defense_ref = &self.defense;
                                let pop_ref = &self.pop;
                                let factories_ref = &self.factories;
                                let empires_ref = &self.empires;
                                let aggro_bias = self.cfg.aggro_bias;
                                let mut checked_wh = 0u32;
                                self.wh_grid.for_each_in_radius(xi, yi, max_entry, |entry| {
                                    if checked_wh >= 16 {
                                        return;
                                    }
                                    let dx = entry.x - xi;
                                    let dy = entry.y - yi;
                                    let d_entry_sq = dx * dx + dy * dy;
                                    if d_entry_sq >= max_entry_sq {
                                        return;
                                    }
                                    checked_wh += 1;
                                    let d_entry = d_entry_sq.sqrt();
                                    let rem_range = ship_range - d_entry;
                                    let w = (entry.idx / 2) as usize;
                                    let dir = (entry.idx % 2) as u8;
                                    let near_exit = if dir == 0 {
                                        &wh_near_b[w]
                                    } else {
                                        &wh_near_a[w]
                                    };
                                    for &(j_u32, d_exit) in near_exit {
                                        if d_exit > rem_range {
                                            break;
                                        }
                                        let j = j_u32 as usize;
                                        let target_owner = owner_ref[j];
                                        if target_owner != 0
                                            && target_owner != emp_u8
                                            && (explored_ref[j] & my_bit) != 0
                                            && (claim_ref[j] & my_bit) == 0
                                        {
                                            let target_e = (target_owner - 1) as usize;
                                            let d = (d_entry + d_exit).max(30.0);
                                            let eff_def = defense_ref[j] * (1.0 - bomb_pen);
                                            let enemy_def = 20.0 + eff_def + pop_ref[j] * 0.35;
                                            let target_share =
                                                empires_ref[target_e].res_total / total_galactic_res;
                                            let leader_aggro_bonus = if target_e == leader_e
                                                && target_share > 0.30
                                            {
                                                1.0 + (target_share - 0.25) * 4.0 * aggro_bias
                                            } else {
                                                1.0
                                            };
                                            if my_attack_est * leader_aggro_bonus
                                                > enemy_def * 0.72
                                            {
                                                let score = (((factories_ref[j] + 15.0)
                                                    * (my_attack_est / enemy_def)
                                                    * leader_aggro_bonus)
                                                    / (d / 180.0).powf(1.35))
                                                    * 1.08;
                                                if score > best_score {
                                                    best_score = score;
                                                    best_target = Some(j);
                                                    best_wh_route = entry.idx as i16;
                                                    mission = c_class;
                                                }
                                            }
                                        }
                                    }
                                });
                            }
                        }
                    }

                    if let Some(target_idx) = best_target {
                        let docked_war = self.orbit_warship[i];
                        if mission.is_combat() && docked_war >= 0 {
                            // Dispatch stationed garrison warship from orbit at zero build cost
                            let fi = docked_war as usize;
                            self.orbit_warship[i] = -1;
                            self.claim_mask[target_idx] |= my_bit;
                            let (sx, sy) = if best_wh_route >= 0 {
                                let w = (best_wh_route / 2) as usize;
                                let dir = best_wh_route % 2;
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
                            let fl = &mut self.fleets[fi];
                            fl.orbiting = false;
                            fl.from = i;
                            fl.target = target_idx;
                            fl.hunting = false;
                            fl.used_wh = false;
                            fl.wh_route = best_wh_route;
                            fl.steer_x = sx;
                            fl.steer_y = sy;
                        } else if mission == FleetClass::Col || emp_upkeep_ratio[e] < 0.42 {
                            let cost_mult = if mission == FleetClass::Col {
                                self.cfg.col_cost_mult.clamp(0.5, 5.0)
                            } else {
                                1.0
                            };
                            let (ci_raw, cb_raw, cg_raw) = self.empires[e].ship_mineral_cost(mission);
                            let (ci, cb, cg) = (ci_raw * cost_mult, cb_raw * cost_mult, cg_raw * cost_mult);
                            let res_cost = effective_ship_cost * cost_mult;
                            if self.ship_prod_buf[i] >= res_cost
                                && self.min_surf[base_idx] >= ci
                                && self.min_surf[base_idx + 1] >= cb
                                && self.min_surf[base_idx + 2] >= cg
                            {
                                self.ship_prod_buf[i] -= res_cost;
                                self.min_surf[base_idx] -= ci;
                                self.min_surf[base_idx + 1] -= cb;
                                self.min_surf[base_idx + 2] -= cg;
                                self.claim_mask[target_idx] |= my_bit;
                                self.pop[i] = (self.pop[i] - 6.0).max(2.0);
                                let power =
                                    self.compute_fleet_power(e, mission, self.factories[i]);
                                self.spawn_fleet(
                                    i,
                                    target_idx,
                                    emp_u8,
                                    mission,
                                    power,
                                    0,
                                    best_wh_route,
                                    silent,
                                );
                            }
                        }
                    } else if emp_upkeep_ratio[e] < 0.28
                        && self.ship_prod_buf[i] >= effective_ship_cost * 1.4
                        && self.orbit_count[i] < 2
                        && self.fleets.len() < 4000
                    {
                        // Build a standing orbital garrison warship when budget & minerals allow
                        let r = self.next_rand();
                        let g_class = if r < ec.cruiser_share {
                            FleetClass::Cruiser
                        } else if r < ec.cruiser_share + ec.bship_share {
                            FleetClass::Battleship
                        } else {
                            FleetClass::Bomber
                        };
                        let (ci, cb, cg) = self.empires[e].ship_mineral_cost(g_class);
                        if self.min_surf[base_idx] >= ci
                            && self.min_surf[base_idx + 1] >= cb
                            && self.min_surf[base_idx + 2] >= cg
                        {
                            self.ship_prod_buf[i] -= effective_ship_cost;
                            self.min_surf[base_idx] -= ci;
                            self.min_surf[base_idx + 1] -= cb;
                            self.min_surf[base_idx + 2] -= cg;
                            let power = self.compute_fleet_power(e, g_class, self.factories[i]);
                            let id = self.next_fleet_id;
                            self.next_fleet_id += 1;
                            let ox = self.pos_x[i];
                            let oy = self.pos_y[i];
                            self.fleets.push(Fleet {
                                id,
                                x: ox,
                                y: oy,
                                from: i,
                                target: i,
                                emp: emp_u8,
                                fleet_type: g_class,
                                min_type: 0,
                                power,
                                max_power: power,
                                hunting: false,
                                used_wh: false,
                                steer_x: ox,
                                steer_y: oy,
                                wh_route: -1,
                                salvage_mode: 0,
                                cargo_iron: 0.0,
                                cargo_bora: 0.0,
                                cargo_germ: 0.0,
                                orbiting: true,
                                upkeep: g_class.upkeep(),
                            });
                            self.orbit_count[i] = self.orbit_count[i].saturating_add(1);
                        }
                    }
                }
                if prof {
                    prof_ai_ms += perf_now_ms() - t_ai;
                }
            }

            let room_for_mines = if total_crust_here > 1.0 {
                (max_buildings - self.mines[i]).max(0.0)
            } else {
                0.0
            };
            self.mines[i] += room_for_mines.min((res_gen * ec.mine_share) * 0.2);

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
            let emp_u8 = self.fleets[f_idx].emp;
            let e = (emp_u8 - 1) as usize;
            let ec = &emp_cache[e];

            // Orbiting garrison / docked fleet maintenance (co-located with host star system)
            if self.fleets[f_idx].orbiting {
                let p = self.fleets[f_idx].target;
                if p >= n || self.owner[p] != emp_u8 {
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
                    let cap = self.fleets[f_idx].max_power.max(45.0);
                    let mut carried = self.fleets[f_idx].cargo_iron
                        + self.fleets[f_idx].cargo_bora
                        + self.fleets[f_idx].cargo_germ;
                    let debris_slice = &mut self.debris_fields;
                    let mut add_i = 0.0f32;
                    let mut add_b = 0.0f32;
                    let mut add_g = 0.0f32;

                    self.debris_grid
                        .for_each_in_radius(sx, sy, 14.0, |entry| {
                            let rem = (cap - carried).max(0.0);
                            if rem <= 0.5 {
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
                            if avail <= 0.1 {
                                deb.ttl = 0.0;
                                return;
                            }
                            let take = avail.min(rem);
                            let frac = (take / avail).clamp(0.0, 1.0);
                            let ti = deb.iron * frac;
                            let tb = deb.bora * frac;
                            let tg = deb.germ * frac;
                            deb.iron -= ti;
                            deb.bora -= tb;
                            deb.germ -= tg;
                            if deb.iron + deb.bora + deb.germ <= 0.5 {
                                deb.ttl = 0.0;
                            }
                            add_i += ti;
                            add_b += tb;
                            add_g += tg;
                            carried += take;
                        });

                    self.fleets[f_idx].cargo_iron += add_i;
                    self.fleets[f_idx].cargo_bora += add_b;
                    self.fleets[f_idx].cargo_germ += add_g;
                    let total_cargo = self.fleets[f_idx].cargo_iron
                        + self.fleets[f_idx].cargo_bora
                        + self.fleets[f_idx].cargo_germ;
                    self.fleets[f_idx].power = total_cargo.max(12.0);

                    // Multi-stop chaining: if hold still has room (>= 15 kT) and another wreck is within 85 ly & fuel range
                    let rem_after = (cap - total_cargo).max(0.0);
                    let mut next_stop: Option<(f32, f32)> = None;
                    if rem_after >= 15.0 {
                        let home_idx = self.fleets[f_idx].from;
                        let hx = self.pos_x[home_idx];
                        let hy = self.pos_y[home_idx];
                        let max_home_sq = (ec.freight_range * 0.95) * (ec.freight_range * 0.95);
                        let mut best_d_sq = 85.0 * 85.0;
                        let debris_ref = &self.debris_fields;
                        self.debris_grid
                            .for_each_in_radius(sx, sy, 85.0, |entry| {
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

                        // If a Colonizer jumped and its target was already taken by an enemy,
                        // pick another free neutral near the exit or return home through the wormhole!
                        if self.fleets[f_idx].fleet_type == FleetClass::Col
                            && self.owner[target_idx] != 0
                            && self.owner[target_idx] != emp_u8
                        {
                            let near_exit = if dir == 0 {
                                &self.wh_near_b[w]
                            } else {
                                &self.wh_near_a[w]
                            };
                            let mut alt: Option<usize> = None;
                            for &(cand_u32, _) in near_exit {
                                let cand = cand_u32 as usize;
                                if self.owner[cand] == 0 {
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

            // Cruiser interception using flat CSR UniformGrid (stable per-fleet role so Cruisers don't zigzag!)
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
                let owner_ref = &self.owner;
                let has_gate_ref = &self.has_gate;
                let defense_ref = &self.defense;
                let pos_x_ref = &self.pos_x;
                let pos_y_ref = &self.pos_y;

                self.fleet_grid.for_each_in_radius(fx, fy, scan_max, |entry| {
                    if entry.tag == emp_u8 {
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
                    let other = &fleets_ref[k];
                    if (entry.flags & 2) != 0 || other.power < f_pow * 0.9 {
                        // Electronic Warfare: target Cloak vs observer Tachyon Detector + local enemy ECM
                        let other_e = (entry.tag - 1) as usize;
                        let other_tc = &empires_ref[other_e].tech_cache;
                        let target_cloak = (if (entry.flags & 1) != 0 {
                            other_tc.cloak_armed_pct
                        } else {
                            other_tc.cloak_unarmed_pct
                        } * ew_mult)
                            .clamp(0.0, 0.96);
                        let mut ecm_pen = 0.0f32;
                        for &p_idx in &[other.from, other.target] {
                            let p_owner = owner_ref[p_idx];
                            if p_owner != 0 && p_owner != emp_u8 {
                                let pe = (p_owner - 1) as usize;
                                let ptc = &empires_ref[pe].tech_cache;
                                let jam_r = ptc.ecm_jammer_range;
                                if ptc.ecm_jammer_strength > 0.0
                                    && jam_r > 0.0
                                    && (has_gate_ref[p_idx] != 0 || defense_ref[p_idx] >= 55.0)
                                {
                                    let jdx = entry.x - pos_x_ref[p_idx];
                                    let jdy = entry.y - pos_y_ref[p_idx];
                                    let jdist_sq = jdx * jdx + jdy * jdy;
                                    if jdist_sq < jam_r * jam_r {
                                        let falloff = 1.0 - jdist_sq.sqrt() / jam_r;
                                        ecm_pen = ecm_pen
                                            .max((ptc.ecm_jammer_strength * falloff * ew_mult).min(0.85));
                                    }
                                }
                            }
                        }
                        let eff_scan = effective_detection_range(
                            scan_max,
                            target_cloak,
                            tachyon_red,
                            ecm_pen,
                        );
                        if d_sq <= eff_scan * eff_scan {
                            best_prey_dist_sq = d_sq;
                            best_prey = Some((entry.x, entry.y));
                        }
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

            // Debris salvage using UniformGrid (only when auto_salvage is enabled)
            let fx = self.fleets[f_idx].x;
            let fy = self.fleets[f_idx].y;
            if has_auto_salvage && !self.debris_fields.is_empty() {
                let from_idx = self.fleets[f_idx].from;
                let salvage_mult = ec.salvage_mult;
                let home_idx = if self.owner[from_idx] == emp_u8 {
                    from_idx
                } else {
                    e
                };
                let debris_slice = &mut self.debris_fields;
                let min_surf = &mut self.min_surf;
                let total_salvaged = &mut self.total_salvaged;

                self.debris_grid
                    .for_each_in_radius(fx, fy, salvage_rad, |entry| {
                        let dx = entry.x - fx;
                        let dy = entry.y - fy;
                        if dx * dx + dy * dy > salvage_rad_sq {
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
                        min_surf[home_idx * 3] += deb.iron * salvage_mult;
                        min_surf[home_idx * 3 + 1] += deb.bora * salvage_mult;
                        min_surf[home_idx * 3 + 2] += deb.germ * salvage_mult;
                        *total_salvaged +=
                            ((deb.iron + deb.bora + deb.germ) * salvage_mult).round() as u32;
                        deb.ttl = 0.0;
                    });
            }

            let step_dist = ec.base_step_dist * self.fleets[f_idx].fleet_type.speed_mod();

            let tdx = self.pos_x[target_idx] - fx;
            let tdy = self.pos_y[target_idx] - fy;
            if !heading_to_wh && tdx * tdx + tdy * tdy <= step_dist * step_dist {
                self.claim_mask[target_idx] &= !(1u16 << e);
                // Arriving fleet's shipboard scanner reveals the target system & nearby space
                let ship_scan = self.effective_scanner_range(e) * 0.65;
                self.reveal_around_planet(target_idx, e, ship_scan);

                let fl_type = self.fleets[f_idx].fleet_type;
                let fl_emp = self.fleets[f_idx].emp;
                if fl_type == FleetClass::Freight {
                    if self.owner[target_idx] == fl_emp {
                        if self.fleets[f_idx].salvage_mode == 2 {
                            let b = target_idx * 3;
                            let mult = ec.salvage_mult;
                            let di = self.fleets[f_idx].cargo_iron * mult;
                            let db = self.fleets[f_idx].cargo_bora * mult;
                            let dg = self.fleets[f_idx].cargo_germ * mult;
                            self.min_surf[b] += di;
                            self.min_surf[b + 1] += db;
                            self.min_surf[b + 2] += dg;
                            self.total_salvaged += (di + db + dg).round() as u32;
                        } else {
                            let mt = self.fleets[f_idx].min_type as usize;
                            self.min_surf[target_idx * 3 + mt] += self.fleets[f_idx].power;
                        }
                        // Dock freighter in orbit at friendly destination for reuse
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
                        fl.max_power = ec.cargo_cap;
                        fl.wh_route = -1;
                        fl.used_wh = false;
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
                    } else if self.owner[target_idx] == fl.emp {
                        self.pop[target_idx] =
                            (self.hab[target_idx] * 170.0).min(self.pop[target_idx] + 7.0);
                    }
                } else if self.owner[target_idx] == fl_emp || self.owner[target_idx] == 0 {
                    // Warship arrives at friendly (or now-empty) world: dock in orbit if friendly
                    if self.owner[target_idx] == fl_emp {
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
                    } else {
                        let home = self.fleets[f_idx].from;
                        if home < n && self.owner[home] == fl_emp {
                            self.fleets[f_idx].target = home;
                            self.fleets[f_idx].steer_x = self.pos_x[home];
                            self.fleets[f_idx].steer_y = self.pos_y[home];
                            self.fleets[f_idx].wh_route = -1;
                        } else {
                            self.fleets.swap_remove(f_idx);
                        }
                    }
                } else {
                    // Warship planetary assault (must first defeat any orbital Starbase)
                    let def_e = (self.owner[target_idx] - 1) as usize;
                    let mut fl_power = self.fleets[f_idx].power;

                    // Orbital Starbase engagement
                    let sb_tier = self.starbase_tier[target_idx];
                    if sb_tier > 0 && self.starbase_hp[target_idx] > 0.0 {
                        let sb_hp = self.starbase_hp[target_idx];
                        let sb_combat_power = sb_hp * 0.22;
                        let dmg_to_sb = fl_power * 1.15;
                        self.starbase_hp[target_idx] = (sb_hp - dmg_to_sb).max(0.0);
                        let dmg_to_fl = sb_combat_power * 0.65;
                        fl_power -= dmg_to_fl;
                        self.fleets[f_idx].power = fl_power;

                        if self.starbase_hp[target_idx] <= 0.0 {
                            // Starbase destroyed! Drop wreckage into debris field
                            let (si, sb, sg, _) = starbase_build_cost(sb_tier);
                            let rate = self.cfg.salvage_rate;
                            if rate > 0.0 && (si + sb + sg) * rate > 0.5 {
                                let ttl = self.cfg.scenario_modifiers.debris_ttl.max(1.0);
                                let sx = self.pos_x[target_idx];
                                let sy = self.pos_y[target_idx];
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

                        if fl_power <= 0.0 {
                            let dead = self.fleets.swap_remove(f_idx);
                            self.spawn_salvage_debris(
                                self.pos_x[target_idx],
                                self.pos_y[target_idx],
                                &dead,
                            );
                            continue;
                        }
                        if self.starbase_tier[target_idx] > 0 {
                            // Starbase held the orbit — repelled attacker is destroyed
                            let dead = self.fleets.swap_remove(f_idx);
                            self.spawn_salvage_debris(
                                self.pos_x[target_idx],
                                self.pos_y[target_idx],
                                &dead,
                            );
                            continue;
                        }
                    }

                    let mut bunker_bonus: f32 = if emp_cache[def_e].prt_is {
                        1.30
                    } else {
                        1.0
                    };
                    let mut effective_def = self.defense[target_idx];
                    if fl_type == FleetClass::Bomber {
                        let pen = (self.cfg.bomber_pen
                            + (self.empires[e].tech_cache.bomb_power * 0.02).min(0.22))
                        .min(0.88);
                        effective_def *= 1.0 - pen;
                        bunker_bonus = (bunker_bonus * 0.75).max(1.0);
                    }
                    let def_threshold =
                        (20.0 + effective_def + self.pop[target_idx] * 0.35) * bunker_bonus;
                    if fl_power > def_threshold {
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
                        self.defense[target_idx] = 20.0;
                        self.invasions_count += 1;
                        if !silent && self.has_gate[target_idx] != 0 {
                            let msg = format!(
                                "{} captured Stargate hub #{}!",
                                self.empires[e].race.name,
                                target_idx + 1
                            );
                            let col = self.empires[e].race.color.clone();
                            self.push_event(msg, &col);
                        }
                        // Victorious warship takes assault damage and stations in orbit around captured planet
                        let assault_dmg = (def_threshold * 0.35).min(fl_power * 0.72);
                        let ox = self.pos_x[target_idx];
                        let oy = self.pos_y[target_idx];
                        let fl = &mut self.fleets[f_idx];
                        fl.power = (fl.power - assault_dmg).max(fl.max_power * 0.15);
                        fl.orbiting = true;
                        fl.from = target_idx;
                        fl.target = target_idx;
                        fl.x = ox;
                        fl.y = oy;
                        fl.steer_x = ox;
                        fl.steer_y = oy;
                        fl.hunting = false;
                        fl.wh_route = -1;
                    } else {
                        let dmg = if fl_type == FleetClass::Bomber {
                            fl_power * 0.85
                        } else {
                            (fl_power / bunker_bonus) * 0.45
                        };
                        self.defense[target_idx] = (self.defense[target_idx] - dmg).max(4.0);
                        if fl_type == FleetClass::Bomber {
                            self.factories[target_idx] =
                                (self.factories[target_idx] * 0.92).max(2.0);
                        }
                        // Repelled attacker is destroyed in planetary fire and leaves battle wreckage
                        let dead = self.fleets.swap_remove(f_idx);
                        self.spawn_salvage_debris(
                            self.pos_x[target_idx],
                            self.pos_y[target_idx],
                            &dead,
                        );
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
                    self.fleet_grid.for_each_in_radius(ax, ay, hit_rad, |entry| {
                        if entry.idx <= i_u32 || entry.tag == emp_a {
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
                    let type_b = self.fleets[j].fleet_type;
                    let b_c = type_b.is_combat();
                    let bx = self.fleets[j].x;
                    let by = self.fleets[j].y;

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
                    let eb = (self.fleets[j].emp - 1) as usize;
                    if emp_cache[ea].prt_ss {
                        mod_a *= 1.12;
                    }
                    if emp_cache[eb].prt_ss {
                        mod_b *= 1.12;
                    }

                    // Electronic Warfare: defender Torpedo Jamming reduces enemy torpedo effectiveness
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
                    } else {
                        self.fleets[j].power *= 1.0 - ((eff_a / eff_b) * 0.65).min(0.85);
                        self.fleets[i].power = 0.0;
                        self.empires[eb].race.doctrine.kills += 1;
                        let dead = self.fleets[i].clone();
                        self.spawn_salvage_debris(mx, my, &dead);
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
                self.claim_mask[target] &= !(1u16 << e);
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
            if self.debris_fields[i].ttl <= 0.0 || total <= 0.5 {
                self.debris_fields.swap_remove(i);
            }
        }

        self.year += DT;
        self.ticks += 1;

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
    fn effective_scanner_range(&self, e: usize) -> f32 {
        let base = self.empires[e].tech_cache.scanner_range
            * self.cfg.scenario_modifiers.scanner_range_mult.clamp(0.2, 3.0);
        if self.empires[e].race.prt == "PRT_SS" {
            base * 1.25
        } else {
            base
        }
    }

    #[inline]
    fn reveal_around_planet(&mut self, center_idx: usize, e: usize, radius: f32) {
        let bit = 1u16 << e;
        self.explored_mask[center_idx] |= bit;
        let start = self.neighbor_offsets[center_idx] as usize;
        let end = self.neighbor_offsets[center_idx + 1] as usize;
        for &(j_u32, dist) in &self.neighbor_data[start..end] {
            if dist > radius {
                break;
            }
            self.explored_mask[j_u32 as usize] |= bit;
        }
    }

    fn compute_fleet_power(&self, e: usize, class: FleetClass, factories_here: f32) -> f32 {
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
        (32.0 + factories_here * 0.45) * wep_scale * dur_scale * class_mod
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_fleet(
        &mut self,
        from_idx: usize,
        target_idx: usize,
        emp_u8: u8,
        fleet_type: FleetClass,
        power: f32,
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
            upkeep: fleet_type.upkeep(),
        });
    }

    fn spawn_salvage_debris(&mut self, x: f32, y: f32, dead_fleet: &Fleet) {
        if self.cfg.salvage_rate <= 0.0 {
            return;
        }
        let ttl = self.cfg.scenario_modifiers.debris_ttl.max(1.0);
        let wreck = WreckEntry {
            emp: dead_fleet.emp,
            ship_type: dead_fleet.fleet_type,
            count: 1,
        };
        let (iron, bora, germ) = if dead_fleet.fleet_type == FleetClass::Freight {
            let e = (dead_fleet.emp - 1) as usize;
            let (hi, hb, hg) = self.empires[e].ship_mineral_cost(FleetClass::Freight);
            if dead_fleet.salvage_mode != 0 {
                (
                    (dead_fleet.cargo_iron + hi * 0.5) * self.cfg.salvage_rate,
                    (dead_fleet.cargo_bora + hb * 0.5) * self.cfg.salvage_rate,
                    (dead_fleet.cargo_germ + hg * 0.5) * self.cfg.salvage_rate,
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
                ci * self.cfg.salvage_rate,
                cb * self.cfg.salvage_rate,
                cg * self.cfg.salvage_rate,
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
            events: self.events.clone(),
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
