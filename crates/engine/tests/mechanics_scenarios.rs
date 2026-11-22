use sistars_engine::config::{FleetClass, SimConfig};
use sistars_engine::galaxy::GalaxySize;
use sistars_engine::sim::Simulation;
use sistars_engine::tech::{classic_races, LRT_UR};
use sistars_engine::types::{
    DebrisField, DiplomaticStance, Fleet, SandboxPlanet, SimEvent, VictoryConfig,
};

#[test]
fn scenario_salvage_respects_cargo_cap_and_mass_conservation() {
    let mut races = classic_races();
    // Equip Empire 1 with Ultimate Recycling (LRT_UR) to ensure racial salvage bonus
    // never overflows physical cargo_cap or fabricates extra minerals on unload.
    races[0].lrt_mask |= LRT_UR;

    // --- Variant A: Large 500 kT debris field vs 220 kT freighter cargo_cap ---
    let planets = vec![SandboxPlanet {
        x: 100.0,
        y: 100.0,
        owner: 1,
        pop: 0.0,       // 0 pop & 0 mines so planetary mining/economy doesn't alter min_surf during transit
        mines: 0.0,
        factories: 0.0,
        defense: 0.0,
        iron_surf: 50.0,
        bora_surf: 50.0,
        germ_surf: 50.0,
        iron_crust: 0.0,
        bora_crust: 0.0,
        germ_crust: 0.0,
        ..Default::default()
    }];
    let fleets = vec![Fleet {
        id: 101,
        x: 100.0,
        y: 100.0,
        from: 0,
        target: 0,
        emp: 1,
        fleet_type: FleetClass::Freight,
        power: 12.0,
        max_power: 220.0,
        steer_x: 120.0,
        steer_y: 100.0,
        salvage_mode: 1,
        ..Default::default()
    }];
    let debris = vec![DebrisField {
        x: 120.0,
        y: 100.0,
        iron: 200.0,
        bora: 150.0,
        germ: 150.0,
        ttl: 100.0,
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

    let mut load_ev = None;
    let mut unload_ev = None;
    for _ in 0..40 {
        sim.step_tick(true);
        for ev in sim.drain_events() {
            match ev {
                SimEvent::SalvageLoaded { .. } => load_ev = Some(ev),
                SimEvent::CargoUnloaded { .. } => unload_ev = Some(ev),
                _ => {}
            }
        }
        if unload_ev.is_some() {
            break;
        }
    }

    assert!(
        sim.anomalies.is_empty(),
        "Expected 0 anomalies from LiveAuditor, got: {:?}",
        sim.anomalies
    );

    let (loaded_amount, debris_rem) = match load_ev.expect("Expected SalvageLoaded event") {
        SimEvent::SalvageLoaded {
            fleet_id,
            debris_before,
            loaded,
            debris_after,
            cargo_cap,
        } => {
            assert_eq!(fleet_id, 101);
            assert!((debris_before - 500.0).abs() < 1e-3);
            assert!((cargo_cap - 220.0).abs() < 1e-3);
            assert!(
                loaded <= 220.0 + 1e-3 && (loaded - 220.0).abs() < 1e-3,
                "Expected loaded == 220.0 kT (capped by cargo_cap), got {loaded}"
            );
            assert!(
                (debris_before - loaded - debris_after).abs() < 1e-3,
                "Mass conservation failed at debris field: {debris_before} - {loaded} != {debris_after}"
            );
            (loaded, debris_after)
        }
        _ => unreachable!(),
    };

    assert!((debris_rem - 280.0).abs() < 1e-3);
    assert_eq!(sim.debris_fields.len(), 1);
    let actual_rem =
        sim.debris_fields[0].iron + sim.debris_fields[0].bora + sim.debris_fields[0].germ;
    assert!(
        (actual_rem - 280.0).abs() < 1e-3,
        "Remaining debris field in world should have 280 kT, got {actual_rem}"
    );

    match unload_ev.expect("Expected CargoUnloaded event") {
        SimEvent::CargoUnloaded {
            fleet_id,
            planet_idx,
            unloaded,
            planet_surf_before,
            planet_surf_after,
        } => {
            assert_eq!(fleet_id, 101);
            assert_eq!(planet_idx, 0);
            assert!(
                (unloaded - loaded_amount).abs() < 1e-3,
                "Unloaded amount ({unloaded}) must equal loaded amount ({loaded_amount})"
            );
            assert!(
                (planet_surf_after - planet_surf_before - loaded_amount).abs() < 1e-3,
                "Planet surface delta ({planet_surf_after} - {planet_surf_before}) must equal loaded ({loaded_amount})"
            );
        }
        _ => unreachable!(),
    }

    // --- Variant B: Small 80 kT debris field vs 220 kT freighter cargo_cap (cleared to zero) ---
    let planets_small = vec![SandboxPlanet {
        x: 100.0,
        y: 100.0,
        owner: 1,
        pop: 0.0,
        mines: 0.0,
        factories: 0.0,
        defense: 0.0,
        iron_surf: 10.0,
        bora_surf: 10.0,
        germ_surf: 10.0,
        iron_crust: 0.0,
        bora_crust: 0.0,
        germ_crust: 0.0,
        ..Default::default()
    }];
    let fleets_small = vec![Fleet {
        id: 102,
        x: 100.0,
        y: 100.0,
        from: 0,
        target: 0,
        emp: 1,
        fleet_type: FleetClass::Freight,
        power: 12.0,
        max_power: 220.0,
        steer_x: 118.0,
        steer_y: 100.0,
        salvage_mode: 1,
        ..Default::default()
    }];
    let debris_small = vec![DebrisField {
        x: 118.0,
        y: 100.0,
        iron: 30.0,
        bora: 25.0,
        germ: 25.0,
        ttl: 100.0,
        wrecks: Vec::new(),
    }];

    let mut sim_small = Simulation::new_sandbox(
        SimConfig::default(),
        races,
        planets_small,
        fleets_small,
        debris_small,
        false,
    );

    let mut small_load = None;
    let mut small_unload = None;
    for _ in 0..40 {
        sim_small.step_tick(true);
        for ev in sim_small.drain_events() {
            match ev {
                SimEvent::SalvageLoaded { .. } => small_load = Some(ev),
                SimEvent::CargoUnloaded { .. } => small_unload = Some(ev),
                _ => {}
            }
        }
        if small_unload.is_some() {
            break;
        }
    }

    assert!(sim_small.anomalies.is_empty());
    match small_load.expect("Expected SalvageLoaded for 80 kT field") {
        SimEvent::SalvageLoaded {
            debris_before,
            loaded,
            debris_after,
            ..
        } => {
            assert!((debris_before - 80.0).abs() < 1e-3);
            assert!((loaded - 80.0).abs() < 1e-3);
            assert!(debris_after.abs() < 1e-3, "Expected debris_after == 0, got {debris_after}");
        }
        _ => unreachable!(),
    }
    assert!(
        sim_small.debris_fields.is_empty(),
        "80 kT debris field should be completely removed after being scooped to zero"
    );
    match small_unload.expect("Expected CargoUnloaded for 80 kT field") {
        SimEvent::CargoUnloaded {
            unloaded,
            planet_surf_before,
            planet_surf_after,
            ..
        } => {
            assert!((unloaded - 80.0).abs() < 1e-3);
            assert!((planet_surf_after - planet_surf_before - 80.0).abs() < 1e-3);
        }
        _ => unreachable!(),
    }
}

#[test]
fn scenario_planet_assault_without_starbase() {
    let races = classic_races();
    let planets = vec![
        SandboxPlanet {
            x: 100.0,
            y: 100.0,
            owner: 1,
            ..Default::default()
        },
        SandboxPlanet {
            x: 115.0,
            y: 100.0,
            owner: 2,
            pop: 20.0,
            factories: 20.0,
            defense: 16.0,
            starbase_tier: 0,
            ..Default::default()
        },
    ];
    let fleets = vec![Fleet {
        id: 1,
        x: 100.0,
        y: 100.0,
        from: 0,
        target: 1,
        emp: 1,
        fleet_type: FleetClass::Battleship,
        power: 160.0,
        max_power: 160.0,
        ship_count: 3,
        ..Default::default()
    }];

    let mut sim =
        Simulation::new_sandbox(SimConfig::default(), races, planets, fleets, Vec::new(), false);

    let mut assault_ev = None;
    let mut sb_ev = None;
    for _ in 0..20 {
        sim.step_tick(true);
        for ev in sim.drain_events() {
            match ev {
                SimEvent::PlanetAssault { .. } => assault_ev = Some(ev),
                SimEvent::StarbaseAssault { .. } => sb_ev = Some(ev),
                _ => {}
            }
        }
        if assault_ev.is_some() {
            break;
        }
    }

    assert!(sim.anomalies.is_empty());
    assert!(sb_ev.is_none(), "No StarbaseAssault should occur when starbase_tier == 0");

    match assault_ev.expect("Expected PlanetAssault event") {
        SimEvent::PlanetAssault {
            planet_idx,
            captured,
            def_before,
            def_after,
        } => {
            assert_eq!(planet_idx, 1);
            assert!(captured, "Expected planet #1 to be captured");
            assert!(
                def_after < def_before,
                "Expected planetary defense to decrease on capture ({def_before} -> {def_after})"
            );
        }
        _ => unreachable!(),
    }

    assert_eq!(sim.owner[1], 1, "Planet #1 should now be owned by Empire 1");
    assert_eq!(sim.fleets.len(), 1);
    let survivor = &sim.fleets[0];
    assert!(survivor.orbiting, "Victorious fleet should station in orbit");
    assert_eq!(survivor.target, 1);
    assert!(survivor.power > 0.0 && survivor.power < 160.0);
}

#[test]
fn scenario_planet_assault_blocked_by_starbase() {
    let races = classic_races();
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
            pop: 0.0, // 0 pop so defense isn't altered by planetary economy before arrival
            mines: 0.0,
            factories: 25.0,
            defense: 45.0,
            starbase_tier: 4, // Ultra Station (1000 HP)
            starbase_hp: Some(1000.0),
            ..Default::default()
        },
    ];
    let fleets = vec![Fleet {
        id: 1,
        x: 111.0,
        y: 100.0,
        from: 0,
        target: 1,
        emp: 1,
        fleet_type: FleetClass::Cruiser,
        power: 120.0,
        max_power: 120.0,
        ship_count: 2,
        ..Default::default()
    }];

    let mut sim =
        Simulation::new_sandbox(SimConfig::default(), races, planets, fleets, Vec::new(), false);
    sim.step_tick(true);

    let events = sim.drain_events();
    assert!(sim.anomalies.is_empty());

    let sb_ev = events
        .iter()
        .find(|ev| matches!(ev, SimEvent::StarbaseAssault { .. }))
        .expect("Expected StarbaseAssault event");

    match sb_ev {
        SimEvent::StarbaseAssault {
            planet_idx,
            sb_hp_before,
            sb_hp_after,
            attacker_survived,
        } => {
            assert_eq!(*planet_idx, 1);
            assert!((*sb_hp_before - 1000.0).abs() < 1.0);
            assert!(*sb_hp_after < *sb_hp_before && *sb_hp_after > 0.0);
            assert!(!*attacker_survived, "Attacker must be destroyed by surviving Starbase");
        }
        _ => unreachable!(),
    }

    let planet_ev = events
        .iter()
        .find(|ev| matches!(ev, SimEvent::PlanetAssault { .. }));
    assert!(
        planet_ev.is_none(),
        "PlanetAssault must not occur when Starbase holds orbit"
    );

    assert_eq!(sim.owner[1], 2, "Planet #1 must remain owned by Empire 2");
    assert!(
        (sim.defense[1] - 45.0).abs() < 1e-4,
        "Planetary ground defense must remain completely untouched (45.0)"
    );
    assert!(sim.fleets.is_empty(), "Attacking fleet must be destroyed");
    assert!(
        !sim.debris_fields.is_empty(),
        "Destroyed attacker must leave a DebrisField at orbit"
    );
}

