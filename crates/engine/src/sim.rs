use std::f32::consts::TAU;

use crate::config::{
    FleetClass, SimConfig, BREAKTHROUGHS, BP_ALCHEMY2, BP_PHASE_DEF, BP_SCANNERS, BP_STARGATE,
    BP_TERRA, BP_WARP10, DT, MAX_DEBRIS_FIELDS, MAX_EVENT_LOG, MAX_POWER_HISTORY, NUM_EMPIRES,
    NUM_TECH_FIELDS, WORLD_H, WORLD_W,
};
use crate::types::{
    default_doctrines, CombatFx, DebrisField, Doctrine, EmpireStats, Fleet, JumpFx, PlanetView,
    SimEvent, SimSnapshot, Wormhole,
};

#[derive(Debug, Clone)]
pub struct Simulation {
    pub cfg: SimConfig,
    pub doctrines: [Doctrine; NUM_EMPIRES],
    pub dna_generation: u32,

    pub year: f32,
    pub ticks: u64,
    pub current_map_seed: u32,
    pub rng_seed: u32,

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
    pub claim_mask: Vec<u8>,

    // Empire state (0-based: 0=Industry, 1=Fleet, 2=Science)
    pub tech_pts: [[f32; NUM_TECH_FIELDS]; NUM_EMPIRES],
    pub tech_lv: [[f32; NUM_TECH_FIELDS]; NUM_EMPIRES],
    pub unlocked_bp: [u8; NUM_EMPIRES],
    pub sprawl_mult: [f32; NUM_EMPIRES],
    pub emp_res_total: [f32; NUM_EMPIRES],
    pub last_stats: [EmpireStats; NUM_EMPIRES],

    // Dynamic world entities
    pub fleets: Vec<Fleet>,
    pub wormholes: Vec<Wormhole>,
    pub debris_fields: Vec<DebrisField>,
    pub pending_combat_fx: Vec<CombatFx>,
    pub pending_jump_fx: Vec<JumpFx>,
    pub power_history: Vec<[f32; NUM_EMPIRES]>,
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
        let mut sim = Self::new(SimConfig::default());
        sim.init(12345, false);
        sim
    }
}

