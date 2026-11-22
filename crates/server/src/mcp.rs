//! Native Model Context Protocol (MCP — Streamable HTTP) endpoint at `/mcp/{token}`.
//!
//! Designed for zero-friction compatibility with Claude Custom Connectors (Capability URLs):
//! - Never returns `401 Unauthorized` or `WWW-Authenticate` on invalid tokens (always `403 Forbidden`),
//!   preventing Claude from triggering an OAuth 2.1 discovery handshake.
//! - Supports both per-match empire tokens (`AppState::token_index`) and optional rebindable
//!   persistent seats configured via `SISTARS_MCP_SEATS=seat_name:secret_token,...`.
//! - Single JSON-RPC 2.0 `application/json` responses (no SSE chunking) so reverse proxies
//!   (Traefik / Pangolin / Cloudflare) forward cleanly without buffering issues.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::header::{ALLOW, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sistars_engine::types::{round1, StrategicCommand, StrategyMemory};

use crate::match_runner::{apply_submissions_locked, QueuedDecision};
use crate::models::{
    ApiErrorBody, ExecutionMode, MatchStatus, SpectatorWsMessage,
};
use crate::routes::AppState;

const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
const DEFAULT_PROTOCOL_VERSION: &str = "2025-03-26";

const MCP_INSTRUCTIONS: &str = "You are the Grand Strategist AI commanding an empire in SI-Stars (4X strategy). Call get_briefing once at the start of a session to learn your racial profile, victory goals, and policy sliders. Then call get_observation to inspect your Fog-of-War state and submit_orders to persist your strategyMemory and issue strategic commands.";

enum McpSeatResolution {
    Bound {
        match_id: String,
        empire_idx: usize,
    },
    Idle {
        seat_name: String,
    },
    Invalid,
}