#[test]
fn scenario_orbiting_garrison_intercepts_before_starbase() {
    let races = classic_races();

    // --- Part 1: Strong orbiting garrison destroys attacker; Starbase & Planet take 0 damage ---
    let planets = vec![
        SandboxPlanet {
            x: 100.0,
            y: 100.0,
            owner: 1,
            ..Default::default()
        },
        SandboxPlanet {
            x: 110.0,
            y: 100.0,
            owner: 2,
            pop: 0.0,
            mines: 0.0,
            factories: 20.0,
            defense: 40.0,
            starbase_tier: 3,
            starbase_hp: Some(500.0),
            ..Default::default()
        },
    ];
    let fleets = vec![
        // Defending garrison stationed in orbit at planet #1
        Fleet {
            id: 1,
            x: 110.0,
            y: 100.0,
            from: 1,
            target: 1,
            emp: 2,
            fleet_type: FleetClass::Battleship,
            power: 300.0,
            max_power: 300.0,
            orbiting: true,
            ship_count: 5,
            ..Default::default()
        },
        // Incoming attacker arriving at planet #1 this tick
        Fleet {
            id: 2,
            x: 109.0,
            y: 100.0,
            from: 0,
            target: 1,
            emp: 1,
            fleet_type: FleetClass::Cruiser,
            power: 90.0,
            max_power: 90.0,
            ship_count: 2,
            ..Default::default()
        },
    ];

    let mut sim = Simulation::new_sandbox(
        SimConfig::default(),
        races.clone(),
        planets,
        fleets,
        Vec::new(),
        false,
    );
    sim.step_tick(true);
    let events = sim.drain_events();
    assert!(sim.anomalies.is_empty());

    let battle_ev = events
        .iter()
        .find(|ev| matches!(ev, SimEvent::SpaceBattle { .. }))
        .expect("Expected orbital SpaceBattle event");
    assert_eq!(
        *battle_ev,
        SimEvent::SpaceBattle {
            winner_emp: 2,
            loser_emp: 1,
            orbital_defense: true,
            planet_idx: Some(1),
        }
    );
    assert!(
        !events.iter().any(|ev| matches!(
            ev,
            SimEvent::StarbaseAssault { .. } | SimEvent::PlanetAssault { .. }
        )),
        "StarbaseAssault and PlanetAssault must NOT occur when garrison wins"
    );
    assert!(
        (sim.starbase_hp[1] - 500.0).abs() < 1e-4,
        "Starbase must take 0 damage when garrison wins"
    );
    assert!(
        (sim.defense[1] - 40.0).abs() < 1e-4,
        "Planet ground defense must take 0 damage when garrison wins"
    );

    // --- Part 2: Overwhelming attacker defeats garrison -> then Starbase -> then Planet in exact order ---
    let planets2 = vec![
        SandboxPlanet {
            x: 100.0,
            y: 100.0,
            owner: 1,
            ..Default::default()
        },
        SandboxPlanet {
            x: 110.0,
            y: 100.0,
            owner: 2,
            pop: 15.0,
            mines: 0.0,
            factories: 20.0,
            defense: 20.0,
            starbase_tier: 1,
            starbase_hp: Some(100.0),
            ..Default::default()
        },
    ];
    let fleets2 = vec![
        Fleet {
            id: 10,
            x: 110.0,
            y: 100.0,
            from: 1,
            target: 1,
            emp: 2,
            fleet_type: FleetClass::Cruiser,
            power: 40.0,
            max_power: 40.0,
            orbiting: true,
            ..Default::default()
        },
        Fleet {
            id: 11,
            x: 109.0,
            y: 100.0,
            from: 0,
            target: 1,
            emp: 1,
            fleet_type: FleetClass::Battleship,
            power: 420.0,
            max_power: 420.0,
            ship_count: 7,
            ..Default::default()
        },
    ];

    let mut sim2 =
        Simulation::new_sandbox(SimConfig::default(), races, planets2, fleets2, Vec::new(), false);
    sim2.step_tick(true);
    let events2 = sim2.drain_events();
    assert!(sim2.anomalies.is_empty());

    let idx_space = events2
        .iter()
        .position(|ev| {
            matches!(
                ev,
                SimEvent::SpaceBattle {
                    orbital_defense: true,
                    planet_idx: Some(1),
                    ..
                }
            )
        })
        .expect("Expected orbital SpaceBattle first");
    let idx_sb = events2
        .iter()
        .position(|ev| matches!(ev, SimEvent::StarbaseAssault { planet_idx: 1, .. }))
        .expect("Expected StarbaseAssault second");
    let idx_planet = events2
        .iter()
        .position(|ev| {
            matches!(
                ev,
                SimEvent::PlanetAssault {
                    planet_idx: 1,
                    captured: true,
                    ..
                }
            )
        })
        .expect("Expected PlanetAssault third");

    assert!(
        idx_space < idx_sb && idx_sb < idx_planet,
        "Strict ordering violated: SpaceBattle({idx_space}) -> StarbaseAssault({idx_sb}) -> PlanetAssault({idx_planet})"
    );
}

