//! Native Model Context Protocol (MCP — Streamable HTTP) endpoints at:
//! - `/mcp` (header auth: `Authorization: Bearer <token>` or `X-Api-Key: <token>`)
//! - `/mcp/{token}` (capability URL auth)
//!
//! Designed for Claude Custom Connectors:
//! - Never returns `401 Unauthorized` or `WWW-Authenticate` on missing/invalid tokens (always `403 Forbidden`),
//!   so Claude stays in `No sign-in` + `Request headers` / Capability URL mode instead of launching OAuth 2.1.
//! - Supports both per-match empire tokens (`AppState::token_index`) and persistent rebindable lobby seats
//!   (`SISTARS_MCP_SEATS=seat_name:secret_token,...`) with strict Anti-Spy FoW Lock per `(seat_name, match_id)`.
//! - Exposes 6 self-service tools: `list_matches`, `join_match`, `leave_match`, `get_briefing`,
//!   `get_observation`, and `submit_orders` (with structured `commandFeedback`).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::header::{ALLOW, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sistars_engine::types::{round1, StrategicCommand, StrategyMemory};

use crate::match_runner::{apply_submissions_locked, prt_short_summary, QueuedDecision};
use crate::models::{
    ApiErrorBody, ExecutionMode, MatchStatus, McpSeatPublicInfo, SpectatorWsMessage,
};
use crate::routes::AppState;

const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
const DEFAULT_PROTOCOL_VERSION: &str = "2025-03-26";

const MCP_INSTRUCTIONS: &str = "You are the Grand Strategist AI commanding an empire in SI-Stars (4X strategy). \
Start by calling list_matches to see available games and seats, then call join_match (for persistent seats) \
to claim an empire, followed by get_briefing once to learn your racial profile, victory goals, Stargate vs Wormhole rules, \
and policy sliders. Then loop on get_observation and submit_orders (always checking commandFeedback for applied/queued/rejected orders).";

enum McpIdentity {
    MatchToken {
        match_id: String,
        empire_idx: usize,
    },
    PersistentSeat {
        seat_name: String,
        bound: Option<(String, usize)>,
    },
    Invalid,
}

impl McpIdentity {
    fn bound_target(&self) -> Option<(&str, usize)> {
        match self {
            McpIdentity::MatchToken {
                match_id,
                empire_idx,
            } => Some((match_id.as_str(), *empire_idx)),
            McpIdentity::PersistentSeat { bound, .. } => {
                bound.as_ref().map(|(m, e)| (m.as_str(), *e))
            }
            McpIdentity::Invalid => None,
        }
    }
}

fn extract_header_token(headers: &HeaderMap) -> Option<String> {
    if let Some(auth) = headers.get("authorization").and_then(|v| v.to_str().ok()) {
        let trimmed = auth.trim();
        if trimmed.len() > 7 && trimmed[..7].eq_ignore_ascii_case("bearer ") {
            let tok = trimmed[7..].trim();
            if !tok.is_empty() {
                return Some(tok.to_string());
            }
        }
    }
    if let Some(key) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        let tok = key.trim();
        if !tok.is_empty() {
            return Some(tok.to_string());
        }
    }
    None
}

async fn resolve_mcp_token(state: &AppState, token: &str) -> McpIdentity {
    // 1. Direct per-match empire token
    {
        let idx = state.token_index.read().await;
        if let Some((match_id, empire_idx)) = idx.get(token) {
            return McpIdentity::MatchToken {
                match_id: match_id.clone(),
                empire_idx: *empire_idx,
            };
        }
    }

    // 2. Persistent MCP seat token (configured via SISTARS_MCP_SEATS)
    let seat_name = {
        let seats = state.persistent_seat_tokens.read().await;
        seats.get(token).cloned()
    };
    if let Some(seat_name) = seat_name {
        let binding = {
            let bindings = state.persistent_seat_bindings.read().await;
            bindings.get(&seat_name).cloned()
        };
        let active_binding = if let Some((mid, eidx)) = binding {
            let exists = {
                let matches = state.matches.read().await;
                matches.contains_key(&mid)
            };
            if exists {
                Some((mid, eidx))
            } else {
                None
            }
        } else {
            None
        };
        return McpIdentity::PersistentSeat {
            seat_name,
            bound: active_binding,
        };
    }

    McpIdentity::Invalid
}

fn forbidden_json(message: &str) -> Response {
    // CRITICAL: Always 403 Forbidden with NO WWW-Authenticate header (never 401!)
    (
        StatusCode::FORBIDDEN,
        Json(ApiErrorBody {
            error: "forbidden".to_string(),
            message: message.to_string(),
        }),
    )
        .into_response()
}

fn jsonrpc_ok(id: Value, result: Value, session_id: Option<String>) -> Response {
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    });
    let mut resp = (StatusCode::OK, Json(body)).into_response();
    resp.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(sid) = session_id {
        if let Ok(val) = HeaderValue::from_str(&sid) {
            resp.headers_mut()
                .insert(HeaderName::from_static("mcp-session-id"), val);
        }
    }
    resp
}

fn jsonrpc_err(id: Value, code: i64, message: impl Into<String>) -> Response {
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message.into(),
        }
    });
    let mut resp = (StatusCode::OK, Json(body)).into_response();
    resp.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    resp
}

fn tool_text_result(text: String, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    })
}

