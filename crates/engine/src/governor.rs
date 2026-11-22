//! Type-isolated Empire Governor (`EmpireView` -> `Vec<GovernorAction>`).
//!
//! This module does NOT import `Simulation` and has zero access to raw global buffers of
//! foreign planets, hidden fleets, undiscovered wormholes, or global GDP statistics.
//! All decisions are made strictly from an empire's own planets, its `PlanetIntel` slice,
//! and scanner-verified enemy fleets and debris fields.

use crate::config::{FleetClass, SimConfig, MAX_EMPIRES};
use crate::types::{
    DetectedEnemyFleet, DiplomaticStance, EmpirePolicy, HubKind, PlanetIntel, SectorMode,
    VisibleDebrisBrief, Wormhole,
};

#[derive(Debug, Clone, Copy)]
pub struct OwnPlanetState {
    pub planet_idx: usize,
    pub x: f32,
    pub y: f32,
    pub pop: f32,
    pub max_pop: f32,
    pub factories: f32,
    pub defense: f32,
    pub ship_prod_buf: f32,
    pub min_surf: [f32; 3],
    pub has_gate: bool,
    pub starbase_tier: u8,
    pub orbit_freight: i32,
    pub orbit_warship: i32,
    pub orbit_warship_power: f32,
    pub orbit_warship_class: Option<FleetClass>,
    pub orbit_count: u16,
    pub ai_turn: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct VisibleWormhole {
    pub wormhole_id: usize,
    pub wh: Wormhole,
}

/// Narrow, Fog-of-War-isolated view passed to `run_empire_governor`.
pub struct EmpireView<'a> {
    pub emp_idx: usize,
    pub emp_u8: u8,
    pub ticks: u64,
    pub year: f32,
    pub world_w: f32,
    pub world_h: f32,
    pub cfg: &'a SimConfig,
    pub policy: &'a EmpirePolicy,
    pub upkeep_ratio: f32,
    pub total_active_fleets: usize,
    pub has_stargate_tech: bool,
    pub max_gates: usize,
    pub min_fac_for_gate: f32,
    pub gate_iron: f32,
    pub gate_germ: f32,
    pub gate_res: f32,
    pub min_hub_sq: f32,
    pub gate_range_sq: f32,
    pub best_starbase_tier: u8,
    pub effective_ship_cost: f32,
    pub cargo_cap: f32,
    pub freight_range: f32,
    pub col_range: f32,
    pub col_i: f32,
    pub col_g: f32,
    pub salvage_mult: f32,
    pub bomb_power: f32,
    pub unit_power_cruiser: f32,
    pub unit_power_bship: f32,
    pub unit_power_bomber: f32,
    pub unit_power_col: f32,
    pub range_cruiser: f32,
    pub range_bship: f32,
    pub range_bomber: f32,
    pub cost_cruiser: (f32, f32, f32),
    pub cost_bship: (f32, f32, f32),
    pub cost_bomber: (f32, f32, f32),
    pub cost_freight: (f32, f32, f32),
    pub sector_cols: usize,
    pub sector_rows: usize,
    pub sector_directives: &'a [SectorMode],
    pub relations: &'a [DiplomaticStance; MAX_EMPIRES],
    pub planet_pos_x: &'a [f32],
    pub planet_pos_y: &'a [f32],
    pub neighbor_offsets: &'a [u32],
    pub neighbor_data: &'a [(u32, f32)],
    pub intel_planets: &'a [PlanetIntel],
    pub claim_mask: &'a [u16],
    pub own_planets: &'a [OwnPlanetState],
    pub own_planet_min_surf: &'a [f32],
    pub own_gates: &'a [usize],
    pub visible_wormholes: &'a [VisibleWormhole],
    pub wh_near_a: &'a [Vec<(u32, f32)>],
    pub wh_near_b: &'a [Vec<(u32, f32)>],
    pub visible_enemy_fleets: &'a [DetectedEnemyFleet],
    pub visible_debris: &'a [VisibleDebrisBrief],
    pub pending_hub_orders: &'a [(usize, HubKind)],
}

#[derive(Debug, Clone)]
pub enum GovernorAction {
    BuildStargate {
        planet_idx: usize,
    },
    BuildStarbase {
        planet_idx: usize,
        target_tier: u8,
    },
    DispatchFreighter {
        from: usize,
        target: usize,
        min_type: u8,
        cargo: f32,
        docked_fleet_idx: Option<usize>,
    },
    DispatchSalvage {
        from: usize,
        dest_x: f32,
        dest_y: f32,
        docked_fleet_idx: Option<usize>,
    },
    LaunchFleet {
        from: usize,
        target: usize,
        class: FleetClass,
        ship_count: u16,
        wh_route: i16,
        docked_fleet_idx: Option<usize>,
    },
    BuildGarrison {
        planet_idx: usize,
        class: FleetClass,
        ship_count: u16,
    },
}

#[inline]
fn next_rand(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    (*seed as f64 / 4294967296.0) as f32
}

#[inline]
pub fn sector_for_xy(x: f32, y: f32, world_w: f32, world_h: f32, cols: usize, rows: usize) -> u16 {
    let c = (((x / world_w.max(1.0)) * cols as f32).floor() as isize)
        .clamp(0, (cols.saturating_sub(1)) as isize) as usize;
    let r = (((y / world_h.max(1.0)) * rows as f32).floor() as isize)
        .clamp(0, (rows.saturating_sub(1)) as isize) as usize;
    (r * cols + c) as u16
}