#[test]
fn scenario_economy_conservation_no_triple_spend() {
    let races = classic_races();
    // Developed planet with 0 Germanium on surface and 0 Germanium in crust -> factory building is blocked,
    // generating a large surplus_res that feeds defense, shipyard, and alchemy sequentially.
    let planets = vec![SandboxPlanet {
        x: 100.0,
        y: 100.0,
        owner: 1,
        hab: 1.0,
        pop: 140.0,
        mines: 80.0,
        factories: 120.0,
        defense: 10.0,
        iron_surf: 50.0,
        bora_surf: 100.0,
        germ_surf: 0.0,
        iron_crust: 0.0,
        bora_crust: 0.0,
        germ_crust: 0.0,
        ..Default::default()
    }];

    let mut sim =
        Simulation::new_sandbox(SimConfig::default(), races, planets, Vec::new(), Vec::new(), false);

    let mut econ_ticks = 0usize;
    for _ in 0..20 {
        sim.step_tick(true);
        for ev in sim.drain_events() {
            if let SimEvent::EconomyTick {
                planet_idx,
                res_gen,
                spent_tech,
                spent_mines,
                spent_fac,
                spent_def,
                spent_ship,
                spent_alchemy,
            } = ev
            {
                assert_eq!(planet_idx, 0);
                econ_ticks += 1;
                let total_spent = spent_tech
                    + spent_mines
                    + spent_fac
                    + spent_def
                    + spent_ship
                    + spent_alchemy;
                assert!(
                    total_spent <= res_gen + 1e-3,
                    "Budget overspend detected: total_spent={total_spent} > res_gen={res_gen}"
                );
                assert!(
                    (total_spent - res_gen).abs() < 1e-3,
                    "Expected full budget accounting: total_spent={total_spent} vs res_gen={res_gen}"
                );
                assert!(spent_def > 0.0, "Expected defense spending from surplus_res");
                assert!(spent_ship > 0.0, "Expected shipyard spending from surplus_res");
                assert!(spent_alchemy > 0.0, "Expected alchemy spending from surplus_res");
            }
        }
    }

    assert_eq!(econ_ticks, 20);
    assert!(
        sim.anomalies.is_empty(),
        "Expected 0 economy anomalies, got: {:?}",
        sim.anomalies
    );
}

