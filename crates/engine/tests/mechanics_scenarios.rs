use sistars_engine::config::{FleetClass, SimConfig};
use sistars_engine::galaxy::GalaxySize;
use sistars_engine::governor::sector_for_xy;
use sistars_engine::sim::Simulation;
use sistars_engine::tech::{classic_races, LRT_UR};
use sistars_engine::types::{
    DebrisField, DiplomaticStance, Fleet, SandboxPlanet, SectorMode, SimEvent, StrategicCommand,
    VictoryConfig, Wormhole,
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

#[test]
fn scenario_fow_intel_freezes_out_of_scanner_range() {
    let races = classic_races();
    // Planet #0 (Empire 1 homeworld at x=100) and Planet #1 (Empire 2 world far away at x=500)
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
            x: 500.0,
            y: 100.0,
            owner: 2,
            pop: 40.0,
            factories: 30.0,
            defense: 20.0,
            starbase_tier: 0,
            ..Default::default()
        },
    ];
    // Empire 1 scout fleet starts right next to Planet #1 (within orbital penetrating scan range <= 4 ly)
    // and is heading back home to Planet #0 (x=100).
    let fleets = vec![Fleet {
        id: 1,
        x: 498.0,
        y: 100.0,
        from: 0,
        target: 0,
        emp: 1,
        fleet_type: FleetClass::Cruiser,
        power: 80.0,
        max_power: 80.0,
        steer_x: 100.0,
        steer_y: 100.0,
        ..Default::default()
    }];

    let mut sim =
        Simulation::new_sandbox(SimConfig::default(), races, planets, fleets, Vec::new(), false);
    sim.empires[0].tech_cache.scanner_range = 60.0;
    sim.empires[0].tech_cache.scanner_pen = 30.0;

    // Tick 1: Fleet at x=498 scans Planet #1 at x=500 (distance 2 ly <= pen_radius)
    sim.step_tick(true);
    let intel_t1 = sim.planet_intel(0, 1);
    assert!(intel_t1.is_explored());
    assert!(intel_t1.is_pen_scanned());
    assert_eq!(intel_t1.owner, 2);
    assert_eq!(intel_t1.starbase_tier, 0);
    let seen_tick_before_leaving = intel_t1.last_seen_tick;
    let seen_fac_before = intel_t1.fac_est;
    let seen_pop_before = intel_t1.pop_est;
    let seen_def_before = intel_t1.def_est;
    assert!((seen_fac_before - sim.factories[1]).abs() < 1.0);

    // Move the fleet back to homeworld (x=100), completely out of scanner range of Planet #1 (distance 400 ly > 60 ly)
    sim.fleets[0].x = 120.0;
    sim.fleets[0].y = 100.0;

    // Now Planet #1 builds +50 factories, +40 defense, grows pop, and commissions a Tier 3 Starbase!
    sim.factories[1] += 50.0;
    sim.defense[1] += 40.0;
    sim.pop[1] += 35.0;
    sim.starbase_tier[1] = 3;
    sim.starbase_hp[1] = 500.0;
    sim.starbase_max_hp[1] = 500.0;

    for _ in 0..15 {
        sim.step_tick(true);
    }

    // Verify Empire 1's intel on Planet #1 is completely frozen at the old scan (zero live-feed leak!)
    let intel_frozen = sim.planet_intel(0, 1);
    assert_eq!(
        intel_frozen.last_seen_tick, seen_tick_before_leaving,
        "last_seen_tick must freeze once planet leaves scanner coverage"
    );
    assert_eq!(
        intel_frozen.starbase_tier, 0,
        "Unseen Starbase construction must not leak into frozen PlanetIntel"
    );
    assert!((intel_frozen.fac_est - seen_fac_before).abs() < 1e-4);
    assert!((intel_frozen.pop_est - seen_pop_before).abs() < 1e-4);
    assert!((intel_frozen.def_est - seen_def_before).abs() < 1e-4);
}

#[test]
fn scenario_fow_scanner_pen_required_for_surface_intel() {
    let races = classic_races();
    let planets = vec![
        // Planet #0: Empire 1 scanner post at (100, 100)
        SandboxPlanet {
            x: 100.0,
            y: 100.0,
            owner: 1,
            pop: 60.0,
            factories: 40.0,
            ..Default::default()
        },
        // Planet #1: Neutral planet at distance 80 ly (x=180, y=100)
        SandboxPlanet {
            x: 180.0,
            y: 100.0,
            owner: 0,
            hab: 0.88,
            iron_crust: 300.0,
            bora_crust: 250.0,
            germ_crust: 350.0,
            ..Default::default()
        },
        // Planet #2: Enemy planet at distance 85 ly (x=100, y=185)
        SandboxPlanet {
            x: 100.0,
            y: 185.0,
            owner: 2,
            hab: 0.75,
            pop: 55.0,
            factories: 48.0,
            defense: 32.0,
            starbase_tier: 2,
            ..Default::default()
        },
    ];

    let mut sim = Simulation::new_sandbox(
        SimConfig::default(),
        races,
        planets,
        Vec::new(),
        Vec::new(),
        false,
    );
    // Basic non-penetrating scanner: range = 120 ly, pen = 0 ly
    sim.empires[0].tech_cache.scanner_range = 120.0;
    sim.empires[0].tech_cache.scanner_pen = 0.0;
    sim.refresh_initial_fow();
    sim.step_tick(true);

    // 1. Non-penetrating scan sees owner & starbase_tier, but ZERO surface intel
    let intel_neutral = sim.planet_intel(0, 1);
    assert!(intel_neutral.is_explored());
    assert!(
        !intel_neutral.is_pen_scanned(),
        "Neutral planet at 80 ly must NOT be pen-scanned when scanner_pen == 0"
    );
    assert_eq!(intel_neutral.owner, 0);
    assert_eq!(intel_neutral.hab_est, 0.0);
    assert_eq!(intel_neutral.crust_sum_est, 0.0);

    let intel_enemy = sim.planet_intel(0, 2);
    assert!(intel_enemy.is_explored());
    assert!(
        !intel_enemy.is_pen_scanned(),
        "Enemy planet at 85 ly must NOT be pen-scanned when scanner_pen == 0"
    );
    assert_eq!(intel_enemy.owner, 2);
    assert_eq!(intel_enemy.starbase_tier, 2);
    assert_eq!(intel_enemy.pop_est, 0.0);
    assert_eq!(intel_enemy.fac_est, 0.0);
    assert_eq!(intel_enemy.def_est, 0.0);

    let obs_non_pen = sim.take_agent_observation(0);
    for fp in &obs_non_pen.enemy_planets {
        assert!(!fp.pen_scanned);
        assert!(fp.pop_est.is_none());
        assert!(fp.fac_est.is_none());
        assert!(fp.def_est.is_none());
    }

    // 2. Unlock penetrating scanner (scanner_pen = 100 ly >= 85 ly) and step 1 tick
    sim.empires[0].tech_cache.scanner_pen = 100.0;
    sim.step_tick(true);

    let intel_neutral_pen = sim.planet_intel(0, 1);
    assert!(intel_neutral_pen.is_pen_scanned());
    assert!((intel_neutral_pen.hab_est - 0.88).abs() < 1e-4);
    assert!((intel_neutral_pen.crust_sum_est - 900.0).abs() < 1.0);

    let intel_enemy_pen = sim.planet_intel(0, 2);
    assert!(intel_enemy_pen.is_pen_scanned());
    assert!(intel_enemy_pen.pop_est >= 50.0);
    assert!(intel_enemy_pen.fac_est >= 45.0);
    assert!(intel_enemy_pen.def_est >= 30.0);
}

