use crate::config::NUM_TECH_FIELDS;
use crate::sim::Simulation;
use crate::types::{CoEvolutionResult, Doctrine, TournamentResult};

pub fn run_tournament(
    sim: &mut Simulation,
    num_games: u32,
    ticks_per_game: u32,
) -> TournamentResult {
    let num_emp = sim.empires.len();
    let mut wins = vec![0u32; num_emp];
    let mut total_res = vec![0.0f32; num_emp];
    let games = num_games.max(1);

    for g in 0..games {
        sim.init(80000u32.wrapping_add(g.wrapping_mul(7919)), true);
        for _ in 0..ticks_per_game {
            sim.step_tick(true);
        }
        let st = &sim.tick_stats;

        let mut winner = 0usize;
        for e in 1..num_emp {
            if st[e].res > st[winner].res {
                winner = e;
            }
        }
        wins[winner] += 1;
        for e in 0..num_emp {
            total_res[e] += st[e].res;
        }
    }

    let mut win_rates = vec![0u32; num_emp];
    let mut sum_wr = 0u32;
    for e in 0..num_emp {
        if e + 1 == num_emp {
            win_rates[e] = 100u32.saturating_sub(sum_wr);
        } else {
            let wr = ((wins[e] as f32 / games as f32) * 100.0).round() as u32;
            win_rates[e] = wr.min(100u32.saturating_sub(sum_wr));
            sum_wr += win_rates[e];
        }
        sim.empires[e].race.doctrine.win_rate = win_rates[e];
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
        for _ in 0..600 {
            sim.step_tick(true);
        }
        let mid_res = sim.tick_stats[emp_idx].res;
        for _ in 0..600 {
            sim.step_tick(true);
        }
        let st_end = &sim.tick_stats;
        let sum_end: f32 = st_end.iter().map(|s| s.res).sum::<f32>() + 1.0;
        let my_end = st_end[emp_idx].res;
        let max_end = st_end.iter().map(|s| s.res).fold(0.0f32, f32::max);
        let is_winner = if my_end >= max_end { 1.0 } else { 0.0 };
        score += (my_end / sum_end) * 1000.0 + is_winner * 350.0 + (mid_res * 0.05);
    }
    score / (seeds.len().max(1) as f32)
}

fn lcg_centered(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    (*state as f64 / 4294967296.0) as f32 - 0.5
}

pub fn mutate_dna(doc: &Doctrine, rng_state: &mut u32) -> Doctrine {
    let mut copy = doc.clone();
    copy.fac_share = (copy.fac_share + lcg_centered(rng_state) * 0.12).clamp(0.28, 0.66);
    copy.ship_share = (copy.ship_share + lcg_centered(rng_state) * 0.12).clamp(0.14, 0.50);
    copy.tech_share = (copy.tech_share + lcg_centered(rng_state) * 0.10).clamp(0.08, 0.40);
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
    let seeds = [base, base + 1337];
    let num_emp = sim.empires.len();

    for _round in 0..3 {
        for emp_idx in 0..num_emp {
            let mut best = sim.empires[emp_idx].race.doctrine.clone();
            let base_score = evaluate_empire_fitness(sim, emp_idx, &seeds);
            for _m in 0..2 {
                let cand = mutate_dna(&best, &mut rng_state);
                sim.empires[emp_idx].race.doctrine = cand.clone();
                if evaluate_empire_fitness(sim, emp_idx, &seeds) > base_score * 1.01 {
                    best = cand;
                    break;
                } else {
                    sim.empires[emp_idx].race.doctrine = best.clone();
                }
            }
        }
        sim.dna_generation += 1;
    }

    let tournament = run_tournament(sim, 18, 1200);
    CoEvolutionResult {
        dna_generation: sim.dna_generation,
        tournament,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SimConfig;
    use crate::tech::classic_races;

    #[test]
    fn tournament_works_for_six_classic_races() {
        let mut sim = Simulation::new(SimConfig::default(), classic_races());
        let res = run_tournament(&mut sim, 6, 400);
        assert_eq!(res.wins.len(), 6);
        assert_eq!(res.wins.iter().sum::<u32>(), 6);
        assert_eq!(res.win_rates.iter().sum::<u32>(), 100);
    }
}