#[test]
fn scenario_diplomacy_prevents_combat_until_war() {
    let races = classic_races();
    let planets = vec![
        SandboxPlanet {
            x: 50.0,
            y: 100.0,
            owner: 1,
            ..Default::default()
        },
        SandboxPlanet {
            x: 450.0,
            y: 100.0,
            owner: 2,
            ..Default::default()
        },
    ];
    // Place two warships within < intercept_rad (e.g. 4 ly apart)
    let fleets = vec![
        Fleet {
            id: 1,
            x: 248.0,
            y: 100.0,
            from: 0,
            target: 1,
            emp: 1,
            fleet_type: FleetClass::Battleship,
            power: 150.0,
            max_power: 150.0,
            ..Default::default()
        },
        Fleet {
            id: 2,
            x: 252.0,
            y: 100.0,
            from: 1,
            target: 0,
            emp: 2,
            fleet_type: FleetClass::Cruiser,
            power: 100.0,
            max_power: 100.0,
            ..Default::default()
        },
    ];

    let mut sim =
        Simulation::new_sandbox(SimConfig::default(), races, planets, fleets, Vec::new(), false);

    // 1. Neutral stance: fleets pass within < intercept_rad without fighting
    sim.set_relation(0, 1, DiplomaticStance::Neutral);
    sim.step_tick(true);
    assert_eq!(sim.space_battles_count, 0);
    assert_eq!(sim.fleets.len(), 2);
    assert!((sim.fleets[0].power - 150.0).abs() < 1e-4);
    assert!((sim.fleets[1].power - 100.0).abs() < 1e-4);

    // 2. Alliance stance: still zero combat and 100% HP
    sim.set_relation(0, 1, DiplomaticStance::Alliance);
    sim.step_tick(true);
    assert_eq!(sim.space_battles_count, 0);
    assert_eq!(sim.fleets.len(), 2);
    assert!((sim.fleets[0].power - 150.0).abs() < 1e-4);
    assert!((sim.fleets[1].power - 100.0).abs() < 1e-4);
    sim.drain_events();

    // 3. Switch to War: immediate battle on the very next tick!
    sim.set_relation(0, 1, DiplomaticStance::War);
    sim.step_tick(true);
    let events = sim.drain_events();
    assert_eq!(sim.space_battles_count, 1);
    assert_eq!(sim.fleets.len(), 1, "Loser fleet must be destroyed on War tick");
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, SimEvent::SpaceBattle { winner_emp: 1, loser_emp: 2, .. })),
        "Expected SpaceBattle event on the tick War was declared"
    );
}

