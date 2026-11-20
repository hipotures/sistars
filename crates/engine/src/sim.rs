use crate::config::{
    points_for_tech_level, tech_level_from_points, FleetClass, SimConfig, BREAKTHROUGHS,
    BP_ALCHEMY2, DT, MAX_DEBRIS_FIELDS, MAX_EMPIRES, MAX_EVENT_LOG, MAX_POWER_HISTORY,
    NUM_TECH_FIELDS, SHIP_COST_SCALE,
};
use crate::galaxy::{generate_galaxy, GalaxyShape, GalaxySize};
use crate::tech::{
    classic_races, starting_tech_for_traits, EmpireTechCache, HullSpec, LRT_ARM, LRT_BET, LRT_CE,
    LRT_GR, LRT_IFE, LRT_ISB, LRT_LSP, LRT_MA, LRT_OBRM, LRT_RS, LRT_TT, LRT_UR,
};
use crate::types::{
    CombatFx, DebrisField, EmpireStats, EmpireSummaryView, Fleet, HudSnapshot, JumpFx, PlanetView,
    RaceDef, ReportsSnapshot, SimEvent, SimSnapshot, Wormhole,
};

const MAX_PLANET_NEIGHBORS: usize = 96;
const AI_DECISION_STRIDE: usize = 6;
pub const PLANET_RENDER_STRIDE: usize = 7;
pub const FLEET_RENDER_STRIDE: usize = 7;
pub const DEBRIS_RENDER_STRIDE: usize = 2;

/// Zero-allocation intrusive linked-list spatial grid reused across ticks.
#[derive(Debug, Clone, Default)]
pub struct UniformGrid {
    cols: usize,
    rows: usize,
    inv_cell: f32,
    head: Vec<i32>,
    next: Vec<i32>,
}

impl UniformGrid {
    pub fn reset(&mut self, world_w: f32, world_h: f32, cell_size: f32, entity_count: usize) {
        let cell = cell_size.max(16.0);
        self.inv_cell = 1.0 / cell;
        self.cols = ((world_w * self.inv_cell).ceil() as usize).max(1);
        self.rows = ((world_h * self.inv_cell).ceil() as usize).max(1);
        let total_cells = self.cols * self.rows;
        if self.head.len() != total_cells {
            self.head.resize(total_cells, -1);
        }
        self.head.fill(-1);
        if self.next.len() < entity_count {
            self.next.resize(entity_count, -1);
        }
    }

    #[inline]
    fn cell_coords(&self, x: f32, y: f32) -> (usize, usize) {
        let cx = ((x * self.inv_cell) as isize).clamp(0, (self.cols - 1) as isize) as usize;
        let cy = ((y * self.inv_cell) as isize).clamp(0, (self.rows - 1) as isize) as usize;
        (cx, cy)
    }

    #[inline]
    pub fn insert(&mut self, idx: usize, x: f32, y: f32) {
        let (cx, cy) = self.cell_coords(x, y);
        let cell = cy * self.cols + cx;
        self.next[idx] = self.head[cell];
        self.head[cell] = idx as i32;
    }

    #[inline]
    pub fn for_each_in_radius<F: FnMut(usize)>(&self, x: f32, y: f32, radius: f32, mut f: F) {
        if self.head.is_empty() {
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
            for cx in min_x..=max_x {
                let mut cur = self.head[row_off + cx];
                while cur >= 0 {
                    let u = cur as usize;
                    f(u);
                    cur = self.next[u];
                }
            }
        }
    }
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
    pub claim_mask: Vec<u16>,

    // Precomputed sorted nearest-neighbor lists per planet (eliminates O(P^2) scans)
    planet_neighbors: Vec<Vec<(u16, f32)>>,

    // Reusable spatial grids & per-tick scratch buffers (zero per-tick allocations)
    fleet_grid: UniformGrid,
    debris_grid: UniformGrid,
    gate_lists: Vec<Vec<usize>>,
    p_counts: Vec<u32>,
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
            claim_mask: Vec::new(),
            planet_neighbors: Vec::new(),
            fleet_grid: UniformGrid::default(),
            debris_grid: UniformGrid::default(),
            gate_lists: vec![Vec::new(); num_emp],
            p_counts: vec![0; num_emp],
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

