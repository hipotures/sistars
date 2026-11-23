use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderName, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

use crate::match_runner::{
    replay_file_name, speed_to_frame_stride, speed_to_tick_delay_ms, MatchHandle, QueuedDecision,
};
use crate::mcp;
use crate::models::{
    ApiErrorBody, CreateMatchRequest, CreateMatchResponse, ExecutionMode, MatchControlRequest,
    MatchPublicSummary, MatchStatus, ObservationEnvelope, SpectatorWsMessage,
    SubmitCommandsRequest, SubmitCommandsResponse,
};
use crate::openapi::build_openapi_spec;

pub struct AppState {
    pub matches: RwLock<HashMap<String, Arc<MatchHandle>>>,
    /// Hard server-side mapping: `bearer_token -> (match_id, empire_idx)`.
    pub token_index: RwLock<HashMap<String, (String, usize)>>,
    /// Persistent MCP seats configured via `SISTARS_MCP_SEATS`: `secret_token -> seat_name`.
    pub persistent_seat_tokens: RwLock<HashMap<String, String>>,
    /// Active bindings for persistent MCP seats: `seat_name -> (match_id, empire_idx)`.
    pub persistent_seat_bindings: RwLock<HashMap<String, (String, usize)>>,
    /// Anti-Spy FoW Lock: `(seat_name, match_id) -> empire_idx` once a persistent seat has joined a match.
    pub seat_match_history: RwLock<HashMap<(String, String), usize>>,
    pub public_url: Option<String>,
    pub admin_token: Option<String>,
    pub logs_dir: PathBuf,
    /// Where finished-match replays are written (`SISTARS_REPLAYS_DIR`, default `<logs_dir>/replays`).
    pub replays_dir: PathBuf,
}

impl AppState {
    pub fn new(admin_token: Option<String>, logs_dir: PathBuf) -> Arc<Self> {
        let public_url = std::env::var("SISTARS_PUBLIC_URL").ok();
        let mcp_seats_raw = std::env::var("SISTARS_MCP_SEATS").ok();
        Self::new_with_mcp(admin_token, logs_dir, public_url, mcp_seats_raw.as_deref())
    }

    pub fn new_with_mcp(
        admin_token: Option<String>,
        logs_dir: PathBuf,
        public_url: Option<String>,
        mcp_seats_raw: Option<&str>,
    ) -> Arc<Self> {
        let mut seat_tokens = HashMap::new();
        if let Some(raw) = mcp_seats_raw {
            for entry in raw.split(',') {
                let trimmed = entry.trim();
                if let Some((seat, tok)) = trimmed.split_once(':') {
                    let s = seat.trim();
                    let t = tok.trim();
                    if !s.is_empty() && !t.is_empty() {
                        seat_tokens.insert(t.to_string(), s.to_string());
                    }
                }
            }
        }
        let cleaned_public = public_url
            .map(|u| u.trim().trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty());
        Arc::new(Self {
            matches: RwLock::new(HashMap::new()),
            token_index: RwLock::new(HashMap::new()),
            persistent_seat_tokens: RwLock::new(seat_tokens),
            persistent_seat_bindings: RwLock::new(HashMap::new()),
            seat_match_history: RwLock::new(HashMap::new()),
            public_url: cleaned_public,
            admin_token: admin_token.filter(|s| !s.trim().is_empty()),
            replays_dir: std::env::var("SISTARS_REPLAYS_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| logs_dir.join("replays")),
            logs_dir,
        })
    }

    pub async fn shutdown_all(&self) {
        let matches = self.matches.read().await;
        for handle in matches.values() {
            let _ = handle.shutdown_tx.send(true);
        }
    }

    /// Graceful server shutdown: stops every match and waits until its replay is written.
    pub async fn shutdown_all_and_save(&self) {
        let handles: Vec<Arc<MatchHandle>> = {
            let matches = self.matches.read().await;
            matches.values().cloned().collect()
        };
        let waits = handles
            .iter()
            .map(|h| h.stop_and_wait("shutdown", std::time::Duration::from_secs(10)));
        futures_util::future::join_all(waits).await;
    }
}

fn resolve_public_url(state: &AppState, headers: &HeaderMap) -> String {
    if let Some(ref u) = state.public_url {
        return u.clone();
    }
    if let Some(host) = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get("host"))
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|h| !h.is_empty())
    {
        let proto = headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .unwrap_or("http");
        return format!("{proto}://{host}");
    }
    "http://127.0.0.1:8080".to_string()
}

