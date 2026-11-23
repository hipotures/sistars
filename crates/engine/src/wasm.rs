use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::ai;
use crate::config::{
    BreakthroughMeta, SimConfig, TechFieldMeta, BREAKTHROUGHS, MAX_TECH_LEVEL, NUM_TECH_FIELDS,
    TECH_FIELDS, WORLD_H, WORLD_W,
};
use crate::sim::Simulation;
use crate::tech::{classic_races, tech_store, LRT_LIST, PRT_LIST};
use crate::types::{RaceDef, ScenarioModifiers, StrategicCommand, TechCatalogView};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EngineMetadata {
    world_w: f32,
    world_h: f32,
    max_tech_level: usize,
    tech_fields: [TechFieldMeta; NUM_TECH_FIELDS],
    breakthroughs: [BreakthroughMeta; 6],
}

#[wasm_bindgen]
pub struct WasmEngine {
    sim: Simulation,
}

#[wasm_bindgen]
impl WasmEngine {
    #[wasm_bindgen(constructor)]
    pub fn new(initial_seed: u32) -> Self {
        let mut sim = Simulation::new(SimConfig::default(), classic_races());
        sim.init(initial_seed, false);
        Self { sim }
    }

    pub fn init(&mut self, seed: u32) {
        self.sim.init(seed, false);
    }