impl Simulation {
    pub fn new(cfg: SimConfig) -> Self {
        Self {
            cfg,
            doctrines: default_doctrines(),
            dna_generation: 0,
            year: 2400.0,
            ticks: 0,
            current_map_seed: 1,
            rng_seed: 12345,
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
            tech_pts: [[0.0; NUM_TECH_FIELDS]; NUM_EMPIRES],
            tech_lv: [[0.0; NUM_TECH_FIELDS]; NUM_EMPIRES],
            unlocked_bp: [0; NUM_EMPIRES],
            sprawl_mult: [1.0; NUM_EMPIRES],
            emp_res_total: [25.0; NUM_EMPIRES],
            last_stats: [EmpireStats::default(); NUM_EMPIRES],
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

    #[inline]
    pub fn next_rand(&mut self) -> f32 {
        self.rng_seed = self
            .rng_seed
            .wrapping_mul(1664525)
            .wrapping_add(1013904223);
        (self.rng_seed as f64 / 4294967296.0) as f32
    }

    #[inline]
    pub fn emp_perks(&self, emp_idx: usize) -> u8 {
        let lv = &self.tech_lv[emp_idx];
        let mut mask = 0u8;
        for bp in &BREAKTHROUGHS {
            if lv[bp.req[0].0] >= bp.req[0].1 && lv[bp.req[1].0] >= bp.req[1].1 {
                mask |= bp.bit;
            }
        }
        mask
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

    pub fn init(&mut self, seed: u32, silent: bool) {
        self.current_map_seed = seed;
        self.rng_seed = seed;
        let n = self.cfg.planets.max(3);

        self.pos_x.resize(n, 0.0);
        self.pos_y.resize(n, 0.0);
        self.pop.clear();
        self.pop.resize(n, 0.0);
        self.hab.resize(n, 0.0);
        self.mines.clear();
        self.mines.resize(n, 0.0);
        self.factories.clear();
        self.factories.resize(n, 0.0);
        self.min_surf.clear();
        self.min_surf.resize(n * 3, 0.0);
        self.min_crust.clear();
        self.min_crust.resize(n * 3, 0.0);
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

        for i in 0..n {
            self.pos_x[i] = 60.0 + self.next_rand() * (WORLD_W - 120.0);
            self.pos_y[i] = 60.0 + self.next_rand() * (WORLD_H - 120.0);
            self.hab[i] = (0.25 + self.next_rand().powf(1.3) * 0.75).clamp(0.25, 1.0);
            for m in 0..3 {
                self.min_crust[i * 3 + m] =
                    self.cfg.crust_richness * (0.2 + self.next_rand().powf(1.4) * 1.5);
                self.min_surf[i * 3 + m] = 0.0;
            }
            self.defense[i] = 0.0;
        }

        let base_angle = self.next_rand() * TAU;
        let cx = WORLD_W * 0.5;
        let cy = WORLD_H * 0.5;
        let rx = 330.0;
        let ry = 185.0;

        for e in 0..NUM_EMPIRES {
            let ang = base_angle + (e as f32) * (TAU / 3.0);
            self.pos_x[e] = cx + ang.cos() * rx;
            self.pos_y[e] = cy + ang.sin() * ry;
            self.hab[e] = 1.0;
            self.pop[e] = 42.0;
            self.mines[e] = 18.0;
            self.factories[e] = 16.0;
            for m in 0..3 {
                self.min_surf[e * 3 + m] = 160.0;
                self.min_crust[e * 3 + m] = self.cfg.crust_richness * 1.6;
            }
            self.defense[e] = 40.0;
            self.owner[e] = (e + 1) as u8;
            self.doctrines[e].kills = 0;
            self.doctrines[e].gates = 0;
            self.tech_pts[e] = [0.0; NUM_TECH_FIELDS];
            self.tech_lv[e] = [0.0; NUM_TECH_FIELDS];
            self.unlocked_bp[e] = 0;
        }

        self.wormholes.clear();
        for _ in 0..self.cfg.wormhole_pairs {
            let x1 = 90.0 + self.next_rand() * 340.0;
            let y1 = 80.0 + self.next_rand() * (WORLD_H - 160.0);
            let x2 = 570.0 + self.next_rand() * 340.0;
            let y2 = 80.0 + self.next_rand() * (WORLD_H - 160.0);
            self.wormholes.push(Wormhole { x1, y1, x2, y2 });
        }

        self.sprawl_mult = [1.0; NUM_EMPIRES];
        self.emp_res_total = [25.0; NUM_EMPIRES];
        self.last_stats = [
            EmpireStats {
                planets: 1,
                factories: 16.0,
                res: 28.0,
                gates: 0,
            },
            EmpireStats {
                planets: 1,
                factories: 16.0,
                res: 25.0,
                gates: 0,
            },
            EmpireStats {
                planets: 1,
                factories: 16.0,
                res: 25.0,
                gates: 0,
            },
        ];

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
                format!("New galaxy #{} ({} systems).", seed % 1000, n),
                "#38bdf8",
            );
        }
    }

    pub fn step_tick(&mut self, silent: bool) -> [EmpireStats; NUM_EMPIRES] {
        let n = self.owner.len();
        let mut stats = [EmpireStats::default(); NUM_EMPIRES];
        let perks = [self.emp_perks(0), self.emp_perks(1), self.emp_perks(2)];

        let mut p_counts = [0u32; NUM_EMPIRES];
        let mut gate_lists: [Vec<usize>; NUM_EMPIRES] = [Vec::new(), Vec::new(), Vec::new()];
        for i in 0..n {
            let emp = self.owner[i];
            if emp > 0 {
                let e = (emp - 1) as usize;
                p_counts[e] += 1;
                if self.has_gate[i] != 0 {
                    gate_lists[e].push(i);
                    stats[e].gates += 1;
                }
            }
        }

        for e in 0..NUM_EMPIRES {
            let excess = p_counts[e].saturating_sub(10) as f32;
            let ele_discount = 1.0 / (1.0 + self.tech_lv[e][4] * 0.08);
            let sprawl_factor = (if e == 1 {
                self.cfg.sprawl_penalty * 0.72
            } else {
                self.cfg.sprawl_penalty
            }) * ele_discount;
            self.sprawl_mult[e] = 1.0 / (1.0 + excess * (sprawl_factor / 18.0));
            self.doctrines[e].gates = stats[e].gates;
        }

        let total_galactic_res =
            self.emp_res_total[0] + self.emp_res_total[1] + self.emp_res_total[2] + 1.0;
        let mut leader_e = 0usize;
        if self.emp_res_total[1] > self.emp_res_total[leader_e] {
            leader_e = 1;
        }
        if self.emp_res_total[2] > self.emp_res_total[leader_e] {
            leader_e = 2;
        }

        // Planetary economy, production, and shipyard decisions
        for i in 0..n {
            let emp = self.owner[i];
            if emp == 0 {
                continue;
            }
            let e = (emp - 1) as usize;
            let pk = perks[e];
            let lv = self.tech_lv[e];
            let base_idx = i * 3;

            let max_pop = self.hab[i]
                * 170.0
                * (1.0 + lv[5] * 0.03)
                * if pk & BP_TERRA != 0 { 1.20 } else { 1.0 };
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

            let ind_boost = if e == 0 { self.cfg.ind_weight } else { 1.0 };
            for m in 0..3 {
                if self.min_crust[base_idx + m] > 0.0 {
                    let extr = self.min_crust[base_idx + m].min(act_mines * 0.55 * ind_boost * DT);
                    self.min_crust[base_idx + m] -= extr;
                    self.min_surf[base_idx + m] += extr;
                }
            }

            let fac_efficiency = if e == 0 {
                1.15 * self.cfg.ind_weight
            } else {
                1.0
            };
            let res_rate = (self.pop[i] * 0.32 + act_fac * fac_efficiency) * self.sprawl_mult[e];
            let res_gen = res_rate * DT;
            self.last_res[i] = res_rate;

            stats[e].planets += 1;
            stats[e].factories += self.factories[i];
            stats[e].res += res_rate;

            let tech_share = self.doctrines[e].tech_share;
            let tech_alloc = self.doctrines[e].tech_alloc;
            let fac_share = self.doctrines[e].fac_share;
            let ship_share = self.doctrines[e].ship_share;
            let col_pop_req = self.doctrines[e].col_pop_req;
            let dist_exp = self.doctrines[e].dist_exp;
            let qual_exp = self.doctrines[e].qual_exp;
            let cruiser_share = self.doctrines[e].cruiser_share;
            let bship_share = self.doctrines[e].bship_share;

            let research_gen = res_gen
                * tech_share
                * if e == 2 {
                    1.35 * self.cfg.tech_weight
                } else {
                    1.0
                };
            for f in 0..NUM_TECH_FIELDS {
                self.tech_pts[e][f] += research_gen * tech_alloc[f];
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
                let alch_base = (if e == 0 {
                    15.0 / self.cfg.ind_weight
                } else {
                    24.0
                }) / (1.0 + lv[0] * 0.08);
                let transmuted = (surplus_res
                    * 0.45
                    * if pk & BP_ALCHEMY2 != 0 { 2.0 } else { 1.0 })
                    / alch_base;
                let mut min_m = 0usize;
                if self.min_surf[base_idx + 1] < self.min_surf[base_idx + min_m] {
                    min_m = 1;
                }
                if self.min_surf[base_idx + 2] < self.min_surf[base_idx + min_m] {
                    min_m = 2;
                }
                self.min_surf[base_idx + min_m] += transmuted;
            }

            if (pk & BP_STARGATE != 0)
                && self.has_gate[i] == 0
                && self.factories[i] >= 42.0
                && self.min_surf[base_idx] >= self.cfg.gate_cost
                && self.min_surf[base_idx + 2] >= self.cfg.gate_cost
                && self.next_rand() < 0.08
            {
                self.min_surf[base_idx] -= self.cfg.gate_cost;
                self.min_surf[base_idx + 2] -= self.cfg.gate_cost;
                self.has_gate[i] = 1;
                gate_lists[e].push(i);
                if !silent && gate_lists[e].len() == 1 {
                    let msg = format!(
                        "{} built its first 🌀 Stargate (#{})!",
                        self.doctrines[e].name,
                        i + 1
                    );
                    let col = self.doctrines[e].color.clone();
                    self.push_event(msg, &col);
                }
            }

            let def_cap = (if e == 0 {
                135.0 * self.cfg.ind_weight
            } else {
                75.0
            }) * (1.0 + lv[0] * 0.07)
                * if pk & BP_PHASE_DEF != 0 { 1.35 } else { 1.0 };
            if self.defense[i] < def_cap && self.min_surf[base_idx + 1] > 2.0 {
                let d_add = (def_cap - self.defense[i])
                    .min(surplus_res * if e == 0 { 0.24 } else { 0.11 })
                    .min(self.min_surf[base_idx + 1] * 0.5);
                self.defense[i] += d_add;
                self.min_surf[base_idx + 1] -= d_add * 0.15;
            }

            if self.next_rand() < 0.05 {
                for m in 0..3 {
                    if self.min_surf[base_idx + m] > 95.0 {
                        let mut needy: Option<usize> = None;
                        let mut lowest_val = 20.0f32;
                        for j in 0..n {
                            if self.owner[j] == emp
                                && j != i
                                && self.min_surf[j * 3 + m] < lowest_val
                            {
                                lowest_val = self.min_surf[j * 3 + m];
                                needy = Some(j);
                            }
                        }
                        if let Some(target_idx) = needy {
                            let cargo = 55.0f32.min(self.min_surf[base_idx + m] * 0.45);
                            self.min_surf[base_idx + m] -= cargo;
                            self.spawn_fleet(
                                i,
                                target_idx,
                                emp,
                                FleetClass::Freight,
                                cargo,
                                m as u8,
                                &gate_lists[e],
                                silent,
                            );
                            break;
                        }
                    }
                }
            }

            let con_discount = 1.0 / (1.0 + lv[3] * 0.06);
            let effective_ship_cost = (if e == 1 {
                self.cfg.col_ship_cost / (1.18 * self.cfg.exp_weight)
            } else {
                self.cfg.col_ship_cost
            }) * con_discount;

            if self.pop[i] > max_pop * col_pop_req {
                self.ship_prod_buf[i] += res_gen * ship_share + surplus_res * 0.5;

                if self.ship_prod_buf[i] >= effective_ship_cost {
                    let mut best_target: Option<usize> = None;
                    let mut best_score = -1.0f32;
                    let mut mission = FleetClass::Col;
                    let my_bit = 1u8 << emp;

                    let col_dsg = FleetClass::Col.design();
                    if self.min_surf[base_idx] >= col_dsg.iron
                        && self.min_surf[base_idx + 2] >= col_dsg.germ
                    {
                        for j in 0..n {
                            if self.owner[j] == 0 && (self.claim_mask[j] & my_bit) == 0 {
                                let d = (self.pos_x[j] - self.pos_x[i])
                                    .hypot(self.pos_y[j] - self.pos_y[i])
                                    .max(25.0);
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
                    }

                    if best_target.is_none() && self.pop[i] > 30.0 {
                        let r_roll = self.next_rand();
                        let try_order = if r_roll < cruiser_share {
                            [FleetClass::Cruiser, FleetClass::Battleship, FleetClass::Bomber]
                        } else if r_roll < cruiser_share + bship_share {
                            [FleetClass::Battleship, FleetClass::Bomber, FleetClass::Cruiser]
                        } else {
                            [FleetClass::Bomber, FleetClass::Cruiser, FleetClass::Battleship]
                        };

                        let mut chosen_class: Option<FleetClass> = None;
                        for c_class in try_order {
                            let dsg = c_class.design();
                            if self.min_surf[base_idx] >= dsg.iron
                                && self.min_surf[base_idx + 1] >= dsg.bora
                                && self.min_surf[base_idx + 2] >= dsg.germ
                            {
                                chosen_class = Some(c_class);
                                break;
                            }
                        }

                        if let Some(c_class) = chosen_class {
                            let wep_bonus = (1.0
                                + lv[1]
                                    * if e == 2 {
                                        0.22 * self.cfg.tech_weight
                                    } else {
                                        0.12
                                    })
                                * if pk & BP_PHASE_DEF != 0 { 1.20 } else { 1.0 };
                            let my_attack_est = (36.0 + self.factories[i] * 0.5) * wep_bonus;

                            for j in 0..n {
                                let target_owner = self.owner[j];
                                if target_owner != 0
                                    && target_owner != emp
                                    && (self.claim_mask[j] & my_bit) == 0
                                {
                                    let target_e = (target_owner - 1) as usize;
                                    let d = (self.pos_x[j] - self.pos_x[i])
                                        .hypot(self.pos_y[j] - self.pos_y[i])
                                        .max(30.0);
                                    let eff_def = if c_class == FleetClass::Bomber {
                                        self.defense[j] * (1.0 - self.cfg.bomber_pen)
                                    } else {
                                        self.defense[j]
                                    };
                                    let enemy_def = 20.0 + eff_def + self.pop[j] * 0.35;
                                    let target_share =
                                        self.emp_res_total[target_e] / total_galactic_res;
                                    let leader_aggro_bonus =
                                        if target_e == leader_e && target_share > 0.38 {
                                            1.0 + (target_share - 0.33) * 4.0 * self.cfg.aggro_bias
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
                        }
                    }

                    if let Some(target_idx) = best_target {
                        let dsg = mission.design();
                        if self.min_surf[base_idx] >= dsg.iron
                            && self.min_surf[base_idx + 1] >= dsg.bora
                            && self.min_surf[base_idx + 2] >= dsg.germ
                        {
                            self.ship_prod_buf[i] -= effective_ship_cost;
                            self.min_surf[base_idx] -= dsg.iron;
                            self.min_surf[base_idx + 1] -= dsg.bora;
                            self.min_surf[base_idx + 2] -= dsg.germ;
                            self.claim_mask[target_idx] |= my_bit;
                            self.pop[i] -= 7.0;

                            let wep_bonus = (1.0
                                + lv[1]
                                    * if e == 2 {
                                        0.22 * self.cfg.tech_weight
                                    } else {
                                        0.12
                                    })
                                * if pk & BP_PHASE_DEF != 0 { 1.20 } else { 1.0 };
                            let class_power_mod = match mission {
                                FleetClass::Battleship => 1.35,
                                FleetClass::Bomber => 1.15,
                                _ => 1.05,
                            };
                            let power =
                                (35.0 + self.factories[i] * 0.5) * wep_bonus * class_power_mod;
                            self.spawn_fleet(
                                i,
                                target_idx,
                                emp,
                                mission,
                                power,
                                0,
                                &gate_lists[e],
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

            let bio_growth = 1.0 + lv[5] * 0.04;
            let exp_growth_mod = if e == 1 {
                1.06 * self.cfg.exp_weight
            } else {
                1.0
            };
            let startup_factor = if self.pop[i] < 15.0 {
                0.4 + 0.6 * (self.pop[i] / 15.0)
            } else {
                1.0
            };
            let growth_delta = self.pop[i]
                * self.cfg.growth
                * bio_growth
                * exp_growth_mod
                * startup_factor
                * (1.0 - self.pop[i] / max_pop)
                * DT;
            self.pop[i] = (self.pop[i] + growth_delta).max(1.0).min(max_pop);
        }

        // Update 6 tech levels (logarithmic progression) and breakthrough notifications
        for e in 0..NUM_EMPIRES {
            for f in 0..NUM_TECH_FIELDS {
                self.tech_lv[e][f] = (1.0 + self.tech_pts[e][f] / 38.0).log2();
            }
            self.emp_res_total[e] = stats[e].res;
            self.doctrines[e].last_fit = stats[e].res.round() as i32;

            if !silent {
                let pk = self.emp_perks(e);
                for bp in &BREAKTHROUGHS {
                    if (pk & bp.bit != 0) && (self.unlocked_bp[e] & bp.bit == 0) {
                        self.unlocked_bp[e] |= bp.bit;
                        let msg = format!(
                            "{} unlocked: {} {}!",
                            self.doctrines[e].name, bp.icon, bp.name
                        );
                        let col = self.doctrines[e].color.clone();
                        self.push_event(msg, &col);
                    }
                }
            }
        }

        // Fleet movement, wormholes, interception pursuit, salvage, arrival
        let hit_rad = self.cfg.intercept_rad;
        let mut f_idx = self.fleets.len();
        while f_idx > 0 {
            f_idx -= 1;
            let emp = self.fleets[f_idx].emp;
            let e = (emp - 1) as usize;
            let pk = perks[e];
            let lv = self.tech_lv[e];
            let target_idx = self.fleets[f_idx].target;
            let mut tx = self.pos_x[target_idx];
            let mut ty = self.pos_y[target_idx];
            self.fleets[f_idx].hunting = false;

            if !self.fleets[f_idx].used_wh && !self.wormholes.is_empty() {
                let fx = self.fleets[f_idx].x;
                let fy = self.fleets[f_idx].y;
                let direct_dist = (tx - fx).hypot(ty - fy);
                for wh in &self.wormholes {
                    if (wh.x1 - fx).hypot(wh.y1 - fy) <= 28.0
                        && (tx - wh.x2).hypot(ty - wh.y2) < direct_dist * 0.78
                    {
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
                    if (wh.x2 - fx).hypot(wh.y2 - fy) <= 28.0
                        && (tx - wh.x1).hypot(ty - wh.y1) < direct_dist * 0.78
                    {
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

            let scan_max = self.cfg.scanner_range
                * (1.0 + lv[4] * 0.08)
                * if pk & BP_SCANNERS != 0 { 1.5 } else { 1.0 };
            if self.fleets[f_idx].fleet_type == FleetClass::Cruiser
                && self.next_rand() < self.doctrines[e].intercept_bias
            {
                let fx = self.fleets[f_idx].x;
                let fy = self.fleets[f_idx].y;
                let f_pow = self.fleets[f_idx].power;
                let mut best_prey: Option<(f32, f32)> = None;
                let mut best_prey_dist = scan_max;

                for other in &self.fleets {
                    if other.emp != emp
                        && (matches!(
                            other.fleet_type,
                            FleetClass::Bomber | FleetClass::Freight | FleetClass::Col
                        ) || other.power < f_pow * 0.9)
                    {
                        let d = (other.x - fx).hypot(other.y - fy);
                        if d < best_prey_dist {
                            best_prey_dist = d;
                            best_prey = Some((other.x, other.y));
                        }
                    }
                }

                if let Some((px, py)) = best_prey {
                    tx = px;
                    ty = py;
                    self.fleets[f_idx].hunting = true;
                }
            }

            let fx = self.fleets[f_idx].x;
            let fy = self.fleets[f_idx].y;
            let from_idx = self.fleets[f_idx].from;
            let mut d_idx = self.debris_fields.len();
            while d_idx > 0 {
                d_idx -= 1;
                let deb = self.debris_fields[d_idx];
                if (deb.x - fx).hypot(deb.y - fy) <= hit_rad * 1.4 {
                    let home_idx = if self.owner[from_idx] == emp {
                        from_idx
                    } else {
                        e
                    };
                    self.min_surf[home_idx * 3] += deb.iron;
                    self.min_surf[home_idx * 3 + 1] += deb.bora;
                    self.min_surf[home_idx * 3 + 2] += deb.germ;
                    self.total_salvaged += (deb.iron + deb.bora + deb.germ).round() as u32;
                    self.debris_fields.remove(d_idx);
                }
            }

            let dist = (tx - fx).hypot(ty - fy);
            let dsg_speed = self.fleets[f_idx].fleet_type.design().speed_mod;
            let step_dist = self.cfg.fleet_speed
                * (1.0 + lv[2] * 0.09)
                * dsg_speed
                * if pk & BP_WARP10 != 0 { 1.35 } else { 1.0 }
                * DT;

            if (self.pos_x[target_idx] - fx).hypot(self.pos_y[target_idx] - fy) <= step_dist {
                let fl = self.fleets.remove(f_idx);
                self.claim_mask[target_idx] &= !(1u8 << fl.emp);

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
                    self.pop[target_idx] = (self.hab[target_idx] * 170.0).min(self.pop[target_idx] + 7.0);
                } else {
                    let def_owner = self.owner[target_idx];
                    let mut bunker_bonus = if def_owner == 1 {
                        1.28 * self.cfg.ind_weight
                    } else {
                        1.0
                    };
                    let mut effective_def = self.defense[target_idx];
                    if fl.fleet_type == FleetClass::Bomber {
                        effective_def *= 1.0 - self.cfg.bomber_pen;
                        bunker_bonus = (bunker_bonus * 0.75).max(1.0);
                    }
                    if fl.power > (20.0 + effective_def + self.pop[target_idx] * 0.35) * bunker_bonus {
                        self.owner[target_idx] = fl.emp;
                        self.pop[target_idx] = (self.pop[target_idx] * 0.65).max(10.0);
                        self.factories[target_idx] *= if fl.fleet_type == FleetClass::Bomber {
                            0.68
                        } else {
                            0.82
                        };
                        self.defense[target_idx] = 20.0;
                        self.invasions_count += 1;
                        if !silent && self.has_gate[target_idx] != 0 {
                            let fe = (fl.emp - 1) as usize;
                            let msg = format!(
                                "{} captured Stargate hub #{}!",
                                self.doctrines[fe].name,
                                target_idx + 1
                            );
                            let col = self.doctrines[fe].color.clone();
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
            } else if dist > 0.1 {
                self.fleets[f_idx].x += ((tx - fx) / dist) * step_dist;
                self.fleets[f_idx].y += ((ty - fy) / dist) * step_dist;
            }
        }

        // Deep space combat & counter-triangle
        let fl_len = self.fleets.len();
        for i in (0..fl_len).rev() {
            if self.fleets[i].power <= 0.0 {
                continue;
            }
            for j in (0..i).rev() {
                if self.fleets[j].power <= 0.0 || self.fleets[i].emp == self.fleets[j].emp {
                    continue;
                }
                let a_c = self.fleets[i].fleet_type.is_combat();
                let b_c = self.fleets[j].fleet_type.is_combat();
                if !a_c && !b_c {
                    continue;
                }

                let ax = self.fleets[i].x;
                let ay = self.fleets[i].y;
                let bx = self.fleets[j].x;
                let by = self.fleets[j].y;

                if (ax - bx).hypot(ay - by) <= hit_rad {
                    self.space_battles_count += 1;
                    let type_a = self.fleets[i].fleet_type;
                    let type_b = self.fleets[j].fleet_type;

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

                    let eff_a = (if a_c { self.fleets[i].power } else { 8.0 }) * mod_a;
                    let eff_b = (if b_c { self.fleets[j].power } else { 8.0 }) * mod_b;
                    let mx = (ax + bx) * 0.5;
                    let my = (ay + by) * 0.5;

                    let emp_a = self.fleets[i].emp;
                    let emp_b = self.fleets[j].emp;

                    if !silent && self.pending_combat_fx.len() < 35 {
                        let winner_e = if eff_a >= eff_b {
                            (emp_a - 1) as usize
                        } else {
                            (emp_b - 1) as usize
                        };
                        self.pending_combat_fx.push(CombatFx {
                            x1: ax,
                            y1: ay,
                            x2: bx,
                            y2: by,
                            mx,
                            my,
                            ttl: 10,
                            col: self.doctrines[winner_e].color.clone(),
                        });
                    }

                    if eff_a >= eff_b {
                        self.fleets[i].power *= 1.0 - ((eff_b / eff_a) * 0.65).min(0.85);
                        self.fleets[j].power = 0.0;
                        self.doctrines[(emp_a - 1) as usize].kills += 1;
                        let dead = self.fleets[j].clone();
                        self.spawn_salvage_debris(mx, my, &dead);
                    } else {
                        self.fleets[j].power *= 1.0 - ((eff_a / eff_b) * 0.65).min(0.85);
                        self.fleets[i].power = 0.0;
                        self.doctrines[(emp_b - 1) as usize].kills += 1;
                        let dead = self.fleets[i].clone();
                        self.spawn_salvage_debris(mx, my, &dead);
                        break;
                    }
                }
            }
        }

        // Remove destroyed fleets and release target claim bits
        let mut i = self.fleets.len();
        while i > 0 {
            i -= 1;
            if self.fleets[i].power <= 0.0 {
                let target = self.fleets[i].target;
                let emp = self.fleets[i].emp;
                self.claim_mask[target] &= !(1u8 << emp);
                self.fleets.remove(i);
            }
        }

        // Decay debris TTL
        let mut i = self.debris_fields.len();
        while i > 0 {
            i -= 1;
            self.debris_fields[i].ttl -= DT;
            if self.debris_fields[i].ttl <= 0.0 {
                self.debris_fields.remove(i);
            }
        }

        self.year += DT;
        self.ticks += 1;
        self.last_stats = stats;

        if !silent && self.ticks % 5 == 0 {
            self.power_history
                .push([stats[0].res, stats[1].res, stats[2].res]);
            if self.power_history.len() > MAX_POWER_HISTORY {
                self.power_history.remove(0);
            }
        }

        stats
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_fleet(
        &mut self,
        from_idx: usize,
        target_idx: usize,
        emp: u8,
        fleet_type: FleetClass,
        power: f32,
        min_type: u8,
        friendly_gates: &[usize],
        silent: bool,
    ) {
        let mut start_x = self.pos_x[from_idx];
        let mut start_y = self.pos_y[from_idx];
        let mut jumped_gate = false;

        if self.has_gate[from_idx] != 0 && friendly_gates.len() > 1 {
            let mut best_gate: Option<usize> = None;
            let mut best_dist =
                (self.pos_x[target_idx] - start_x).hypot(self.pos_y[target_idx] - start_y) * 0.72;
            for &g_idx in friendly_gates {
                if g_idx == from_idx {
                    continue;
                }
                let d = (self.pos_x[target_idx] - self.pos_x[g_idx])
                    .hypot(self.pos_y[target_idx] - self.pos_y[g_idx]);
                if d < best_dist {
                    best_dist = d;
                    best_gate = Some(g_idx);
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
            emp,
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
            let dsg = dead_fleet.fleet_type.design();
            DebrisField {
                x,
                y,
                iron: dsg.iron * self.cfg.salvage_rate,
                bora: dsg.bora * self.cfg.salvage_rate,
                germ: dsg.germ * self.cfg.salvage_rate,
                ttl: 12.0,
            }
        };
        self.debris_fields.push(field);
        if self.debris_fields.len() > MAX_DEBRIS_FIELDS {
            self.debris_fields.remove(0);
        }
    }

    pub fn update_param(&mut self, key: &str, value: f32) -> bool {
        let mut needs_reset = false;
        match key {
            "planets" => {
                self.cfg.planets = (value.round() as usize).clamp(30, 200);
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
            "indWeight" => self.cfg.ind_weight = value.clamp(0.5, 2.5),
            "expWeight" => self.cfg.exp_weight = value.clamp(0.5, 2.5),
            "techWeight" => self.cfg.tech_weight = value.clamp(0.5, 2.5),
            "aggroBias" => self.cfg.aggro_bias = (value / 100.0).clamp(0.0, 1.0),
            "sprawlPenalty" => self.cfg.sprawl_penalty = (value / 100.0).clamp(0.0, 0.5),
            _ => {}
        }
        needs_reset
    }

    pub fn take_snapshot(&mut self) -> SimSnapshot {
        let n = self.owner.len();
        let perks = [self.emp_perks(0), self.emp_perks(1), self.emp_perks(2)];
        let mut planets = Vec::with_capacity(n);

        for i in 0..n {
            let b = i * 3;
            let emp = self.owner[i];
            let max_pop = if emp > 0 {
                let e = (emp - 1) as usize;
                self.hab[i]
                    * 170.0
                    * (1.0 + self.tech_lv[e][5] * 0.03)
                    * if perks[e] & BP_TERRA != 0 { 1.20 } else { 1.0 }
            } else {
                self.hab[i] * 170.0
            };
            planets.push(PlanetView {
                id: i + 1,
                x: self.pos_x[i],
                y: self.pos_y[i],
                owner: emp,
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
            });
        }

        let new_combat_fx = std::mem::take(&mut self.pending_combat_fx);
        let new_jump_fx = std::mem::take(&mut self.pending_jump_fx);

        SimSnapshot {
            year: self.year,
            ticks: self.ticks,
            map_seed: self.current_map_seed,
            dna_generation: self.dna_generation,
            stats: self.last_stats,
            tech_lv: self.tech_lv,
            perks,
            doctrines: self.doctrines.clone(),
            planets,
            fleets: self.fleets.clone(),
            wormholes: self.wormholes.clone(),
            debris_fields: self.debris_fields.clone(),
            new_combat_fx,
            new_jump_fx,
            power_history: self.power_history.clone(),
            events: self.events.clone(),
            space_battles_count: self.space_battles_count,
            invasions_count: self.invasions_count,
            total_salvaged: self.total_salvaged,
            gate_jumps_count: self.gate_jumps_count,
        }
    }

    pub fn export_state_json(&self) -> String {
        let payload = serde_json::json!({
            "year": (self.year * 10.0).round() / 10.0,
            "dnaGeneration": self.dna_generation,
            "config": self.cfg,
            "trainedDoctrines": self.doctrines,
        });
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_simulation_produces_identical_results() {
        let mut a = Simulation::new(SimConfig::default());
        let mut b = Simulation::new(SimConfig::default());
        a.init(80000, true);
        b.init(80000, true);

        let mut sa = [EmpireStats::default(); NUM_EMPIRES];
        let mut sb = [EmpireStats::default(); NUM_EMPIRES];
        for _ in 0..400 {
            sa = a.step_tick(true);
            sb = b.step_tick(true);
        }

        for e in 0..NUM_EMPIRES {
            assert_eq!(sa[e].planets, sb[e].planets);
            assert!((sa[e].res - sb[e].res).abs() < 1e-5);
            assert_eq!(a.doctrines[e].kills, b.doctrines[e].kills);
        }
        assert_eq!(a.space_battles_count, b.space_battles_count);
        assert_eq!(a.invasions_count, b.invasions_count);
        assert!(sa.iter().map(|s| s.planets).sum::<u32>() > 3);
    }

    #[test]
    fn simulation_expands_researches_and_fights() {
        let mut sim = Simulation::new(SimConfig::default());
        sim.init(87919, false);

        for _ in 0..1400 {
            sim.step_tick(false);
        }

        let total_planets: u32 = sim.last_stats.iter().map(|s| s.planets).sum();
        let total_tech: f32 = sim.tech_lv.iter().flatten().sum();
        assert!(total_planets > 30, "expected colonization, got {total_planets}");
        assert!(total_tech > 15.0, "expected tech growth, got {total_tech}");
        assert!(
            sim.space_battles_count > 0 || sim.invasions_count > 0,
            "expected combat activity over 1400 ticks"
        );
    }
}