#[test]
fn scenario_victory_condition_triggers() {
    let races = classic_races().into_iter().take(2).collect();
    // 3 planets: Empire 1 owns 1 (33.3%), Empire 2 owns 2 (Planet #1 is weakly defended, Planet #2 is distant)
    let planets = vec![
        SandboxPlanet {
            x: 100.0,
            y: 100.0,
            owner: 1,
            pop: 60.0,
            factories: 40.0,
            ..Default::default()
        },
        SandboxPlanet {
            x: 112.0,
            y: 100.0,
            owner: 2,
            pop: 15.0,
            factories: 10.0,
            defense: 10.0,
            ..Default::default()
        },
        SandboxPlanet {
            x: 350.0,
            y: 100.0,
            owner: 2,
            pop: 30.0,
            factories: 20.0,
            defense: 30.0,
            ..Default::default()
        },
    ];
    let fleets = vec![Fleet {
        id: 1,
        x: 111.0,
        y: 100.0,
        from: 0,
        target: 1,
        emp: 1,
        fleet_type: FleetClass::Battleship,
        power: 200.0,
        max_power: 200.0,
        ship_count: 4,
        ..Default::default()
    }];

    let mut sim =
        Simulation::new_sandbox(SimConfig::default(), races, planets, fleets, Vec::new(), false);
    sim.victory_cfg = VictoryConfig {
        enabled: true,
        planet_share_threshold: 0.60,
        res_share_threshold: 0.65,
        min_planets_for_share: 2,
        elimination_victory: true,
        max_year: None,
    };

    assert_eq!(sim.winner, None);
    sim.step_tick(true);

    assert_eq!(
        sim.winner,
        Some(0),
        "Empire 1 (index 0) now controls 2/3 (66.7% >= 60%) of planets and should win"
    );
    let events = sim.drain_events();
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, SimEvent::VictoryTriggered { winner_emp: 1, .. })),
        "Expected VictoryTriggered event for Empire 1"
    );
}

