use std::time::Instant;

use sistars_engine::ai;
use sistars_engine::config::SimConfig;
use sistars_engine::galaxy::{GalaxyShape, GalaxySize};
use sistars_engine::sim::Simulation;
use sistars_engine::tech::{classic_races, tech_store};

fn main() {
    let store = tech_store();
    let races = classic_races();

    // 1. Huge galaxy (1000 planets, 6 races, 6000 ticks) stability & late-game Stargate benchmark
    let mut huge_cfg = SimConfig::default();
    huge_cfg.galaxy_size = GalaxySize::Huge;
    huge_cfg.galaxy_shape = GalaxyShape::Random;
    let mut huge_sim = Simulation::new(huge_cfg, races.clone());
    huge_sim.init(592144, true);

    println!("Huge Stability Benchmark (1000 planets | Random | 6 empires | 6000 ticks = 300 yrs):");
    let total_start = Instant::now();
    for chunk in 0..6 {
        let c_start = Instant::now();
        for _ in 0..1000 {
            huge_sim.step_tick(true);
        }
        let c_ms = c_start.elapsed().as_secs_f64() * 1000.0;
        let owned: u32 = huge_sim.empires.iter().map(|e| e.last_stats.planets).sum();
        let gates: u32 = huge_sim.empires.iter().map(|e| e.last_stats.gates).sum();
        println!(
            "  ticks {:4}..{:4} (yr {:6.1}): {:6.1} ms ({:.3} ms/tick) | owned={:4} gates={:3} fleets={:4}",
            chunk * 1000,
            (chunk + 1) * 1000,
            huge_sim.year,
            c_ms,
            c_ms / 1000.0,
            owned,
            gates,
            huge_sim.fleets.len()
        );
    }
    let total_s = total_start.elapsed().as_secs_f64();
    println!(
        "  TOTAL 6000 ticks: {:.3}s ({:.0} ticks/sec | {:.3} ms/tick)",
        total_s,
        6000.0 / total_s,
        (total_s * 1000.0) / 6000.0
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