async fn resolve_mcp_token(state: &AppState, token: &str) -> McpSeatResolution {
    // 1. Direct per-match empire token (same 48-hex token as Authorization: Bearer)
    {
        let idx = state.token_index.read().await;
        if let Some((match_id, empire_idx)) = idx.get(token) {
            return McpSeatResolution::Bound {
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
        if let Some((match_id, empire_idx)) = binding {
            let exists = {
                let matches = state.matches.read().await;
                matches.contains_key(&match_id)
            };
            if exists {
                return McpSeatResolution::Bound {
                    match_id,
                    empire_idx,
                };
            }
        }
        return McpSeatResolution::Idle { seat_name };
    }

    McpSeatResolution::Invalid
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
    resp.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
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
    resp.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
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
        "PRT_IT" => "Interstellar Traveler (IT): Cheaper, longer-range Stargates with safe mass transit and wormhole stability. Dominates logistics and rapid multi-front force projection.",
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
                "name": "get_briefing",
                "description": "Returns the complete SI-Stars strategic manual tailored to your civilization: racial PRT profile, exact VictoryConfig goals, clock execution mode (realtimeAsync vs lockstepEpoch), and full glossary of observation fields, policy sliders, sector directives, and diplomacy.",
                "inputSchema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }
            },
            {
                "name": "get_observation",
                "description": "Returns your current Fog-of-War-isolated empire state (matchId, epoch, tick, year, status, executionMode, paused, speedMult, alreadySubmitted, previousStrategy, delta, state). Strictly isolated to your empire's token.",
                "inputSchema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }
            },
            {
                "name": "submit_orders",
                "description": "Persists your private strategyMemory for future turns and applies high-level strategic commands (camelCase) to your empire's Governor AI.",
                "inputSchema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "required": ["strategyMemory"],
                    "properties": {
                        "strategyMemory": {
                            "type": "object",
                            "description": "Private strategic continuity memory persisted server-side and returned on your next get_observation as previousStrategy.",
                            "required": ["phase", "priorities", "rationale", "diplomacyNotes", "pivotTriggers"],
                            "properties": {
                                "phase": { "type": "string", "description": "Short phase label, e.g. 'Early Expansion', 'Fortify Frontier', 'Total War'." },
                                "priorities": { "type": "array", "items": { "type": "string" }, "description": "1-4 concise operational priorities." },
                                "rationale": { "type": "string", "description": "1-3 sentences explaining WHY you chose this direction." },
                                "diplomacyNotes": { "type": "string", "description": "Concise notes on neighbor relations and intentions." },
                                "pivotTriggers": { "type": "array", "items": { "type": "string" }, "description": "Concrete measurable conditions to pivot strategy in future epochs." }
                            }
                        },
                        "commands": {
                            "type": "array",
                            "default": [],
                            "description": "Optional list of StrategicCommand objects (camelCase). Omit or pass [] to keep current Governor policy.",
                            "items": {
                                "type": "object",
                                "oneOf": [
                                    {
                                        "type": "object",
                                        "required": ["setPolicy"],
                                        "properties": {
                                            "setPolicy": {
                                                "type": "object",
                                                "description": "Partial merge patch for active empire policy sliders (omitted fields stay unchanged).",
                                                "properties": {
                                                    "facShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                                                    "shipShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                                                    "techShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                                                    "mineShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                                                    "colonizeVsAttackBias": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                                                    "minForceRatio": { "type": "number", "minimum": 0.5, "maximum": 4.0 },
                                                    "cruiserShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                                                    "bshipShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                                                    "bomberShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                                                    "interceptBias": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                                                    "maxUpkeepRatio": { "type": "number", "minimum": 0.1, "maximum": 0.95 },
                                                    "colPopReq": { "type": "number", "minimum": 0.05, "maximum": 0.95 },
                                                    "minShipyardPopRatio": { "type": "number", "minimum": 0.05, "maximum": 0.95 },
                                                    "minColonizeHab": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                                                    "minGarrisonCount": { "type": "integer", "minimum": 0, "maximum": 10 },
                                                    "techAlloc": {
                                                        "type": "object",
                                                        "properties": {
                                                            "ene": { "type": "number" },
                                                            "wep": { "type": "number" },
                                                            "pro": { "type": "number" },
                                                            "con": { "type": "number" },
                                                            "ele": { "type": "number" },
                                                            "bio": { "type": "number" }
                                                        }
                                                    },
                                                    "mineralWeights": {
                                                        "type": "object",
                                                        "properties": {
                                                            "iron": { "type": "number" },
                                                            "bora": { "type": "number" },
                                                            "germ": { "type": "number" }
                                                        }
                                                    }
                                                }
                                            }
                                        }
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
                                                    "mode": { "type": "string", "enum": ["Expand", "Fortify", "Attack", "Ignore"] }
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
                                                    "planetId": { "type": "integer", "minimum": 0, "description": "Own planet index (also accepts alias planetIdx)." },
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

pub async fn handle_mcp_get(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Response {
    if matches!(resolve_mcp_token(&state, &token).await, McpSeatResolution::Invalid) {
        return forbidden_json("Invalid or unknown MCP token");
    }
    let mut resp = StatusCode::METHOD_NOT_ALLOWED.into_response();
    resp.headers_mut()
        .insert(ALLOW, HeaderValue::from_static("POST, DELETE, OPTIONS"));
    resp
}

pub async fn handle_mcp_delete(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Response {
    if matches!(resolve_mcp_token(&state, &token).await, McpSeatResolution::Invalid) {
        return forbidden_json("Invalid or unknown MCP token");
    }
    StatusCode::NO_CONTENT.into_response()
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
        HeaderValue::from_static("Content-Type, Accept, Mcp-Session-Id, MCP-Protocol-Version"),
    );
    h.insert(
        HeaderName::from_static("access-control-expose-headers"),
        HeaderValue::from_static("Mcp-Session-Id"),
    );
    resp
}

pub async fn handle_mcp_post(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let resolution = resolve_mcp_token(&state, &token).await;
    if matches!(resolution, McpSeatResolution::Invalid) {
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

            let result = match resolution {
                McpSeatResolution::Idle { seat_name } => {
                    let idle_json = json!({
                        "status": "idle",
                        "seat": seat_name,
                        "message": "No active match bound to this seat yet."
                    });
                    tool_text_result(idle_json.to_string(), false)
                }
                McpSeatResolution::Bound {
                    match_id,
                    empire_idx,
                } => match tool_name {
                    "get_briefing" => handle_tool_briefing(&state, &match_id, empire_idx).await,
                    "get_observation" => {
                        handle_tool_observation(&state, &match_id, empire_idx).await
                    }
                    "submit_orders" => {
                        handle_tool_submit_orders(&state, &match_id, empire_idx, args).await
                    }
                    other => tool_text_result(
                        format!("Validation error: unknown tool '{other}'"),
                        true,
                    ),
                },
                McpSeatResolution::Invalid => unreachable!(),
            };
            jsonrpc_ok(id, result, client_session)
        }
        other => jsonrpc_err(id, -32601, format!("Method not found: {other}")),
    }
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
            "realtimeAsync (smooth RTS at {:.2}x speed, paused={}): simulation runs continuously; your submit_orders commands apply immediately without blocking the clock.",
            guard.speed_mult, guard.paused
        ),
        ExecutionMode::LockstepEpoch => format!(
            "lockstepEpoch (turn-based barrier, {} ticks = 1.0 yr, timeout={}ms where 0=infinite): simulation waits at each epoch boundary for all external agent seats to call submit_orders.",
            guard.ticks_per_epoch, guard.epoch_timeout_ms
        ),
    };

    let briefing = format!(
        "=== SI-STARS GRAND STRATEGIST BRIEFING ===\n\
        Match ID: {match_id}\n\
        Empire: {} (empireId={}, Seat #{empire_idx})\n\
        Primary Racial Trait ({}): {}\n\
        Victory Conditions: {victory_text}\n\
        Clock Mode: {clock_text}\n\n\
        === ARCHITECTURE ===\n\
        - A built-in Governor continuously handles low-level micro (individual ship routing, freighters, colonizer dispatch, local defense, build queues) strictly within your active policy sliders and sector directives.\n\
        - Fog of War is strictly isolated: you only see owned worlds, explored sectors, scanned enemy fleets/planets, and diplomatic cables sent to you.\n\n\
        === OBSERVATION & POLICY GLOSSARY ===\n\
        - galaxyOverview: totalPlanets, gridRows x gridCols sector grid, explored/unexplored counts, victoryGoal.\n\
        - empire: resPerYear, minerals {{iron, bora, germ}}, upkeepRatio (fleet upkeep / resPerYear), planetCount, fleetCount, techLevels {{ene, wep, pro, con, ele, bio}}, policy.\n\
        - sectors: non-empty grid sectors with sectorId, row, col, directive, unexploredStars, ownPlanets, ownResPerYear, neutralKnown, neutralPenScanned, avgNeutralHab, enemyPlanetsByEmp [[empId, count]], detectedEnemyFleetPower, avgIntelAgeYears.\n\
        - ownHubs: top 10 own worlds with planetId, sectorId, pop, maxPop, factories, defense, starbaseTier, hasGate, shipProdBuf, minerals.\n\
        - enemyPlanets: strictly foreign worlds (never neutrals) with planetId, sectorId, ownerEmp, starbaseTier, hasStargate, intelAgeYears, penScanned (true reveals popEst/facEst/defEst).\n\
        - wormholes: [{{wormholeId, fromSector, toSector}}] (toSector is null until far end is discovered).\n\
        - detectedEnemyFleets: spotted enemy fleets with distFromNearestOwnLy.\n\
        - Policy Sliders (setPolicy accepts PARTIAL camelCase patches — omit unchanged fields):\n\
          * facShare, shipShare, techShare, mineShare: planetary budget shares (normalized to 1.0).\n\
          * colonizeVsAttackBias: 1.0 = max colonization priority, 0.0 = max military war priority.\n\
          * minForceRatio: required fleet advantage over known defense+Starbase before attacking (e.g. 1.25 = +25% advantage).\n\
          * cruiserShare, bshipShare, bomberShare: warship mix (Cruiser counters Bomber & intercepts; Battleship counters Cruiser; Bomber destroys planetary bunkers).\n\
          * techAlloc {{ene, wep, pro, con, ele, bio}}: research split.\n\
          * mineralWeights {{iron, bora, germ}}: mining/colonization priority.\n\
          * maxUpkeepRatio: above this threshold Governor pauses shipbuilding and scraps old hulls.\n\
        - Sector Directives (setSectorDirective): Expand, Fortify, Attack, Ignore.\n\
        - Diplomacy Stances (setDiplomacy): War, Neutral, Ceasefire, Alliance.\n\
        - Hub Construction (buildHub): {{planetId, kind: \"Stargate\" | \"Starbase\"}} (alias planetIdx also accepted).",
        emp.race.name,
        empire_idx + 1,
        emp.race.prt,
        prt_profile(&emp.race.prt),
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
        let mut notify_barrier = false;

        if guard.execution_mode == ExecutionMode::RealtimeAsync {
            // Apply immediately in RealtimeAsync so follow-up get_observation sees updated policy/memory right away
            feed_events = apply_submissions_locked(&mut guard, true);
        } else {
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindSeatRequest {
    pub match_id: String,
    pub empire_idx: usize,
}

pub async fn bind_persistent_seat(
    State(state): State<Arc<AppState>>,
    Path(seat_name): Path<String>,
    headers: HeaderMap,
    Json(req): Json<BindSeatRequest>,
) -> Result<Json<Value>, (StatusCode, Json<ApiErrorBody>)> {
    if let Some(ref expected_admin) = state.admin_token {
        let supplied = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer ").or_else(|| s.strip_prefix("bearer ")))
            .map(str::trim)
            .or_else(|| {
                headers
                    .get("x-admin-token")
                    .and_then(|v| v.to_str().ok())
                    .map(str::trim)
            });
        if supplied != Some(expected_admin.as_str()) {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorBody {
                    error: "admin_auth_required".to_string(),
                    message: "Valid SISTARS_ADMIN_TOKEN required to bind MCP seat".to_string(),
                }),
            ));
        }
    }

    // Verify seat exists in configured persistent seats
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

    // Verify target match & empire exist
    let empire_count = {
        let matches = state.matches.read().await;
        let Some(handle) = matches.get(&req.match_id) else {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorBody {
                    error: "match_not_found".to_string(),
                    message: format!("Match '{}' not found", req.match_id),
                }),
            ));
        };
        let guard = handle.inner.read().await;
        guard.sim.empires.len()
    };
    if req.empire_idx >= empire_count {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorBody {
                error: "invalid_empire_idx".to_string(),
                message: format!("empireIdx {} out of range (0..{empire_count})", req.empire_idx),
            }),
        ));
    }

    {
        let mut bindings = state.persistent_seat_bindings.write().await;
        bindings.insert(seat_name.clone(), (req.match_id.clone(), req.empire_idx));
    }

    Ok(Json(json!({
        "bound": true,
        "seat": seat_name,
        "matchId": req.match_id,
        "empireIdx": req.empire_idx,
    })))
}