fn prt_profile(prt: &str) -> &'static str {
    match prt {
        "PRT_JOAT" => "Jack of All Trades (JOAT): Balanced generalist with built-in intrinsic scanners, +20% max population, and no fatal weaknesses. Excels at adaptive macro pivots.",
        "PRT_WM" => "War Monger (WM): +50% combat firepower and faster warships, but slower factory output and weaker planetary defenses. Must snowball through early military aggression.",
        "PRT_HE" => "Hyper-Expansion (HE): Double population growth rate and cheap colonizers, but half max population per planet. Wins by flooding sectors rapidly and out-claiming neutrals.",
        "PRT_IT" => "Interstellar Traveler (IT): Cheaper, longer-range Stargates (requires >= 34 factories, min 95 ly spacing, up to 12 gates) and wormhole stability. Dominates logistics and rapid multi-front force projection.",
        "PRT_IS" => "Inner Strength (IS): Stronger planetary defenses, faster Starbase repair, and superior freighter/colonist resilience. Excels at fortifying chokepoints and wars of attrition.",
        "PRT_SS" => "Super Stealth (SS): Intrinsic 75% cloaking on all ships, superior espionage/ECM, and safe ambush strikes. Excels at scouting behind Fog of War and surgical raids.",
        "PRT_AR" => "Alternate Reality (AR): Orbital Starbase economy (no conventional planetary factories); population and Starbase tiers drive output. Protect orbital Starbases at all costs.",
        "PRT_CA" => "Claim Adjuster (CA): Instant terraforming upon colonization and rapid habitability improvement. Turns marginal worlds into rich core hubs faster than any rival.",
        "PRT_PP" => "Packet Physics (PP): Strong mineral logistics and long-range offensive bombardment synergy. Excels at industrial pressure and breaking fortified star systems.",
        "PRT_SD" => "Space Demolition (SD): Area-denial and defensive fleet efficiency around owned hubs. Punishes overextended enemy offensives and locks down frontier sectors.",
        _ => "Custom interstellar civilization: adapt economy, technology, and fleet doctrine to galaxy conditions.",
    }
}