pub fn build_router(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .expose_headers([HeaderName::from_static("mcp-session-id")]);

    Router::new()
        .route("/openapi.json", get(get_openapi))
        .route("/health", get(health_check))
        .route(
            "/mcp",
            post(mcp::handle_mcp_header_post)
                .get(mcp::handle_mcp_header_get)
                .delete(mcp::handle_mcp_header_delete)
                .options(mcp::handle_mcp_options),
        )
        .route(
            "/mcp/",
            post(mcp::handle_mcp_header_post)
                .get(mcp::handle_mcp_header_get)
                .delete(mcp::handle_mcp_header_delete)
                .options(mcp::handle_mcp_options),
        )
        .route(
            "/mcp/{token}",
            post(mcp::handle_mcp_post)
                .get(mcp::handle_mcp_get)
                .delete(mcp::handle_mcp_delete)
                .options(mcp::handle_mcp_options),
        )
        .route(
            "/mcp/{token}/",
            post(mcp::handle_mcp_post)
                .get(mcp::handle_mcp_get)
                .delete(mcp::handle_mcp_delete)
                .options(mcp::handle_mcp_options),
        )
        .route("/api/v1/mcp/seats", get(mcp::list_persistent_seats))
        .route(
            "/api/v1/mcp/seats/{seat_name}/bind",
            post(mcp::bind_persistent_seat),
        )
        .route("/api/v1/matches", get(list_matches).post(create_match))
        .route("/api/v1/matches/{id}", get(get_match).delete(delete_match))
        .route("/api/v1/matches/{id}/control", post(control_match))
        .route("/api/v1/matches/{id}/start", post(start_match))
        .route("/api/v1/matches/{id}/observation", get(get_observation))
        .route("/api/v1/matches/{id}/commands", post(submit_commands))
        .route(
            "/api/v1/matches/{id}/system/{planet_idx}",
            get(get_star_system),
        )
        .route(
            "/api/v1/matches/{id}/planet/{planet_idx}",
            get(get_planet_detail),
        )
        .route(
            "/api/v1/matches/{id}/planet/{planet_idx}/fleets",
            get(get_planet_fleets),
        )
        .route(
            "/api/v1/matches/{id}/fleet/by-id/{fleet_id}",
            get(get_fleet_by_id),
        )
        .route(
            "/api/v1/matches/{id}/fleet/by-index/{idx}",
            get(get_fleet_by_index),
        )
        .route("/api/v1/matches/{id}/debris/{idx}", get(get_debris_detail))
        .route("/api/v1/matches/{id}/reports", get(get_reports))
        .route("/api/v1/matches/{id}/power-history", get(get_power_history))
        .route("/api/v1/matches/{id}/export-state", get(get_export_state))
        .route("/api/v1/matches/{id}/anomalies", get(get_anomalies))
        .route("/api/v1/matches/{id}/replay", get(get_replay))
        .route("/ws/matches/{id}/spectate", get(ws_spectate))
        .layer(cors)
        .with_state(state)
}

fn api_error(
    status: StatusCode,
    error: &str,
    message: impl Into<String>,
) -> (StatusCode, Json<ApiErrorBody>) {
    (
        status,
        Json(ApiErrorBody {
            error: error.to_string(),
            message: message.into(),
        }),
    )
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    let auth = headers.get("authorization")?.to_str().ok()?;
    let trimmed = auth.trim();
    if trimmed.len() > 7 && trimmed[..7].eq_ignore_ascii_case("bearer ") {
        let tok = trimmed[7..].trim();
        if !tok.is_empty() {
            return Some(tok);
        }
    }
    None
}

