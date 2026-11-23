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
    println!(
        "\n--- Scenario 1: Salvage Cargo Cap (220 kT hold vs 500 kT debris, LRT_UR active) ---"
    );
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

fn run_phase_bench(
    label: &str,
    size: GalaxySize,
    shape: GalaxyShape,
    seed: u32,
    total_ticks: usize,
    chunk_size: usize,
) {
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

/// `headless --replay <file.replay.jsonl[.gz]> [--until-tick N] [--audit] [--dump-state out.json]`
///
/// Re-simulates a recorded arena match with the current engine and reports whether it matches
/// the recording, or the first divergent governor action / state hash.
#[cfg(not(target_arch = "wasm32"))]
fn run_replay(args: &[String]) -> i32 {
    use sistars_engine::replay::{read_file, verify, Divergence};

    let Some(path) = args.get(2).filter(|a| !a.starts_with("--")) else {
        eprintln!(
            "usage: headless --replay <file.replay.jsonl[.gz]> [--until-tick N] [--audit] [--dump-state out.json]"
        );
        return 2;
    };
    let flag_value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let until_tick = match flag_value("--until-tick").map(|v| v.parse::<u64>()) {
        Some(Ok(t)) => Some(t),
        Some(Err(_)) => {
            eprintln!("--until-tick expects a number");
            return 2;
        }
        None => None,
    };
    let audit = args.iter().any(|a| a == "--audit");
    let dump = flag_value("--dump-state");

    let lines = match read_file(std::path::Path::new(path)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to read replay: {e}");
            return 1;
        }
    };
    let started = Instant::now();
    let report = match verify(&lines, until_tick, audit) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Invalid replay: {e}");
            return 1;
        }
    };
    let h = &report.header;
    println!(
        "Replay {} | recorded with {} | seed {} | {} empires | scenario '{}'",
        h.match_id,
        h.engine_commit,
        h.seed,
        h.races.len(),
        h.scenario_name
    );
    if let Some(f) = &report.footer {
        println!(
            "Recorded end: {} at tick {} (year {:.1}), winner {:?}, {} actions",
            f.end_reason, f.final_tick, f.final_year, f.winner, f.action_count
        );
    } else {
        println!("Recorded end: <no footer, replay truncated>");
    }
    if audit && !h.debug_audit {
        println!(
            "NOTE: --audit enables tracing, which changes scan cadence vs. the original run; \
             governor/hash comparison is skipped."
        );
    }
    println!(
        "Re-simulated {} ticks in {:.2}s; checked {} governor actions and {} state hashes.",
        report.ticks_run,
        started.elapsed().as_secs_f64(),
        report.governor_actions_checked,
        report.hashes_checked
    );
    if audit {
        println!(
            "LiveAuditor anomalies: {}\n{}",
            report.sim.anomalies.len(),
            report.sim.export_anomalies_json()
        );
    }
    if let Some(out) = dump {
        let sim = &report.sim;
        let state = serde_json::json!({
            "tick": sim.ticks,
            "year": sim.year,
            "winner": sim.winner,
            "victoryReason": sim.victory_reason,
            "stateHash": sistars_engine::replay::format_hash(sim.state_hash()),
            "planets": {
                "owner": sim.owner,
                "pop": sim.pop,
                "mines": sim.mines,
                "factories": sim.factories,
                "defense": sim.defense,
                "hasGate": sim.has_gate,
                "starbaseTier": sim.starbase_tier,
            },
            "fleets": sim.fleets,
            "debris": sim.debris_fields,
            "empireResTotal": sim.empires.iter().map(|e| e.res_total).collect::<Vec<_>>(),
        });
        match std::fs::write(
            &out,
            serde_json::to_string_pretty(&state).unwrap_or_default(),
        ) {
            Ok(()) => println!("State at tick {} written to {out}", sim.ticks),
            Err(e) => eprintln!("Failed to write {out}: {e}"),
        }
    }
    match &report.divergence {
        None => {
            println!(
                "OK: identical to the recording up to tick {}.",
                report.ticks_run
            );
            0
        }
        Some(Divergence::GovernorAction {
            t,
            emp,
            expected,
            actual,
        }) => {
            println!(
                "DIVERGED at tick {t}, empire #{}: governor action differs\n  recorded: {expected:?}\n  replayed: {actual:?}",
                emp + 1
            );
            3
        }
        Some(Divergence::Hash {
            t,
            expected,
            actual,
        }) => {
            println!("DIVERGED: state hash differs after tick {t} (recorded {expected}, replayed {actual})");
            3
        }
        Some(Divergence::FinalHash {
            t,
            expected,
            actual,
        }) => {
            println!("DIVERGED: final state hash differs at tick {t} (recorded {expected}, replayed {actual})");
            3
        }
    }
}

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    let args: Vec<String> = std::env::args().collect();
    #[cfg(not(target_arch = "wasm32"))]
    if args.get(1).map(String::as_str) == Some("--replay") {
        std::process::exit(run_replay(&args));
    }
    if std::env::args().any(|a| a == "--trace-scenario") {
        run_trace_scenarios();
        return;
    }

    let store = tech_store();
    let races = classic_races();

    // 1. Phase-profiled late-game benchmarks (Huge 1,000 planets & Colossal 10,000 planets)
    run_phase_bench(
        "Huge Phase Benchmark",
        GalaxySize::Huge,
        GalaxyShape::Random,
        592144,
        4000,
        1000,
    );
    run_phase_bench(
        "Colossal Phase Benchmark",
        GalaxySize::Colossal,
        GalaxyShape::Spiral,
        592144,
        3000,
        500,
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