fn build_tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "list_matches",
                "description": "Lists available SI-Stars Arena matches with public metadata (scenarioName, galaxySize, totalPlanets, numEmpires, executionMode, year, victoryGoal) and empire seats with availability ('free' | 'occupied' | 'yours'). Zero Fog-of-War leaks.",
                "inputSchema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": {
                        "includeFinished": {
                            "type": "boolean",
                            "default": false,
                            "description": "Include already-finished matches in the listing."
                        }
                    },
                    "additionalProperties": false
                }
            },
            {
                "name": "join_match",
                "description": "Claims an available empire seat in a match for a persistent MCP seat (SISTARS_MCP_SEATS). Enforces Anti-Spy FoW Lock: once a seat claims an empire in a match, it cannot switch to a different empire in that same match.",
                "inputSchema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "required": ["matchId", "empireIdx"],
                    "properties": {
                        "matchId": { "type": "string", "description": "Target match ID from list_matches." },
                        "empireIdx": { "type": "integer", "minimum": 0, "description": "0-based empire index (empireIdx) to claim." }
                    },
                    "additionalProperties": false
                }
            },
            {
                "name": "leave_match",
                "description": "Detaches a persistent MCP seat from its current match, returning it to idle status (retains Anti-Spy FoW Lock history for the previous match).",
                "inputSchema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }
            },
            {
                "name": "get_briefing",
                "description": "Returns the complete SI-Stars strategic manual tailored to your bound civilization: racial PRT profile, exact VictoryConfig goals, Stargate vs Wormhole rules & construction requirements, clock execution mode, and full glossary of fields, policy sliders, sector directives, and exploration commands.",
                "inputSchema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }
            },
            {
                "name": "get_observation",
                "description": "Returns your current Fog-of-War-isolated empire state (matchId, epoch, tick, year, status, executionMode, paused, speedMult, alreadySubmitted, previousStrategy, delta, state including ownFleetsSummary, pendingHubOrders, activeExplorationOrders).",
                "inputSchema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }
            },
            {
                "name": "submit_orders",
                "description": "Persists your private strategyMemory and applies high-level strategic commands (camelCase) to your empire's Governor AI. Returns structured commandFeedback ({command, status: applied|queued|rejected, reason}) for every submitted command.",
                "inputSchema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "required": ["strategyMemory"],
                    "properties": {
                        "strategyMemory": {
                            "type": "object",
                            "required": ["phase", "priorities", "rationale", "diplomacyNotes", "pivotTriggers"],
                            "properties": {
                                "phase": { "type": "string" },
                                "priorities": { "type": "array", "items": { "type": "string" } },
                                "rationale": { "type": "string" },
                                "diplomacyNotes": { "type": "string" },
                                "pivotTriggers": { "type": "array", "items": { "type": "string" } }
                            }
                        },
                        "commands": {
                            "type": "array",
                            "default": [],
                            "items": {
                                "type": "object",
                                "oneOf": [
                                    {
                                        "type": "object",
                                        "required": ["setPolicy"],
                                        "properties": { "setPolicy": { "type": "object" } }
                                    },
                                    {
                                        "type": "object",
                                        "required": ["setSectorDirective"],
                                        "properties": {
                                            "setSectorDirective": {
                                                "type": "object",
                                                "required": ["sectorId", "mode"],
                                                "properties": {
                                                    "sectorId": { "type": "integer", "minimum": 0 },
                                                    "mode": { "type": "string", "enum": ["Balanced", "Expand", "Scout", "Fortify", "Attack", "Ignore"] }
                                                }
                                            }
                                        }
                                    },
                                    {
                                        "type": "object",
                                        "required": ["scoutSector"],
                                        "properties": {
                                            "scoutSector": {
                                                "type": "object",
                                                "required": ["sectorId"],
                                                "properties": {
                                                    "sectorId": { "type": "integer", "minimum": 0 }
                                                }
                                            }
                                        }
                                    },
                                    {
                                        "type": "object",
                                        "required": ["probeWormhole"],
                                        "properties": {
                                            "probeWormhole": {
                                                "type": "object",
                                                "required": ["wormholeId"],
                                                "properties": {
                                                    "wormholeId": { "type": "integer", "minimum": 0 }
                                                }
                                            }
                                        }
                                    },
                                    {
                                        "type": "object",
                                        "required": ["setDiplomacy"],
                                        "properties": {
                                            "setDiplomacy": {
                                                "type": "object",
                                                "required": ["targetEmp", "stance"],
                                                "properties": {
                                                    "targetEmp": { "type": "integer", "minimum": 0 },
                                                    "stance": { "type": "string", "enum": ["War", "Neutral", "Ceasefire", "Alliance"] }
                                                }
                                            }
                                        }
                                    },
                                    {
                                        "type": "object",
                                        "required": ["sendDipMessage"],
                                        "properties": {
                                            "sendDipMessage": {
                                                "type": "object",
                                                "required": ["toEmp", "text"],
                                                "properties": {
                                                    "toEmp": { "type": "integer", "minimum": 0 },
                                                    "text": { "type": "string" }
                                                }
                                            }
                                        }
                                    },
                                    {
                                        "type": "object",
                                        "required": ["buildHub"],
                                        "properties": {
                                            "buildHub": {
                                                "type": "object",
                                                "required": ["planetId", "kind"],
                                                "properties": {
                                                    "planetId": { "type": "integer", "minimum": 0 },
                                                    "kind": { "type": "string", "enum": ["Stargate", "Starbase"] }
                                                }
                                            }
                                        }
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        ]
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitOrdersArgs {
    strategy_memory: StrategyMemory,
    #[serde(default)]
    commands: Vec<StrategicCommand>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JoinMatchArgs {
    match_id: String,
    empire_idx: usize,
}

// ============================================================================
// HTTP Handlers: Header-Auth `/mcp` AND Path-Token `/mcp/{token}`
// ============================================================================

pub async fn handle_mcp_header_get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let Some(token) = extract_header_token(&headers) else {
        return forbidden_json("Missing Authorization: Bearer <token> or X-Api-Key header");
    };
    handle_mcp_get_inner(&state, &token).await
}

pub async fn handle_mcp_header_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let Some(token) = extract_header_token(&headers) else {
        return forbidden_json("Missing Authorization: Bearer <token> or X-Api-Key header");
    };
    handle_mcp_delete_inner(&state, &token).await
}

pub async fn handle_mcp_header_post(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Some(token) = extract_header_token(&headers) else {
        return forbidden_json("Missing Authorization: Bearer <token> or X-Api-Key header");
    };
    handle_mcp_post_inner(&state, &token, &headers, body).await
}

pub async fn handle_mcp_get(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Response {
    handle_mcp_get_inner(&state, &token).await
}

pub async fn handle_mcp_delete(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Response {
    handle_mcp_delete_inner(&state, &token).await
}

pub async fn handle_mcp_post(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    handle_mcp_post_inner(&state, &token, &headers, body).await
}

pub async fn handle_mcp_options() -> Response {
    let mut resp = StatusCode::NO_CONTENT.into_response();
    let h = resp.headers_mut();
    h.insert(
        HeaderName::from_static("access-control-allow-origin"),
        HeaderValue::from_static("*"),
    );
    h.insert(
        HeaderName::from_static("access-control-allow-methods"),
        HeaderValue::from_static("POST, DELETE, OPTIONS"),
    );
    h.insert(
        HeaderName::from_static("access-control-allow-headers"),
        HeaderValue::from_static(
            "Authorization, X-Api-Key, Content-Type, Accept, Mcp-Session-Id, MCP-Protocol-Version",
        ),
    );
    h.insert(
        HeaderName::from_static("access-control-expose-headers"),
        HeaderValue::from_static("Mcp-Session-Id"),
    );
    resp
}

async fn handle_mcp_get_inner(state: &AppState, token: &str) -> Response {
    if matches!(resolve_mcp_token(state, token).await, McpIdentity::Invalid) {
        return forbidden_json("Invalid or unknown MCP token");
    }
    let mut resp = StatusCode::METHOD_NOT_ALLOWED.into_response();
    resp.headers_mut()
        .insert(ALLOW, HeaderValue::from_static("POST, DELETE, OPTIONS"));
    resp
}

async fn handle_mcp_delete_inner(state: &AppState, token: &str) -> Response {
    if matches!(resolve_mcp_token(state, token).await, McpIdentity::Invalid) {
        return forbidden_json("Invalid or unknown MCP token");
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn handle_mcp_post_inner(
    state: &AppState,
    token: &str,
    headers: &HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let identity = resolve_mcp_token(state, token).await;
    if matches!(identity, McpIdentity::Invalid) {
        return forbidden_json("Invalid or unknown MCP token");
    }

    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return jsonrpc_err(Value::Null, -32700, format!("Parse error: {e}")),
    };

    let method = match req.get("method").and_then(Value::as_str) {
        Some(m) => m,
        None => {
            let id = req.get("id").cloned().unwrap_or(Value::Null);
            return jsonrpc_err(id, -32600, "Invalid JSON-RPC request: missing method");
        }
    };

    // JSON-RPC notifications have no `id` field -> MUST return 202 Accepted with empty body!
    let Some(id) = req.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };

    let client_session = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    match method {
        "initialize" => {
            let requested_ver = req
                .get("params")
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_PROTOCOL_VERSION);
            let negotiated = if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested_ver) {
                requested_ver
            } else {
                DEFAULT_PROTOCOL_VERSION
            };
            let prefix: String = token.chars().take(12).collect();
            let sid = client_session.unwrap_or_else(|| format!("sistars-{prefix}"));
            jsonrpc_ok(
                id,
                json!({
                    "protocolVersion": negotiated,
                    "capabilities": {
                        "tools": { "listChanged": false }
                    },
                    "serverInfo": {
                        "name": "sistars-arena",
                        "version": "0.1.0"
                    },
                    "instructions": MCP_INSTRUCTIONS
                }),
                Some(sid),
            )
        }
        "ping" => jsonrpc_ok(id, json!({}), client_session),
        "tools/list" => jsonrpc_ok(id, build_tools_list(), client_session),
        "tools/call" => {
            let tool_name = req
                .get("params")
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = req
                .get("params")
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));

            let result = match tool_name {
                "list_matches" => handle_tool_list_matches(state, &identity, args).await,
                "join_match" => handle_tool_join_match(state, &identity, args).await,
                "leave_match" => handle_tool_leave_match(state, &identity).await,
                "get_briefing" => match identity.bound_target() {
                    Some((mid, eidx)) => handle_tool_briefing(state, mid, eidx).await,
                    None => tool_text_result(
                        json!({
                            "status": "idle",
                            "message": "No active match bound to this seat yet.",
                            "hint": "Call list_matches and join_match first."
                        })
                        .to_string(),
                        false,
                    ),
                },
                "get_observation" => match identity.bound_target() {
                    Some((mid, eidx)) => handle_tool_observation(state, mid, eidx).await,
                    None => tool_text_result(
                        json!({
                            "status": "idle",
                            "message": "No active match bound to this seat yet.",
                            "hint": "Call list_matches and join_match first."
                        })
                        .to_string(),
                        false,
                    ),
                },
                "submit_orders" => match identity.bound_target() {
                    Some((mid, eidx)) => handle_tool_submit_orders(state, mid, eidx, args).await,
                    None => tool_text_result(
                        json!({
                            "status": "idle",
                            "message": "No active match bound to this seat yet.",
                            "hint": "Call list_matches and join_match first."
                        })
                        .to_string(),
                        false,
                    ),
                },
                other => tool_text_result(
                    format!("Validation error: unknown tool '{other}'"),
                    true,
                ),
            };
            jsonrpc_ok(id, result, client_session)
        }
        other => jsonrpc_err(id, -32601, format!("Method not found: {other}")),
    }
}

// ============================================================================
// MCP Tool Handlers (6 tools)
// ============================================================================

async fn handle_tool_list_matches(state: &AppState, identity: &McpIdentity, args: Value) -> Value {
    let include_finished = args
        .get("includeFinished")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let (seat_name, current_binding, restrict_match_id) = match identity {
        McpIdentity::PersistentSeat { seat_name, bound } => (
            Some(seat_name.clone()),
            bound.as_ref().map(|(m, e)| json!({ "matchId": m, "empireIdx": e })),
            None,
        ),
        McpIdentity::MatchToken {
            match_id,
            empire_idx,
        } => (
            None,
            Some(json!({ "matchId": match_id, "empireIdx": empire_idx })),
            Some(match_id.clone()),
        ),
        McpIdentity::Invalid => unreachable!(),
    };

    let handles: Vec<Arc<crate::match_runner::MatchHandle>> = {
        let matches = state.matches.read().await;
        if let Some(ref only_mid) = restrict_match_id {
            matches.get(only_mid).cloned().into_iter().collect()
        } else {
            matches.values().cloned().collect()
        }
    };

    let bound_pair = identity.bound_target().map(|(m, e)| (m.to_string(), e));
    let mut out_matches = Vec::new();
    for h in handles {
        let summary = {
            let guard = h.inner.read().await;
            guard.public_summary()
        };
        if !include_finished && summary.status == MatchStatus::Finished {
            continue;
        }
        let seats_json: Vec<Value> = summary
            .seats
            .iter()
            .map(|s| {
                let is_yours = bound_pair
                    .as_ref()
                    .is_some_and(|(bm, be)| bm == &summary.match_id && *be == s.empire_idx);
                let availability = if is_yours {
                    "yours"
                } else if s.occupied {
                    "occupied"
                } else {
                    "free"
                };
                json!({
                    "empireIdx": s.empire_idx,
                    "empireId": s.empire_id,
                    "name": s.name,
                    "prt": s.prt,
                    "prtSummary": s.prt_summary,
                    "alive": s.alive,
                    "status": availability,
                    "controlledBy": s.controlled_by,
                })
            })
            .collect();

        out_matches.push(json!({
            "matchId": summary.match_id,
            "scenarioName": summary.scenario_name,
            "status": summary.status,
            "executionMode": summary.execution_mode,
            "paused": summary.paused,
            "speedMult": summary.speed_mult,
            "year": summary.year,
            "epoch": summary.epoch,
            "galaxySize": summary.galaxy_size,
            "totalPlanets": summary.total_planets,
            "numEmpires": summary.num_empires,
            "victoryGoal": summary.victory_goal,
            "seats": seats_json,
        }));
    }
    out_matches.sort_by(|a, b| {
        a["matchId"]
            .as_str()
            .unwrap_or("")
            .cmp(b["matchId"].as_str().unwrap_or(""))
    });

    let payload = json!({
        "seatName": seat_name,
        "currentBinding": current_binding,
        "matches": out_matches,
    });
    tool_text_result(
        serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()),
        false,
    )
}

async fn handle_tool_join_match(state: &AppState, identity: &McpIdentity, args: Value) -> Value {
    let seat_name = match identity {
        McpIdentity::PersistentSeat { seat_name, .. } => seat_name.clone(),
        McpIdentity::MatchToken { .. } => {
            return tool_text_result(
                "Validation error: join_match is only available for persistent MCP seats (SISTARS_MCP_SEATS). Your token is already bound to a specific empire.".to_string(),
                true,
            );
        }
        McpIdentity::Invalid => unreachable!(),
    };

    let parsed: JoinMatchArgs = match serde_json::from_value(args) {
        Ok(v) => v,
        Err(e) => return tool_text_result(format!("Validation error: {e}"), true),
    };

    // 1. Anti-Spy FoW Lock check: once a seat joined empire X in match M, it can never join empire Y != X in match M!
    {
        let history = state.seat_match_history.read().await;
        if let Some(&prev_emp) = history.get(&(seat_name.clone(), parsed.match_id.clone())) {
            if prev_emp != parsed.empire_idx {
                return tool_text_result(
                    format!(
                        "Anti-Spy FoW Lock: seat '{seat_name}' already commanded empireIdx {prev_emp} in match '{}'. Switching to a different empire (empireIdx {}) in the same match is prohibited to protect Fog of War integrity. Rejoin empireIdx {prev_emp} or join a different match.",
                        parsed.match_id, parsed.empire_idx
                    ),
                    true,
                );
            }
        }
    }

    // 2. Verify match exists, is not finished, empire exists & is alive, and seat is not occupied by another agent
    let handle = {
        let matches = state.matches.read().await;
        matches.get(&parsed.match_id).cloned()
    };
    let Some(handle) = handle else {
        return tool_text_result(
            format!("Validation error: match '{}' not found.", parsed.match_id),
            true,
        );
    };

    // Check if another persistent seat is currently bound to (match_id, empire_idx)
    {
        let bindings = state.persistent_seat_bindings.read().await;
        if let Some((other_seat, _)) = bindings
            .iter()
            .find(|(s, (m, e))| *s != &seat_name && m == &parsed.match_id && *e == parsed.empire_idx)
        {
            return tool_text_result(
                format!(
                    "Seat occupied: empireIdx {} in match '{}' is already controlled by '{other_seat}'.",
                    parsed.empire_idx, parsed.match_id
                ),
                true,
            );
        }
    }

    let (emp_name, emp_prt) = {
        let mut guard = handle.inner.write().await;
        if guard.status == MatchStatus::Finished {
            return tool_text_result(
                format!("Validation error: match '{}' has already finished.", parsed.match_id),
                true,
            );
        }
        if parsed.empire_idx >= guard.sim.empires.len() {
            return tool_text_result(
                format!(
                    "Validation error: empireIdx {} out of range (0..{}).",
                    parsed.empire_idx,
                    guard.sim.empires.len()
                ),
                true,
            );
        }
        let emp_u8 = (parsed.empire_idx + 1) as u8;
        let alive = guard.sim.owner.iter().any(|&o| o == emp_u8)
            || guard.sim.fleets.iter().any(|f| f.emp == emp_u8 && f.power > 0.0);
        if !alive {
            return tool_text_result(
                format!("Validation error: empireIdx {} has been eliminated.", parsed.empire_idx),
                true,
            );
        }
        // Check if already claimed in agent_seats by someone else (e.g. a dedicated empire token seat)
        let already_in_agents = guard.agent_seats.contains(&parsed.empire_idx);
        let previously_ours = {
            let history = state.seat_match_history.read().await;
            history.get(&(seat_name.clone(), parsed.match_id.clone())) == Some(&parsed.empire_idx)
        };
        if already_in_agents && !previously_ours {
            return tool_text_result(
                format!(
                    "Seat occupied: empireIdx {} in match '{}' is already assigned to another agent.",
                    parsed.empire_idx, parsed.match_id
                ),
                true,
            );
        }
        if !already_in_agents {
            guard.agent_seats.push(parsed.empire_idx);
            guard.agent_seats.sort_unstable();
        }
        let emp = &guard.sim.empires[parsed.empire_idx];
        let summary = guard.public_summary();
        let _ = handle.ws_tx.send(SpectatorWsMessage::Status { summary });
        (emp.race.name.clone(), emp.race.prt.clone())
    };

    {
        let mut bindings = state.persistent_seat_bindings.write().await;
        bindings.insert(seat_name.clone(), (parsed.match_id.clone(), parsed.empire_idx));
    }
    {
        let mut history = state.seat_match_history.write().await;
        history.insert((seat_name.clone(), parsed.match_id.clone()), parsed.empire_idx);
    }

    let ack = json!({
        "joined": true,
        "seatName": seat_name,
        "matchId": parsed.match_id,
        "empireIdx": parsed.empire_idx,
        "empireId": parsed.empire_idx + 1,
        "empireName": emp_name,
        "prt": emp_prt,
        "prtSummary": prt_short_summary(&emp_prt),
        "nextStep": "Call get_briefing once, then get_observation and submit_orders."
    });
    tool_text_result(
        serde_json::to_string(&ack).unwrap_or_else(|_| "{}".to_string()),
        false,
    )
}

async fn handle_tool_leave_match(state: &AppState, identity: &McpIdentity) -> Value {
    let seat_name = match identity {
        McpIdentity::PersistentSeat { seat_name, .. } => seat_name.clone(),
        McpIdentity::MatchToken { .. } => {
            return tool_text_result(
                "Validation error: leave_match is only available for persistent MCP seats.".to_string(),
                true,
            );
        }
        McpIdentity::Invalid => unreachable!(),
    };

    let prev = {
        let mut bindings = state.persistent_seat_bindings.write().await;
        bindings.remove(&seat_name)
    };

    // Note: keep entry in `seat_match_history` so Anti-Spy FoW Lock remains active!
    let ack = json!({
        "left": prev.is_some(),
        "seatName": seat_name,
        "status": "idle",
        "previousMatchId": prev.map(|(m, _)| m),
    });
    tool_text_result(
        serde_json::to_string(&ack).unwrap_or_else(|_| "{}".to_string()),
        false,
    )
}

async fn handle_tool_briefing(state: &AppState, match_id: &str, empire_idx: usize) -> Value {
    let handle = {
        let matches = state.matches.read().await;
        matches.get(match_id).cloned()
    };
    let Some(handle) = handle else {
        return tool_text_result(
            json!({"status": "idle", "message": "No active match bound to this seat yet."})
                .to_string(),
            false,
        );
    };

    let guard = handle.inner.read().await;
    let emp = match guard.sim.empires.get(empire_idx) {
        Some(e) => e,
        None => {
            return tool_text_result(
                format!("Validation error: invalid empire index {empire_idx}"),
                true,
            )
        }
    };

    let (min_fac, min_hub_dist, max_gates, gate_res, gate_iron, gate_germ) =
        guard.sim.stargate_requirements(empire_idx);
    let best_sb = emp.tech_cache.best_starbase_tier.max(1);
    let p_dom = round1(guard.sim.victory_cfg.planet_share_threshold * 100.0);
    let r_dom = round1(guard.sim.victory_cfg.res_share_threshold * 100.0);
    let max_yr = guard.sim.victory_cfg.max_year.unwrap_or(0.0);
    let victory_text = if max_yr > 0.0 {
        format!(
            "Control >= {p_dom}% of galaxy planets OR >= {r_dom}% of galaxy resources (or lead at year {}).",
            round1(max_yr)
        )
    } else {
        format!("Control >= {p_dom}% of galaxy planets OR >= {r_dom}% of galaxy resources.")
    };

    let clock_text = match guard.execution_mode {
        ExecutionMode::RealtimeAsync => format!(
            "realtimeAsync (smooth RTS at {:.2}x speed, paused={}): simulation runs continuously; submit_orders applies immediately without blocking the clock.",
            guard.speed_mult, guard.paused
        ),
        ExecutionMode::LockstepEpoch => format!(
            "lockstepEpoch (turn-based barrier, {} ticks = 1.0 yr, timeout={}ms where 0=infinite): simulation waits at each epoch boundary for all external agent seats.",
            guard.ticks_per_epoch, guard.epoch_timeout_ms
        ),
    };

    let briefing = format!(
        "=== SI-STARS GRAND STRATEGIST BRIEFING ===\n\
        Match ID: {match_id} | Scenario: {}\n\
        Empire: {} (empireId={}, Seat #{empire_idx})\n\
        Primary Racial Trait ({}): {}\n\
        Victory Conditions: {victory_text}\n\
        Clock Mode: {clock_text}\n\n\
        === STARGATE vs WORMHOLE (CRITICAL DISTINCTION) ===\n\
        - STARGATE (artificial hub built via buildHub): allows instant fleet teleportation ONLY between TWO of your OWN Stargates. Building a Stargate does NOT discover or open Wormholes.\n\
          Your Stargate requirements ({}): planet must be owned, >= {min_fac:.0} factories, >= {min_hub_dist:.0} ly from any existing own Stargate, empire limit {max_gates} gates, cost ~{gate_res:.0} shipProdBuf + {gate_iron:.0} iron + {gate_germ:.0} germ.\n\
        - STARBASE (orbital defense & shipyard via buildHub): upgrades planet up to your max unlocked Starbase tier (currently T{best_sb}).\n\
        - WORMHOLE (natural spacetime tunnel in observation.wormholes): its far endpoint `toSector` stays `null` until your ship physically transits through it (or scans the far side). Use `probeWormhole` (or `scoutSector` / `Scout` mode) to send a ship through and reveal `toSector`.\n\n\
        === SECTOR DIRECTIVES & EXPLORATION COMMANDS ===\n\
        - Default sector mode is `Balanced` (standard baseline). Setting `Expand` or `Scout` on specific sectors concentrates Governor effort there.\n\
        - Sector modes (`setSectorDirective`): `Balanced` (default), `Expand` (high colonization/development priority), `Scout` (high priority for pen-scanning neutrals, exploring dark stars & probing wormholes), `Fortify` (defense/garrison), `Attack` (offensive push), `Ignore`.\n\
        - Explicit exploration commands: `scoutSector {{sectorId}}` (priority scout mission into a sector) and `probeWormhole {{wormholeId}}` (priority ship transit through a discovered wormhole to reveal `toSector`).\n\n\
        === OBSERVATION & POLICY GLOSSARY ===\n\
        - galaxyOverview: totalPlanets, gridRows x gridCols, explored/unexplored counts, victoryGoal.\n\
        - empire: resPerYear, minerals {{iron, bora, germ}}, upkeepRatio, planetCount, fleetCount, techLevels {{ene, wep, pro, con, ele, bio}}, policy.\n\
        - ownHubs: top 10 own worlds with planetId, sectorId, pop, maxPop, factories, defense, starbaseTier, hasGate, shipProdBuf, minerals.\n\
        - ownFleetsSummary: total, orbiting, inTransit, byClass, bySector (with activeMissionSummary).\n\
        - pendingHubOrders & activeExplorationOrders: live queues so you never duplicate orders.\n\
        - enemyPlanets: strictly foreign worlds (never neutrals) with intelAgeYears & penScanned (true reveals popEst/facEst/defEst).\n\
        - Policy Sliders (`setPolicy` accepts PARTIAL camelCase patches — omit unchanged fields):\n\
          * facShare, shipShare, techShare, mineShare: planetary budget shares (normalized to 1.0).\n\
          * colonizeVsAttackBias: 1.0 = max colonization priority, 0.0 = max military war priority.\n\
          * minForceRatio: required fleet advantage over known defense+Starbase before attacking (e.g. 1.25).\n\
          * cruiserShare, bshipShare, bomberShare: warship mix (Cruiser counters Bomber & intercepts; Battleship counters Cruiser; Bomber destroys planetary bunkers).\n\
          * techAlloc {{ene, wep, pro, con, ele, bio}} & mineralWeights {{iron, bora, germ}}.\n\
          * maxUpkeepRatio: above this threshold Governor pauses shipbuilding and scraps obsolete ships.\n\
        - Diplomacy (`setDiplomacy` / `sendDipMessage`): stances War, Neutral, Ceasefire, Alliance.\n\
        - Always inspect `commandFeedback` returned by `submit_orders` to confirm whether each command was `applied`, `queued`, or `rejected` (with exact reason).",
        guard.scenario_name,
        emp.race.name,
        empire_idx + 1,
        emp.race.prt,
        prt_profile(&emp.race.prt),
        emp.race.prt,
    );

    tool_text_result(briefing, false)
}

async fn handle_tool_observation(state: &AppState, match_id: &str, empire_idx: usize) -> Value {
    let handle = {
        let matches = state.matches.read().await;
        matches.get(match_id).cloned()
    };
    let Some(handle) = handle else {
        return tool_text_result(
            json!({"status": "idle", "message": "No active match bound to this seat yet."})
                .to_string(),
            false,
        );
    };

    let guard = handle.inner.read().await;
    let obs = guard.sim.take_agent_observation(empire_idx);
    let already_submitted = guard.execution_mode == ExecutionMode::LockstepEpoch
        && guard.pending_submissions.contains_key(&empire_idx);
    let previous_strategy = guard.latest_strategy.get(&empire_idx).cloned();
    let delta = guard.compute_epoch_delta(empire_idx, &obs);

    let payload = json!({
        "matchId": guard.match_id,
        "epoch": guard.epoch,
        "tick": guard.sim.ticks,
        "year": round1(guard.sim.year),
        "status": guard.status,
        "executionMode": guard.execution_mode,
        "paused": guard.paused,
        "speedMult": guard.speed_mult,
        "alreadySubmitted": already_submitted,
        "previousStrategy": previous_strategy,
        "delta": delta,
        "state": obs,
    });

    tool_text_result(
        serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()),
        false,
    )
}