#[test]
fn scenario_fow_cloak_and_ecm_hide_fleet_from_observation_and_intercept() {
    let races = classic_races();
    let planets = vec![
        // Planet #0: Empire 1 base far away at (50, 100)
        SandboxPlanet {
            x: 50.0,
            y: 100.0,
            owner: 1,
            pop: 50.0,
            factories: 30.0,
            ..Default::default()
        },
        // Planet #1: Empire 2 fortress with active ECM jammer at (300, 100)
        SandboxPlanet {
            x: 300.0,
            y: 100.0,
            owner: 2,
            pop: 80.0,
            factories: 60.0,
            defense: 75.0, // >= 55.0 activates planetary ECM jammer
            has_gate: true,
            ..Default::default()
        },
        // Planet #2: Neutral waypoint at (200, 250) so Empire 1 Cruiser has a flight target
        SandboxPlanet {
            x: 200.0,
            y: 250.0,
            owner: 0,
            ..Default::default()
        },
    ];
    // Empire 1 Cruiser at (200, 100) with power=150; Empire 2 Bomber at (285, 100) (distance 85 ly, inside Planet #1's ECM field)
    let fleets = vec![
        Fleet {
            id: 1,
            x: 200.0,
            y: 100.0,
            from: 0,
            target: 2,
            emp: 1,
            fleet_type: FleetClass::Cruiser,
            power: 150.0,
            max_power: 150.0,
            steer_x: 200.0,
            steer_y: 250.0,
            ..Default::default()
        },
        Fleet {
            id: 2,
            x: 285.0,
            y: 100.0,
            from: 1,
            target: 1,
            emp: 2,
            fleet_type: FleetClass::Bomber,
            power: 40.0,
            max_power: 40.0,
            steer_x: 300.0,
            steer_y: 100.0,
            ..Default::default()
        },
    ];

    let mut sim =
        Simulation::new_sandbox(SimConfig::default(), races, planets, fleets, Vec::new(), false);
    // Guarantee Cruiser interception roll passes when target is detected
    sim.empires[0].race.doctrine.intercept_bias = 1.0;
    sim.empires[0].tech_cache.scanner_range = 100.0;
    sim.empires[0].tech_cache.tachyon_cloak_reduction = 0.0;

    // Give Empire 2 heavy stealth (75% armed cloak) + strong planetary ECM jammer (80% strength, 120 ly radius)
    sim.empires[1].tech_cache.cloak_armed_pct = 0.75;
    sim.empires[1].tech_cache.ecm_jammer_strength = 0.80;
    sim.empires[1].tech_cache.ecm_jammer_range = 120.0;

    sim.step_tick(true);

    // 1. While cloaked and under ECM cover at 85 ly, effective_detection_range is < 20 ly:
    //    Cruiser MUST NOT intercept, and AgentObservation MUST NOT see the enemy Bomber!
    assert!(
        !sim.fleets[0].hunting,
        "Cruiser must not intercept a cloaked/ECM-protected fleet outside effective detection range"
    );
    let obs_hidden = sim.take_agent_observation(0);
    assert!(
        obs_hidden.detected_enemy_fleets.is_empty(),
        "Cloaked/ECM-jammed fleet must not appear in AgentObservation.detected_enemy_fleets"
    );

    // 2. Equip Empire 1 with a Tachyon Detector (85% cloak reduction) and move the enemy Bomber
    //    out of the ECM umbrella to (245, 100) (~45 ly from the Cruiser, well within countered detection range)
    sim.empires[0].tech_cache.tachyon_cloak_reduction = 0.85;
    sim.fleets[1].x = 245.0;
    sim.fleets[1].y = 100.0;

    let obs_revealed = sim.take_agent_observation(0);
    assert_eq!(
        obs_revealed.detected_enemy_fleets.len(),
        1,
        "Fleet should now be detected once Tachyon counters cloak outside ECM"
    );

    sim.step_tick(true);
    assert!(
        sim.fleets[0].hunting,
        "Cruiser should immediately enter hunting/interception mode once target enters effective detection range"
    );
}