async fn resolve_agent_seat(
    state: &AppState,
    headers: &HeaderMap,
    match_id: &str,
) -> Result<(usize, String), (StatusCode, Json<ApiErrorBody>)> {
    let token = extract_bearer_token(headers).ok_or_else(|| {
        api_error(
            StatusCode::UNAUTHORIZED,
            "missing_token",
            "Missing or malformed Authorization: Bearer <empire_token> header",
        )
    })?;

    let index = state.token_index.read().await;
    let Some((tok_match_id, empire_idx)) = index.get(token) else {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "Unknown or expired empire token",
        ));
    };

    if tok_match_id != match_id {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "match_mismatch",
            "Token does not belong to this match",
        ));
    }

    let prefix: String = token.chars().take(12).collect();
    Ok((*empire_idx, format!("rest:{prefix}")))
}

/// Enforces `SISTARS_ADMIN_TOKEN` (via `Authorization: Bearer` or `X-Admin-Token`) when configured.
/// With no admin token configured the server runs in open dev mode and every caller is admin.
fn require_admin(
    state: &AppState,
    headers: &HeaderMap,
    action: &str,
) -> Result<(), (StatusCode, Json<ApiErrorBody>)> {
    let Some(ref expected_admin) = state.admin_token else {
        return Ok(());
    };
    let supplied = extract_bearer_token(headers).or_else(|| {
        headers
            .get("x-admin-token")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
    });
    if supplied != Some(expected_admin.as_str()) {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "admin_auth_required",
            format!("Valid SISTARS_ADMIN_TOKEN required to {action}"),
        ));
    }
    Ok(())
}

async fn get_openapi() -> Json<Value> {
    Json(build_openapi_spec())
}

async fn health_check() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "sistars-server" }))
}

