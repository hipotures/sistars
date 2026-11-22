use std::time::Instant;

use sistars_engine::ai;
use sistars_engine::config::{FleetClass, SimConfig};
use sistars_engine::galaxy::{GalaxyShape, GalaxySize};
use sistars_engine::sim::Simulation;
use sistars_engine::tech::{classic_races, tech_store, LRT_UR};
use sistars_engine::types::{
    DebrisField, DiplomaticStance, Fleet, SandboxPlanet, SimEvent, StepProfile,
};

fn run_trace_scenarios() {
    println!("=== SI-Stars Diagnostic Sandbox & Event Trace (--trace-scenario) ===");
    let mut races = classic_races();
    races[0].lrt_mask |= LRT_UR; // Enable Ultimate Recycling on Empire 1

    // 1. Salvage Cargo Cap & Conservation Trace
    println!("\n--- Scenario 1: Salvage Cargo Cap (220 kT hold vs 500 kT debris, LRT_UR active) ---");
    let planets = vec![SandboxPlanet {
        x: 100.0,
        y: 100.0,
        owner: 1,
        iron_surf: 50.0,
        bora_surf: 50.0,
        germ_surf: 50.0,
        ..Default::default()
    }];
    let fleets = vec![Fleet {
        id: 1,
        x: 100.0,
        y: 100.0,
        from: 0,
        target: 0,
        emp: 1,
        fleet_type: FleetClass::Freight,
        power: 12.0,
        max_power: 220.0,
        steer_x: 125.0,
        steer_y: 100.0,
        salvage_mode: 1,
        ..Default::default()
    }];
    let debris = vec![DebrisField {
        x: 125.0,
        y: 100.0,
        iron: 200.0,
        bora: 150.0,
        germ: 150.0,
        ttl: 50.0,
        wrecks: Vec::new(),
    }];
    let mut sim = Simulation::new_sandbox(
        SimConfig::default(),
        races.clone(),
        planets,
        fleets,
        debris,
        false,
    );
    for t in 1..=30 {
        sim.step_tick(true);
        for ev in sim.drain_events() {
            if !matches!(ev, SimEvent::EconomyTick { .. }) {
                println!("  tick {t:02} (yr {:.2}): {ev}", sim.year);
            }
        }
        if sim.fleets.first().map(|f| f.orbiting).unwrap_or(false) {
            break;
        }
    }
    println!(
        "  Auditor Summary: audited={}, closed_tx={}, anomalies={}",
        sim.auditor.audited_events_count,
        sim.auditor.closed_transactions_count,
        sim.anomalies.len()
    );

    // 2. Orbital Defense Sequence Trace (Garrison -> Starbase -> Ground Defense)
    println!("\n--- Scenario 2: Orbital Defense Sequence (Garrison -> Starbase -> Planet) ---");
    let planets = vec![
        SandboxPlanet {
            x: 100.0,
            y: 100.0,
            owner: 1,
            ..Default::default()
        },
        SandboxPlanet {
            x: 112.0,
            y: 100.0,
            owner: 2,
            defense: 25.0,
            pop: 20.0,
            starbase_tier: 1,
            starbase_hp: Some(100.0),
            ..Default::default()
        },
    ];
    let fleets = vec![
        // defending orbital garrison at planet #1
        Fleet {
            id: 10,
            x: 112.0,
            y: 100.0,
            from: 1,
            target: 1,
            emp: 2,
            fleet_type: FleetClass::Cruiser,
            power: 45.0,
            max_power: 45.0,
            orbiting: true,
            ..Default::default()
        },
        // overwhelming attacking fleet from Empire 1
        Fleet {
            id: 11,
            x: 111.0,
            y: 100.0,
            from: 0,
            target: 1,
            emp: 1,
            fleet_type: FleetClass::Battleship,
            power: 380.0,
            max_power: 380.0,
            ship_count: 6,
            ..Default::default()
        },
    ];
    let mut sim2 = Simulation::new_sandbox(
        SimConfig::default(),
        races,
        planets,
        fleets,
        Vec::new(),
        false,
    );
    sim2.set_relation(0, 1, DiplomaticStance::War);
    sim2.step_tick(true);
    for ev in sim2.drain_events() {
        if !matches!(ev, SimEvent::EconomyTick { .. }) {
            println!("  tick 01 (yr {:.2}): {ev}", sim2.year);
        }
    }
    println!("  Anomalies JSON:\n{}", sim2.export_anomalies_json());
}

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
    if std::env::args().any(|a| a == "--trace-scenario") {
        run_trace_scenarios();
        return;
    }

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