#[test]
fn scenario_fow_debris_and_wormholes_hidden_outside_scanners() {
    let races = classic_races();
    let planets = vec![
        // Planet #0: Empire 1 homeworld at (100, 100) with docked freighter & colonizer resources
        SandboxPlanet {
            x: 100.0,
            y: 100.0,
            owner: 1,
            hab: 1.0,
            pop: 120.0,
            mines: 50.0,
            factories: 80.0,
            iron_surf: 400.0,
            bora_surf: 400.0,
            germ_surf: 400.0,
            ..Default::default()
        },
        // Planet #1: Rich neutral world at (750, 750), reachable ONLY via the wormhole
        SandboxPlanet {
            x: 750.0,
            y: 750.0,
            owner: 0,
            hab: 0.95,
            iron_crust: 800.0,
            bora_crust: 800.0,
            germ_crust: 800.0,
            ..Default::default()
        },
    ];
    // Docked freighter at Planet #0 ready for immediate salvage dispatch if debris is spotted
    let fleets = vec![Fleet {
        id: 1,
        x: 100.0,
        y: 100.0,
        from: 0,
        target: 0,
        emp: 1,
        fleet_type: FleetClass::Freight,
        power: 12.0,
        max_power: 200.0,
        orbiting: true,
        ..Default::default()
    }];
    // Rich 300 kT debris field at (260, 100) -> distance 160 ly (inside freight_range=350, outside scanner_range=80)
    let debris = vec![DebrisField {
        x: 260.0,
        y: 100.0,
        iron: 100.0,
        bora: 100.0,
        germ: 100.0,
        ttl: 200.0,
        wrecks: Vec::new(),
    }];

    let mut sim =
        Simulation::new_sandbox(SimConfig::default(), races, planets, fleets, debris, true);
    sim.empires[0].tech_cache.scanner_range = 80.0;
    sim.empires[0].tech_cache.scanner_pen = 40.0;
    sim.empires[0].tech_cache.range_freight = 400.0;
    sim.empires[0].tech_cache.range_col = 400.0;

    // Add a wormhole at (250, 100) <-> (740, 750): entry is 150 ly from Planet #0 (outside 80 ly scanner!)
    sim.wormholes.push(Wormhole {
        x1: 250.0,
        y1: 100.0,
        x2: 740.0,
        y2: 750.0,
    });
    sim.build_planet_neighbors();
    sim.refresh_initial_fow();

    // Run 25 ticks with ai_enabled=true while debris (160 ly) & wormhole (150 ly) are outside 80 ly scanner range
    for _ in 0..25 {
        sim.step_tick(true);
    }

    let obs_hidden = sim.take_agent_observation(0);
    assert!(
        obs_hidden.visible_debris.is_empty(),
        "Debris at 160 ly must be invisible to an 80 ly scanner"
    );
    assert!(
        obs_hidden.wormholes.is_empty(),
        "Wormhole at 150 ly must remain undiscovered to an 80 ly scanner"
    );
    assert!(
        !sim.fleets.iter().any(|fl| fl.salvage_mode == 1),
        "Governor must NOT dispatch salvage freighters to debris outside active scanner range"
    );
    assert!(
        !sim.fleets
            .iter()
            .any(|fl| fl.fleet_type == FleetClass::Col && fl.target == 1),
        "Governor must NOT colonize through an undiscovered wormhole to an unexplored planet"
    );

    // Now upgrade Empire 1's planetary scanner range to 200 ly so both the debris field (160 ly)
    // and wormhole entry (150 ly) enter active scanner coverage!
    sim.empires[0].tech_cache.scanner_range = 200.0;
    let mut dispatched_salvage = false;
    for _ in 0..40 {
        sim.step_tick(true);
        if sim.fleets.iter().any(|fl| fl.salvage_mode == 1) {
            dispatched_salvage = true;
            break;
        }
    }

    let obs_visible = sim.take_agent_observation(0);
    assert_eq!(
        obs_visible.wormholes.len(),
        1,
        "Wormhole at 150 ly must be discovered once scanner_range reaches 200 ly"
    );
    assert_eq!(
        obs_visible.visible_debris.len(),
        1,
        "Debris at 160 ly must appear in visible_debris once scanner_range reaches 200 ly"
    );
    assert!(
        dispatched_salvage,
        "Governor should dispatch a salvage freighter once debris enters active scanner coverage"
    );
}

#[test]
fn scenario_governor_respects_starbase_and_min_force_ratio() {
    let races = classic_races();
    let planets = vec![
        // Planet #0: Empire 1 staging world at (100, 100)
        SandboxPlanet {
            x: 100.0,
            y: 100.0,
            owner: 1,
            hab: 1.0,
            pop: 110.0,
            mines: 10.0,
            factories: 20.0,
            defense: 20.0,
            iron_surf: 15.0,
            bora_surf: 15.0,
            germ_surf: 15.0,
            ..Default::default()
        },
        // Planet #1: Empire 2 heavily fortified world at (160, 100) with Tier 3 Starbase (500 HP)
        SandboxPlanet {
            x: 160.0,
            y: 100.0,
            owner: 2,
            hab: 1.0,
            pop: 45.0,
            mines: 20.0,
            factories: 35.0,
            defense: 40.0,
            starbase_tier: 3,
            starbase_hp: Some(500.0),
            ..Default::default()
        },
    ];
    // Single weak Cruiser in orbit at Planet #0 (power = 90 << required_power ~ 660)
    let fleets = vec![Fleet {
        id: 1,
        x: 100.0,
        y: 100.0,
        from: 0,
        target: 0,
        emp: 1,
        fleet_type: FleetClass::Cruiser,
        power: 90.0,
        max_power: 90.0,
        orbiting: true,
        ship_count: 1,
        ..Default::default()
    }];

    let mut sim =
        Simulation::new_sandbox(SimConfig::default(), races, planets, fleets, Vec::new(), true);
    sim.empires[0].tech_cache.scanner_range = 120.0;
    sim.empires[0].tech_cache.scanner_pen = 120.0;
    sim.empires[0].race.doctrine.min_force_ratio = 1.25;
    sim.empires[0].race.doctrine.colonize_vs_attack_bias = 0.0; // pure attack focus
    // Disable Empire 2's shipyard so it stays a static fortified target
    sim.empires[1].race.doctrine.ship_share = 0.0;
    sim.empires[1].race.doctrine.surplus_ship_share = 0.0;
    sim.refresh_initial_fow();

    // 1. With only 1 Cruiser (90 power) and low minerals against a 500 HP Starbase + 40 def,
    //    the Governor MUST NOT launch a suicidal attack!
    for _ in 0..8 {
        sim.step_tick(true);
        assert!(
            !sim.fleets
                .iter()
                .any(|fl| fl.emp == 1 && !fl.orbiting && fl.target == 1),
            "Governor must not send an under-powered fleet against a Tier 3 Starbase"
        );
    }

    // 2. Now supply Planet #0 with massive shipyard budget and minerals so it can build/reinforce
    //    a strike fleet that exceeds `enemy_def * min_force_ratio`!
    sim.ship_prod_buf[0] = 2500.0;
    sim.min_surf[0] = 2500.0;
    sim.min_surf[1] = 2500.0;
    sim.min_surf[2] = 2500.0;

    let mut launched_power = 0.0f32;
    for _ in 0..10 {
        sim.step_tick(true);
        if let Some(strike) = sim
            .fleets
            .iter()
            .find(|fl| fl.emp == 1 && !fl.orbiting && fl.target == 1)
        {
            launched_power = strike.power;
            break;
        }
    }

    assert!(
        launched_power >= 550.0,
        "Expected Governor to accumulate and launch a heavy strike force (>= 550 power) to crack the Starbase, got {launched_power}"
    );
}

