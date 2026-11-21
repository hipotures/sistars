use std::time::Instant;

use sistars_engine::ai;
use sistars_engine::config::SimConfig;
use sistars_engine::galaxy::{GalaxyShape, GalaxySize};
use sistars_engine::sim::Simulation;
use sistars_engine::tech::{classic_races, tech_store};
use sistars_engine::types::StepProfile;

fn run_phase_bench(label: &str, size: GalaxySize, shape: GalaxyShape, seed: u32, total_ticks: usize, chunk_size: usize) {
    let races = classic_races();
    let mut cfg = SimConfig::default();
    cfg.galaxy_size = size;
    cfg.galaxy_shape = shape;
    let mut sim = Simulation::new(cfg, races);
    sim.init(seed, true);
    sim.profile_enabled = true;

    let num_chunks = total_ticks / chunk_size;
    println!(
        "\n{} ({} planets | {:?} | {} empires | {} ticks):",
        label,
        sim.owner.len(),
        shape,
        sim.empires.len(),
        total_ticks
    );
    let mut grand = StepProfile::default();
    for chunk in 0..num_chunks {
        let mut acc = StepProfile::default();
        for _ in 0..chunk_size {
            sim.step_tick(true);
            let p = sim.last_profile;
            acc.econ_ms += p.econ_ms;
            acc.ai_ms += p.ai_ms;
            acc.move_ms += p.move_ms;
            acc.ew_scan_ms += p.ew_scan_ms;
            acc.combat_ms += p.combat_ms;
            acc.total_ms += p.total_ms;
        }
        grand.econ_ms += acc.econ_ms;
        grand.ai_ms += acc.ai_ms;
        grand.move_ms += acc.move_ms;
        grand.ew_scan_ms += acc.ew_scan_ms;
        grand.combat_ms += acc.combat_ms;
        grand.total_ms += acc.total_ms;
        let inv = 1.0 / chunk_size as f64;
        let owned: u32 = sim.empires.iter().map(|e| e.last_stats.planets).sum();
        let gates: u32 = sim.empires.iter().map(|e| e.last_stats.gates).sum();
        println!(
            "  ticks {:4}..{:4} (yr {:6.1}): total={:6.3} ms/t [econ={:5.3} ai={:5.3} move={:5.3} ew={:5.3} combat={:5.3}] | owned={:5} gates={:4} fleets={:5}",
            chunk * chunk_size,
            (chunk + 1) * chunk_size,
            sim.year,
            acc.total_ms * inv,
            acc.econ_ms * inv,
            acc.ai_ms * inv,
            acc.move_ms * inv,
            acc.ew_scan_ms * inv,
            acc.combat_ms * inv,
            owned,
            gates,
            sim.fleets.len()
        );
    }
    let inv_all = 1.0 / total_ticks as f64;
    println!(
        "  AVG {} ticks: total={:.3} ms/t [econ={:.3} ai={:.3} move={:.3} ew={:.3} combat={:.3}]",
        total_ticks,
        grand.total_ms * inv_all,
        grand.econ_ms * inv_all,
        grand.ai_ms * inv_all,
        grand.move_ms * inv_all,
        grand.ew_scan_ms * inv_all,
        grand.combat_ms * inv_all,
    );
}

fn main() {
    let store = tech_store();
    let races = classic_races();

    // 1. Phase-profiled late-game benchmarks (Huge 1,000 planets & Colossal 10,000 planets)
    run_phase_bench("Huge Phase Benchmark", GalaxySize::Huge, GalaxyShape::Random, 592144, 4000, 1000);
    run_phase_bench("Colossal Phase Benchmark", GalaxySize::Colossal, GalaxyShape::Spiral, 592144, 3000, 500);

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