async fn create_match(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Option<Json<CreateMatchRequest>>,
) -> Result<(StatusCode, Json<CreateMatchResponse>), (StatusCode, Json<ApiErrorBody>)> {
    require_admin(&state, &headers, "create a match")?;

    let req = payload.map(|Json(r)| r).unwrap_or_default();
    let match_id = Uuid::new_v4().simple().to_string()[..12].to_string();
    let auto_bind = req.auto_bind_seats.unwrap_or(false);
    let (handle, empire_tokens, mut open_agent_seats) = MatchHandle::spawn(
        match_id.clone(),
        req,
        state.logs_dir.clone(),
        state.replays_dir.clone(),
    );

    if auto_bind {
        let mut seat_names: Vec<String> = {
            let seats = state.persistent_seat_tokens.read().await;
            seats.values().cloned().collect()
        };
        seat_names.sort();
        seat_names.dedup();
        let mut bindings = state.persistent_seat_bindings.write().await;
        let mut history = state.seat_match_history.write().await;
        let mut guard = handle.inner.write().await;
        let num_emp = guard.sim.empires.len();
        for seat_name in seat_names {
            if bindings.contains_key(&seat_name) {
                continue;
            }
            let target_idx = guard
                .open_agent_seats
                .iter()
                .copied()
                .find(|idx| !guard.joined_players.contains_key(idx))
                .or_else(|| {
                    if guard.open_agent_seats.is_empty() {
                        (0..num_emp).find(|idx| !guard.joined_players.contains_key(idx))
                    } else {
                        None
                    }
                });
            if let Some(free_idx) = target_idx {
                if !guard.open_agent_seats.contains(&free_idx) {
                    guard.open_agent_seats.push(free_idx);
                    guard.open_agent_seats.sort_unstable();
                }
                let _ = guard.register_player_join(free_idx, format!("mcp:{seat_name}"));
                open_agent_seats = guard.open_agent_seats.clone();
                bindings.insert(seat_name.clone(), (match_id.clone(), free_idx));
                history.insert((seat_name, match_id.clone()), free_idx);
            }
        }
        handle.control_notify.notify_waiters();
    }

    let summary_snap = {
        let guard = handle.inner.read().await;
        guard.public_summary()
    };

    {
        let mut tok_idx = state.token_index.write().await;
        for (&seat, token) in &empire_tokens {
            tok_idx.insert(token.clone(), (match_id.clone(), seat));
        }
    }
    {
        let mut matches = state.matches.write().await;
        matches.insert(match_id.clone(), handle);
    }

    let base_url = resolve_public_url(&state, &headers);
    let mcp_urls: HashMap<usize, String> = empire_tokens
        .iter()
        .map(|(&seat, tok)| (seat, format!("{base_url}/mcp/{tok}")))
        .collect();

    let spectator_ws_url = format!("/ws/matches/{match_id}/spectate");
    Ok((
        StatusCode::CREATED,
        Json(CreateMatchResponse {
            match_id,
            spectator_ws_url,
            empire_tokens,
            mcp_urls,
            status: summary_snap.status,
            execution_mode: summary_snap.execution_mode,
            paused: summary_snap.paused,
            speed_mult: summary_snap.speed_mult,
            epoch: summary_snap.epoch,
            ticks_per_epoch: summary_snap.ticks_per_epoch,
            epoch_timeout_ms: summary_snap.epoch_timeout_ms,
            agent_seats: open_agent_seats.clone(),
            open_agent_seats,
            lobby_timeout_sec: summary_snap.lobby_timeout_sec,
            lobby_remaining_sec: summary_snap.lobby_remaining_sec,
            lobby_deadline_ms: summary_snap.lobby_deadline_ms,
            required_agent_count: summary_snap.required_agent_count,
            joined_agent_count: summary_snap.joined_agent_count,
            start_reason: summary_snap.start_reason,
            debug_audit: summary_snap.debug_audit,
        }),
    ))
}

async fn list_matches(State(state): State<Arc<AppState>>) -> Json<Vec<MatchPublicSummary>> {
    let handles: Vec<Arc<MatchHandle>> = {
        let matches = state.matches.read().await;
        matches.values().cloned().collect()
    };
    let mut list = Vec::with_capacity(handles.len());
    for h in handles {
        let guard = h.inner.read().await;
        list.push(guard.public_summary());
    }
    list.sort_by(|a, b| a.match_id.cmp(&b.match_id));
    Json(list)
}

async fn delete_match(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorBody>)> {
    require_admin(&state, &headers, "delete a match")?;
    let removed = {
        let mut matches = state.matches.write().await;
        matches.remove(&id)
    };
    let Some(handle) = removed else {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "match_not_found",
            format!("Match '{id}' not found"),
        ));
    };
    handle
        .stop_and_wait("deleted", std::time::Duration::from_secs(10))
        .await;
    {
        let mut tok_idx = state.token_index.write().await;
        tok_idx.retain(|_, (mid, _)| mid != &id);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn get_match(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<MatchPublicSummary>, (StatusCode, Json<ApiErrorBody>)> {
    let handle = {
        let matches = state.matches.read().await;
        matches.get(&id).cloned()
    }
    .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "match_not_found", "Match not found"))?;

    let guard = handle.inner.read().await;
    Ok(Json(guard.public_summary()))
}