#[test]
fn scenario_governor_sector_directive_and_diplomacy_commands() {
    let races = classic_races();
    // Map is 900x900 (4x4 sectors, each 225x225).
    // Sector 0 is (0..225, 0..225); Sector 1 is (225..450, 0..225).
    let planets = vec![
        // Planet #0: Empire 1 homeworld at (210, 100) right on the border of Sector 0 and Sector 1
        SandboxPlanet {
            x: 210.0,
            y: 100.0,
            owner: 1,
            hab: 1.0,
            pop: 140.0,
            mines: 60.0,
            factories: 110.0,
            iron_surf: 800.0,
            bora_surf: 800.0,
            germ_surf: 800.0,
            ..Default::default()
        },
        // Planet #1: Neutral world in Sector 0 at (150, 100) (distance 60 ly)
        SandboxPlanet {
            x: 150.0,
            y: 100.0,
            owner: 0,
            hab: 0.90,
            ..Default::default()
        },
        // Planet #2: Weak Empire 2 world in Sector 1 at (270, 100) (distance 60 ly)
        SandboxPlanet {
            x: 270.0,
            y: 100.0,
            owner: 2,
            hab: 0.90,
            pop: 15.0,
            factories: 10.0,
            defense: 5.0,
            ..Default::default()
        },
    ];

    let mut sim = Simulation::new_sandbox(
        SimConfig::default(),
        races,
        planets,
        Vec::new(),
        Vec::new(),
        true,
    );
    sim.empires[0].tech_cache.scanner_range = 150.0;
    sim.empires[0].tech_cache.scanner_pen = 150.0;
    sim.empires[1].race.doctrine.ship_share = 0.0;
    sim.empires[1].race.doctrine.surplus_ship_share = 0.0;
    sim.refresh_initial_fow();
    sim.ship_prod_buf[0] = 600.0;

    let sec0 = sector_for_xy(150.0, 100.0, sim.world_w, sim.world_h, 4, 4);
    let sec1 = sector_for_xy(270.0, 100.0, sim.world_w, sim.world_h, 4, 4);
    assert_eq!(sec0, 0);
    assert_eq!(sec1, 1);

    // 1. Submit StrategicCommands:
    //    - Ignore Sector 0 (so Planet #1 in Sector 0 must NOT be colonized)
    //    - Attack Sector 1
    //    - Set Ceasefire with Empire 2 (so Planet #2 must NOT be attacked yet despite Sector 1 Attack)
    //    - Send a diplomatic message from Empire 1 (idx 0) to Empire 2 (idx 1)
    sim.submit_commands(
        0,
        vec![
            StrategicCommand::SetSectorDirective {
                sector_id: sec0,
                mode: SectorMode::Ignore,
            },
            StrategicCommand::SetSectorDirective {
                sector_id: sec1,
                mode: SectorMode::Attack,
            },
            StrategicCommand::SetDiplomacy {
                target_emp: 2,
                stance: DiplomaticStance::Ceasefire,
            },
            StrategicCommand::SendDipMessage {
                to_emp: 2,
                text: "OfferPeace: Hold fire along Sector 1 border.".to_string(),
            },
        ],
    );

    // Verify Empire 2 received the diplomatic message in its AgentObservation inbox
    let obs_emp2 = sim.take_agent_observation(1);
    let emp1_summary = obs_emp2
        .known_empires
        .iter()
        .find(|k| k.emp_id == 1)
        .expect("Empire 2 should know Empire 1 after diplomatic transmission");
    assert_eq!(emp1_summary.stance, DiplomaticStance::Ceasefire);
    assert_eq!(emp1_summary.inbox.len(), 1);
    assert!(emp1_summary.inbox[0].text.contains("OfferPeace"));

    // Step 10 ticks under Ceasefire + Sector 0 Ignore:
    // Neither Planet #1 (in Ignored Sector 0) nor Planet #2 (protected by Ceasefire) may be targeted!
    for _ in 0..10 {
        sim.step_tick(true);
    }
    assert!(
        !sim.fleets.iter().any(|fl| fl.emp == 1 && !fl.orbiting && fl.target == 1),
        "Governor must not colonize Planet #1 in SectorMode::Ignore"
    );
    assert!(
        !sim.fleets.iter().any(|fl| fl.emp == 1 && !fl.orbiting && fl.target == 2),
        "Governor must not attack Planet #2 while DiplomaticStance::Ceasefire is active"
    );

    // 2. Now declare War on Empire 2 via StrategicCommand::SetDiplomacy
    sim.submit_commands(
        0,
        vec![StrategicCommand::SetDiplomacy {
            target_emp: 2,
            stance: DiplomaticStance::War,
        }],
    );

    let mut attacked_p2 = false;
    for _ in 0..10 {
        sim.step_tick(true);
        if sim
            .fleets
            .iter()
            .any(|fl| fl.emp == 1 && !fl.orbiting && fl.target == 2)
            || sim.owner[2] == 1
        {
            attacked_p2 = true;
            break;
        }
    }
    assert!(
        attacked_p2,
        "Governor must launch an attack on Planet #2 in Sector 1 once War is declared"
    );
    assert!(
        !sim.fleets.iter().any(|fl| fl.emp == 1 && !fl.orbiting && fl.target == 1),
        "Sector 0 must still remain ignored"
    );
}