#[inline]
fn starbase_est_hp(tier: u8) -> f32 {
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
fn starbase_build_cost(tier: u8) -> (f32, f32, f32, f32) {
    match tier {
        1 => (18.0, 5.0, 22.0, 45.0),
        2 => (28.0, 10.0, 32.0, 90.0),
        3 => (65.0, 40.0, 110.0, 200.0),
        4 => (90.0, 60.0, 160.0, 320.0),
        5 => (120.0, 80.0, 210.0, 450.0),
        _ => (0.0, 0.0, 0.0, 0.0),
    }
}

/// Estimates enemy resistance at planet `_j` strictly from `PlanetIntel` and scanner-detected orbiting enemy fleets.
fn estimate_known_enemy_defense(
    _j: usize,
    intel: &PlanetIntel,
    bomb_pen: f32,
    planet_x: f32,
    planet_y: f32,
    visible_enemy_fleets: &[DetectedEnemyFleet],
) -> f32 {
    let ground_def = if intel.is_pen_scanned() {
        intel.def_est * (1.0 - bomb_pen) + intel.pop_est * 0.35
    } else {
        35.0 * (1.0 - bomb_pen * 0.5)
    };
    // An orbital Starbase inflicts heavy damage and requires fl_power * 1.15 >= starbase_hp to break
    let sb_hp = starbase_est_hp(intel.starbase_tier);
    let sb_def = sb_hp * 0.90;

    // Include any scanner-detected enemy fleets orbiting or near planet j
    let mut orbit_def = 0.0f32;
    for ef in visible_enemy_fleets {
        let dx = ef.x - planet_x;
        let dy = ef.y - planet_y;
        if dx * dx + dy * dy <= 25.0 * 25.0 {
            orbit_def += ef.power;
        }
    }

    20.0 + ground_def + sb_def + orbit_def
}

/// Selects an interception prey target for a Cruiser strictly from scanner-verified `visible_enemy_fleets`
/// (which have already passed Cloak, Tachyon, and local planetary ECM checks).
pub fn choose_cruiser_intercept_target(
    fx: f32,
    fy: f32,
    f_pow: f32,
    scan_max: f32,
    visible_enemy_fleets: &[DetectedEnemyFleet],
) -> Option<(f32, f32)> {
    let mut best_prey: Option<(f32, f32)> = None;
    let mut best_dist_sq = scan_max * scan_max;

    for ef in visible_enemy_fleets {
        let dx = ef.x - fx;
        let dy = ef.y - fy;
        let d_sq = dx * dx + dy * dy;
        if d_sq >= best_dist_sq {
            continue;
        }
        let is_prey_class = matches!(
            ef.fleet_type,
            FleetClass::Bomber | FleetClass::Freight | FleetClass::Col
        );
        if is_prey_class || ef.power < f_pow * 0.9 {
            best_dist_sq = d_sq;
            best_prey = Some((ef.x, ef.y));
        }
    }
    best_prey
}

/// Executes one tick of governor decisions for a single empire using exclusively its Fog-of-War-isolated `EmpireView`.
pub fn run_empire_governor(view: &EmpireView<'_>, rng_seed: &mut u32) -> Vec<GovernorAction> {
    let mut actions = Vec::new();
    let policy = view.policy;
    let my_bit = 1u16 << view.emp_idx;
    let inv_crust_norm = 1.0 / (view.cfg.crust_richness * 3.0).max(1.0);
    let max_upkeep = policy.max_upkeep_ratio.clamp(0.15, 0.85);
    let min_shipyard_pop = policy
        .min_shipyard_pop_ratio
        .max(policy.col_pop_req)
        .clamp(0.05, 0.90);
    let min_force_ratio = policy.min_force_ratio.clamp(0.75, 4.0);

    // 1. Estimate known rivals' GDP strictly from our own PlanetIntel (zero global omniscience!)
    //    Only scanned when at least one owned planet is on its AI turn and ready for shipyard decisions.
    let any_shipyard_turn = view.own_planets.iter().any(|op| {
        op.ai_turn
            && op.pop > op.max_pop * min_shipyard_pop
            && (op.ship_prod_buf >= view.effective_ship_cost || op.orbit_warship >= 0)
    });
    let mut est_emp_res = [0.0f32; MAX_EMPIRES];
    let mut total_known_res = 1.0f32;
    let mut known_neutral_count = 0usize;
    let mut est_leader_e = view.emp_idx;
    if any_shipyard_turn {
        for op in view.own_planets {
            let r = op.pop * 0.32 + op.factories;
            est_emp_res[view.emp_idx] += r;
            total_known_res += r;
        }
        for intel in view.intel_planets {
            if !intel.is_explored() || intel.owner == 0 {
                known_neutral_count += 1;
            } else if intel.owner != view.emp_u8 {
                let te = (intel.owner - 1) as usize;
                if te < MAX_EMPIRES {
                    let r = if intel.is_pen_scanned() {
                        (intel.pop_est * 0.32 + intel.fac_est).max(10.0)
                    } else {
                        30.0
                    };
                    est_emp_res[te] += r;
                    total_known_res += r;
                }
            }
        }
        for e in 0..MAX_EMPIRES {
            if est_emp_res[e] > est_emp_res[est_leader_e] {
                est_leader_e = e;
            }
        }
    }

    let mut local_gates: Vec<usize> = view.own_gates.to_vec();
    let mut claimed_targets: Vec<usize> = Vec::new();

    for op in view.own_planets {
        let i = op.planet_idx;
        let mut rem_prod_buf = op.ship_prod_buf;
        let mut rem_surf = op.min_surf;
        let mut has_gate = op.has_gate;
        let cur_sb = op.starbase_tier;

        // Check explicit BuildHub commands for this planet first
        for &(hub_planet, kind) in view.pending_hub_orders {
            if hub_planet != i {
                continue;
            }
            match kind {
                HubKind::Stargate => {
                    if view.has_stargate_tech
                        && !has_gate
                        && rem_prod_buf >= view.gate_res
                        && rem_surf[0] >= view.gate_iron
                        && rem_surf[2] >= view.gate_germ
                    {
                        rem_prod_buf -= view.gate_res;
                        rem_surf[0] -= view.gate_iron;
                        rem_surf[2] -= view.gate_germ;
                        has_gate = true;
                        local_gates.push(i);
                        actions.push(GovernorAction::BuildStargate { planet_idx: i });
                    }
                }
                HubKind::Starbase => {
                    let target_tier = view.best_starbase_tier.max(1);
                    if cur_sb < target_tier {
                        let (ti, tb, tg, tr) = starbase_build_cost(target_tier);
                        let (pi, pb, pg, pr) = starbase_build_cost(cur_sb);
                        let (ci, cb, cg, cr) = (
                            (ti - pi * 0.5).max(0.0),
                            (tb - pb * 0.5).max(0.0),
                            (tg - pg * 0.5).max(0.0),
                            (tr - pr * 0.5).max(0.0),
                        );
                        if rem_prod_buf >= cr
                            && rem_surf[0] >= ci
                            && rem_surf[1] >= cb
                            && rem_surf[2] >= cg
                        {
                            rem_prod_buf -= cr;
                            rem_surf[0] -= ci;
                            rem_surf[1] -= cb;
                            rem_surf[2] -= cg;
                            actions.push(GovernorAction::BuildStarbase {
                                planet_idx: i,
                                target_tier,
                            });
                        }
                    }
                }
            }
        }

        // 1a. Autonomous Stargate & Starbase construction (runs on planet's AI decision turn)
        if op.ai_turn
            && view.has_stargate_tech
            && !has_gate
            && local_gates.len() < view.max_gates
            && op.factories >= view.min_fac_for_gate
            && rem_prod_buf >= view.gate_res
            && rem_surf[0] >= view.gate_iron
            && rem_surf[2] >= view.gate_germ
            && next_rand(rng_seed) < 0.06
        {
            let too_close = local_gates.iter().any(|&g| {
                let dx = view.planet_pos_x[g] - op.x;
                let dy = view.planet_pos_y[g] - op.y;
                dx * dx + dy * dy < view.min_hub_sq
            });
            if !too_close {
                rem_prod_buf -= view.gate_res;
                rem_surf[0] -= view.gate_iron;
                rem_surf[2] -= view.gate_germ;
                local_gates.push(i);
                actions.push(GovernorAction::BuildStargate { planet_idx: i });
            }
        }

        // Autonomous Starbase construction / upgrade
        if op.ai_turn
            && cur_sb < view.best_starbase_tier
            && op.pop > 28.0
            && op.factories >= 14.0
            && view.upkeep_ratio < max_upkeep * 0.75
            && next_rand(rng_seed) < 0.04
        {
            let target_tier = if cur_sb == 0 {
                if op.factories >= 36.0 && view.best_starbase_tier >= 3 {
                    3
                } else if view.best_starbase_tier >= 2 && op.factories >= 22.0 {
                    2
                } else {
                    1
                }
            } else {
                view.best_starbase_tier
            };
            if target_tier > cur_sb {
                let (ti, tb, tg, tr) = starbase_build_cost(target_tier);
                let (pi, pb, pg, pr) = starbase_build_cost(cur_sb);
                let (ci, cb, cg, cr) = (
                    (ti - pi * 0.5).max(0.0),
                    (tb - pb * 0.5).max(0.0),
                    (tg - pg * 0.5).max(0.0),
                    (tr - pr * 0.5).max(0.0),
                );
                if rem_prod_buf >= cr
                    && rem_surf[0] >= ci
                    && rem_surf[1] >= cb
                    && rem_surf[2] >= cg
                {
                    rem_prod_buf -= cr;
                    rem_surf[0] -= ci;
                    rem_surf[1] -= cb;
                    rem_surf[2] -= cg;
                    actions.push(GovernorAction::BuildStarbase {
                        planet_idx: i,
                        target_tier,
                    });
                }
            }
        }

        if !op.ai_turn {
            continue;
        }

        let mut used_orbit_freight = false;

        // 2. Freighter mineral redistribution across own planets (O(1) lookup via intel_planets[j].owner == emp_u8)
        if next_rand(rng_seed) < 0.25 {
            let nb_start = view.neighbor_offsets[i] as usize;
            let nb_end = (view.neighbor_offsets[i + 1] as usize).min(nb_start + 16);
            let exp_thresh = policy.freight_export_threshold.max(30.0);
            let imp_thresh = policy.freight_import_threshold.max(5.0);
            for m in 0..3 {
                if rem_surf[m] > exp_thresh {
                    let mut needy: Option<usize> = None;
                    let mut lowest_val = imp_thresh;
                    for &(j_u32, dist) in &view.neighbor_data[nb_start..nb_end] {
                        if dist > view.freight_range {
                            break;
                        }
                        let j = j_u32 as usize;
                        if view.intel_planets[j].owner == view.emp_u8 {
                            let dest_surf = view.own_planet_min_surf[j * 3 + m];
                            if dest_surf < lowest_val {
                                lowest_val = dest_surf;
                                needy = Some(j);
                            }
                        }
                    }
                    if let Some(target_idx) = needy {
                        let cargo = view.cargo_cap.min(rem_surf[m] * 0.45);
                        if op.orbit_freight >= 0 && !used_orbit_freight {
                            used_orbit_freight = true;
                            rem_surf[m] -= cargo;
                            actions.push(GovernorAction::DispatchFreighter {
                                from: i,
                                target: target_idx,
                                min_type: m as u8,
                                cargo,
                                docked_fleet_idx: Some(op.orbit_freight as usize),
                            });
                            break;
                        } else if view.upkeep_ratio < max_upkeep * 0.82
                            && rem_prod_buf >= view.effective_ship_cost * 0.35
                            && view.total_active_fleets < 4000
                        {
                            let (hi, hb, hg) = view.cost_freight;
                            let (ci, cb, cg) = (hi * 0.5, hb * 0.5, hg * 0.5);
                            if rem_surf[0] >= ci && rem_surf[1] >= cb && rem_surf[2] >= cg {
                                rem_prod_buf -= view.effective_ship_cost * 0.35;
                                rem_surf[0] -= ci;
                                rem_surf[1] -= cb;
                                rem_surf[2] -= cg;
                                let actual_cargo = cargo.min(rem_surf[m]).max(1.0);
                                rem_surf[m] = (rem_surf[m] - actual_cargo).max(0.0);
                                actions.push(GovernorAction::DispatchFreighter {
                                    from: i,
                                    target: target_idx,
                                    min_type: m as u8,
                                    cargo: actual_cargo,
                                    docked_fleet_idx: None,
                                });
                                break;
                            }
                        }
                    }
                }
            }
        }

        // 3. Salvage Freighter dispatch (strictly limited to scanner-visible `view.visible_debris`!)
        if !view.visible_debris.is_empty()
            && op.pop > 20.0
            && next_rand(rng_seed) < 0.06
            && view.total_active_fleets < 4000
        {
            let f_range = view.freight_range * 0.92;
            let f_range_sq = f_range * f_range;
            let min_worth = if view.salvage_mult > 1.1 { 15.0 } else { 25.0 };
            let mut best_deb: Option<(f32, f32)> = None;
            let mut best_deb_score = 0.0f32;

            for deb in view.visible_debris {
                if deb.ttl <= 0.5 {
                    continue;
                }
                let tot = deb.iron + deb.bora + deb.germ;
                if tot < min_worth {
                    continue;
                }
                let dx = deb.x - op.x;
                let dy = deb.y - op.y;
                let d_sq = dx * dx + dy * dy;
                if d_sq > f_range_sq {
                    continue;
                }
                let score = tot / (d_sq.sqrt() + 35.0);
                if score > best_deb_score {
                    best_deb_score = score;
                    best_deb = Some((deb.x, deb.y));
                }
            }

            if let Some((dx, dy)) = best_deb {
                if op.orbit_freight >= 0 && !used_orbit_freight {
                    actions.push(GovernorAction::DispatchSalvage {
                        from: i,
                        dest_x: dx,
                        dest_y: dy,
                        docked_fleet_idx: Some(op.orbit_freight as usize),
                    });
                } else if view.upkeep_ratio < max_upkeep * 0.82
                    && rem_prod_buf >= view.effective_ship_cost * 0.35
                {
                    let (hi, hb, hg) = view.cost_freight;
                    let (ci, cb, cg) = (hi * 0.5, hb * 0.5, hg * 0.5);
                    if rem_surf[0] >= ci && rem_surf[1] >= cb && rem_surf[2] >= cg {
                        rem_prod_buf -= view.effective_ship_cost * 0.35;
                        rem_surf[0] -= ci;
                        rem_surf[1] -= cb;
                        rem_surf[2] -= cg;
                        actions.push(GovernorAction::DispatchSalvage {
                            from: i,
                            dest_x: dx,
                            dest_y: dy,
                            docked_fleet_idx: None,
                        });
                    }
                }
            }
        }

        // 4. Shipyard colonization & military attack decisions
        let my_sector = sector_for_xy(
            op.x,
            op.y,
            view.world_w,
            view.world_h,
            view.sector_cols,
            view.sector_rows,
        );
        let my_sector_mode = view
            .sector_directives
            .get(my_sector as usize)
            .copied()
            .unwrap_or(SectorMode::Expand);

        let has_forced_attack_directive = view
            .sector_directives
            .iter()
            .any(|&m| m == SectorMode::Attack);

        if op.pop > op.max_pop * min_shipyard_pop
            && (rem_prod_buf >= view.effective_ship_cost || op.orbit_warship >= 0)
        {
            let nb_start = view.neighbor_offsets[i] as usize;
            let nb_end = view.neighbor_offsets[i + 1] as usize;
            let dist_exp = policy.dist_exp;
            let qual_exp = policy.qual_exp;
            let min_hab = policy.min_colonize_hab;
            let m_weight_avg =
                ((policy.mineral_weights[0] + policy.mineral_weights[1] + policy.mineral_weights[2])
                    / 3.0)
                    .clamp(0.2, 3.0);

            // 4a. Evaluate best colonization candidate strictly using PlanetIntel & discovered wormholes
            let mut best_col_target: Option<usize> = None;
            let mut best_col_score = -1.0f32;
            let mut best_col_wh: i16 = -1;
            let col_res_cost =
                view.effective_ship_cost * view.cfg.col_cost_mult.clamp(0.5, 5.0);

            let can_afford_col = known_neutral_count > 0
                && rem_prod_buf >= col_res_cost
                && rem_surf[0] >= view.col_i
                && rem_surf[2] >= view.col_g;

            let score_col_candidate = |j: usize, d: f32, require_explored: bool| -> f32 {
                if (view.claim_mask[j] & my_bit) != 0 || claimed_targets.contains(&j) {
                    return -1.0;
                }
                let intel = &view.intel_planets[j];
                if require_explored && !intel.is_explored() {
                    return -1.0;
                }
                if intel.is_explored() && intel.owner != 0 {
                    return -1.0;
                }
                let t_sec = sector_for_xy(
                    view.planet_pos_x[j],
                    view.planet_pos_y[j],
                    view.world_w,
                    view.world_h,
                    view.sector_cols,
                    view.sector_rows,
                );
                let sec_mode = view
                    .sector_directives
                    .get(t_sec as usize)
                    .copied()
                    .unwrap_or(SectorMode::Expand);
                let sec_mult = match sec_mode {
                    SectorMode::Ignore => return -1.0,
                    SectorMode::Expand => 1.8,
                    SectorMode::Attack => 0.4,
                    SectorMode::Fortify => 0.7,
                };
                let dist_clamped = d.max(25.0);
                if intel.is_pen_scanned() {
                    if intel.hab_est < min_hab {
                        return -1.0;
                    }
                    let h = intel.hab_est;
                    let quality = (h * h.sqrt())
                        * (0.4 + intel.crust_sum_est * inv_crust_norm * m_weight_avg);
                    (quality.powf(qual_exp) / (dist_clamped / 180.0).powf(dist_exp)) * sec_mult
                } else {
                    // Unexplored or non-pen-scanned star: blind exploratory colonization score
                    (0.22 / (dist_clamped / 180.0).powf(dist_exp)) * sec_mult
                }
            };

            if can_afford_col {
                for &(j_u32, dist) in &view.neighbor_data[nb_start..nb_end] {
                    if dist > view.col_range {
                        break;
                    }
                    let j = j_u32 as usize;
                    let sc = score_col_candidate(j, dist, false);
                    if sc > best_col_score {
                        best_col_score = sc;
                        best_col_target = Some(j);
                        best_col_wh = -1;
                    }
                }

                // Stargate relay colonization (strictly requires target to be explored in PlanetIntel!)
                if best_col_target.is_none() && has_gate && local_gates.len() > 1 {
                    let gl_len = local_gates.len();
                    let sample_count = gl_len.min(12);
                    let stride = (gl_len / sample_count).max(1);
                    let offset = (i + view.ticks as usize) % gl_len;
                    for s in 0..sample_count {
                        let g = local_gates[(offset + s * stride) % gl_len];
                        if g == i {
                            continue;
                        }
                        let dx = view.planet_pos_x[g] - op.x;
                        let dy = view.planet_pos_y[g] - op.y;
                        if dx * dx + dy * dy > view.gate_range_sq {
                            continue;
                        }
                        let g_start = view.neighbor_offsets[g] as usize;
                        let g_end = (view.neighbor_offsets[g + 1] as usize).min(g_start + 16);
                        for &(j_u32, dist) in &view.neighbor_data[g_start..g_end] {
                            if dist > view.col_range {
                                break;
                            }
                            let j = j_u32 as usize;
                            let sc = score_col_candidate(j, dist, false);
                            if sc > best_col_score {
                                best_col_score = sc;
                                best_col_target = Some(j);
                                best_col_wh = -1;
                            }
                        }
                    }
                }

                // Discovered Wormhole relay colonization (ONLY discovered wormholes & explored targets!)
                if !view.visible_wormholes.is_empty() {
                    let max_entry = view.col_range * 0.88;
                    let max_entry_sq = max_entry * max_entry;
                    for vwh in view.visible_wormholes.iter().take(16) {
                        let w = vwh.wormhole_id;
                        for (dir, ex, ey, near_exit) in [
                            (0i16, vwh.wh.x1, vwh.wh.y1, &view.wh_near_b[w]),
                            (1i16, vwh.wh.x2, vwh.wh.y2, &view.wh_near_a[w]),
                        ] {
                            let dx = ex - op.x;
                            let dy = ey - op.y;
                            let d_entry_sq = dx * dx + dy * dy;
                            if d_entry_sq >= max_entry_sq {
                                continue;
                            }
                            let d_entry = d_entry_sq.sqrt();
                            let rem_range = view.col_range - d_entry;
                            for &(j_u32, d_exit) in near_exit {
                                if d_exit > rem_range {
                                    break;
                                }
                                let j = j_u32 as usize;
                                let sc = score_col_candidate(j, d_entry + d_exit, false) * 1.08;
                                if sc > best_col_score {
                                    best_col_score = sc;
                                    best_col_target = Some(j);
                                    best_col_wh = (w as i16) * 2 + dir;
                                }
                            }
                        }
                    }
                }
            }

            // 4b. Evaluate best military attack candidate using PlanetIntel, Starbase tier & min_force_ratio
            let mut best_atk_target: Option<usize> = None;
            let mut best_atk_score = -1.0f32;
            let mut best_atk_wh: i16 = -1;
            let mut best_atk_class = FleetClass::Cruiser;
            let mut best_atk_build_count = 0u16;
            let mut saw_fortified_enemy_needing_buildup = false;

            if op.pop > 28.0 || has_forced_attack_directive {
                let r_roll = next_rand(rng_seed);
                let try_order = if let Some(orb_cls) = op.orbit_warship_class {
                    match orb_cls {
                        FleetClass::Cruiser => [
                            FleetClass::Cruiser,
                            FleetClass::Battleship,
                            FleetClass::Bomber,
                        ],
                        FleetClass::Battleship => [
                            FleetClass::Battleship,
                            FleetClass::Cruiser,
                            FleetClass::Bomber,
                        ],
                        _ => [
                            FleetClass::Bomber,
                            FleetClass::Battleship,
                            FleetClass::Cruiser,
                        ],
                    }
                } else if r_roll < policy.cruiser_share {
                    [
                        FleetClass::Cruiser,
                        FleetClass::Battleship,
                        FleetClass::Bomber,
                    ]
                } else if r_roll < policy.cruiser_share + policy.bship_share {
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
                    let (ci, cb, cg) = match c_class {
                        FleetClass::Cruiser => view.cost_cruiser,
                        FleetClass::Battleship => view.cost_bship,
                        FleetClass::Bomber => view.cost_bomber,
                        _ => view.cost_cruiser,
                    };
                    if (rem_prod_buf >= view.effective_ship_cost
                        && rem_surf[0] >= ci
                        && rem_surf[1] >= cb
                        && rem_surf[2] >= cg)
                        || op.orbit_warship >= 0
                    {
                        chosen_class = Some(c_class);
                        break;
                    }
                }

                if let Some(c_class) = chosen_class {
                    let (ci_u, cb_u, cg_u) = match c_class {
                        FleetClass::Cruiser => view.cost_cruiser,
                        FleetClass::Battleship => view.cost_bship,
                        FleetClass::Bomber => view.cost_bomber,
                        _ => view.cost_cruiser,
                    };
                    let unit_power = match c_class {
                        FleetClass::Cruiser => view.unit_power_cruiser,
                        FleetClass::Battleship => view.unit_power_bship,
                        FleetClass::Bomber => view.unit_power_bomber,
                        _ => view.unit_power_cruiser,
                    };
                    let ship_range = match c_class {
                        FleetClass::Cruiser => view.range_cruiser,
                        FleetClass::Battleship => view.range_bship,
                        FleetClass::Bomber => view.range_bomber,
                        _ => view.range_cruiser,
                    };
                    let max_by_res = if view.upkeep_ratio < max_upkeep * 0.78 {
                        (rem_prod_buf / view.effective_ship_cost.max(1.0)).floor() as u16
                    } else {
                        0
                    };
                    let max_by_i = (rem_surf[0] / ci_u.max(0.1)).floor() as u16;
                    let max_by_b = (rem_surf[1] / cb_u.max(0.1)).floor() as u16;
                    let max_by_g = (rem_surf[2] / cg_u.max(0.1)).floor() as u16;
                    let max_buildable = max_by_res.min(max_by_i).min(max_by_b).min(max_by_g).min(12);

                    let can_merge_orbit =
                        op.orbit_warship >= 0 && op.orbit_warship_class == Some(c_class);
                    let total_available_power = if can_merge_orbit {
                        op.orbit_warship_power + (max_buildable as f32) * unit_power
                    } else {
                        op.orbit_warship_power.max((max_buildable as f32) * unit_power)
                    };

                    let bomb_pen = if c_class == FleetClass::Bomber {
                        (view.cfg.bomber_pen + (view.bomb_power * 0.02).min(0.22)).min(0.88)
                    } else {
                        0.0
                    };

                    let eval_attack_score = |j: usize,
                                             d: f32,
                                             mult: f32,
                                             saw_fortified: &mut bool|
                     -> Option<f32> {
                        if (view.claim_mask[j] & my_bit) != 0 || claimed_targets.contains(&j) {
                            return None;
                        }
                        let intel = &view.intel_planets[j];
                        if !intel.is_explored()
                            || intel.owner == 0
                            || intel.owner == view.emp_u8
                        {
                            return None;
                        }
                        let target_e = (intel.owner - 1) as usize;
                        if target_e >= MAX_EMPIRES
                            || view.relations[target_e] != DiplomaticStance::War
                        {
                            return None;
                        }
                        let t_sec = sector_for_xy(
                            view.planet_pos_x[j],
                            view.planet_pos_y[j],
                            view.world_w,
                            view.world_h,
                            view.sector_cols,
                            view.sector_rows,
                        );
                        let sec_mode = view
                            .sector_directives
                            .get(t_sec as usize)
                            .copied()
                            .unwrap_or(SectorMode::Expand);
                        let sec_mult = match sec_mode {
                            SectorMode::Ignore => return None,
                            SectorMode::Attack => 4.5,
                            SectorMode::Expand => 0.85,
                            SectorMode::Fortify => 0.65,
                        };

                        let enemy_def = estimate_known_enemy_defense(
                            j,
                            intel,
                            bomb_pen,
                            view.planet_pos_x[j],
                            view.planet_pos_y[j],
                            view.visible_enemy_fleets,
                        );
                        let target_share = est_emp_res[target_e] / total_known_res;
                        let leader_aggro_bonus =
                            if target_e == est_leader_e && target_share > 0.30 {
                                1.0 + (target_share - 0.25) * 4.0 * view.cfg.aggro_bias
                            } else {
                                1.0
                            };

                        let required_power = enemy_def * min_force_ratio;
                        if total_available_power * leader_aggro_bonus >= required_power {
                            let est_fac = if intel.is_pen_scanned() {
                                intel.fac_est
                            } else {
                                20.0
                            };
                            let dist_clamped = d.max(30.0);
                            let score = (((est_fac + 15.0)
                                * (total_available_power / enemy_def)
                                * leader_aggro_bonus)
                                / (dist_clamped / 180.0).powf(1.35))
                                * mult
                                * sec_mult;
                            Some(score)
                        } else {
                            *saw_fortified = true;
                            None
                        }
                    };

                    for &(j_u32, dist) in &view.neighbor_data[nb_start..nb_end] {
                        if dist > ship_range {
                            break;
                        }
                        let j = j_u32 as usize;
                        if let Some(score) = eval_attack_score(
                            j,
                            dist,
                            1.0,
                            &mut saw_fortified_enemy_needing_buildup,
                        ) {
                            if score > best_atk_score {
                                best_atk_score = score;
                                best_atk_target = Some(j);
                                best_atk_wh = -1;
                                best_atk_class = c_class;
                                best_atk_build_count = max_buildable;
                            }
                        }
                    }

                    if best_atk_target.is_none() && has_gate && local_gates.len() > 1 {
                        let gl_len = local_gates.len();
                        let sample_count = gl_len.min(12);
                        let stride = (gl_len / sample_count).max(1);
                        let offset = (i + view.ticks as usize) % gl_len;
                        for s in 0..sample_count {
                            let g = local_gates[(offset + s * stride) % gl_len];
                            if g == i {
                                continue;
                            }
                            let dx = view.planet_pos_x[g] - op.x;
                            let dy = view.planet_pos_y[g] - op.y;
                            if dx * dx + dy * dy > view.gate_range_sq {
                                continue;
                            }
                            let g_start = view.neighbor_offsets[g] as usize;
                            let g_end = (view.neighbor_offsets[g + 1] as usize).min(g_start + 16);
                            for &(j_u32, dist) in &view.neighbor_data[g_start..g_end] {
                                if dist > ship_range {
                                    break;
                                }
                                let j = j_u32 as usize;
                                if let Some(score) = eval_attack_score(
                                    j,
                                    dist,
                                    1.0,
                                    &mut saw_fortified_enemy_needing_buildup,
                                ) {
                                    if score > best_atk_score {
                                        best_atk_score = score;
                                        best_atk_target = Some(j);
                                        best_atk_wh = -1;
                                        best_atk_class = c_class;
                                        best_atk_build_count = max_buildable;
                                    }
                                }
                            }
                        }
                    }

                    if !view.visible_wormholes.is_empty() {
                        let max_entry = ship_range * 0.88;
                        let max_entry_sq = max_entry * max_entry;
                        for vwh in view.visible_wormholes.iter().take(16) {
                            let w = vwh.wormhole_id;
                            for (dir, ex, ey, near_exit) in [
                                (0i16, vwh.wh.x1, vwh.wh.y1, &view.wh_near_b[w]),
                                (1i16, vwh.wh.x2, vwh.wh.y2, &view.wh_near_a[w]),
                            ] {
                                let dx = ex - op.x;
                                let dy = ey - op.y;
                                let d_entry_sq = dx * dx + dy * dy;
                                if d_entry_sq >= max_entry_sq {
                                    continue;
                                }
                                let d_entry = d_entry_sq.sqrt();
                                let rem_range = ship_range - d_entry;
                                let wh_route = (w as i16) * 2 + dir;
                                for &(j_u32, d_exit) in near_exit {
                                    if d_exit > rem_range {
                                        break;
                                    }
                                    let j = j_u32 as usize;
                                    if let Some(score) = eval_attack_score(
                                        j,
                                        d_entry + d_exit,
                                        1.08,
                                        &mut saw_fortified_enemy_needing_buildup,
                                    ) {
                                        if score > best_atk_score {
                                            best_atk_score = score;
                                            best_atk_target = Some(j);
                                            best_atk_wh = wh_route;
                                            best_atk_class = c_class;
                                            best_atk_build_count = max_buildable;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 4c. Decide between Colonization, Attack, or Garrison Buildup using `colonize_vs_attack_bias`
            let col_bias = policy.colonize_vs_attack_bias.clamp(0.0, 1.0);
            let choose_attack = match (best_col_target, best_atk_target) {
                (Some(_), Some(atk_j)) => {
                    let atk_sec = sector_for_xy(
                        view.planet_pos_x[atk_j],
                        view.planet_pos_y[atk_j],
                        view.world_w,
                        view.world_h,
                        view.sector_cols,
                        view.sector_rows,
                    );
                    let atk_sec_mode = view
                        .sector_directives
                        .get(atk_sec as usize)
                        .copied()
                        .unwrap_or(SectorMode::Expand);
                    if atk_sec_mode == SectorMode::Attack {
                        true
                    } else {
                        best_atk_score * (1.0 - col_bias) * 0.15 > best_col_score * col_bias
                    }
                }
                (None, Some(_)) => true,
                _ => false,
            };

            if choose_attack {
                if let Some(target_idx) = best_atk_target {
                    claimed_targets.push(target_idx);
                    if op.orbit_warship >= 0 {
                        let reinforce_count = if op.orbit_warship_class == Some(best_atk_class) {
                            best_atk_build_count
                        } else {
                            0
                        };
                        actions.push(GovernorAction::LaunchFleet {
                            from: i,
                            target: target_idx,
                            class: op.orbit_warship_class.unwrap_or(best_atk_class),
                            ship_count: reinforce_count,
                            wh_route: best_atk_wh,
                            docked_fleet_idx: Some(op.orbit_warship as usize),
                        });
                    } else if best_atk_build_count > 0 {
                        actions.push(GovernorAction::LaunchFleet {
                            from: i,
                            target: target_idx,
                            class: best_atk_class,
                            ship_count: best_atk_build_count.max(1),
                            wh_route: best_atk_wh,
                            docked_fleet_idx: None,
                        });
                    }
                }
            } else if let Some(target_idx) = best_col_target {
                claimed_targets.push(target_idx);
                actions.push(GovernorAction::LaunchFleet {
                    from: i,
                    target: target_idx,
                    class: FleetClass::Col,
                    ship_count: 1,
                    wh_route: best_col_wh,
                    docked_fleet_idx: None,
                });
            } else {
                // Build or reinforce standing orbital garrison (especially when preparing to crack a fortified Starbase!)
                let min_gar = if my_sector_mode == SectorMode::Fortify {
                    policy.min_garrison_count.max(2)
                } else {
                    policy.min_garrison_count
                };
                let max_gar = if saw_fortified_enemy_needing_buildup {
                    12
                } else {
                    min_gar
                };
                let upkeep_cap = if saw_fortified_enemy_needing_buildup {
                    max_upkeep * 0.78
                } else {
                    max_upkeep * 0.52
                };
                let prod_req = if saw_fortified_enemy_needing_buildup {
                    view.effective_ship_cost
                } else {
                    view.effective_ship_cost * 1.4
                };
                if view.upkeep_ratio < upkeep_cap
                    && rem_prod_buf >= prod_req
                    && op.orbit_count < max_gar
                    && view.total_active_fleets < 2400
                    && (saw_fortified_enemy_needing_buildup
                        || my_sector_mode == SectorMode::Fortify
                        || next_rand(rng_seed) < 0.18)
                {
                    let g_class = if let Some(existing_cls) = op.orbit_warship_class {
                        existing_cls
                    } else {
                        let r = next_rand(rng_seed);
                        if r < policy.cruiser_share {
                            FleetClass::Cruiser
                        } else if r < policy.cruiser_share + policy.bship_share {
                            FleetClass::Battleship
                        } else {
                            FleetClass::Bomber
                        }
                    };
                    let (ci, cb, cg) = match g_class {
                        FleetClass::Cruiser => view.cost_cruiser,
                        FleetClass::Battleship => view.cost_bship,
                        FleetClass::Bomber => view.cost_bomber,
                        _ => view.cost_cruiser,
                    };
                    let max_g_res = (rem_prod_buf / view.effective_ship_cost.max(1.0)).floor() as u16;
                    let max_g_i = (rem_surf[0] / ci.max(0.1)).floor() as u16;
                    let max_g_b = (rem_surf[1] / cb.max(0.1)).floor() as u16;
                    let max_g_g = (rem_surf[2] / cg.max(0.1)).floor() as u16;
                    let max_g = max_g_res.min(max_g_i).min(max_g_b).min(max_g_g);
                    if max_g >= 1 {
                        let build_cnt = if saw_fortified_enemy_needing_buildup {
                            max_g.clamp(1, 6)
                        } else {
                            1
                        };
                        actions.push(GovernorAction::BuildGarrison {
                            planet_idx: i,
                            class: g_class,
                            ship_count: build_cnt,
                        });
                    }
                }
            }
        }
    }

    actions
}