async fn handle_tool_submit_orders(
    state: &AppState,
    match_id: &str,
    empire_idx: usize,
    args: Value,
) -> Value {
    let parsed: SubmitOrdersArgs = match serde_json::from_value(args) {
        Ok(v) => v,
        Err(e) => {
            return tool_text_result(format!("Validation error: {e}"), true);
        }
    };

    let handle = {
        let matches = state.matches.read().await;
        matches.get(match_id).cloned()
    };
    let Some(handle) = handle else {
        return tool_text_result(
            json!({"status": "idle", "message": "No active match bound to this seat yet."})
                .to_string(),
            false,
        );
    };

    let applied_commands = parsed.commands.len();
    let (resp_json, feed_events, should_notify_barrier) = {
        let mut guard = handle.inner.write().await;
        if guard.status == MatchStatus::Finished {
            return tool_text_result(
                "Validation error: match has already finished.".to_string(),
                true,
            );
        }

        guard.pending_submissions.insert(
            empire_idx,
            QueuedDecision {
                commands: parsed.commands,
                strategy_memory: Some(parsed.strategy_memory),
                reasoning_note: None,
            },
        );

        let mut feed_events = Vec::new();
        let mut command_feedback = Vec::new();
        let mut notify_barrier = false;

        if guard.execution_mode == ExecutionMode::RealtimeAsync {
            let (evs, mut fb_map) = apply_submissions_locked(&mut guard, true);
            feed_events = evs;
            command_feedback = fb_map.remove(&empire_idx).unwrap_or_default();
        } else {
            // In LockstepEpoch, validate & preview feedback immediately if desired, or let barrier apply.
            // Applying immediately on single-seat or queuing for barrier:
            let waiting = guard.waiting_for_seats();
            notify_barrier = guard.status == MatchStatus::WaitingForAgents && waiting.is_empty();
        }

        let resp = json!({
            "accepted": true,
            "empireId": empire_idx + 1,
            "epoch": guard.epoch,
            "tick": guard.sim.ticks,
            "year": round1(guard.sim.year),
            "appliedCommands": applied_commands,
            "executionMode": guard.execution_mode,
            "winner": guard.sim.winner.map(|w| w + 1),
            "commandFeedback": command_feedback,
        });
        (resp, feed_events, notify_barrier)
    };

    for ev in feed_events {
        let _ = handle.ws_tx.send(SpectatorWsMessage::Feed { event: ev });
    }
    if should_notify_barrier {
        handle.barrier_notify.notify_waiters();
    }

    tool_text_result(
        serde_json::to_string(&resp_json).unwrap_or_else(|_| "{}".to_string()),
        false,
    )
}