#[test]
fn scenario_agent_observation_compact_and_isolated() {
    let mut cfg = SimConfig::default();
    cfg.galaxy_size = GalaxySize::Huge; // 1,000 planets!
    cfg.planets = 1000;
    let mut sim = Simulation::new(cfg, classic_races());
    sim.init(777123, true);

    for _ in 0..25 {
        sim.step_tick(true);
    }

    let obs = sim.take_agent_observation(0);
    let json = sim.take_agent_observation_json(0);

    // 1. Compactness: even on a 1,000-planet map, JSON payload must be compact (< 10 KB)
    assert!(
        json.len() < 10_000,
        "Expected compact AgentObservation JSON (< 10 KB) on 1,000-planet map, got {} bytes",
        json.len()
    );
    assert!(!obs.sectors.is_empty() && obs.sectors.len() <= 16);
    assert_eq!(
        obs.galaxy_overview.grid_rows * obs.galaxy_overview.grid_cols,
        16
    );

    // 2. Total planet accounting across all sectors must equal 1,000
    let total_sector_planets: u32 = obs
        .sectors
        .iter()
        .map(|s| {
            let enemy_sum: u32 = s.enemy_planets_by_emp.iter().map(|&(_, cnt)| cnt as u32).sum();
            s.own_planets as u32
                + enemy_sum
                + s.neutral_known as u32
                + s.unexplored_stars as u32
        })
        .sum();
    assert_eq!(total_sector_planets, 1000);
    assert_eq!(obs.galaxy_overview.total_planets, 1000);
    assert_eq!(
        obs.galaxy_overview.explored_planets + obs.galaxy_overview.unexplored_planets,
        1000
    );

    // 3. No tournament noise (`kills`, `gates`, `lastFit`, `winRate`) or raw `x`/`y` coordinates in AgentObservation JSON
    for forbidden in ["\"kills\"", "\"gates\"", "\"lastFit\"", "\"winRate\"", "\"x\":", "\"y\":"] {
        assert!(
            !json.contains(forbidden),
            "Forbidden field {forbidden} leaked into AgentObservation JSON!"
        );
    }

    // 4. Strict Fog-of-War isolation & separation of enemyPlanets (never neutral owner == 0)
    for ep in &obs.enemy_planets {
        assert_ne!(ep.owner_emp, 0, "Neutral planet leaked into enemyPlanets!");
        assert_ne!(ep.owner_emp, 1, "Own planet leaked into enemyPlanets!");
        let intel = sim.planet_intel(0, ep.planet_id);
        assert!(
            intel.is_explored(),
            "Unexplored planet #{} leaked into AgentObservation!",
            ep.planet_id
        );
        if !intel.is_pen_scanned() {
            assert!(
                ep.pop_est.is_none() && ep.fac_est.is_none() && ep.def_est.is_none(),
                "Non-pen-scanned planet #{} leaked surface stats into AgentObservation!",
                ep.planet_id
            );
        }
    }

    //    - Every wormhole in `wormholes` must have at least one endpoint discovered by Empire 0
    for wh in &obs.wormholes {
        assert!(
            sim.is_wormhole_discovered(0, wh.wormhole_id),
            "Undiscovered wormhole #{} leaked into AgentObservation!",
            wh.wormhole_id
        );
    }
}

#[test]
fn scenario_wormhole_one_sided_discovery_and_camelcase_partial_policy() {
    use sistars_engine::types::{StrategicCommand, Wormhole};

    let races = classic_races().into_iter().take(2).collect();
    // Place Empire 1 at (100, 100), Empire 2 at (850, 550), and a wormhole from (120, 100) -> (820, 520).
    let planets = vec![
        SandboxPlanet {
            x: 100.0,
            y: 100.0,
            owner: 1,
            pop: 50.0,
            factories: 40.0,
            ..Default::default()
        },
        SandboxPlanet {
            x: 850.0,
            y: 550.0,
            owner: 2,
            pop: 50.0,
            factories: 40.0,
            ..Default::default()
        },
    ];
    let wormholes = vec![Wormhole {
        x1: 120.0,
        y1: 100.0,
        x2: 820.0,
        y2: 520.0,
    }];
    let mut sim =
        Simulation::new_sandbox(SimConfig::default(), races, planets, Vec::new(), Vec::new(), false);
    sim.wormholes = wormholes;
    sim.build_planet_neighbors();
    sim.refresh_initial_fow();

    // 1. Endpoint A is within Empire 1's scanner range, Endpoint B is far away -> toSector MUST be None!
    assert!(sim.is_wormhole_end_a_discovered(0, 0));
    assert!(!sim.is_wormhole_end_b_discovered(0, 0));
    let obs_one_end = sim.take_agent_observation(0);
    assert_eq!(obs_one_end.wormholes.len(), 1);
    assert_eq!(obs_one_end.wormholes[0].to_sector, None);

    // 2. Scan Endpoint B -> now BOTH ends are discovered and toSector MUST be Some(...)!
    sim.scan_area_for_empire(820.0, 520.0, 0, 50.0, 0.0, None);
    assert!(sim.is_wormhole_end_b_discovered(0, 0));
    let obs_both_ends = sim.take_agent_observation(0);
    assert_eq!(obs_both_ends.wormholes.len(), 1);
    assert!(
        obs_both_ends.wormholes[0].to_sector.is_some(),
        "Expected toSector to be populated once endpoint B is discovered"
    );

    // 3. Test 100% camelCase StrategicCommand deserialization + partial SetPolicy merge
    let orig_min_force = sim.empires[0].race.doctrine.min_force_ratio;
    let orig_garrison = sim.empires[0].race.doctrine.min_garrison_count;
    let cmds_json = r#"[
        {"setPolicy": {"colonizeVsAttackBias": 0.25, "techAlloc": {"pro": 0.40}}},
        {"setSectorDirective": {"sectorId": 3, "mode": "Fortify"}},
        {"setDiplomacy": {"targetEmp": 2, "stance": "Ceasefire"}},
        {"sendDipMessage": {"toEmp": 2, "text": "CamelCase test cable"}},
        {"buildHub": {"planetIdx": 0, "kind": "Stargate"}}
    ]"#;
    let parsed_cmds: Vec<StrategicCommand> =
        serde_json::from_str(cmds_json).expect("All camelCase StrategicCommands must deserialize");
    sim.submit_commands(0, parsed_cmds);

    assert!((sim.empires[0].race.doctrine.colonize_vs_attack_bias - 0.25).abs() < 1e-4);
    assert!(
        (sim.empires[0].race.doctrine.min_force_ratio - orig_min_force).abs() < 1e-4,
        "Partial SetPolicy patch must NOT overwrite omitted fields!"
    );
    assert_eq!(sim.empires[0].race.doctrine.min_garrison_count, orig_garrison);
    assert_eq!(sim.empires[0].sector_directives[3], SectorMode::Fortify);
    assert_eq!(sim.relations[0][1], DiplomaticStance::Ceasefire);
    assert!(sim.empires[1]
        .inbox
        .iter()
        .any(|m| m.text == "CamelCase test cable"));
}



