use std::time::Instant;

use sistars_engine::ai;
use sistars_engine::config::SimConfig;
use sistars_engine::galaxy::{GalaxyShape, GalaxySize};
use sistars_engine::sim::Simulation;
use sistars_engine::tech::{classic_races, tech_store};

fn main() {
    let store = tech_store();
    let races = classic_races();

    // 1. Huge galaxy (1000 planets, 6 races, 2000 ticks) single-game stress benchmark
    let mut huge_cfg = SimConfig::default();
    huge_cfg.galaxy_size = GalaxySize::Huge;
    huge_cfg.galaxy_shape = GalaxyShape::Spiral;
    let mut huge_sim = Simulation::new(huge_cfg, races.clone());
    huge_sim.init(80000, true);
    let huge_ticks = 2000u32;
    let t0 = Instant::now();
    let mut max_fleets = 0usize;
    for _ in 0..huge_ticks {
        huge_sim.step_tick(true);
        if huge_sim.fleets.len() > max_fleets {
            max_fleets = huge_sim.fleets.len();
        }
    }
    let huge_elapsed = t0.elapsed();
    let owned_planets: u32 = huge_sim.empires.iter().map(|e| e.last_stats.planets).sum();
    println!(
        "Huge Benchmark (1000 planets | 6 empires | {huge_ticks} ticks): {:.3}s ({:.0} ticks/sec | {:.3} ms/tick)",
        huge_elapsed.as_secs_f64(),
        huge_ticks as f64 / huge_elapsed.as_secs_f64(),
        (huge_elapsed.as_secs_f64() * 1000.0) / huge_ticks as f64
    );
    println!(
        "  Owned: {owned_planets}/1000 | Active fleets: {} (peak {max_fleets}) | Battles: {} | Invasions: {}",
        huge_sim.fleets.len(),
        huge_sim.space_battles_count,
        huge_sim.invasions_count
    );

    // 2. Small galaxy tournament (30 games x 1200 ticks)
    let num_games = 30u32;
    let ticks_per_game = 1200u32;
    let mut sim = Simulation::new(SimConfig::default(), races);
    let start = Instant::now();
    let res = ai::run_tournament(&mut sim, num_games, ticks_per_game);
    let elapsed = start.elapsed();
    let total_ticks = (num_games * ticks_per_game) as f64;
    let tps = total_ticks / elapsed.as_secs_f64();

    println!(
        "\nSmall Tournament ({} techs | {} empires | {num_games} games x {ticks_per_game} ticks): {:.3}s ({:.0} ticks/sec)",
        store.entries.len(),
        sim.empires.len(),
        elapsed.as_secs_f64(),
        tps
    );
    for (idx, emp) in sim.empires.iter().enumerate() {
        println!(
            "  {:14} ({:8}): {:3}% ({:2} wins)",
            emp.race.name, emp.race.prt, res.win_rates[idx], res.wins[idx]
        );
    }
}