    pub fn step(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.sim.step_tick(false);
        }
    }

    pub fn set_profiling(&mut self, enabled: bool) {
        self.sim.profile_enabled = enabled;
    }

    pub fn get_metadata(&self) -> JsValue {
        let meta = EngineMetadata {
            world_w: WORLD_W,
            world_h: WORLD_H,
            max_tech_level: MAX_TECH_LEVEL,
            tech_fields: TECH_FIELDS,
            breakthroughs: BREAKTHROUGHS,
        };
        serde_wasm_bindgen::to_value(&meta).unwrap_or(JsValue::NULL)
    }

    pub fn get_tech_catalog(&self) -> JsValue {
        let store = tech_store();
        let view = TechCatalogView {
            entries: store.entries.clone(),
            prts: PRT_LIST.to_vec(),
            lrts: LRT_LIST.to_vec(),
            classic_races: classic_races(),
        };
        serde_wasm_bindgen::to_value(&view).unwrap_or(JsValue::NULL)
    }

    pub fn set_races(&mut self, races_js: JsValue, seed: u32) -> bool {
        let parsed: Result<Vec<RaceDef>, _> = serde_wasm_bindgen::from_value(races_js);
        match parsed {
            Ok(races) => self.sim.set_races(races, seed),
            Err(_) => false,
        }
    }

    pub fn get_config(&self) -> JsValue {
        serde_wasm_bindgen::to_value(&self.sim.cfg).unwrap_or(JsValue::NULL)
    }

    pub fn update_param(&mut self, key: &str, value: f32, new_seed: u32) -> bool {
        let needs_reset = self.sim.update_param(key, value);
        if needs_reset {
            self.sim.init(new_seed, false);
        }
        needs_reset
    }

    pub fn set_galaxy_options(&mut self, size: &str, shape: &str, new_seed: u32) -> bool {
        if self.sim.set_galaxy_options(size, shape) {
            self.sim.init(new_seed, false);
            true
        } else {
            false
        }
    }

    pub fn set_scenario_modifiers(
        &mut self,
        mods_js: JsValue,
        new_seed: u32,
        reset_map: bool,
    ) -> bool {
        let parsed: Result<ScenarioModifiers, _> = serde_wasm_bindgen::from_value(mods_js);
        match parsed {
            Ok(mods) => {
                self.sim.cfg.scenario_modifiers = mods;
                if reset_map {
                    self.sim.init(new_seed, false);
                }
                true
            }
            Err(_) => false,
        }
    }

    /// Lightweight per-frame HUD snapshot + updates flat render buffers in WASM linear memory.
    pub fn take_hud_snapshot(&mut self) -> JsValue {
        let hud = self.sim.take_hud_snapshot();
        serde_wasm_bindgen::to_value(&hud).unwrap_or(JsValue::NULL)
    }

    /// Zero-copy pointers & lengths into WASM linear memory for flat Float32Array views.
    pub fn planets_render_ptr(&self) -> *const f32 {
        self.sim.planet_render_buf.as_ptr()
    }

    pub fn planets_render_len(&self) -> usize {
        self.sim.planet_render_buf.len()
    }

    pub fn fleets_render_ptr(&self) -> *const f32 {
        self.sim.fleet_render_buf.as_ptr()
    }

    pub fn fleets_render_len(&self) -> usize {
        self.sim.fleet_render_buf.len()
    }

    pub fn debris_render_ptr(&self) -> *const f32 {
        self.sim.debris_render_buf.as_ptr()
    }

    pub fn debris_render_len(&self) -> usize {
        self.sim.debris_render_buf.len()
    }

    /// On-demand detail for a single inspected planet.
    pub fn get_planet_detail(&self, idx: usize) -> JsValue {
        match self.sim.planet_view(idx) {
            Some(pv) => serde_wasm_bindgen::to_value(&pv).unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        }
    }

    /// On-demand lazy deterministic star system generation anchored to planet `idx`.
    pub fn get_star_system(&self, idx: usize) -> JsValue {
        match self.sim.get_star_system(idx) {
            Some(sys) => serde_wasm_bindgen::to_value(&sys).unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        }
    }

    /// On-demand detail for a clicked in-flight fleet by render buffer index or stable fleet ID.
    pub fn get_fleet_by_index(&self, idx: usize) -> JsValue {
        match self.sim.fleets.iter().filter(|fl| !fl.orbiting).nth(idx) {
            Some(fl) => serde_wasm_bindgen::to_value(fl).unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        }
    }

    pub fn get_fleet_by_id(&self, id: u32) -> JsValue {
        match self.sim.fleets.iter().find(|fl| fl.id == id) {
            Some(fl) => serde_wasm_bindgen::to_value(fl).unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        }
    }

    /// On-demand list of fleets currently stationed in orbit around planet `idx`.
    pub fn get_planet_fleets(&self, idx: usize) -> JsValue {
        let list = self.sim.orbiting_fleets_at(idx);
        serde_wasm_bindgen::to_value(&list).unwrap_or(JsValue::NULL)
    }

    /// On-demand detail for a clicked salvage debris field (including inbound salvage fleets).
    pub fn get_debris_detail(&self, idx: usize) -> JsValue {
        match self.sim.debris_detail(idx) {
            Some(detail) => serde_wasm_bindgen::to_value(&detail).unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        }
    }

    /// On-demand full planet/fleet tables (only called when Reports drawer is open).
    pub fn get_reports_snapshot(&self) -> JsValue {
        let rep = self.sim.take_reports_snapshot();
        serde_wasm_bindgen::to_value(&rep).unwrap_or(JsValue::NULL)
    }

    /// On-demand power history (only called when Charts & AI drawer is open).
    pub fn get_power_history(&self) -> JsValue {
        serde_wasm_bindgen::to_value(&self.sim.power_history).unwrap_or(JsValue::NULL)
    }

    pub fn take_snapshot(&mut self) -> JsValue {
        let snap = self.sim.take_snapshot();
        serde_wasm_bindgen::to_value(&snap).unwrap_or(JsValue::NULL)
    }

    pub fn run_tournament(
        &mut self,
        num_games: u32,
        ticks_per_game: u32,
        reset_seed: u32,
    ) -> JsValue {
        let res = ai::run_tournament(&mut self.sim, num_games, ticks_per_game);
        self.sim.init(reset_seed, false);
        serde_wasm_bindgen::to_value(&res).unwrap_or(JsValue::NULL)
    }

    pub fn run_co_evolution(&mut self, evo_seed: u32, reset_seed: u32) -> JsValue {
        let res = ai::run_co_evolution(&mut self.sim, evo_seed);
        self.sim.init(reset_seed, false);
        serde_wasm_bindgen::to_value(&res).unwrap_or(JsValue::NULL)
    }

    pub fn export_state_json(&self) -> String {
        self.sim.export_state_json()
    }

    pub fn set_trace_enabled(&mut self, enabled: bool, prune_valid: bool) {
        self.sim.trace_enabled = enabled;
        self.sim.auditor.prune_valid_events = prune_valid;
    }

    pub fn drain_trace_events(&mut self) -> JsValue {
        let drained = self.sim.drain_events();
        serde_wasm_bindgen::to_value(&drained).unwrap_or(JsValue::NULL)
    }

    pub fn export_anomalies_json(&self) -> String {
        self.sim.export_anomalies_json()
    }

    /// Returns a compact, Fog-of-War-isolated intelligence report for an external AI / LLM agent (`0..num_empires`).
    pub fn take_agent_observation(&self, empire_idx: usize) -> JsValue {
        let obs = self.sim.take_agent_observation(empire_idx);
        serde_wasm_bindgen::to_value(&obs).unwrap_or(JsValue::NULL)
    }

    /// Serializes `AgentObservation` directly to a compact JSON string (~2-4 KB).
    pub fn take_agent_observation_json(&self, empire_idx: usize) -> String {
        self.sim.take_agent_observation_json(empire_idx)
    }

    /// Applies a batch of high-level strategic commands (`StrategicCommand`) from an external AI agent.
    pub fn submit_commands(&mut self, empire_idx: usize, cmds_js: JsValue) -> bool {
        let parsed: Result<Vec<StrategicCommand>, _> = serde_wasm_bindgen::from_value(cmds_js);
        match parsed {
            Ok(cmds) => {
                self.sim.submit_commands(empire_idx, cmds);
                true
            }
            Err(_) => false,
        }
    }
}