#[test]
fn scenario_live_galaxy_invariants() {
    let mut cfg = SimConfig::default();
    cfg.galaxy_size = GalaxySize::Small;
    let mut sim = Simulation::new(cfg, classic_races());
    sim.init(424242, true);
    sim.trace_enabled = true;

    for tick in 0..200 {
        sim.step_tick(true);
        let events = sim.drain_events();

        assert!(
            sim.anomalies.is_empty(),
            "LiveAuditor caught anomalies at tick {tick}: {}",
            sim.export_anomalies_json()
        );

        for ev in &events {
            match ev {
                SimEvent::EconomyTick {
                    res_gen,
                    spent_tech,
                    spent_mines,
                    spent_fac,
                    spent_def,
                    spent_ship,
                    spent_alchemy,
                    ..
                } => {
                    let sum = spent_tech
                        + spent_mines
                        + spent_fac
                        + spent_def
                        + spent_ship
                        + spent_alchemy;
                    assert!(
                        sum <= *res_gen + 1e-3,
                        "Tick {tick}: planet overspent {sum} > {res_gen}"
                    );
                }
                SimEvent::SalvageLoaded {
                    debris_before,
                    loaded,
                    debris_after,
                    cargo_cap,
                    ..
                } => {
                    assert!(*loaded <= *cargo_cap + 1e-3);
                    assert!((*debris_before - *loaded - *debris_after).abs() < 1e-2);
                }
                _ => {}
            }
        }

        for i in 0..sim.owner.len() {
            assert!(sim.pop[i].is_finite() && sim.pop[i] >= 0.0);
            assert!(sim.mines[i].is_finite() && sim.mines[i] >= 0.0);
            assert!(sim.factories[i].is_finite() && sim.factories[i] >= 0.0);
            assert!(sim.defense[i].is_finite() && sim.defense[i] >= 0.0);
            for m in 0..3 {
                assert!(sim.min_surf[i * 3 + m].is_finite() && sim.min_surf[i * 3 + m] >= 0.0);
                assert!(sim.min_crust[i * 3 + m].is_finite() && sim.min_crust[i * 3 + m] >= 0.0);
            }
        }

        for fl in &sim.fleets {
            assert!(fl.x.is_finite() && fl.y.is_finite());
            assert!(fl.power.is_finite() && fl.power >= 0.0);
            assert!(fl.ship_count >= 1);
            if fl.fleet_type == FleetClass::Freight {
                let cargo = fl.cargo_iron + fl.cargo_bora + fl.cargo_germ;
                assert!(
                    cargo <= fl.max_power + 1e-3,
                    "Tick {tick}: freighter #{} cargo {cargo} exceeds max_power {}",
                    fl.id,
                    fl.max_power
                );
            }
        }
    }
}