// ============================================================================
// REST Seat Management Endpoints (Zero Secrets Exposed!)
// ============================================================================

pub async fn list_persistent_seats(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<McpSeatPublicInfo>> {
    let mut seat_names: Vec<String> = {
        let seats = state.persistent_seat_tokens.read().await;
        seats.values().cloned().collect()
    };
    seat_names.sort();
    seat_names.dedup();

    let bindings = state.persistent_seat_bindings.read().await.clone();
    let matches = state.matches.read().await.clone();

    let mut out = Vec::with_capacity(seat_names.len());
    for seat_name in seat_names {
        if let Some((mid, eidx)) = bindings.get(&seat_name) {
            if let Some(handle) = matches.get(mid) {
                let emp_name = {
                    let guard = handle.inner.read().await;
                    guard
                        .sim
                        .empires
                        .get(*eidx)
                        .map(|e| e.race.name.clone())
                };
                out.push(McpSeatPublicInfo {
                    seat_name,
                    status: "bound".to_string(),
                    match_id: Some(mid.clone()),
                    empire_idx: Some(*eidx),
                    empire_name: emp_name,
                });
                continue;
            }
        }
        out.push(McpSeatPublicInfo {
            seat_name,
            status: "idle".to_string(),
            match_id: None,
            empire_idx: None,
            empire_name: None,
        });
    }
    Json(out)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindSeatRequest {
    pub match_id: Option<String>,
    pub empire_idx: Option<usize>,
}

pub async fn bind_persistent_seat(
    State(state): State<Arc<AppState>>,
    Path(seat_name): Path<String>,
    headers: HeaderMap,
    Json(req): Json<BindSeatRequest>,
) -> Result<Json<Value>, (StatusCode, Json<ApiErrorBody>)> {
    if let Some(ref expected_admin) = state.admin_token {
        let supplied = extract_header_token(&headers).or_else(|| {
            headers
                .get("x-admin-token")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
        });
        if supplied.as_deref() != Some(expected_admin.as_str()) {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorBody {
                    error: "admin_auth_required".to_string(),
                    message: "Valid SISTARS_ADMIN_TOKEN required to bind MCP seat".to_string(),
                }),
            ));
        }
    }

    let seat_exists = {
        let seats = state.persistent_seat_tokens.read().await;
        seats.values().any(|s| s == &seat_name)
    };
    if !seat_exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorBody {
                error: "seat_not_found".to_string(),
                message: format!("Persistent MCP seat '{seat_name}' is not configured"),
            }),
        ));
    }

    // matchId: null -> unbind seat to idle
    let Some(match_id) = req.match_id.filter(|m| !m.trim().is_empty()) else {
        {
            let mut bindings = state.persistent_seat_bindings.write().await;
            bindings.remove(&seat_name);
        }
        return Ok(Json(json!({
            "bound": false,
            "status": "idle",
            "seat": seat_name,
        })));
    };

    let empire_idx = req.empire_idx.unwrap_or(0);
    let handle = {
        let matches = state.matches.read().await;
        matches.get(&match_id).cloned()
    };
    let Some(handle) = handle else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorBody {
                error: "match_not_found".to_string(),
                message: format!("Match '{match_id}' not found"),
            }),
        ));
    };

    {
        let mut guard = handle.inner.write().await;
        let empire_count = guard.sim.empires.len();
        if empire_idx >= empire_count {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorBody {
                    error: "invalid_empire_idx".to_string(),
                    message: format!("empireIdx {empire_idx} out of range (0..{empire_count})"),
                }),
            ));
        }
        if !guard.agent_seats.contains(&empire_idx) {
            guard.agent_seats.push(empire_idx);
            guard.agent_seats.sort_unstable();
        }
        let summary = guard.public_summary();
        let _ = handle.ws_tx.send(SpectatorWsMessage::Status { summary });
    }

    {
        let mut bindings = state.persistent_seat_bindings.write().await;
        bindings.insert(seat_name.clone(), (match_id.clone(), empire_idx));
    }
    {
        let mut history = state.seat_match_history.write().await;
        history.insert((seat_name.clone(), match_id.clone()), empire_idx);
    }

    Ok(Json(json!({
        "bound": true,
        "status": "bound",
        "seat": seat_name,
        "matchId": match_id,
        "empireIdx": empire_idx,
    })))
}