async fn control_match(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<MatchControlRequest>,
) -> Result<Json<MatchPublicSummary>, (StatusCode, Json<ApiErrorBody>)> {
    require_admin(&state, &headers, "control a match")?;

    let handle = {
        let matches = state.matches.read().await;
        matches.get(&id).cloned()
    }
    .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "match_not_found", "Match not found"))?;

    let mut aborted = false;
    let summary = {
        let mut guard = handle.inner.write().await;
        if let Some(p) = req.paused {
            guard.paused = p;
        }
        if let Some(sm) = req.speed_mult {
            let clamped = sm.clamp(0.05, 20.0);
            guard.speed_mult = clamped;
            guard.tick_delay_ms = speed_to_tick_delay_ms(clamped);
            guard.frame_stride_ticks = speed_to_frame_stride(clamped);
        }
        if let Some(mode) = req.execution_mode {
            guard.execution_mode = mode;
            if mode == ExecutionMode::RealtimeAsync
                && !guard.lobby_active
                && guard.status == MatchStatus::WaitingForAgents
            {
                guard.status = MatchStatus::Running;
                guard.epoch_deadline_ms = None;
            }
        }
        if let Some(d) = req.tick_delay_ms {
            guard.tick_delay_ms = d.min(2_000);
        }
        if let Some(s) = req.frame_stride_ticks {
            guard.frame_stride_ticks = s.clamp(1, 50);
        }
        if let Some(steps) = req.step_ticks {
            guard.pending_step_ticks = guard.pending_step_ticks.saturating_add(steps.min(100));
        }
        let lobby_started = if req.force_start == Some(true) && !guard.status.is_over() {
            guard.transition_lobby_to_running("manualStart")
        } else {
            false
        };
        if req.abort == Some(true) {
            if let Some(ev) = guard.abort() {
                aborted = true;
                let _ = handle.ws_tx.send(SpectatorWsMessage::Feed { event: ev });
            }
        }
        let summary = guard.public_summary();
        if lobby_started {
            let _ = handle.ws_tx.send(guard.build_lobby_ws_message());
        }
        let _ = handle.ws_tx.send(SpectatorWsMessage::Control {
            paused: guard.paused,
            speed_mult: guard.speed_mult,
            execution_mode: guard.execution_mode,
        });
        let _ = handle.ws_tx.send(SpectatorWsMessage::Status {
            summary: summary.clone(),
        });
        summary
    };

    if aborted {
        // Stops the match loop at its next check; `finalize_match` re-asserts the Aborted status.
        let _ = handle.shutdown_tx.send(true);
    }
    handle.control_notify.notify_waiters();
    handle.barrier_notify.notify_waiters();
    Ok(Json(summary))
}

async fn start_match(
    state: State<Arc<AppState>>,
    path: Path<String>,
    headers: HeaderMap,
) -> Result<Json<MatchPublicSummary>, (StatusCode, Json<ApiErrorBody>)> {
    control_match(
        state,
        path,
        headers,
        Json(MatchControlRequest {
            force_start: Some(true),
            paused: Some(false),
            ..Default::default()
        }),
    )
    .await
}

async fn get_observation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ObservationEnvelope>, (StatusCode, Json<ApiErrorBody>)> {
    let (empire_idx, player_id) = resolve_agent_seat(&state, &headers, &id).await?;
    let handle = {
        let matches = state.matches.read().await;
        matches.get(&id).cloned()
    }
    .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "match_not_found", "Match not found"))?;

    let (envelope, should_notify) = {
        let mut guard = handle.inner.write().await;
        let mut notify = false;
        if guard.open_agent_seats.contains(&empire_idx)
            && !guard.joined_players.contains_key(&empire_idx)
        {
            if let Ok(autostarted) = guard.register_player_join(empire_idx, player_id) {
                let _ = handle.ws_tx.send(guard.build_lobby_ws_message());
                let summary = guard.public_summary();
                let _ = handle.ws_tx.send(SpectatorWsMessage::Status { summary });
                if autostarted {
                    notify = true;
                }
            }
        }
        let observation = guard.sim.take_agent_observation(empire_idx);
        let already_submitted = guard.execution_mode == ExecutionMode::LockstepEpoch
            && !guard.lobby_active
            && guard.pending_submissions.contains_key(&empire_idx);
        let previous_strategy = guard.latest_strategy.get(&empire_idx).cloned();
        let delta = guard.compute_epoch_delta(empire_idx, &observation);
        let env = ObservationEnvelope {
            match_id: guard.match_id.clone(),
            epoch: guard.epoch,
            tick: guard.sim.ticks,
            year: sistars_engine::types::round1(guard.sim.year),
            status: guard.status,
            execution_mode: guard.execution_mode,
            paused: guard.paused,
            speed_mult: guard.speed_mult,
            epoch_timeout_ms: guard.epoch_timeout_ms,
            deadline_ms: guard.epoch_deadline_ms,
            lobby_remaining_sec: guard.lobby_remaining_sec(),
            lobby_deadline_ms: guard.lobby_deadline_ms,
            required_agent_count: guard.open_agent_seats.len(),
            joined_agent_count: guard.distinct_joined_count(),
            already_submitted,
            previous_strategy,
            delta,
            observation,
        };
        (env, notify)
    };
    if should_notify {
        handle.control_notify.notify_waiters();
        handle.barrier_notify.notify_waiters();
    }
    Ok(Json(envelope))
}

