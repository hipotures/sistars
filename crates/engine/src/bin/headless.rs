use std::time::Instant;

use sistars_engine::ai;
use sistars_engine::config::SimConfig;
use sistars_engine::sim::Simulation;

fn main() {
    let num_games = 40u32;
    let ticks_per_game = 1400u32;

    let mut sim = Simulation::new(SimConfig::default());
    let start = Instant::now();
    let res = ai::run_tournament(&mut sim, num_games, ticks_per_game);
    let elapsed = start.elapsed();
    let total_ticks = (num_games * ticks_per_game) as f64;
    let tps = total_ticks / elapsed.as_secs_f64();

    println!(
        "SI-Stars Headless Tournament ({num_games} games x {ticks_per_game} ticks = {} ticks)",
        num_games * ticks_per_game
    );
    println!(
        "Elapsed: {:.3}s | Throughput: {:.0} ticks/sec",
        elapsed.as_secs_f64(),
        tps
    );
    println!(
        "Win-Rate: Industry {}% ({}) | Fleet {}% ({}) | Science {}% ({})",
        res.win_rates[0],
        res.wins[0],
        res.win_rates[1],
        res.wins[1],
        res.win_rates[2],
        res.wins[2],
    );
}