#[test]
fn scenario_governor_scouts_unscanned_neutrals_and_expands() {
    let mut cfg = SimConfig::default();
    cfg.galaxy_size = GalaxySize::Small;
    cfg.planets = 120;
    let mut sim = Simulation::new(cfg, classic_races());
    sim.init(777555, true);

    // Force high colonization bias and zero penetrating scanner initially to reproduce the old paralysis trap
    sim.empires[0].race.doctrine.colonize_vs_attack_bias = 1.0;
    sim.empires[0].race.doctrine.col_pop_req = 0.20;
    sim.empires[0].race.doctrine.min_shipyard_pop_ratio = 0.20;
    sim.empires[0].tech_cache.scanner_pen = 0.0;

    for _ in 0..450 {
        sim.step_tick(true);
    }

    let obs = sim.take_agent_observation(0);
    assert!(
        obs.empire.planet_count > 4,
        "Governor must actively scout unscanned neutrals and colonize > 4 worlds (got {})",
        obs.empire.planet_count
    );
    assert!(
        obs.galaxy_overview.explored_planets > obs.empire.planet_count as usize + 4,
        "Expected active exploration well beyond owned planets, got {} explored vs {} owned",
        obs.galaxy_overview.explored_planets,
        obs.empire.planet_count
    );
}

#[test]
fn scenario_probe_wormhole_reveals_far_endpoint() {
    use sistars_engine::types::{StrategicCommand, Wormhole};

    let races = classic_races().into_iter().take(2).collect();
    let planets = vec![
        SandboxPlanet {
            x: 100.0,
            y: 100.0,
            owner: 1,
            pop: 70.0,
            factories: 40.0,
            ship_prod_buf: 300.0,
            iron_surf: 300.0,
            bora_surf: 300.0,
            germ_surf: 300.0,
            ..Default::default()
        },
        SandboxPlanet {
            x: 840.0,
            y: 540.0,
            owner: 0,
            ..Default::default()
        },
        SandboxPlanet {
            x: 880.0,
            y: 560.0,
            owner: 2,
            pop: 50.0,
            factories: 30.0,
            ..Default::default()
        },
    ];
    let wormholes = vec![Wormhole {
        x1: 135.0,
        y1: 100.0,
        x2: 810.0,
        y2: 530.0,
    }];
    let fleets = vec![Fleet {
        id: 1,
        x: 100.0,
        y: 100.0,
        from: 0,
        target: 0,
        emp: 1,
        fleet_type: FleetClass::Cruiser,
        power: 80.0,
        max_power: 80.0,
        ship_count: 2,
        orbiting: true,
        ..Default::default()
    }];

    let mut sim =
        Simulation::new_sandbox(SimConfig::default(), races, planets, fleets, Vec::new(), true);
    sim.wormholes = wormholes;
    sim.build_planet_neighbors();
    sim.refresh_initial_fow();

    // Initially only endpoint A is discovered -> toSector MUST be None
    let obs0 = sim.take_agent_observation(0);
    assert_eq!(obs0.wormholes.len(), 1);
    assert_eq!(obs0.wormholes[0].to_sector, None);

    // Issue ProbeWormhole command -> returns queued feedback
    let fb = sim.submit_commands(0, vec![StrategicCommand::ProbeWormhole { wormhole_id: 0 }]);
    assert_eq!(fb.len(), 1);
    assert_eq!(fb[0].status, "queued");

    // Step ticks until ship transits wormhole and maps endpoint B
    let mut revealed = false;
    for _ in 0..80 {
        sim.step_tick(true);
        let obs = sim.take_agent_observation(0);
        if obs.wormholes.first().and_then(|w| w.to_sector).is_some() {
            revealed = true;
            break;
        }
    }
    assert!(
        revealed,
        "ProbeWormhole must dispatch a ship through the wormhole and reveal toSector"
    );
    assert!(
        sim.planet_intel(0, 1).is_explored(),
        "Planet near wormhole exit must be explored after transit"
    );
}

