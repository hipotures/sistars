use std::time::Instant;

use sistars_engine::ai;
use sistars_engine::config::SimConfig;
use sistars_engine::sim::Simulation;
use sistars_engine::tech::{classic_races, tech_store};

fn main() {
    let store = tech_store();
    let races = classic_races();
    let num_games = 30u32;
    let ticks_per_game = 1200u32;

    let mut sim = Simulation::new(SimConfig::default(), races);
    let start = Instant::now();
    let res = ai::run_tournament(&mut sim, num_games, ticks_per_game);
    let elapsed = start.elapsed();
    let total_ticks = (num_games * ticks_per_game) as f64;
    let tps = total_ticks / elapsed.as_secs_f64();

    println!(
        "SI-Stars Headless Tournament ({} craig-stars techs | {} empires | {num_games} games x {ticks_per_game} ticks)",
        store.entries.len(),
        sim.empires.len()
    );
    println!(
        "Elapsed: {:.3}s | Throughput: {:.0} ticks/sec",
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
