use crate::config::{NUM_EMPIRES, NUM_TECH_FIELDS};
use crate::sim::Simulation;
use crate::types::{AutoBalanceResult, CoEvolutionResult, Doctrine, EmpireStats, TournamentResult};

pub fn run_tournament(
    sim: &mut Simulation,
    num_games: u32,
    ticks_per_game: u32,
) -> TournamentResult {
    let mut wins = [0u32; NUM_EMPIRES];
    let mut total_res = [0.0f32; NUM_EMPIRES];
    let games = num_games.max(1);

    for g in 0..games {
        sim.init(80000u32.wrapping_add(g.wrapping_mul(7919)), true);
        let mut st = [EmpireStats::default(); NUM_EMPIRES];
        for _ in 0..ticks_per_game {
            st = sim.step_tick(true);
        }

        let mut winner = 0usize;
        if st[1].res > st[winner].res {
            winner = 1;
        }
        if st[2].res > st[winner].res {
            winner = 2;
        }
        wins[winner] += 1;
        for e in 0..NUM_EMPIRES {
            total_res[e] += st[e].res;
        }
    }

    let w0 = ((wins[0] as f32 / games as f32) * 100.0).round() as u32;
    let w1 = ((wins[1] as f32 / games as f32) * 100.0).round() as u32;
    let w2 = 100u32.saturating_sub(w0 + w1);
    let win_rates = [w0, w1, w2];

    for e in 0..NUM_EMPIRES {
        sim.doctrines[e].win_rate = win_rates[e];
    }

    TournamentResult {
        num_games: games,
        wins,
        win_rates,
        total_res,
    }
}

pub fn evaluate_empire_fitness(sim: &mut Simulation, emp_idx: usize, seeds: &[u32]) -> f32 {
    let mut score = 0.0f32;
    for &seed in seeds {
        sim.init(seed, true);
        let mut st_mid = [EmpireStats::default(); NUM_EMPIRES];
        let mut st_end = [EmpireStats::default(); NUM_EMPIRES];
        for _ in 0..700 {
            st_mid = sim.step_tick(true);
        }
        for _ in 0..700 {
            st_end = sim.step_tick(true);
        }
        let sum_end = st_end[0].res + st_end[1].res + st_end[2].res + 1.0;
        let my_end = st_end[emp_idx].res;
        let is_winner =
            if my_end >= st_end[0].res && my_end >= st_end[1].res && my_end >= st_end[2].res {
                1.0
            } else {
                0.0
            };
        score += (my_end / sum_end) * 1000.0 + is_winner * 350.0 + (st_mid[emp_idx].res * 0.05);
    }
    score / (seeds.len().max(1) as f32)
}

fn lcg_centered(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    (*state as f64 / 4294967296.0) as f32 - 0.5
}

pub fn mutate_dna(doc: &Doctrine, emp_idx: usize, rng_state: &mut u32) -> Doctrine {
    let mut copy = doc.clone();
    copy.fac_share = (copy.fac_share + lcg_centered(rng_state) * 0.12).clamp(0.28, 0.66);
    copy.ship_share = (copy.ship_share + lcg_centered(rng_state) * 0.12).clamp(0.14, 0.50);
    let min_tech = if emp_idx == 2 { 0.15 } else { 0.06 };
    copy.tech_share = (copy.tech_share + lcg_centered(rng_state) * 0.10).clamp(min_tech, 0.40);
    let sum = copy.fac_share + copy.ship_share + copy.tech_share;
    if sum > 0.92 {
        let sc = 0.92 / sum;
        copy.fac_share *= sc;
        copy.ship_share *= sc;
        copy.tech_share *= sc;
    }

    copy.cruiser_share = (copy.cruiser_share + lcg_centered(rng_state) * 0.18).clamp(0.12, 0.75);
    copy.bship_share = (copy.bship_share + lcg_centered(rng_state) * 0.18).clamp(0.12, 0.75);
    copy.bomber_share = (copy.bomber_share + lcg_centered(rng_state) * 0.18).clamp(0.12, 0.75);
    let s_sum = copy.cruiser_share + copy.bship_share + copy.bomber_share;
    copy.cruiser_share /= s_sum;
    copy.bship_share /= s_sum;
    copy.bomber_share /= s_sum;
    copy.intercept_bias = (copy.intercept_bias + lcg_centered(rng_state) * 0.22).clamp(0.10, 0.95);

    let mut t_sum = 0.0f32;
    for f in 0..NUM_TECH_FIELDS {
        copy.tech_alloc[f] =
            (copy.tech_alloc[f] + lcg_centered(rng_state) * 0.10).clamp(0.05, 0.45);
        t_sum += copy.tech_alloc[f];
    }
    for f in 0..NUM_TECH_FIELDS {
        copy.tech_alloc[f] /= t_sum;
    }
    copy
}

pub fn run_co_evolution(sim: &mut Simulation, seed: u32) -> CoEvolutionResult {
    let mut rng_state = if seed == 0 { 424242 } else { seed };
    let base = rng_state % 80000;
    let seeds = [base, base + 1337, base + 4201];

    for _round in 0..4 {
        for emp_idx in 0..NUM_EMPIRES {
            let mut best = sim.doctrines[emp_idx].clone();
            let base_score = evaluate_empire_fitness(sim, emp_idx, &seeds);
            for _m in 0..2 {
                let cand = mutate_dna(&best, emp_idx, &mut rng_state);
                sim.doctrines[emp_idx] = cand.clone();
                if evaluate_empire_fitness(sim, emp_idx, &seeds) > base_score * 1.01 {
                    best = cand;
                    break;
                } else {
                    sim.doctrines[emp_idx] = best.clone();
                }
            }
        }
        sim.dna_generation += 1;
    }

    let tournament = run_tournament(sim, 24, 1350);
    CoEvolutionResult {
        dna_generation: sim.dna_generation,
        tournament,
    }
}

pub fn run_auto_balance(sim: &mut Simulation) -> AutoBalanceResult {
    for _iter in 0..5 {
        let res = run_tournament(sim, 15, 1350);
        let r0 = res.total_res[0].max(10.0);
        let r1 = res.total_res[1].max(10.0);
        let r2 = res.total_res[2].max(10.0);
        let avg = (r0 + r1 + r2) / 3.0;

        sim.cfg.ind_weight =
            (((sim.cfg.ind_weight * (avg / r0).powf(0.28)) * 100.0).round() / 100.0).clamp(0.6, 2.2);
        sim.cfg.exp_weight =
            (((sim.cfg.exp_weight * (avg / r1).powf(0.28)) * 100.0).round() / 100.0).clamp(0.6, 2.2);
        sim.cfg.tech_weight =
            (((sim.cfg.tech_weight * (avg / r2).powf(0.28)) * 100.0).round() / 100.0)
                .clamp(0.6, 2.2);
    }

    let tournament = run_tournament(sim, 25, 1350);
    AutoBalanceResult {
        ind_weight: sim.cfg.ind_weight,
        exp_weight: sim.cfg.exp_weight,
        tech_weight: sim.cfg.tech_weight,
        tournament,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SimConfig;

    #[test]
    fn tournament_returns_valid_win_rates() {
        let mut sim = Simulation::new(SimConfig::default());
        let res = run_tournament(&mut sim, 6, 400);
        assert_eq!(res.wins.iter().sum::<u32>(), 6);
        assert_eq!(res.win_rates.iter().sum::<u32>(), 100);
    }
}
