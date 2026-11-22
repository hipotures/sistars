#![recursion_limit = "256"]

pub mod match_runner;
pub mod mcp;
pub mod models;
pub mod openapi;
pub mod routes;

pub use models::{
    ArenaFeedEvent, ArenaFeedKind, CreateMatchRequest, CreateMatchResponse, MatchPublicSummary,
    MatchStatus, ObservationEnvelope, SpectatorWsMessage, SubmitCommandsRequest,
    SubmitCommandsResponse,
};
pub use routes::{build_router, AppState};