async fn submit_commands(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<SubmitCommandsRequest>,
) -> Result<Json<SubmitCommandsResponse>, (StatusCode, Json<ApiErrorBody>)> {
    let (empire_idx, player_id) = resolve_agent_seat(&state, &headers, &id).await?;
    let handle = {
        let matches = state.matches.read().await;
        matches.get(&id).cloned()
    }
    .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "match_not_found", "Match not found"))?;

    let (epoch, commands_queued, waiting_for_seats, should_notify, command_feedback) = {
        let mut guard = handle.inner.write().await;
        if guard.status == MatchStatus::Aborted {
            return Err(api_error(
                StatusCode::CONFLICT,
                "match_aborted",
                "Match was aborted by the server admin",
            ));
        }
        if guard.status == MatchStatus::Finished {
            return Err(api_error(
                StatusCode::CONFLICT,
                "match_finished",
                "Match has already finished",
            ));
        }
        let mut lobby_autostarted = false;
        if guard.open_agent_seats.contains(&empire_idx)
            && !guard.joined_players.contains_key(&empire_idx)
        {
            if let Ok(autostarted) = guard.register_player_join(empire_idx, player_id) {
                let _ = handle.ws_tx.send(guard.build_lobby_ws_message());
                let summary = guard.public_summary();
                let _ = handle.ws_tx.send(SpectatorWsMessage::Status { summary });
                lobby_autostarted = autostarted;
            }
        }
        if guard.execution_mode == ExecutionMode::LockstepEpoch {
            if let Some(req_epoch) = req.epoch {
                if req_epoch != guard.epoch {
                    return Err(api_error(
                        StatusCode::CONFLICT,
                        "epoch_mismatch",
                        format!(
                            "Submitted epoch {req_epoch} does not match current epoch {}",
                            guard.epoch
                        ),
                    ));
                }
            }
        }
        let commands_queued = req.commands.len();
        guard.pending_submissions.insert(
            empire_idx,
            QueuedDecision {
                commands: req.commands,
                strategy_memory: req.strategy_memory,
                reasoning_note: req.reasoning_note,
            },
        );
        let mut feedback = Vec::new();
        let mut feed_events = Vec::new();
        if guard.execution_mode == ExecutionMode::RealtimeAsync {
            let (evs, mut fb_map) = crate::match_runner::apply_submissions_locked(&mut guard, true);
            feed_events = evs;
            feedback = fb_map.remove(&empire_idx).unwrap_or_default();
        }
        let waiting = guard.waiting_for_seats();
        let all_done = lobby_autostarted
            || (guard.status == MatchStatus::WaitingForAgents && waiting.is_empty());
        for ev in feed_events {
            let _ = handle.ws_tx.send(SpectatorWsMessage::Feed { event: ev });
        }
        (guard.epoch, commands_queued, waiting, all_done, feedback)
    };

    if should_notify {
        handle.control_notify.notify_waiters();
        handle.barrier_notify.notify_waiters();
    }

    Ok(Json(SubmitCommandsResponse {
        accepted: true,
        match_id: id,
        epoch,
        empire_idx,
        commands_queued,
        waiting_for_seats,
        command_feedback,
    }))
}

