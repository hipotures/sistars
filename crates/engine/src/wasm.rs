use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::ai;
use crate::config::{
    BreakthroughMeta, SimConfig, TechFieldMeta, BREAKTHROUGHS, NUM_TECH_FIELDS, TECH_FIELDS,
    WORLD_H, WORLD_W,
};
use crate::sim::Simulation;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EngineMetadata {
    world_w: f32,
    world_h: f32,
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
        let mut sim = Simulation::new(SimConfig::default());
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

    pub fn get_metadata(&self) -> JsValue {
        let meta = EngineMetadata {
            world_w: WORLD_W,
            world_h: WORLD_H,
            tech_fields: TECH_FIELDS,
            breakthroughs: BREAKTHROUGHS,
        };
        serde_wasm_bindgen::to_value(&meta).unwrap_or(JsValue::NULL)
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

    pub fn take_snapshot(&mut self) -> JsValue {
        let snap = self.sim.take_snapshot();
        serde_wasm_bindgen::to_value(&snap).unwrap_or(JsValue::NULL)
    }

    pub fn run_tournament(&mut self, num_games: u32, ticks_per_game: u32, reset_seed: u32) -> JsValue {
        let res = ai::run_tournament(&mut self.sim, num_games, ticks_per_game);
        self.sim.init(reset_seed, false);
        serde_wasm_bindgen::to_value(&res).unwrap_or(JsValue::NULL)
    }

    pub fn run_co_evolution(&mut self, evo_seed: u32, reset_seed: u32) -> JsValue {
        let res = ai::run_co_evolution(&mut self.sim, evo_seed);
        self.sim.init(reset_seed, false);
        serde_wasm_bindgen::to_value(&res).unwrap_or(JsValue::NULL)
    }

    pub fn run_auto_balance(&mut self, reset_seed: u32) -> JsValue {
        let res = ai::run_auto_balance(&mut self.sim);
        self.sim.init(reset_seed, false);
        serde_wasm_bindgen::to_value(&res).unwrap_or(JsValue::NULL)
    }

    pub fn export_state_json(&self) -> String {
        self.sim.export_state_json()
    }
}