#[test]
fn live_auditor_prunes_valid_events_and_captures_anomalies_to_json() {
    let races = classic_races();
    let planets = vec![SandboxPlanet {
        x: 100.0,
        y: 100.0,
        owner: 1,
        pop: 80.0,
        mines: 30.0,
        factories: 40.0,
        ..Default::default()
    }];
    let mut sim =
        Simulation::new_sandbox(SimConfig::default(), races, planets, Vec::new(), Vec::new(), false);
    sim.auditor.prune_valid_events = true;

    for _ in 0..50 {
        sim.step_tick(true);
    }

    // With prune_valid_events == true, valid events are immediately discarded after LiveAuditor verification
    assert!(
        sim.events.is_empty(),
        "Expected valid events to be immediately pruned from RAM when prune_valid_events=true"
    );
    assert!(sim.auditor.audited_events_count >= 50);
    assert!(sim.auditor.closed_transactions_count >= 50);
    assert!(sim.anomalies.is_empty());

    // Inject an artificial violation event to verify LiveAuditor catches it and serializes to JSON
    sim.record_trace_event(SimEvent::SalvageLoaded {
        fleet_id: 999,
        debris_before: 500.0,
        loaded: 350.0,
        debris_after: 150.0,
        cargo_cap: 220.0,
    });
    assert!(!sim.anomalies.is_empty());
    assert_eq!(sim.anomalies[0].kind, "SalvageCargoCapExceeded");
    let json = sim.export_anomalies_json();
    assert!(json.contains("SalvageCargoCapExceeded"));
}

