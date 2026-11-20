pub mod ai;
pub mod config;
pub mod galaxy;
pub mod sim;
pub mod system_gen;
pub mod tech;
pub mod types;

#[cfg(target_arch = "wasm32")]
pub mod wasm;