#[test]
fn scenario_build_hub_deterministic_and_validation_feedback() {
    use sistars_engine::types::{HubKind, StrategicCommand};

    let races = classic_races().into_iter().take(2).collect();
    // Planet 0: low factories (20 < 58) -> Stargate must be REJECTED with explicit reason
    // Planet 1: high factories (70 >= 58), abundant ship_prod_buf & minerals -> Stargate must build DETERMINISTICALLY!
    let planets = vec![
        SandboxPlanet {
            x: 100.0,
            y: 100.0,
            owner: 1,
            pop: 60.0,
            factories: 20.0,
            ship_prod_buf: 400.0,
            iron_surf: 300.0,
            bora_surf: 300.0,
            germ_surf: 300.0,
            ..Default::default()
        },
        SandboxPlanet {
            x: 350.0,
            y: 100.0,
            owner: 1,
            pop: 80.0,
            factories: 70.0,
            ship_prod_buf: 400.0,
            iron_surf: 300.0,
            bora_surf: 300.0,
            germ_surf: 300.0,
            ..Default::default()
        },
        SandboxPlanet {
            x: 800.0,
            y: 500.0,
            owner: 2,
            pop: 50.0,
            factories: 30.0,
            ..Default::default()
        },
    ];

    let mut sim =
        Simulation::new_sandbox(SimConfig::default(), races, planets, Vec::new(), Vec::new(), true);
    sim.empires[0].tech_cache.stargate_max_range = 300.0;
    sim.empires[0].tech_cache.stargate_name = Some("Stargate 100/250".to_string());

    // 1. Planet 0 has only 20 factories -> rejected immediately with numbers in reason
    let fb_reject = sim.submit_commands(
        0,
        vec![StrategicCommand::BuildHub {
            planet_idx: 0,
            kind: HubKind::Stargate,
        }],
    );
    assert_eq!(fb_reject[0].status, "rejected");
    assert!(
        fb_reject[0]
            .reason
            .as_deref()
            .unwrap_or("")
            .contains("factories"),
        "Expected factory requirement in rejection reason, got: {:?}",
        fb_reject[0]
    );

    // 2. Planet 1 meets all requirements -> queued and built deterministically within 2 ticks (zero RNG gate!)
    let fb_ok = sim.submit_commands(
        0,
        vec![StrategicCommand::BuildHub {
            planet_idx: 1,
            kind: HubKind::Stargate,
        }],
    );
    assert_eq!(fb_ok[0].status, "queued");
    for _ in 0..3 {
        sim.step_tick(true);
    }
    assert_eq!(
        sim.has_gate[1], 1,
        "Explicit BuildHub(Stargate) must build deterministically within a few ticks without RNG gating"
    );
}

#[test]
fn scenario_diplomacy_uses_1_based_empire_id_not_index() {
    let mut cfg = SimConfig::default();
    cfg.galaxy_size = GalaxySize::Small;
    cfg.planets = 80;
    let mut sim = Simulation::new(cfg, classic_races());
    sim.init(424201, true);
    assert_eq!(sim.empires.len(), 6);
    assert_eq!(sim.empires[0].race.name, "Humanoids");
    assert_eq!(sim.empires[2].race.name, "Insectoids");
    assert_eq!(sim.empires[3].race.name, "Nucleids");

    // Start with Ceasefire across all pairs so War transition is unambiguous
    for a in 0..6 {
        for b in (a + 1)..6 {
            sim.set_relation(a, b, DiplomaticStance::Ceasefire);
        }
    }

    // Empire 1 (Humanoids, idx 0) declares War on empireId=3 (Insectoids, idx 2)
    let fb = sim.submit_commands(
        0,
        vec![StrategicCommand::SetDiplomacy {
            target_emp: 3,
            stance: DiplomaticStance::War,
        }],
    );
    assert_eq!(fb.len(), 1);
    assert_eq!(fb[0].status, "applied");
    let reason = fb[0].reason.as_deref().unwrap_or("");
    assert!(
        reason.contains("Insectoids") && reason.contains("empireId=3"),
        "Feedback must identify Insectoids (empireId=3), got: {:?}",
        fb[0]
    );
    assert_eq!(
        sim.get_relation(0, 2),
        DiplomaticStance::War,
        "Insectoids (idx 2, empireId=3) must now be at War with Humanoids"
    );
    assert_eq!(
        sim.get_relation(0, 3),
        DiplomaticStance::Ceasefire,
        "Nucleids (idx 3, empireId=4) must remain untouched at Ceasefire"
    );

    // Eliminate Empire 6 (Antethereals, idx 5) by clearing its planets & fleets
    for o in &mut sim.owner {
        if *o == 6 {
            *o = 0;
        }
    }
    sim.fleets.retain(|fl| fl.emp != 6);
    assert!(!sim.is_empire_alive(5));

    // Reject targetEmp=0 (neutral), targetEmp=1 (self), targetEmp=99 (out of bounds), targetEmp=6 (eliminated)
    let fb_invalid = sim.submit_commands(
        0,
        vec![
            StrategicCommand::SetDiplomacy {
                target_emp: 0,
                stance: DiplomaticStance::War,
            },
            StrategicCommand::SetDiplomacy {
                target_emp: 1,
                stance: DiplomaticStance::War,
            },
            StrategicCommand::SetDiplomacy {
                target_emp: 99,
                stance: DiplomaticStance::War,
            },
            StrategicCommand::SetDiplomacy {
                target_emp: 6,
                stance: DiplomaticStance::War,
            },
        ],
    );
    assert_eq!(fb_invalid.len(), 4);
    for item in &fb_invalid {
        assert_eq!(
            item.status, "rejected",
            "Expected rejection for invalid diplomacy target: {:?}",
            item
        );
    }
    assert!(fb_invalid[0].reason.as_deref().unwrap_or("").contains("Unknown empireId 0"));
    assert!(fb_invalid[1].reason.as_deref().unwrap_or("").contains("Cannot target own empire"));
    assert!(fb_invalid[2].reason.as_deref().unwrap_or("").contains("Unknown empireId 99"));
    assert!(fb_invalid[3].reason.as_deref().unwrap_or("").contains("eliminated"));
}