    fn build_planet_neighbors(&mut self) {
        let n = self.pos_x.len();
        self.planet_neighbors.clear();
        self.planet_neighbors.reserve(n);
        let mut scratch: Vec<(u16, f32)> = Vec::with_capacity(n);

        for i in 0..n {
            scratch.clear();
            let xi = self.pos_x[i];
            let yi = self.pos_y[i];
            for j in 0..n {
                if i == j {
                    continue;
                }
                let dx = self.pos_x[j] - xi;
                let dy = self.pos_y[j] - yi;
                let d_sq = dx * dx + dy * dy;
                scratch.push((j as u16, d_sq));
            }
            let keep = MAX_PLANET_NEIGHBORS.min(scratch.len());
            if keep < scratch.len() {
                scratch.select_nth_unstable_by(keep, |a, b| a.1.total_cmp(&b.1));
                scratch.truncate(keep);
            }
            scratch.sort_unstable_by(|a, b| a.1.total_cmp(&b.1));
            let list = scratch.iter().map(|&(j, d_sq)| (j, d_sq.sqrt())).collect();
            self.planet_neighbors.push(list);
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
        self.claim_mask.clear();
        self.claim_mask.resize(n, 0);

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
            };
        }

        self.fleets.clear();
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

    pub fn step_tick(&mut self, silent: bool) -> Vec<EmpireStats> {
        let n = self.owner.len();
        let num_emp = self.empires.len();
        let mut stats = vec![EmpireStats::default(); num_emp];

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
                        stats[e].gates += 1;
                    }
                }
            }
        }

        let mut total_galactic_res = 1.0f32;
        let mut leader_e = 0usize;
        for e in 0..num_emp {
            let excess = self.p_counts[e].saturating_sub(10) as f32;
            let ele_discount = 1.0 / (1.0 + self.empires[e].tech_lv[4] * 0.05);
            let prt_sprawl = if self.empires[e].race.prt == "PRT_IT" {
                0.72
            } else {
                1.0
            };
            let sprawl_factor = self.cfg.sprawl_penalty * prt_sprawl * ele_discount;
            self.empires[e].sprawl_mult = 1.0 / (1.0 + excess * (sprawl_factor / 18.0));
            self.empires[e].race.doctrine.gates = stats[e].gates;

            total_galactic_res += self.empires[e].res_total;
            if self.empires[e].res_total > self.empires[leader_e].res_total {
                leader_e = e;
            }
        }

        let tick_usize = self.ticks as usize;

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

            let prt_he = self.empires[e].race.prt == "PRT_HE";
            let prt_ca = self.empires[e].race.prt == "PRT_CA";
            let prt_is = self.empires[e].race.prt == "PRT_IS";
            let prt_sd = self.empires[e].race.prt == "PRT_SD";
            let prt_pp = self.empires[e].race.prt == "PRT_PP";
            let prt_wm = self.empires[e].race.prt == "PRT_WM";
            let prt_ar = self.empires[e].race.prt == "PRT_AR";
            let prt_joat = self.empires[e].race.prt == "PRT_JOAT";

            let lrt = self.empires[e].race.lrt_mask;
            let lv = self.empires[e].tech_lv;
            let sprawl = self.empires[e].sprawl_mult;
            let tc_terra = self.empires[e].tech_cache.terraform_ability;
            let tc_mining = self.empires[e].tech_cache.mining_rate;

            let terra_bonus = (tc_terra / 100.0)
                * if lrt & LRT_TT != 0 { 1.30 } else { 1.0 }
                * if prt_ca { 1.25 } else { 1.0 };
            let eff_hab = (self.hab[i] + terra_bonus).min(1.15);

            let prt_pop_mod = if prt_he {
                0.72
            } else if prt_joat {
                1.18
            } else {
                1.0
            } * if lrt & LRT_OBRM != 0 { 1.08 } else { 1.0 };

            let max_pop = eff_hab * 170.0 * (1.0 + lv[5] * 0.015) * prt_pop_mod;
            let max_buildings = (max_pop / 10.0) * self.cfg.op_limit;
            let max_operable = (self.pop[i] / 10.0) * self.cfg.op_limit;

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

            let mining_mult = (1.0 + (tc_mining - 2.0).max(0.0) * 0.025)
                * if lrt & LRT_ARM != 0 { 1.18 } else { 1.0 }
                * if prt_ar { 1.15 } else { 1.0 };
            for m in 0..3 {
                if self.min_crust[base_idx + m] > 0.0 {
                    let extr =
                        self.min_crust[base_idx + m].min(act_mines * 0.55 * mining_mult * DT);
                    self.min_crust[base_idx + m] -= extr;
                    self.min_surf[base_idx + m] += extr;
                }
            }

            let fac_efficiency = if prt_is {
                1.12
            } else if prt_ar {
                1.15
            } else {
                1.0
            };
            let res_rate = (self.pop[i] * 0.32 + act_fac * fac_efficiency) * sprawl;
            let res_gen = res_rate * DT;
            self.last_res[i] = res_rate;

            stats[e].planets += 1;
            stats[e].factories += self.factories[i];
            stats[e].res += res_rate;

            let fac_share = self.empires[e].race.doctrine.fac_share;
            let ship_share = self.empires[e].race.doctrine.ship_share;
            let tech_share = self.empires[e].race.doctrine.tech_share;
            let col_pop_req = self.empires[e].race.doctrine.col_pop_req;
            let dist_exp = self.empires[e].race.doctrine.dist_exp;
            let qual_exp = self.empires[e].race.doctrine.qual_exp;
            let cruiser_share = self.empires[e].race.doctrine.cruiser_share;
            let bship_share = self.empires[e].race.doctrine.bship_share;
            let tech_alloc = self.empires[e].race.doctrine.tech_alloc;

            let research_mult = 1.0
                + if lrt & LRT_BET != 0 { 0.15 } else { 0.0 }
                + if lrt & LRT_GR != 0 { 0.12 } else { 0.0 };
            let research_gen = res_gen * tech_share * research_mult;
            for f in 0..NUM_TECH_FIELDS {
                self.empires[e].tech_pts[f] += research_gen * tech_alloc[f];
            }

            let room_for_fac = (max_buildings - self.factories[i]).max(0.0);
            let fac_budget = res_gen * fac_share;
            let built_fac = room_for_fac
                .min(fac_budget / self.cfg.fac_cost_res)
                .min(self.min_surf[base_idx + 2] / self.cfg.fac_cost_germ);
            self.factories[i] += built_fac;
            self.min_surf[base_idx + 2] -= built_fac * self.cfg.fac_cost_germ;

            let surplus_res = (fac_budget - built_fac * self.cfg.fac_cost_res).max(0.0);

            if surplus_res > 0.0 {
                let prt_alch = if prt_pp { 1.35 } else { 1.0 };
                let lrt_alch = if lrt & LRT_MA != 0 { 1.8 } else { 1.0 };
                let perk_alch = if self.empires[e].tech_cache.perk_mask & BP_ALCHEMY2 != 0 {
                    1.6
                } else {
                    1.0
                };
                let alch_base = 22.0 / (1.0 + lv[0] * 0.05);
                let transmuted = (surplus_res * 0.45 * prt_alch * lrt_alch * perk_alch) / alch_base;
                let mut min_m = 0usize;
                if self.min_surf[base_idx + 1] < self.min_surf[base_idx + min_m] {
                    min_m = 1;
                }
                if self.min_surf[base_idx + 2] < self.min_surf[base_idx + min_m] {
                    min_m = 2;
                }
                self.min_surf[base_idx + min_m] += transmuted;
            }

            // Stargate construction
            let has_stargate_tech = self.empires[e].tech_cache.stargate_name.is_some();
            if has_stargate_tech && self.has_gate[i] == 0 && self.factories[i] >= 38.0 {
                let gate_iron = self
                    .empires[e]
                    .tech_cache
                    .stargate_cost_iron
                    .min(self.cfg.gate_cost);
                let gate_germ = self
                    .empires[e]
                    .tech_cache
                    .stargate_cost_germ
                    .min(self.cfg.gate_cost);
                if self.min_surf[base_idx] >= gate_iron
                    && self.min_surf[base_idx + 2] >= gate_germ
                    && self.next_rand() < 0.08
                {
                    self.min_surf[base_idx] -= gate_iron;
                    self.min_surf[base_idx + 2] -= gate_germ;
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

            // Planetary defense
            let def_cov = self.empires[e].tech_cache.defense_coverage;
            let prt_def_mult = if prt_is || prt_sd { 1.28 } else { 1.0 }
                * if lrt & LRT_ISB != 0 { 1.15 } else { 1.0 };
            let def_cap = 65.0 * (0.7 + def_cov * 0.45) * prt_def_mult;
            if self.defense[i] < def_cap && self.min_surf[base_idx + 1] > 2.0 {
                let d_add = (def_cap - self.defense[i])
                    .min(surplus_res * if prt_is { 0.22 } else { 0.12 })
                    .min(self.min_surf[base_idx + 1] * 0.5);
                self.defense[i] += d_add;
                self.min_surf[base_idx + 1] -= d_add * 0.15;
            }

            // Accumulate shipyard buffer every tick
            if self.pop[i] > max_pop * col_pop_req {
                self.ship_prod_buf[i] += res_gen * ship_share + surplus_res * 0.5;
            }

            // Time-sliced AI target search (freighters, colonizers, warships) using precomputed neighbor lists
            let ai_turn = (i + tick_usize) % AI_DECISION_STRIDE == 0;
            if ai_turn {
                let gate_range = self.empires[e].tech_cache.stargate_max_range.max(250.0);

                // Freighter logistics (scaled probability for stride)
                if self.next_rand() < 0.25 {
                    let cargo_cap = (self.empires[e].tech_cache.hull_freight.cargo * 0.75)
                        .clamp(45.0, 220.0);
                    let freight_range = self.empires[e].ship_range(FleetClass::Freight);
                    for m in 0..3 {
                        if self.min_surf[base_idx + m] > 95.0 {
                            let mut needy: Option<usize> = None;
                            let mut lowest_val = 20.0f32;
                            for &(j_u16, dist) in &self.planet_neighbors[i] {
                                if dist > freight_range {
                                    break;
                                }
                                let j = j_u16 as usize;
                                if self.owner[j] == emp_u8 && self.min_surf[j * 3 + m] < lowest_val
                                {
                                    lowest_val = self.min_surf[j * 3 + m];
                                    needy = Some(j);
                                }
                            }
                            if let Some(target_idx) = needy {
                                let cargo = cargo_cap.min(self.min_surf[base_idx + m] * 0.45);
                                self.min_surf[base_idx + m] -= cargo;
                                self.spawn_fleet(
                                    i,
                                    target_idx,
                                    emp_u8,
                                    FleetClass::Freight,
                                    cargo,
                                    m as u8,
                                    silent,
                                );
                                break;
                            }
                        }
                    }
                }

                // Shipyard launch decision
                let con_discount = 1.0 / (1.0 + lv[3] * 0.035);
                let trait_ship_discount = if prt_wm { 0.85 } else { 1.0 }
                    * if lrt & LRT_CE != 0 { 0.88 } else { 1.0 };
                let effective_ship_cost =
                    self.cfg.col_ship_cost * con_discount * trait_ship_discount;

                if self.pop[i] > max_pop * col_pop_req
                    && self.ship_prod_buf[i] >= effective_ship_cost
                {
                    let mut best_target: Option<usize> = None;
                    let mut best_score = -1.0f32;
                    let mut mission = FleetClass::Col;
                    let my_bit = 1u16 << e;

                    let (col_i, _, col_g) = self.empires[e].ship_mineral_cost(FleetClass::Col);
                    let col_range = self.empires[e].ship_range(FleetClass::Col);
                    if neutral_planets_count > 0
                        && self.min_surf[base_idx] >= col_i
                        && self.min_surf[base_idx + 2] >= col_g
                    {
                        for &(j_u16, dist) in &self.planet_neighbors[i] {
                            if dist > col_range {
                                break;
                            }
                            let j = j_u16 as usize;
                            if self.owner[j] == 0 && (self.claim_mask[j] & my_bit) == 0 {
                                let d = dist.max(25.0);
                                let sum_crust = self.min_crust[j * 3]
                                    + self.min_crust[j * 3 + 1]
                                    + self.min_crust[j * 3 + 2];
                                let quality = self.hab[j].powf(1.5)
                                    * (0.4 + sum_crust / (self.cfg.crust_richness * 3.0));
                                let score = quality.powf(qual_exp) / (d / 180.0).powf(dist_exp);
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
                            let gate_range_sq = gate_range * gate_range;
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
                                for &(j_u16, dist) in self.planet_neighbors[g].iter().take(16) {
                                    if dist > col_range {
                                        break;
                                    }
                                    let j = j_u16 as usize;
                                    if self.owner[j] == 0 && (self.claim_mask[j] & my_bit) == 0 {
                                        let d = dist.max(25.0);
                                        let sum_crust = self.min_crust[j * 3]
                                            + self.min_crust[j * 3 + 1]
                                            + self.min_crust[j * 3 + 2];
                                        let quality = self.hab[j].powf(1.5)
                                            * (0.4 + sum_crust / (self.cfg.crust_richness * 3.0));
                                        let score =
                                            quality.powf(qual_exp) / (d / 180.0).powf(dist_exp);
                                        if score > best_score {
                                            best_score = score;
                                            best_target = Some(j);
                                            mission = FleetClass::Col;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if best_target.is_none() && self.pop[i] > 28.0 {
                        let r_roll = self.next_rand();
                        let try_order = if r_roll < cruiser_share {
                            [
                                FleetClass::Cruiser,
                                FleetClass::Battleship,
                                FleetClass::Bomber,
                            ]
                        } else if r_roll < cruiser_share + bship_share {
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

                            for &(j_u16, dist) in &self.planet_neighbors[i] {
                                if dist > ship_range {
                                    break;
                                }
                                let j = j_u16 as usize;
                                let target_owner = self.owner[j];
                                if target_owner != 0
                                    && target_owner != emp_u8
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
                                let gate_range_sq = gate_range * gate_range;
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
                                    for &(j_u16, dist) in self.planet_neighbors[g].iter().take(16) {
                                        if dist > ship_range {
                                            break;
                                        }
                                        let j = j_u16 as usize;
                                        let target_owner = self.owner[j];
                                        if target_owner != 0
                                            && target_owner != emp_u8
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
                                                    mission = c_class;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if let Some(target_idx) = best_target {
                        let (ci, cb, cg) = self.empires[e].ship_mineral_cost(mission);
                        if self.min_surf[base_idx] >= ci
                            && self.min_surf[base_idx + 1] >= cb
                            && self.min_surf[base_idx + 2] >= cg
                        {
                            self.ship_prod_buf[i] -= effective_ship_cost;
                            self.min_surf[base_idx] -= ci;
                            self.min_surf[base_idx + 1] -= cb;
                            self.min_surf[base_idx + 2] -= cg;
                            self.claim_mask[target_idx] |= my_bit;
                            self.pop[i] = (self.pop[i] - 6.0).max(2.0);
                            let power = self.compute_fleet_power(e, mission, self.factories[i]);
                            self.spawn_fleet(
                                i,
                                target_idx,
                                emp_u8,
                                mission,
                                power,
                                0,
                                silent,
                            );
                        }
                    }
                }
            }

            let mine_share = (1.0 - fac_share - ship_share - tech_share).max(0.05);
            let room_for_mines = if total_crust_here > 1.0 {
                (max_buildings - self.mines[i]).max(0.0)
            } else {
                0.0
            };
            self.mines[i] += room_for_mines.min((res_gen * mine_share) / 5.0);

            let bio_growth = 1.0 + lv[5] * 0.025;
            let prt_growth = if prt_he { 1.65 } else { 1.0 };
            let startup_factor = if self.pop[i] < 15.0 {
                0.4 + 0.6 * (self.pop[i] / 15.0)
            } else {
                1.0
            };
            let growth_delta = self.pop[i]
                * self.cfg.growth
                * bio_growth
                * prt_growth
                * startup_factor
                * (1.0 - self.pop[i] / max_pop)
                * DT;
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
            self.empires[e].res_total = stats[e].res;
            self.empires[e].last_stats = stats[e];
            self.empires[e].race.doctrine.last_fit = stats[e].res.round() as i32;

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

        // 3. Populate spatial grids for O(1) scanner pursuit & debris salvage
        let hit_rad = self.cfg.intercept_rad;
        let hit_rad_sq = hit_rad * hit_rad;
        let salvage_rad = hit_rad * 1.4;
        let salvage_rad_sq = salvage_rad * salvage_rad;
        let grid_cell = (hit_rad * 3.0).max(64.0);

        self.fleet_grid
            .reset(self.world_w, self.world_h, grid_cell, self.fleets.len());
        for (idx, fl) in self.fleets.iter().enumerate() {
            self.fleet_grid.insert(idx, fl.x, fl.y);
        }

        let has_debris = !self.debris_fields.is_empty();
        if has_debris {
            self.debris_grid.reset(
                self.world_w,
                self.world_h,
                grid_cell,
                self.debris_fields.len(),
            );
            for (idx, deb) in self.debris_fields.iter().enumerate() {
                self.debris_grid.insert(idx, deb.x, deb.y);
            }
        }

        // 4. Fleet movement, wormholes, grid-accelerated interception pursuit, salvage, arrival
        let mut f_idx = self.fleets.len();
        while f_idx > 0 {
            f_idx -= 1;
            let emp_u8 = self.fleets[f_idx].emp;
            let e = (emp_u8 - 1) as usize;
            let target_idx = self.fleets[f_idx].target;
            let mut tx = self.pos_x[target_idx];
            let mut ty = self.pos_y[target_idx];
            self.fleets[f_idx].hunting = false;

            if !self.fleets[f_idx].used_wh && !self.wormholes.is_empty() {
                let fx = self.fleets[f_idx].x;
                let fy = self.fleets[f_idx].y;
                let direct_dist = (tx - fx).hypot(ty - fy);
                for wh in &self.wormholes {
                    let d1_sq = (wh.x1 - fx) * (wh.x1 - fx) + (wh.y1 - fy) * (wh.y1 - fy);
                    if d1_sq <= 784.0 && (tx - wh.x2).hypot(ty - wh.y2) < direct_dist * 0.78 {
                        if !silent && self.pending_jump_fx.len() < 25 {
                            self.pending_jump_fx.push(JumpFx {
                                x1: fx,
                                y1: fy,
                                x2: wh.x2,
                                y2: wh.y2,
                                ttl: 12,
                                col: "#d946ef".to_string(),
                            });
                        }
                        self.fleets[f_idx].x = wh.x2;
                        self.fleets[f_idx].y = wh.y2;
                        self.fleets[f_idx].used_wh = true;
                        self.gate_jumps_count += 1;
                        break;
                    }
                    let d2_sq = (wh.x2 - fx) * (wh.x2 - fx) + (wh.y2 - fy) * (wh.y2 - fy);
                    if d2_sq <= 784.0 && (tx - wh.x1).hypot(ty - wh.y1) < direct_dist * 0.78 {
                        if !silent && self.pending_jump_fx.len() < 25 {
                            self.pending_jump_fx.push(JumpFx {
                                x1: fx,
                                y1: fy,
                                x2: wh.x1,
                                y2: wh.y1,
                                ttl: 12,
                                col: "#d946ef".to_string(),
                            });
                        }
                        self.fleets[f_idx].x = wh.x1;
                        self.fleets[f_idx].y = wh.y1;
                        self.fleets[f_idx].used_wh = true;
                        self.gate_jumps_count += 1;
                        break;
                    }
                }
            }

            // Cruiser interception using UniformGrid (only queries nearby cells instead of all fleets!)
            if self.fleets[f_idx].fleet_type == FleetClass::Cruiser
                && self.next_rand() < self.empires[e].race.doctrine.intercept_bias
            {
                let tc_scan = self.empires[e].tech_cache.scanner_range;
                let scan_max = (self.cfg.scanner_range * 0.65 + tc_scan * 0.45)
                    * if self.empires[e].race.prt == "PRT_SS" {
                        1.25
                    } else {
                        1.0
                    };
                let fx = self.fleets[f_idx].x;
                let fy = self.fleets[f_idx].y;
                let f_pow = self.fleets[f_idx].power;
                let mut best_prey: Option<(f32, f32)> = None;
                let mut best_prey_dist_sq = scan_max * scan_max;
                let fleets_ref = &self.fleets;

                self.fleet_grid.for_each_in_radius(fx, fy, scan_max, |k| {
                    if k >= fleets_ref.len() || k == f_idx {
                        return;
                    }
                    let other = &fleets_ref[k];
                    if other.emp != emp_u8
                        && (matches!(
                            other.fleet_type,
                            FleetClass::Bomber | FleetClass::Freight | FleetClass::Col
                        ) || other.power < f_pow * 0.9)
                    {
                        let dx = other.x - fx;
                        let dy = other.y - fy;
                        let d_sq = dx * dx + dy * dy;
                        if d_sq < best_prey_dist_sq {
                            best_prey_dist_sq = d_sq;
                            best_prey = Some((other.x, other.y));
                        }
                    }
                });

                if let Some((px, py)) = best_prey {
                    tx = px;
                    ty = py;
                    self.fleets[f_idx].hunting = true;
                }
            }

            // Debris salvage using UniformGrid
            let fx = self.fleets[f_idx].x;
            let fy = self.fleets[f_idx].y;
            if has_debris && !self.debris_fields.is_empty() {
                let from_idx = self.fleets[f_idx].from;
                let salvage_mult = if self.empires[e].race.lrt_mask & LRT_UR != 0 {
                    1.30
                } else {
                    1.0
                };
                let home_idx = if self.owner[from_idx] == emp_u8 {
                    from_idx
                } else {
                    e
                };
                let debris_slice = &mut self.debris_fields;
                let min_surf = &mut self.min_surf;
                let total_salvaged = &mut self.total_salvaged;

                self.debris_grid
                    .for_each_in_radius(fx, fy, salvage_rad, |d_idx| {
                        if d_idx >= debris_slice.len() {
                            return;
                        }
                        let deb = &mut debris_slice[d_idx];
                        if deb.ttl <= 0.0 {
                            return;
                        }
                        let dx = deb.x - fx;
                        let dy = deb.y - fy;
                        if dx * dx + dy * dy <= salvage_rad_sq {
                            min_surf[home_idx * 3] += deb.iron * salvage_mult;
                            min_surf[home_idx * 3 + 1] += deb.bora * salvage_mult;
                            min_surf[home_idx * 3 + 2] += deb.germ * salvage_mult;
                            *total_salvaged +=
                                ((deb.iron + deb.bora + deb.germ) * salvage_mult).round() as u32;
                            deb.ttl = 0.0;
                        }
                    });
            }

            let warp_mult = (self.empires[e].tech_cache.engine_warp / 6.0).clamp(0.75, 2.0);
            let ife_mult = if self.empires[e].race.lrt_mask & LRT_IFE != 0 {
                1.10
            } else {
                1.0
            };
            let step_dist = self.cfg.fleet_speed
                * warp_mult
                * ife_mult
                * self.fleets[f_idx].fleet_type.speed_mod()
                * DT;

            let tdx = self.pos_x[target_idx] - fx;
            let tdy = self.pos_y[target_idx] - fy;
            if tdx * tdx + tdy * tdy <= step_dist * step_dist {
                let fl = self.fleets.swap_remove(f_idx);
                self.claim_mask[target_idx] &= !(1u16 << e);

                if fl.fleet_type == FleetClass::Freight {
                    self.min_surf[target_idx * 3 + fl.min_type as usize] += fl.power;
                } else if self.owner[target_idx] == 0 {
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
                } else {
                    let def_e = (self.owner[target_idx] - 1) as usize;
                    let mut bunker_bonus: f32 = if self.empires[def_e].race.prt == "PRT_IS" {
                        1.30
                    } else {
                        1.0
                    };
                    let mut effective_def = self.defense[target_idx];
                    if fl.fleet_type == FleetClass::Bomber {
                        let pen = (self.cfg.bomber_pen
                            + (self.empires[e].tech_cache.bomb_power * 0.02).min(0.22))
                        .min(0.88);
                        effective_def *= 1.0 - pen;
                        bunker_bonus = (bunker_bonus * 0.75).max(1.0);
                    }
                    if fl.power
                        > (20.0 + effective_def + self.pop[target_idx] * 0.35) * bunker_bonus
                    {
                        self.owner[target_idx] = fl.emp;
                        self.pop[target_idx] = (self.pop[target_idx] * 0.65).max(10.0);
                        let struct_surv = if fl.fleet_type == FleetClass::Bomber {
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
                    } else {
                        let dmg = if fl.fleet_type == FleetClass::Bomber {
                            fl.power * 0.85
                        } else {
                            (fl.power / bunker_bonus) * 0.45
                        };
                        self.defense[target_idx] = (self.defense[target_idx] - dmg).max(4.0);
                        if fl.fleet_type == FleetClass::Bomber {
                            self.factories[target_idx] =
                                (self.factories[target_idx] * 0.92).max(2.0);
                        }
                    }
                }
            } else {
                let dx = tx - fx;
                let dy = ty - fy;
                let dist_sq = dx * dx + dy * dy;
                if dist_sq > 0.01 {
                    let inv_d = step_dist / dist_sq.sqrt();
                    self.fleets[f_idx].x += dx * inv_d;
                    self.fleets[f_idx].y += dy * inv_d;
                }
            }
        }

        // 5. Deep space combat using UniformGrid (O(F) average, 3x3 cell neighborhood only!)
        let fl_len = self.fleets.len();
        if fl_len > 1 {
            let combat_cell = hit_rad.max(20.0);
            self.fleet_grid
                .reset(self.world_w, self.world_h, combat_cell, fl_len);
            for (idx, fl) in self.fleets.iter().enumerate() {
                self.fleet_grid.insert(idx, fl.x, fl.y);
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
                self.fleet_grid.for_each_in_radius(ax, ay, hit_rad, |j| {
                    if j > i {
                        self.combat_candidates.push(j);
                    }
                });

                for c_idx in 0..self.combat_candidates.len() {
                    let j = self.combat_candidates[c_idx];
                    if self.fleets[i].power <= 0.0 {
                        break;
                    }
                    if self.fleets[j].power <= 0.0 || emp_a == self.fleets[j].emp {
                        continue;
                    }
                    let type_b = self.fleets[j].fleet_type;
                    let b_c = type_b.is_combat();
                    if !a_c && !b_c {
                        continue;
                    }

                    let bx = self.fleets[j].x;
                    let by = self.fleets[j].y;
                    let dx = ax - bx;
                    let dy = ay - by;
                    if dx * dx + dy * dy <= hit_rad_sq {
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
                        if self.empires[ea].race.prt == "PRT_SS" {
                            mod_a *= 1.12;
                        }
                        if self.empires[eb].race.prt == "PRT_SS" {
                            mod_b *= 1.12;
                        }

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
            if self.debris_fields[i].ttl <= 0.0 {
                self.debris_fields.swap_remove(i);
            }
        }

        self.year += DT;
        self.ticks += 1;

        if !silent && self.ticks % 5 == 0 {
            self.power_history
                .push(stats.iter().map(|s| s.res).collect());
            if self.power_history.len() > MAX_POWER_HISTORY {
                self.power_history.remove(0);
            }
        }

        stats
    }

    fn compute_fleet_power(&self, e: usize, class: FleetClass, factories_here: f32) -> f32 {
        let emp = &self.empires[e];
        let tc = &emp.tech_cache;
        let hull = emp.hull_for_class(class);

        let wep_scale = (1.0 + (tc.weapon_power / 45.0).sqrt() * 0.65)
            * if emp.race.prt == "PRT_WM" { 1.22 } else { 1.0 };
        let shield_mult = if emp.race.lrt_mask & LRT_RS != 0 {
            1.25
        } else {
            1.0
        };
        let dur_scale =
            1.0 + ((hull.armor + tc.armor_val + tc.shield_val * shield_mult) / 400.0).sqrt() * 0.35;
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
        silent: bool,
    ) {
        let e = (emp_u8 - 1) as usize;
        let mut start_x = self.pos_x[from_idx];
        let mut start_y = self.pos_y[from_idx];
        let mut jumped_gate = false;
        let gate_range = self.empires[e].tech_cache.stargate_max_range.max(250.0);

        if self.has_gate[from_idx] != 0 && self.gate_lists[e].len() > 1 {
            // Find the closest friendly Stargate to the target via target's sorted neighbor list (O(1))
            let direct_thresh =
                (self.pos_x[target_idx] - start_x).hypot(self.pos_y[target_idx] - start_y) * 0.72;
            let gate_range_sq = gate_range * gate_range;
            let mut best_gate: Option<usize> = None;
            for &(cand_u16, dist_to_target) in &self.planet_neighbors[target_idx] {
                if dist_to_target >= direct_thresh {
                    break;
                }
                let g_idx = cand_u16 as usize;
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
        });
    }

    fn spawn_salvage_debris(&mut self, x: f32, y: f32, dead_fleet: &Fleet) {
        if self.cfg.salvage_rate <= 0.0 {
            return;
        }
        let field = if dead_fleet.fleet_type == FleetClass::Freight {
            let mut m = [0.0f32; 3];
            m[dead_fleet.min_type as usize] = dead_fleet.max_power * self.cfg.salvage_rate;
            DebrisField {
                x,
                y,
                iron: m[0],
                bora: m[1],
                germ: m[2],
                ttl: 12.0,
            }
        } else {
            let e = (dead_fleet.emp - 1) as usize;
            let (ci, cb, cg) = self.empires[e].ship_mineral_cost(dead_fleet.fleet_type);
            DebrisField {
                x,
                y,
                iron: ci * self.cfg.salvage_rate,
                bora: cb * self.cfg.salvage_rate,
                germ: cg * self.cfg.salvage_rate,
                ttl: 12.0,
            }
        };
        if self.debris_fields.len() >= MAX_DEBRIS_FIELDS {
            self.debris_fields.swap_remove(0);
        }
        self.debris_fields.push(field);
    }

    pub fn update_param(&mut self, key: &str, value: f32) -> bool {
        let mut needs_reset = false;
        match key {
            "planets" => {
                self.cfg.planets = (value.round() as usize).clamp(30, 1000);
                needs_reset = true;
            }
            "wormholePairs" => {
                self.cfg.wormhole_pairs = (value.round() as usize).clamp(0, 5);
                needs_reset = true;
            }
            "crustRichness" => {
                self.cfg.crust_richness = value.clamp(300.0, 2500.0);
                needs_reset = true;
            }
            "gateCost" => self.cfg.gate_cost = value.clamp(15.0, 120.0),
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
            self.planet_render_buf.push(self.pos_x[i]);
            self.planet_render_buf.push(self.pos_y[i]);
            self.planet_render_buf.push(self.owner[i] as f32);
            self.planet_render_buf.push(self.factories[i]);
            self.planet_render_buf.push(self.hab[i]);
            self.planet_render_buf.push(self.has_gate[i] as f32);
            self.planet_render_buf.push(crust_sum);
        }

        self.fleet_render_buf.clear();
        self.fleet_render_buf
            .reserve(self.fleets.len() * FLEET_RENDER_STRIDE);
        for fl in &self.fleets {
            let type_code = match fl.fleet_type {
                FleetClass::Col => 0.0,
                FleetClass::Cruiser => 1.0,
                FleetClass::Battleship => 2.0,
                FleetClass::Bomber => 3.0,
                FleetClass::Freight => 4.0,
            };
            let flags = (if fl.hunting { 1 } else { 0 }) | (if fl.used_wh { 2 } else { 0 });
            self.fleet_render_buf.push(fl.x);
            self.fleet_render_buf.push(fl.y);
            self.fleet_render_buf.push(self.pos_x[fl.target]);
            self.fleet_render_buf.push(self.pos_y[fl.target]);
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
        }
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
        })
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
        }
    }

    pub fn take_reports_snapshot(&self) -> ReportsSnapshot {
        let planets = (0..self.owner.len())
            .filter_map(|i| self.planet_view(i))
            .collect();
        ReportsSnapshot {
            planets,
            fleets: self.fleets.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_six_empire_simulation() {
        let mut a = Simulation::new(SimConfig::default(), classic_races());
        let mut b = Simulation::new(SimConfig::default(), classic_races());
        a.init(80000, true);
        b.init(80000, true);

        let mut sa = Vec::new();
        let mut sb = Vec::new();
        for _ in 0..400 {
            sa = a.step_tick(true);
            sb = b.step_tick(true);
        }

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
}