async fn get_star_system(
    State(state): State<Arc<AppState>>,
    Path((id, planet_idx)): Path<(String, usize)>,
) -> Result<Json<Value>, (StatusCode, Json<ApiErrorBody>)> {
    let handle = {
        let matches = state.matches.read().await;
        matches.get(&id).cloned()
    }
    .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "match_not_found", "Match not found"))?;

    let guard = handle.inner.read().await;
    let sys = guard.sim.get_star_system(planet_idx).ok_or_else(|| {
        api_error(
            StatusCode::NOT_FOUND,
            "planet_not_found",
            format!("Planet #{planet_idx} not found"),
        )
    })?;
    Ok(Json(serde_json::to_value(sys).unwrap_or(Value::Null)))
}

async fn get_planet_detail(
    State(state): State<Arc<AppState>>,
    Path((id, planet_idx)): Path<(String, usize)>,
) -> Result<Json<Value>, (StatusCode, Json<ApiErrorBody>)> {
    let handle = {
        let matches = state.matches.read().await;
        matches.get(&id).cloned()
    }
    .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "match_not_found", "Match not found"))?;

    let guard = handle.inner.read().await;
    let pv = guard.sim.planet_view(planet_idx).ok_or_else(|| {
        api_error(
            StatusCode::NOT_FOUND,
            "planet_not_found",
            format!("Planet #{planet_idx} not found"),
        )
    })?;
    Ok(Json(serde_json::to_value(pv).unwrap_or(Value::Null)))
}

async fn get_planet_fleets(
    State(state): State<Arc<AppState>>,
    Path((id, planet_idx)): Path<(String, usize)>,
) -> Result<Json<Value>, (StatusCode, Json<ApiErrorBody>)> {
    let handle = {
        let matches = state.matches.read().await;
        matches.get(&id).cloned()
    }
    .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "match_not_found", "Match not found"))?;

    let guard = handle.inner.read().await;
    let list = guard.sim.orbiting_fleets_at(planet_idx);
    Ok(Json(serde_json::to_value(list).unwrap_or(Value::Null)))
}

async fn get_fleet_by_id(
    State(state): State<Arc<AppState>>,
    Path((id, fleet_id)): Path<(String, u32)>,
) -> Result<Json<Value>, (StatusCode, Json<ApiErrorBody>)> {
    let handle = {
        let matches = state.matches.read().await;
        matches.get(&id).cloned()
    }
    .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "match_not_found", "Match not found"))?;

    let guard = handle.inner.read().await;
    let fl = guard.sim.fleets.iter().find(|fl| fl.id == fleet_id);
    Ok(Json(serde_json::to_value(fl).unwrap_or(Value::Null)))
}

async fn get_fleet_by_index(
    State(state): State<Arc<AppState>>,
    Path((id, idx)): Path<(String, usize)>,
) -> Result<Json<Value>, (StatusCode, Json<ApiErrorBody>)> {
    let handle = {
        let matches = state.matches.read().await;
        matches.get(&id).cloned()
    }
    .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "match_not_found", "Match not found"))?;

    let guard = handle.inner.read().await;
    let fl = guard.sim.fleets.iter().filter(|fl| !fl.orbiting).nth(idx);
    Ok(Json(serde_json::to_value(fl).unwrap_or(Value::Null)))
}

async fn get_debris_detail(
    State(state): State<Arc<AppState>>,
    Path((id, idx)): Path<(String, usize)>,
) -> Result<Json<Value>, (StatusCode, Json<ApiErrorBody>)> {
    let handle = {
        let matches = state.matches.read().await;
        matches.get(&id).cloned()
    }
    .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "match_not_found", "Match not found"))?;

    let guard = handle.inner.read().await;
    let deb = guard.sim.debris_detail(idx);
    Ok(Json(serde_json::to_value(deb).unwrap_or(Value::Null)))
}