#[test]
fn scenario_send_dip_message_uses_1_based_empire_id() {
    let mut cfg = SimConfig::default();
    cfg.galaxy_size = GalaxySize::Small;
    cfg.planets = 80;
    let mut sim = Simulation::new(cfg, classic_races());
    sim.init(424202, true);

    let msg_text = "Border treaty strictly for Insectoids (empireId=3)";
    let fb = sim.submit_commands(
        0,
        vec![StrategicCommand::SendDipMessage {
            to_emp: 3,
            text: msg_text.to_string(),
        }],
    );
    assert_eq!(fb.len(), 1);
    assert_eq!(fb[0].status, "applied");
    let reason = fb[0].reason.as_deref().unwrap_or("");
    assert!(
        reason.contains("Insectoids") && reason.contains("empireId=3"),
        "Feedback must identify Insectoids (empireId=3), got: {:?}",
        fb[0]
    );

    // Insectoids (idx 2, empireId=3) must receive the message; Nucleids (idx 3, empireId=4) must NOT
    let obs_insectoids = sim.take_agent_observation(2);
    let from_humanoids = obs_insectoids
        .known_empires
        .iter()
        .find(|k| k.emp_id == 1)
        .expect("Insectoids must see Humanoids (empId=1) in knownEmpires");
    assert_eq!(from_humanoids.inbox.len(), 1);
    assert_eq!(from_humanoids.inbox[0].from_emp, 1);
    assert_eq!(from_humanoids.inbox[0].to_emp, 3);
    assert_eq!(from_humanoids.inbox[0].text, msg_text);

    let obs_nucleids = sim.take_agent_observation(3);
    let nucleids_msgs: usize = obs_nucleids
        .known_empires
        .iter()
        .map(|k| k.inbox.len())
        .sum();
    assert_eq!(
        nucleids_msgs, 0,
        "Nucleids (idx 3, empireId=4) must NOT receive a cable addressed to empireId=3"
    );
}

#[test]
fn scenario_early_colonizer_before_cruiser_spam_and_buffer_drain() {
    // Part A: With colonize_vs_attack_bias = 1.0 and col_cost_mult = 1.5, the empire must colonize
    // a 2nd planet quickly without building 7 scout cruisers before its first colonizer.
    let mut cfg = SimConfig::default();
    cfg.galaxy_size = GalaxySize::Small;
    cfg.planets = 120;
    cfg.col_cost_mult = 1.5;
    let mut sim = Simulation::new(cfg, classic_races());
    sim.init(777556, true);

    sim.empires[0].race.doctrine.colonize_vs_attack_bias = 1.0;
    sim.empires[0].race.doctrine.col_pop_req = 0.20;
    sim.empires[0].race.doctrine.min_shipyard_pop_ratio = 0.20;
    sim.empires[0].race.doctrine.ship_share = 0.35;

    let mut first_col_tick: Option<u64> = None;
    let mut cruisers_at_first_col: u32 = 0;
    let mut second_planet_tick: Option<u64> = None;

    for tick in 1..=350 {
        sim.step_tick(true);
        if first_col_tick.is_none()
            && sim.fleets.iter().any(|fl| fl.emp == 1 && fl.fleet_type == FleetClass::Col)
        {
            first_col_tick = Some(tick);
            cruisers_at_first_col = sim
                .fleets
                .iter()
                .filter(|fl| fl.emp == 1 && fl.fleet_type == FleetClass::Cruiser)
                .map(|fl| fl.ship_count as u32)
                .sum();
        }
        let own_planets = sim.owner.iter().filter(|&&o| o == 1).count();
        if second_planet_tick.is_none() && own_planets >= 2 {
            second_planet_tick = Some(tick);
            break;
        }
    }

    assert!(
        first_col_tick.is_some(),
        "Empire with colonizeVsAttackBias=1.0 must build its first colonizer well within 350 ticks"
    );
    assert!(
        cruisers_at_first_col <= 3,
        "Governor must not spam 7 scout cruisers before building first colonizer (saw {cruisers_at_first_col} cruisers at tick {:?})",
        first_col_tick
    );
    assert!(
        second_planet_tick.is_some(),
        "Empire with colonizeVsAttackBias=1.0 must establish a 2nd planet within 350 ticks"
    );

    // Part B: Large accumulated ship_prod_buf with low upkeep_ratio & ample minerals must drain
    // smoothly into ships rather than stagnating forever at thousands of unused points.
    let races = classic_races().into_iter().take(2).collect();
    let planets = vec![
        SandboxPlanet {
            x: 100.0,
            y: 100.0,
            owner: 1,
            pop: 110.0,
            factories: 80.0,
            mines: 40.0,
            ship_prod_buf: 4500.0,
            iron_surf: 5000.0,
            bora_surf: 5000.0,
            germ_surf: 5000.0,
            starbase_tier: 3,
            ..Default::default()
        },
        SandboxPlanet {
            x: 900.0,
            y: 900.0,
            owner: 2,
            pop: 80.0,
            factories: 40.0,
            ..Default::default()
        },
    ];
    let mut sb_sim =
        Simulation::new_sandbox(SimConfig::default(), races, planets, Vec::new(), Vec::new(), true);
    // Peace so no attack launches — pure shipyard buffer drain into orbital task force
    sb_sim.set_relation(0, 1, DiplomaticStance::Ceasefire);
    let initial_buf = sb_sim.ship_prod_buf[0];

    for _ in 0..40 {
        sb_sim.step_tick(true);
    }

    let final_buf = sb_sim.ship_prod_buf[0];
    let total_ships: u32 = sb_sim
        .fleets
        .iter()
        .filter(|fl| fl.emp == 1 && fl.power > 0.0)
        .map(|fl| fl.ship_count as u32)
        .sum();
    assert!(
        final_buf < initial_buf * 0.85 && total_ships >= 6,
        "Large shipProdBuf ({initial_buf:.1}) must drain into ships when upkeep is low and minerals are available (final_buf={final_buf:.1}, total_ships={total_ships})"
    );
}
