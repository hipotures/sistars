use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sistars_engine::config::{
    BreakthroughMeta, SimConfig, TechFieldMeta, MAX_TECH_LEVEL, NUM_TECH_FIELDS,
};
use sistars_engine::types::{
    AgentObservation, EpochDelta, HudSnapshot, RaceDef, ScenarioModifiers, StrategicCommand,
    StrategyMemory, TechCatalogView, VictoryConfig,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionMode {
    #[default]
    RealtimeAsync,
    LockstepEpoch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchStatus {
    WaitingForAgents,
    Running,
    Finished,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineMetadata {
    pub world_w: f32,
    pub world_h: f32,
    pub max_tech_level: usize,
    pub tech_fields: [TechFieldMeta; NUM_TECH_FIELDS],
    pub breakthroughs: [BreakthroughMeta; 6],
}

impl EngineMetadata {
    pub fn for_world(world_w: f32, world_h: f32) -> Self {
        Self {
            world_w,
            world_h,
            max_tech_level: MAX_TECH_LEVEL,
            tech_fields: sistars_engine::config::TECH_FIELDS,
            breakthroughs: sistars_engine::config::BREAKTHROUGHS,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMatchRequest {
    pub seed: Option<u32>,
    pub sim_config: Option<SimConfig>,
    pub scenario_modifiers: Option<ScenarioModifiers>,
    pub victory_config: Option<VictoryConfig>,
    pub races: Option<Vec<RaceDef>>,
    /// Shorthand galaxy size override ("small" | "medium" | "large" | "huge" | "colossal" | "galactic").
    pub galaxy_size: Option<String>,
    /// Shorthand galaxy shape override ("spiral" | "random" | "ring" | "clusters" | "elliptical").
    pub galaxy_shape: Option<String>,
    /// Number of simulation ticks per macro-epoch (default: 20 = 1.0 game year).
    pub ticks_per_epoch: Option<u32>,
    /// Maximum time (ms) to wait at the macro-epoch barrier before defaulting missing seats to the built-in Governor (default: 20000).
    pub epoch_timeout_ms: Option<u64>,
    /// Empire indices (`0..num_empires`) controlled by external AI agents.
    pub agent_seats: Option<Vec<usize>>,
    /// Enable LiveAuditor invariant checking & anomaly logging to `logs/match_{id}_anomalies.jsonl`.
    pub debug_audit: Option<bool>,
    /// Broadcast a WebSocket render frame every N ticks during an epoch (default: 2).
    pub frame_stride_ticks: Option<u32>,
    /// Optional per-tick delay in milliseconds for human spectator viewing (default: 0).
    pub tick_delay_ms: Option<u64>,
    /// Execution clock mode: `realtimeAsync` (default smooth RTS) or `lockstepEpoch` (turn-based barrier).
    pub execution_mode: Option<ExecutionMode>,
    /// Simulation speed multiplier (`1.0` = 20 ticks/s, range `0.05..=20.0`).
    pub speed_mult: Option<f32>,
    /// Start the match paused at tick 0 until unpaused via `/control` (default: false).
    pub start_paused: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMatchResponse {
    pub match_id: String,
    pub spectator_ws_url: String,
    pub empire_tokens: HashMap<usize, String>,
    pub mcp_urls: HashMap<usize, String>,
    pub status: MatchStatus,
    pub execution_mode: ExecutionMode,
    pub paused: bool,
    pub speed_mult: f32,
    pub epoch: u64,
    pub ticks_per_epoch: u32,
    pub epoch_timeout_ms: u64,
    pub agent_seats: Vec<usize>,
    pub debug_audit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchPublicSummary {
    pub match_id: String,
    pub year: f32,
    pub tick: u64,
    pub epoch: u64,
    pub status: MatchStatus,
    pub execution_mode: ExecutionMode,
    pub paused: bool,
    pub speed_mult: f32,
    pub tick_delay_ms: u64,
    pub winner: Option<usize>,
    pub planet_count: usize,
    pub empire_count: usize,
    pub agent_seats: Vec<usize>,
    pub submitted_seats: Vec<usize>,
    pub waiting_for_seats: Vec<usize>,
    pub ticks_per_epoch: u32,
    pub epoch_timeout_ms: u64,
    pub epoch_deadline_ms: Option<u64>,
    pub debug_audit: bool,
    pub anomalies_count: usize,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchControlRequest {
    pub paused: Option<bool>,
    pub speed_mult: Option<f32>,
    pub execution_mode: Option<ExecutionMode>,
    pub tick_delay_ms: Option<u64>,
    pub frame_stride_ticks: Option<u32>,
    pub step_ticks: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationEnvelope {
    pub match_id: String,
    pub epoch: u64,
    pub tick: u64,
    pub year: f32,
    pub status: MatchStatus,
    pub execution_mode: ExecutionMode,
    pub paused: bool,
    pub speed_mult: f32,
    pub epoch_timeout_ms: u64,
    pub deadline_ms: Option<u64>,
    pub already_submitted: bool,
    pub previous_strategy: Option<StrategyMemory>,
    pub delta: Option<EpochDelta>,
    pub observation: AgentObservation,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitCommandsRequest {
    pub epoch: Option<u64>,
    #[serde(default)]
    pub commands: Vec<StrategicCommand>,
    pub strategy_memory: Option<StrategyMemory>,
    pub reasoning_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitCommandsResponse {
    pub accepted: bool,
    pub match_id: String,
    pub epoch: u64,
    pub empire_idx: usize,
    pub commands_queued: usize,
    pub waiting_for_seats: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArenaFeedKind {
    StrategyUpdate,
    Reasoning,
    DipMessage,
    DiplomacyChange,
    SectorDirective,
    BuildHub,
    PolicyUpdate,
    EpochStatus,
    MatchFinished,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArenaFeedEvent {
    pub id: u64,
    pub epoch: u64,
    pub tick: u64,
    pub year: f32,
    pub empire_idx: Option<usize>,
    pub empire_name: String,
    pub empire_color: String,
    pub kind: ArenaFeedKind,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy: Option<StrategyMemory>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SpectatorWsMessage {
    #[serde(rename_all = "camelCase")]
    Init {
        match_id: String,
        meta: EngineMetadata,
        catalog: TechCatalogView,
        config: SimConfig,
        summary: MatchPublicSummary,
        hud: HudSnapshot,
        planets_buf: Vec<f32>,
        fleets_buf: Vec<f32>,
        debris_buf: Vec<f32>,
        feed_history: Vec<ArenaFeedEvent>,
    },
    #[serde(rename_all = "camelCase")]
    Frame {
        epoch: u64,
        tick: u64,
        status: MatchStatus,
        winner: Option<usize>,
        waiting_for_seats: Vec<usize>,
        deadline_ms: Option<u64>,
        hud: HudSnapshot,
        planets_buf: Vec<f32>,
        fleets_buf: Vec<f32>,
        debris_buf: Vec<f32>,
        tps: f32,
    },
    #[serde(rename_all = "camelCase")]
    Feed {
        event: ArenaFeedEvent,
    },
    #[serde(rename_all = "camelCase")]
    Status {
        summary: MatchPublicSummary,
    },
    #[serde(rename_all = "camelCase")]
    Control {
        paused: bool,
        speed_mult: f32,
        execution_mode: ExecutionMode,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiErrorBody {
    pub error: String,
    pub message: String,
}