async fn get_reports(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ApiErrorBody>)> {
    let handle = {
        let matches = state.matches.read().await;
        matches.get(&id).cloned()
    }
    .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "match_not_found", "Match not found"))?;

    let guard = handle.inner.read().await;
    let rep = guard.sim.take_reports_snapshot();
    Ok(Json(serde_json::to_value(rep).unwrap_or(Value::Null)))
}

async fn get_power_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Vec<f32>>>, (StatusCode, Json<ApiErrorBody>)> {
    let handle = {
        let matches = state.matches.read().await;
        matches.get(&id).cloned()
    }
    .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "match_not_found", "Match not found"))?;

    let guard = handle.inner.read().await;
    Ok(Json(guard.sim.power_history.clone()))
}

async fn get_export_state(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<String, (StatusCode, Json<ApiErrorBody>)> {
    let handle = {
        let matches = state.matches.read().await;
        matches.get(&id).cloned()
    }
    .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "match_not_found", "Match not found"))?;

    let guard = handle.inner.read().await;
    Ok(guard.sim.export_state_json())
}

/// Serves the gzip-compressed JSONL replay of an ended match (also after it was deleted).
async fn get_replay(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiErrorBody>)> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_match_id",
            "Invalid match id",
        ));
    }
    let name = replay_file_name(&id);
    let bytes = tokio::fs::read(state.replays_dir.join(&name))
        .await
        .map_err(|_| {
            api_error(
                StatusCode::NOT_FOUND,
                "replay_not_found",
                "No replay for this match yet (it is written when the match ends)",
            )
        })?;
    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/gzip".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        bytes,
    ))
}

async fn get_anomalies(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ApiErrorBody>)> {
    let handle = {
        let matches = state.matches.read().await;
        matches.get(&id).cloned()
    }
    .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "match_not_found", "Match not found"))?;

    let guard = handle.inner.read().await;
    let raw = guard.sim.export_anomalies_json();
    let parsed: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({ "anomalies": [] }));
    Ok(Json(parsed))
}

async fn ws_spectate(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiErrorBody>)> {
    let handle = {
        let matches = state.matches.read().await;
        matches.get(&id).cloned()
    }
    .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "match_not_found", "Match not found"))?;

    Ok(ws.on_upgrade(move |socket| handle_spectator_socket(socket, handle)))
}

async fn handle_spectator_socket(socket: WebSocket, handle: Arc<MatchHandle>) {
    let (mut sender, mut receiver) = socket.split();

    // Subscribe BEFORE building Init snapshot so zero frames/events are missed during handshake
    let mut rx = handle.ws_tx.subscribe();

    let (init_msg, lobby_msg_opt) = {
        let mut guard = handle.inner.write().await;
        let init = guard.build_init_ws_message();
        let lobby = if guard.lobby_active {
            Some(guard.build_lobby_ws_message())
        } else {
            None
        };
        (init, lobby)
    };
    if let Ok(json_str) = serde_json::to_string(&init_msg) {
        if sender.send(Message::Text(json_str.into())).await.is_err() {
            return;
        }
    }
    if let Some(lobby_msg) = lobby_msg_opt {
        if let Ok(json_str) = serde_json::to_string(&lobby_msg) {
            if sender.send(Message::Text(json_str.into())).await.is_err() {
                return;
            }
        }
    }

    let send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if let Ok(json_str) = serde_json::to_string(&msg) {
                        if sender.send(Message::Text(json_str.into())).await.is_err() {
                            break;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Drop stale intermediate frames if spectator fell behind; keep stream alive
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Drain incoming ping/close frames until client disconnects
    while let Some(Ok(msg)) = receiver.next().await {
        if matches!(msg, Message::Close(_)) {
            break;
        }
    }

    send_task.abort();
}
