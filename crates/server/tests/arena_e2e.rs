use std::net::SocketAddr;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::StatusCode;
use serde_json::{json, Value};
use sistars_server::{build_router, AppState};
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;

struct TestServer {
    base_url: String,
    ws_base_url: String,
    state: std::sync::Arc<AppState>,
    client: reqwest::Client,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let state = self.state.clone();
        tokio::spawn(async move {
            state.shutdown_all().await;
        });
    }
}

async fn spawn_test_server() -> TestServer {
    spawn_test_server_with_admin(None).await
}

async fn spawn_test_server_with_admin(admin_token: Option<String>) -> TestServer {
    let logs_dir = std::env::temp_dir().join(format!(
        "sistars_test_logs_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let state = AppState::new(admin_token, logs_dir);
    let app = build_router(state.clone());

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind random localhost port");
    let addr: SocketAddr = listener.local_addr().expect("Failed to read local_addr");

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    TestServer {
        base_url: format!("http://{addr}"),
        ws_base_url: format!("ws://{addr}"),
        state,
        client: reqwest::Client::new(),
    }
}

async fn wait_for_epoch(
    srv: &TestServer,
    match_id: &str,
    target_epoch: u64,
    timeout: Duration,
) -> Value {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let resp = srv
            .client
            .get(format!("{}/api/v1/matches/{match_id}", srv.base_url))
            .send()
            .await
            .expect("GET match failed");
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = resp.json().await.expect("Invalid JSON");
        if body["epoch"].as_u64().unwrap_or(0) >= target_epoch {
            return body;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Timed out waiting for match {match_id} to reach epoch {target_epoch}; last summary: {body}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn e2e_token_isolation_and_fow() {
    let srv = spawn_test_server().await;

    // Create Match 1 with 2 agent seats (0 and 1)
    let m1: Value = srv
        .client
        .post(format!("{}/api/v1/matches", srv.base_url))
        .json(&json!({
            "seed": 111111,
            "galaxySize": "small",
            "executionMode": "lockstepEpoch",
            "ticksPerEpoch": 10,
            "epochTimeoutMs": 5000,
            "agentSeats": [0, 1]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let match1_id = m1["matchId"].as_str().unwrap().to_string();
    let tok0 = m1["empireTokens"]["0"].as_str().unwrap().to_string();
    let tok1 = m1["empireTokens"]["1"].as_str().unwrap().to_string();

    // Create Match 2 to test cross-match token rejection
    let m2: Value = srv
        .client
        .post(format!("{}/api/v1/matches", srv.base_url))
        .json(&json!({
            "seed": 222222,
            "galaxySize": "small",
            "executionMode": "lockstepEpoch",
            "ticksPerEpoch": 10,
            "epochTimeoutMs": 5000,
            "agentSeats": [0]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let m2_tok0 = m2["empireTokens"]["0"].as_str().unwrap().to_string();

    // 1. Missing token -> 401
    let r_missing = srv
        .client
        .get(format!(
            "{}/api/v1/matches/{match1_id}/observation",
            srv.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(r_missing.status(), StatusCode::UNAUTHORIZED);

    // 2. Bogus token -> 401
    let r_bad = srv
        .client
        .get(format!(
            "{}/api/v1/matches/{match1_id}/observation",
            srv.base_url
        ))
        .bearer_auth("invalid_token_12345")
        .send()
        .await
        .unwrap();
    assert_eq!(r_bad.status(), StatusCode::UNAUTHORIZED);

    // 3. Valid token from Match 2 used against Match 1 -> 403 Forbidden
    let r_cross = srv
        .client
        .get(format!(
            "{}/api/v1/matches/{match1_id}/observation",
            srv.base_url
        ))
        .bearer_auth(&m2_tok0)
        .send()
        .await
        .unwrap();
    assert_eq!(r_cross.status(), StatusCode::FORBIDDEN);

    // 4. Token 0 returns strictly Empire 1 (seat 0) FoW; Token 1 returns strictly Empire 2 (seat 1) FoW
    let obs0: Value = srv
        .client
        .get(format!(
            "{}/api/v1/matches/{match1_id}/observation",
            srv.base_url
        ))
        .bearer_auth(&tok0)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let obs1: Value = srv
        .client
        .get(format!(
            "{}/api/v1/matches/{match1_id}/observation",
            srv.base_url
        ))
        .bearer_auth(&tok1)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(obs0["observation"]["empire"]["empireId"].as_u64(), Some(1));
    assert_eq!(obs1["observation"]["empire"]["empireId"].as_u64(), Some(2));

    let p0_idx = obs0["observation"]["ownHubs"][0]["planetId"]
        .as_u64()
        .unwrap();
    let p1_idx = obs1["observation"]["ownHubs"][0]["planetId"]
        .as_u64()
        .unwrap();
    assert_ne!(
        p0_idx, p1_idx,
        "Empires 0 and 1 must see their own distinct homeworlds"
    );
    assert!(
        obs0["previousStrategy"].is_null(),
        "Epoch 0 previousStrategy must be null"
    );
    assert!(obs0["delta"].is_null(), "Epoch 0 delta must be null");

    // 5. Private reasoning_note from Empire 0 MUST NEVER leak to Empire 1's observation
    let secret_note = "SECRET_OMEGA_BACKSTAB_PLAN_999";
    let sub0 = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match1_id}/commands",
            srv.base_url
        ))
        .bearer_auth(&tok0)
        .json(&json!({
            "epoch": 0,
            "commands": [],
            "reasoningNote": secret_note
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(sub0.status(), StatusCode::OK);

    let sub1 = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match1_id}/commands",
            srv.base_url
        ))
        .bearer_auth(&tok1)
        .json(&json!({
            "epoch": 0,
            "commands": []
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(sub1.status(), StatusCode::OK);

    wait_for_epoch(&srv, &match1_id, 1, Duration::from_secs(3)).await;

    let obs1_after_text = srv
        .client
        .get(format!(
            "{}/api/v1/matches/{match1_id}/observation",
            srv.base_url
        ))
        .bearer_auth(&tok1)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        !obs1_after_text.contains(secret_note),
        "Empire 0's private reasoning_note leaked into Empire 1's observation!"
    );

    // Public GET /matches/{id} must also not leak secret notes or tokens
    let public_text = srv
        .client
        .get(format!("{}/api/v1/matches/{match1_id}", srv.base_url))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!public_text.contains(secret_note));
    assert!(!public_text.contains(&tok0));
}

#[tokio::test]
async fn e2e_epoch_barrier_and_timeout() {
    let srv = spawn_test_server().await;

    let created: Value = srv
        .client
        .post(format!("{}/api/v1/matches", srv.base_url))
        .json(&json!({
            "seed": 333333,
            "galaxySize": "small",
            "executionMode": "lockstepEpoch",
            "ticksPerEpoch": 10,
            "epochTimeoutMs": 450,
            "lobbyTimeoutSec": 0,
            "agentSeats": [0, 1]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let match_id = created["matchId"].as_str().unwrap().to_string();
    let tok0 = created["empireTokens"]["0"].as_str().unwrap().to_string();
    let tok1 = created["empireTokens"]["1"].as_str().unwrap().to_string();

    // 1. Submit ONLY Seat 0 -> barrier must still wait for Seat 1
    let r0 = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/commands",
            srv.base_url
        ))
        .bearer_auth(&tok0)
        .json(&json!({ "epoch": 0, "commands": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r0.status(), StatusCode::OK);

    tokio::time::sleep(Duration::from_millis(60)).await;
    let mid_summary: Value = srv
        .client
        .get(format!("{}/api/v1/matches/{match_id}", srv.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(mid_summary["epoch"].as_u64(), Some(0));
    assert_eq!(mid_summary["status"].as_str(), Some("WaitingForAgents"));
    assert_eq!(mid_summary["waitingForSeats"], json!([1]));

    // 2. Submit Seat 1 -> barrier immediately advances epoch 0 -> 1 (10 ticks)
    let r1 = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/commands",
            srv.base_url
        ))
        .bearer_auth(&tok1)
        .json(&json!({ "epoch": 0, "commands": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::OK);

    let ep1 = wait_for_epoch(&srv, &match_id, 1, Duration::from_secs(2)).await;
    assert_eq!(ep1["epoch"].as_u64(), Some(1));
    assert_eq!(ep1["tick"].as_u64(), Some(10));

    // 3. On epoch 1, only Seat 0 responds while Seat 1 times out (> 450 ms) -> epoch MUST still advance to 2
    let r0_ep1 = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/commands",
            srv.base_url
        ))
        .bearer_auth(&tok0)
        .json(&json!({ "epoch": 1, "commands": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r0_ep1.status(), StatusCode::OK);

    let ep2 = wait_for_epoch(&srv, &match_id, 2, Duration::from_secs(3)).await;
    assert!(ep2["epoch"].as_u64().unwrap_or(0) >= 2);
    assert!(ep2["tick"].as_u64().unwrap_or(0) >= 20);
}

#[tokio::test]
async fn e2e_diplomacy_and_spectator_ws_feed() {
    let srv = spawn_test_server().await;

    let created: Value = srv
        .client
        .post(format!("{}/api/v1/matches", srv.base_url))
        .json(&json!({
            "seed": 444444,
            "galaxySize": "small",
            "executionMode": "lockstepEpoch",
            "ticksPerEpoch": 10,
            "epochTimeoutMs": 5000,
            "frameStrideTicks": 2,
            "agentSeats": [0, 1]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let match_id = created["matchId"].as_str().unwrap().to_string();
    let tok0 = created["empireTokens"]["0"].as_str().unwrap().to_string();
    let tok1 = created["empireTokens"]["1"].as_str().unwrap().to_string();

    // Connect Spectator WebSocket client
    let ws_url = format!("{}/ws/matches/{match_id}/spectate", srv.ws_base_url);
    let (mut ws_stream, _) = connect_async(&ws_url)
        .await
        .expect("Failed to connect to Spectator WebSocket");

    // First message must be Init with map geometry and HUD
    let first_msg = tokio::time::timeout(Duration::from_secs(3), ws_stream.next())
        .await
        .expect("Timed out waiting for WS Init")
        .expect("WS closed")
        .expect("WS error");
    let init_json: Value = serde_json::from_str(first_msg.to_text().unwrap()).unwrap();
    assert_eq!(init_json["type"].as_str(), Some("init"));
    assert!(init_json["planetsBuf"].as_array().unwrap().len() >= 120 * 7);

    // Seat 0 sends SendDipMessage + SetDiplomacy(Ceasefire) + private strategyMemory (all camelCase!)
    let cable_text = "Ceasefire proposal along northern spiral arm";
    let secret_rationale = "SECRET_STRATEGY_RATIONALE_SECURE_NORTH";
    let res0 = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/commands",
            srv.base_url
        ))
        .bearer_auth(&tok0)
        .json(&json!({
            "epoch": 0,
            "strategyMemory": {
                "phase": "Early Expansion",
                "priorities": ["Colonize northern arm", "Push Electronics"],
                "rationale": secret_rationale,
                "diplomacyNotes": "Seeking ceasefire with Empire 2",
                "pivotTriggers": ["Shift to war if Empire 2 violates ceasefire"]
            },
            "commands": [
                { "setDiplomacy": { "targetEmp": 2, "stance": "Ceasefire" } },
                { "sendDipMessage": { "toEmp": 2, "text": cable_text } }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res0.status(), StatusCode::OK);

    // Seat 1 sets a SectorDirective (camelCase)
    let res1 = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/commands",
            srv.base_url
        ))
        .bearer_auth(&tok1)
        .json(&json!({
            "epoch": 0,
            "commands": [
                { "setSectorDirective": { "sectorId": 0, "mode": "Fortify" } }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res1.status(), StatusCode::OK);

    // Wait for epoch 1 to complete
    wait_for_epoch(&srv, &match_id, 1, Duration::from_secs(3)).await;

    // Verify Seat 0 gets its own previousStrategy & non-null EpochDelta at Epoch 1
    let obs0_ep1: Value = srv
        .client
        .get(format!(
            "{}/api/v1/matches/{match_id}/observation",
            srv.base_url
        ))
        .bearer_auth(&tok0)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        obs0_ep1["previousStrategy"]["phase"].as_str(),
        Some("Early Expansion")
    );
    assert_eq!(
        obs0_ep1["previousStrategy"]["rationale"].as_str(),
        Some(secret_rationale)
    );
    assert!(
        obs0_ep1["delta"].is_object(),
        "Epoch 1 delta must be populated, got: {}",
        obs0_ep1["delta"]
    );
    assert!(obs0_ep1["delta"]["resPerYearDelta"].is_number());
    assert!(obs0_ep1["delta"]["planetsDelta"].is_i64());

    // Verify Seat 1 received Seat 0's diplomatic message in its inbox & Ceasefire stance,
    // and NEVER sees Seat 0's private strategyMemory!
    let obs1: Value = srv
        .client
        .get(format!(
            "{}/api/v1/matches/{match_id}/observation",
            srv.base_url
        ))
        .bearer_auth(&tok1)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let known = obs1["observation"]["knownEmpires"].as_array().unwrap();
    let emp1_entry = known
        .iter()
        .find(|e| e["empId"].as_u64() == Some(1))
        .expect("Seat 1 must see Empire 1 in knownEmpires after diplomatic contact");
    assert_eq!(emp1_entry["stance"].as_str(), Some("Ceasefire"));
    let inbox = emp1_entry["inbox"].as_array().unwrap();
    assert!(
        inbox.iter().any(|m| m["text"].as_str() == Some(cable_text)),
        "Expected diplomatic message in Seat 1 inbox, got {inbox:?}"
    );
    assert!(
        !serde_json::to_string(&obs1)
            .unwrap()
            .contains(secret_rationale),
        "Private strategyMemory must NEVER leak to Seat 1 observation"
    );
    assert!(obs1["previousStrategy"].is_null());
    assert!(obs1["delta"]["newMessagesCount"].as_u64().unwrap_or(0) >= 1);

    // Drain Spectator WS messages and verify we received Frames + StrategyUpdate + DipMessage + DiplomacyChange + SectorDirective
    let mut saw_frame = false;
    let mut saw_strategy = false;
    let mut saw_dip_msg = false;
    let mut saw_dip_change = false;
    let mut saw_sector_dir = false;

    let collect_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::Instant::now() < collect_deadline {
        if saw_frame && saw_strategy && saw_dip_msg && saw_dip_change && saw_sector_dir {
            break;
        }
        match tokio::time::timeout(Duration::from_millis(250), ws_stream.next()).await {
            Ok(Some(Ok(msg))) if msg.is_text() => {
                let v: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
                match v["type"].as_str() {
                    Some("frame") => {
                        if v["planetsBuf"].as_array().is_some() {
                            saw_frame = true;
                        }
                    }
                    Some("feed") => {
                        let ev = &v["event"];
                        match ev["kind"].as_str() {
                            Some("StrategyUpdate")
                                if ev["strategy"]["rationale"].as_str()
                                    == Some(secret_rationale) =>
                            {
                                saw_strategy = true;
                            }
                            Some("DipMessage")
                                if ev["summary"]
                                    .as_str()
                                    .is_some_and(|s| s.contains(cable_text)) =>
                            {
                                saw_dip_msg = true;
                            }
                            Some("DiplomacyChange") => saw_dip_change = true,
                            Some("SectorDirective") => saw_sector_dir = true,
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            _ => break,
        }
    }

    assert!(
        saw_frame,
        "Spectator WS must receive live map render frames"
    );
    assert!(
        saw_strategy,
        "Spectator WS must receive StrategyUpdate ArenaFeedEvent with full StrategyMemory"
    );
    assert!(
        saw_dip_msg,
        "Spectator WS must receive DipMessage ArenaFeedEvent"
    );
    assert!(
        saw_dip_change,
        "Spectator WS must receive DiplomacyChange ArenaFeedEvent"
    );
    assert!(
        saw_sector_dir,
        "Spectator WS must receive SectorDirective ArenaFeedEvent"
    );
}

#[tokio::test]
async fn e2e_openapi_system_view_and_live_auditor() {
    let srv = spawn_test_server().await;

    // 1. OpenAPI 3.0 spec validation
    let spec: Value = srv
        .client
        .get(format!("{}/openapi.json", srv.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(spec["openapi"].as_str().unwrap().starts_with("3.0"));
    assert!(spec["components"]["schemas"]["AgentObservation"].is_object());
    assert!(spec["components"]["schemas"]["AgentPolicyPatch"].is_object());
    assert!(spec["components"]["schemas"]["StrategyMemory"].is_object());
    assert!(spec["components"]["schemas"]["EpochDelta"].is_object());
    assert!(spec["components"]["schemas"]["StrategicCommand"].is_object());

    // 2. Create match with debug_audit = true and run 3 epochs
    let created: Value = srv
        .client
        .post(format!("{}/api/v1/matches", srv.base_url))
        .json(&json!({
            "seed": 555555,
            "galaxySize": "small",
            "executionMode": "lockstepEpoch",
            "ticksPerEpoch": 15,
            "epochTimeoutMs": 5000,
            "agentSeats": [0],
            "debugAudit": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let match_id = created["matchId"].as_str().unwrap().to_string();
    let tok0 = created["empireTokens"]["0"].as_str().unwrap().to_string();

    // 3. On-demand Spectator REST endpoints: /system/0, /planet/0, /reports
    let sys: Value = srv
        .client
        .get(format!(
            "{}/api/v1/matches/{match_id}/system/0",
            srv.base_url
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(sys["planetId"].as_u64(), Some(1));
    assert!(sys["star"]["spectralClass"].is_string());
    assert!(!sys["bodies"].as_array().unwrap().is_empty());

    let planet: Value = srv
        .client
        .get(format!(
            "{}/api/v1/matches/{match_id}/planet/0",
            srv.base_url
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(planet["id"].as_u64(), Some(1));

    let reports: Value = srv
        .client
        .get(format!(
            "{}/api/v1/matches/{match_id}/reports",
            srv.base_url
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!reports["planets"].as_array().unwrap().is_empty());

    // Advance 3 full epochs with LiveAuditor active
    for ep in 0..3u64 {
        let r = srv
            .client
            .post(format!(
                "{}/api/v1/matches/{match_id}/commands",
                srv.base_url
            ))
            .bearer_auth(&tok0)
            .json(&json!({ "epoch": ep, "commands": [] }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        wait_for_epoch(&srv, &match_id, ep + 1, Duration::from_secs(3)).await;
    }

    // 4. Verify LiveAuditor recorded 0 anomalies
    let anomalies: Value = srv
        .client
        .get(format!(
            "{}/api/v1/matches/{match_id}/anomalies",
            srv.base_url
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        anomalies["anomalies"].as_array().map(|a| a.len()),
        Some(0),
        "Expected 0 LiveAuditor anomalies, got: {anomalies}"
    );
}

#[tokio::test]
async fn e2e_realtime_async_and_live_control() {
    let srv = spawn_test_server().await;

    // 1. Default executionMode must be `realtimeAsync`, advancing ticks smoothly without blocking on agentSeats
    let created: Value = srv
        .client
        .post(format!("{}/api/v1/matches", srv.base_url))
        .json(&json!({
            "seed": 666001,
            "galaxySize": "small",
            "speedMult": 5.0,
            "ticksPerEpoch": 20,
            "lobbyTimeoutSec": 0,
            "agentSeats": [0]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(created["executionMode"].as_str(), Some("realtimeAsync"));
    let match_id = created["matchId"].as_str().unwrap().to_string();
    let tok0 = created["empireTokens"]["0"].as_str().unwrap().to_string();

    // Ticks must advance smoothly even though Seat 0 has NOT submitted anything yet
    tokio::time::sleep(Duration::from_millis(200)).await;
    let s1: Value = srv
        .client
        .get(format!("{}/api/v1/matches/{match_id}", srv.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let t1 = s1["tick"].as_u64().unwrap();
    assert!(
        t1 > 0,
        "RealtimeAsync match must advance ticks without blocking on agents"
    );

    // 2. Live pause via POST /control -> ticks MUST freeze
    let ctrl_pause: Value = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/control",
            srv.base_url
        ))
        .json(&json!({ "paused": true }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ctrl_pause["paused"].as_bool(), Some(true));
    tokio::time::sleep(Duration::from_millis(80)).await;
    let t_paused_a = srv
        .client
        .get(format!("{}/api/v1/matches/{match_id}", srv.base_url))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["tick"]
        .as_u64()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(120)).await;
    let t_paused_b = srv
        .client
        .get(format!("{}/api/v1/matches/{match_id}", srv.base_url))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["tick"]
        .as_u64()
        .unwrap();
    assert_eq!(
        t_paused_a, t_paused_b,
        "Paused match must not advance ticks"
    );

    // 3. stepTicks: 2 while paused -> advances by exactly 2 ticks and stays paused
    let _ = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/control",
            srv.base_url
        ))
        .json(&json!({ "stepTicks": 2 }))
        .send()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let s_stepped: Value = srv
        .client
        .get(format!("{}/api/v1/matches/{match_id}", srv.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(s_stepped["tick"].as_u64(), Some(t_paused_b + 2));
    assert_eq!(s_stepped["paused"].as_bool(), Some(true));

    // 4. Resume at speedMult: 10.0 and submit an async POST /commands in flight (even with stale epoch: 0!)
    let ctrl_resume: Value = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/control",
            srv.base_url
        ))
        .json(&json!({ "paused": false, "speedMult": 10.0 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ctrl_resume["paused"].as_bool(), Some(false));
    assert_eq!(ctrl_resume["speedMult"].as_f64(), Some(10.0));

    let cmd_resp = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/commands",
            srv.base_url
        ))
        .bearer_auth(&tok0)
        .json(&json!({
            "epoch": 0,
            "strategyMemory": {
                "phase": "Async Blitz",
                "priorities": ["Maintain tempo"],
                "rationale": "Testing async command application in RealtimeAsync mode",
                "diplomacyNotes": "None",
                "pivotTriggers": []
            },
            "commands": [
                { "setPolicy": { "colonizeVsAttackBias": 0.2 } }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(cmd_resp.status(), StatusCode::OK);

    tokio::time::sleep(Duration::from_millis(180)).await;
    let obs_after: Value = srv
        .client
        .get(format!(
            "{}/api/v1/matches/{match_id}/observation",
            srv.base_url
        ))
        .bearer_auth(&tok0)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        obs_after["previousStrategy"]["phase"].as_str(),
        Some("Async Blitz"),
        "Async strategyMemory must be applied on the fly without blocking clock"
    );
    let bias = obs_after["observation"]["empire"]["policy"]["colonizeVsAttackBias"]
        .as_f64()
        .unwrap();
    assert!(
        (bias - 0.2).abs() < 0.02,
        "Expected patched bias 0.2, got {bias}"
    );
    assert!(obs_after["tick"].as_u64().unwrap() > t_paused_b + 2);
}

#[tokio::test]
async fn e2e_lockstep_no_timeout_and_exact_epoch_ticks() {
    let srv = spawn_test_server().await;

    let created: Value = srv
        .client
        .post(format!("{}/api/v1/matches", srv.base_url))
        .json(&json!({
            "seed": 666002,
            "galaxySize": "small",
            "executionMode": "lockstepEpoch",
            "ticksPerEpoch": 20,
            "epochTimeoutMs": 0,
            "agentSeats": [0]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let match_id = created["matchId"].as_str().unwrap().to_string();
    let tok0 = created["empireTokens"]["0"].as_str().unwrap().to_string();

    // Epoch 0 -> submit -> must execute EXACTLY 20 ticks (tick == 20, epoch == 1)
    let r0 = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/commands",
            srv.base_url
        ))
        .bearer_auth(&tok0)
        .json(&json!({ "epoch": 0, "commands": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r0.status(), StatusCode::OK);
    let ep1 = wait_for_epoch(&srv, &match_id, 1, Duration::from_secs(3)).await;
    assert_eq!(ep1["tick"].as_u64(), Some(20));

    // With epochTimeoutMs == 0, Epoch 1 must wait indefinitely without timing out
    tokio::time::sleep(Duration::from_millis(250)).await;
    let still_ep1: Value = srv
        .client
        .get(format!("{}/api/v1/matches/{match_id}", srv.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(still_ep1["epoch"].as_u64(), Some(1));
    assert_eq!(still_ep1["tick"].as_u64(), Some(20));
    assert_eq!(still_ep1["status"].as_str(), Some("WaitingForAgents"));
    assert!(still_ep1["epochDeadlineMs"].is_null());

    // Submit Epoch 1 -> must execute EXACTLY 20 ticks (tick == 40, epoch == 2, zero drift!)
    let r1 = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/commands",
            srv.base_url
        ))
        .bearer_auth(&tok0)
        .json(&json!({ "epoch": 1, "commands": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::OK);
    let ep2 = wait_for_epoch(&srv, &match_id, 2, Duration::from_secs(3)).await;
    assert_eq!(ep2["tick"].as_u64(), Some(40));
}

#[tokio::test]
async fn e2e_observation_hub_metrics_and_build_hub_alias() {
    let srv = spawn_test_server().await;

    let created: Value = srv
        .client
        .post(format!("{}/api/v1/matches", srv.base_url))
        .json(&json!({
            "seed": 666003,
            "galaxySize": "small",
            "executionMode": "lockstepEpoch",
            "ticksPerEpoch": 10,
            "epochTimeoutMs": 0,
            "agentSeats": [0]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let match_id = created["matchId"].as_str().unwrap().to_string();
    let tok0 = created["empireTokens"]["0"].as_str().unwrap().to_string();

    // 1. Verify ownHubs includes `maxPop` and `shipProdBuf`
    let obs: Value = srv
        .client
        .get(format!(
            "{}/api/v1/matches/{match_id}/observation",
            srv.base_url
        ))
        .bearer_auth(&tok0)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let hub0 = &obs["observation"]["ownHubs"][0];
    assert!(
        hub0["maxPop"].as_f64().unwrap_or(0.0) > 10.0,
        "ownHubs[0] must include maxPop, got {hub0}"
    );
    assert!(
        hub0["shipProdBuf"].is_number(),
        "ownHubs[0] must include shipProdBuf, got {hub0}"
    );
    let pid = hub0["planetId"].as_u64().unwrap();

    // 2. Verify buildHub accepts BOTH `planetId` alias AND `planetIdx`
    let res = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/commands",
            srv.base_url
        ))
        .bearer_auth(&tok0)
        .json(&json!({
            "epoch": 0,
            "commands": [
                { "buildHub": { "planetId": pid, "kind": "Stargate" } },
                { "buildHub": { "planetIdx": pid, "kind": "Starbase" } }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["commandsQueued"].as_u64(), Some(2));
}

#[tokio::test]
async fn e2e_mcp_full_handshake_and_gameplay() {
    let srv = spawn_test_server().await;

    let created: Value = srv
        .client
        .post(format!("{}/api/v1/matches", srv.base_url))
        .json(&json!({
            "seed": 777010,
            "galaxySize": "small",
            "executionMode": "realtimeAsync",
            "speedMult": 2.0,
            "startPaused": true,
            "agentSeats": [0, 1]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let match_id = created["matchId"].as_str().unwrap().to_string();
    let tok0 = created["empireTokens"]["0"].as_str().unwrap().to_string();
    assert!(
        created["mcpUrls"]["0"]
            .as_str()
            .unwrap()
            .ends_with(&format!("/mcp/{tok0}")),
        "mcpUrls must contain ready-to-use /mcp/<token> URL"
    );

    let mcp_url = format!("{}/mcp/{tok0}", srv.base_url);

    // Connect Spectator WebSocket so we can verify StrategyUpdate arrives from MCP submit_orders
    let ws_url = format!("{}/ws/matches/{match_id}/spectate", srv.ws_base_url);
    let (mut ws_stream, _) = connect_async(&ws_url).await.unwrap();
    let _init = tokio::time::timeout(Duration::from_secs(2), ws_stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // 1. initialize
    let init_resp = srv
        .client
        .post(&mcp_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "claude-test", "version": "1.0" }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(init_resp.status(), StatusCode::OK);
    assert!(init_resp.headers().get("mcp-session-id").is_some());
    let init_body: Value = init_resp.json().await.unwrap();
    assert_eq!(
        init_body["result"]["protocolVersion"].as_str(),
        Some("2025-06-18")
    );
    assert!(init_body["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("get_briefing"));

    // 2. notifications/initialized -> MUST return 202 Accepted with EMPTY body
    let notif_resp = srv
        .client
        .post(&mcp_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(notif_resp.status(), StatusCode::ACCEPTED);
    assert_eq!(notif_resp.text().await.unwrap(), "");

    // 3. ping -> {}
    let ping_body: Value = srv
        .client
        .post(&mcp_url)
        .json(&json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ping_body["result"], json!({}));

    // 4. tools/list -> exactly 6 tools
    let list_body: Value = srv
        .client
        .post(&mcp_url)
        .json(&json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tools = list_body["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 6);
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        vec![
            "list_matches",
            "join_match",
            "leave_match",
            "get_briefing",
            "get_observation",
            "submit_orders"
        ]
    );

    // 5. tools/call get_briefing
    let brief_body: Value = srv
        .client
        .post(&mcp_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": { "name": "get_briefing", "arguments": {} }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(brief_body["result"]["isError"].as_bool(), Some(false));
    let brief_txt = brief_body["result"]["content"][0]["text"].as_str().unwrap();
    assert!(brief_txt.contains("SI-STARS GRAND STRATEGIST BRIEFING"));
    assert!(
        !brief_txt.contains("year 0.0"),
        "Briefing must not mention year 0.0 when maxYear is None"
    );

    // 6. tools/call get_observation
    let obs_body: Value = srv
        .client
        .post(&mcp_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": { "name": "get_observation", "arguments": {} }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(obs_body["result"]["isError"].as_bool(), Some(false));
    let obs_json: Value =
        serde_json::from_str(obs_body["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(obs_json["state"]["empire"]["empireId"].as_u64(), Some(1));
    assert!(obs_json["previousStrategy"].is_null());

    // 7. tools/call submit_orders (strategyMemory + setPolicy)
    let sub_body: Value = srv
        .client
        .post(&mcp_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "submit_orders",
                "arguments": {
                    "strategyMemory": {
                        "phase": "MCP Expansion",
                        "priorities": ["Colonize core"],
                        "rationale": "MCP Claude opening strategy",
                        "diplomacyNotes": "Peaceful start",
                        "pivotTriggers": ["Enemy contact"]
                    },
                    "commands": [
                        { "setPolicy": { "colonizeVsAttackBias": 0.88 } }
                    ]
                }
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(sub_body["result"]["isError"].as_bool(), Some(false));
    let sub_ack: Value =
        serde_json::from_str(sub_body["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(sub_ack["accepted"].as_bool(), Some(true));
    assert_eq!(sub_ack["appliedCommands"].as_u64(), Some(1));

    // 8. Second get_observation confirms persisted previousStrategy & patched policy
    let obs2_body: Value = srv
        .client
        .post(&mcp_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": { "name": "get_observation", "arguments": {} }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let obs2_json: Value =
        serde_json::from_str(obs2_body["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        obs2_json["previousStrategy"]["phase"].as_str(),
        Some("MCP Expansion")
    );
    let bias = obs2_json["state"]["empire"]["policy"]["colonizeVsAttackBias"]
        .as_f64()
        .unwrap();
    assert!((bias - 0.88).abs() < 0.02);

    // 9. Spectator WebSocket received StrategyUpdate from MCP submit_orders
    let mut saw_strategy = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline && !saw_strategy {
        if let Ok(Some(Ok(msg))) =
            tokio::time::timeout(Duration::from_millis(200), ws_stream.next()).await
        {
            if msg.is_text() {
                let v: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
                if v["type"].as_str() == Some("feed")
                    && v["event"]["kind"].as_str() == Some("StrategyUpdate")
                    && v["event"]["strategy"]["phase"].as_str() == Some("MCP Expansion")
                {
                    saw_strategy = true;
                }
            }
        }
    }
    assert!(
        saw_strategy,
        "Spectator WS must receive StrategyUpdate emitted via MCP submit_orders"
    );
}

#[tokio::test]
async fn e2e_mcp_auth_isolation_and_http_methods() {
    let srv = spawn_test_server().await;

    let created: Value = srv
        .client
        .post(format!("{}/api/v1/matches", srv.base_url))
        .json(&json!({
            "seed": 777011,
            "galaxySize": "small",
            "executionMode": "realtimeAsync",
            "startPaused": true,
            "agentSeats": [0, 1]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tok0 = created["empireTokens"]["0"].as_str().unwrap().to_string();
    let tok1 = created["empireTokens"]["1"].as_str().unwrap().to_string();

    // 1. Unknown token MUST return 403 Forbidden with NO WWW-Authenticate header (never 401!)
    let bad_resp = srv
        .client
        .post(format!("{}/mcp/unknown_bogus_token_xyz", srv.base_url))
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_resp.status(), StatusCode::FORBIDDEN);
    assert!(
        bad_resp.headers().get("www-authenticate").is_none(),
        "MCP 403 must NEVER include WWW-Authenticate header (breaks Claude capability URL)"
    );

    // 2. GET /mcp/{token} -> 405 Method Not Allowed with Allow header
    let get_resp = srv
        .client
        .get(format!("{}/mcp/{tok0}", srv.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert!(get_resp.headers().get("allow").is_some());

    // 3. DELETE /mcp/{token} -> 204 No Content
    let del_resp = srv
        .client
        .delete(format!("{}/mcp/{tok0}", srv.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);

    // 4. Isolation: Empire 0 submits secret strategyMemory -> Empire 1 MUST NEVER see it
    let secret = "MCP_TOP_SECRET_AMBUSH_999";
    let _ = srv
        .client
        .post(format!("{}/mcp/{tok0}", srv.base_url))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/call",
            "params": {
                "name": "submit_orders",
                "arguments": {
                    "strategyMemory": {
                        "phase": "Secret Plan",
                        "priorities": [secret],
                        "rationale": secret,
                        "diplomacyNotes": secret,
                        "pivotTriggers": []
                    },
                    "commands": []
                }
            }
        }))
        .send()
        .await
        .unwrap();

    let obs1_text = srv
        .client
        .post(format!("{}/mcp/{tok1}", srv.base_url))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "tools/call",
            "params": { "name": "get_observation", "arguments": {} }
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        !obs1_text.contains(secret),
        "Empire 0's MCP strategyMemory leaked into Empire 1's observation!"
    );
}

#[tokio::test]
async fn e2e_mcp_persistent_seat_rebinding() {
    let logs_dir =
        std::env::temp_dir().join(format!("sistars_mcp_seat_test_{}", std::process::id()));
    let seat_token = "persistent_claude_seat_secret_token_01";
    let state = AppState::new_with_mcp(
        None,
        logs_dir,
        Some("https://sistars.example.test".to_string()),
        Some(&format!("claude_alpha:{seat_token}")),
    );
    let app = build_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let client = reqwest::Client::new();
    let mcp_url = format!("{base_url}/mcp/{seat_token}");

    // 1. Before binding: get_observation returns clean idle JSON
    let idle_resp: Value = client
        .post(&mcp_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "get_observation", "arguments": {} }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(idle_resp["result"]["isError"].as_bool(), Some(false));
    let idle_inner: Value =
        serde_json::from_str(idle_resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(idle_inner["status"].as_str(), Some("idle"));

    // 2. Create a match and bind `claude_alpha` to empireIdx 2
    let created: Value = client
        .post(format!("{base_url}/api/v1/matches"))
        .json(&json!({
            "seed": 777012,
            "galaxySize": "small",
            "startPaused": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let match_id = created["matchId"].as_str().unwrap().to_string();
    assert!(created["mcpUrls"]["0"]
        .as_str()
        .unwrap()
        .starts_with("https://sistars.example.test/mcp/"));

    let bind_resp = client
        .post(format!("{base_url}/api/v1/mcp/seats/claude_alpha/bind"))
        .json(&json!({ "matchId": match_id, "empireIdx": 2 }))
        .send()
        .await
        .unwrap();
    assert_eq!(bind_resp.status(), StatusCode::OK);

    // 3. Now get_observation returns live observation for Empire 3 (empireIdx 2)
    let bound_resp: Value = client
        .post(&mcp_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "get_observation", "arguments": {} }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let bound_inner: Value =
        serde_json::from_str(bound_resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(bound_inner["matchId"].as_str(), Some(match_id.as_str()));
    assert_eq!(bound_inner["state"]["empire"]["empireId"].as_u64(), Some(3));

    state.shutdown_all().await;
}

#[tokio::test]
async fn e2e_mcp_header_auth_lobby_and_self_join() {
    let logs_dir =
        std::env::temp_dir().join(format!("sistars_mcp_lobby_test_{}", std::process::id()));
    let secret1 = "claude_seat_1_secret_token_abc";
    let secret2 = "gemini_seat_2_secret_token_xyz";
    let state = AppState::new_with_mcp(
        None,
        logs_dir,
        Some("https://sistars.lobby.test".to_string()),
        Some(&format!("claude-1:{secret1},gemini-1:{secret2}")),
    );
    let app = build_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let client = reqwest::Client::new();
    let mcp_url = format!("{base_url}/mcp");

    // 1. POST /mcp without header & with bad token -> 403 Forbidden WITHOUT WWW-Authenticate
    let no_auth = client
        .post(&mcp_url)
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }))
        .send()
        .await
        .unwrap();
    assert_eq!(no_auth.status(), StatusCode::FORBIDDEN);
    assert!(no_auth.headers().get("www-authenticate").is_none());

    let bad_auth = client
        .post(&mcp_url)
        .header("Authorization", "Bearer totally_wrong_token")
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_auth.status(), StatusCode::FORBIDDEN);
    assert!(bad_auth.headers().get("www-authenticate").is_none());

    // 2. Valid Authorization: Bearer + X-Api-Key handshake: initialize -> notifications/initialized (202) -> tools/list (6 tools)
    let init_resp = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret1}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "claude", "version": "1.0" } }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(init_resp.status(), StatusCode::OK);
    let notif_resp = client
        .post(&mcp_url)
        .header("X-Api-Key", secret1)
        .json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        .send()
        .await
        .unwrap();
    assert_eq!(notif_resp.status(), StatusCode::ACCEPTED);
    assert_eq!(notif_resp.text().await.unwrap(), "");

    let tools_body: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret1}"))
        .json(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(tools_body["result"]["tools"].as_array().unwrap().len(), 6);

    // 3. Create 2 matches with scenarioName (m1 explicitly reserves seats [0, 2] for external agents!)
    let m1: Value = client
        .post(format!("{base_url}/api/v1/matches"))
        .json(&json!({
            "seed": 888001,
            "scenarioName": "Wormhole Chaos",
            "galaxySize": "small",
            "agentSeats": [0, 2],
            "startPaused": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let m1_id = m1["matchId"].as_str().unwrap().to_string();

    let m2: Value = client
        .post(format!("{base_url}/api/v1/matches"))
        .json(&json!({
            "seed": 888002,
            "scenarioName": "Balanced Standard",
            "galaxySize": "small",
            "openAgentSeats": [0, 1],
            "lobbyTimeoutSec": 0,
            "startPaused": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let m2_id = m2["matchId"].as_str().unwrap().to_string();

    // 4. In idle state, list_matches returns rich public metadata & get_observation returns idle
    let list_res: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret1}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "list_matches", "arguments": {} }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let list_json: Value =
        serde_json::from_str(list_res["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(list_json["seatName"].as_str(), Some("claude-1"));
    assert_eq!(list_json["matches"].as_array().unwrap().len(), 2);
    let m1_entry = list_json["matches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["matchId"].as_str() == Some(m1_id.as_str()))
        .unwrap();
    assert_eq!(m1_entry["scenarioName"].as_str(), Some("Wormhole Chaos"));
    assert_eq!(m1_entry["totalPlanets"].as_u64(), Some(120));
    let m1_seats = m1_entry["seats"].as_array().unwrap();
    assert_eq!(m1_seats.len(), 6);
    // Seats reserved in agentSeats ([0, 2]) must show as `open` (agent role), others as `bot`
    assert_eq!(m1_seats[0]["joinState"].as_str(), Some("open"));
    assert_eq!(m1_seats[0]["seatRole"].as_str(), Some("agent"));
    assert_eq!(m1_seats[1]["joinState"].as_str(), Some("bot"));
    assert_eq!(m1_seats[1]["seatRole"].as_str(), Some("bot"));
    assert_eq!(m1_seats[2]["joinState"].as_str(), Some("open"));

    // 5. claude-1 joins m1 as empireIdx 0
    let join1: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret1}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": { "name": "join_match", "arguments": { "matchId": m1_id, "empireIdx": 0 } }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(join1["result"]["isError"].as_bool(), Some(false));

    // 6. gemini-1 CANNOT claim occupied empireIdx 0 in m1 -> isError: true
    let join_conflict: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret2}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": { "name": "join_match", "arguments": { "matchId": m1_id, "empireIdx": 0 } }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(join_conflict["result"]["isError"].as_bool(), Some(true));

    // 7. claude-1 calls get_observation (verifying ownFleetsSummary) & submit_orders (verifying commandFeedback + scoutSector)
    let obs_res: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret1}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": { "name": "get_observation", "arguments": {} }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let obs_json: Value =
        serde_json::from_str(obs_res["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(obs_json["state"]["ownFleetsSummary"].is_object());

    let sub_res: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret1}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "submit_orders",
                "arguments": {
                    "strategyMemory": {
                        "phase": "Lobby Test",
                        "priorities": ["Scout sector 5"],
                        "rationale": "Testing commandFeedback",
                        "diplomacyNotes": "None",
                        "pivotTriggers": []
                    },
                    "commands": [
                        { "scoutSector": { "sectorId": 5 } },
                        { "buildHub": { "planetId": 0, "kind": "Stargate" } }
                    ]
                }
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sub_json: Value =
        serde_json::from_str(sub_res["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let fb = sub_json["commandFeedback"].as_array().unwrap();
    assert_eq!(fb.len(), 2);
    assert_eq!(fb[0]["status"].as_str(), Some("queued"));
    assert_eq!(fb[1]["status"].as_str(), Some("rejected")); // Planet 0 has 16 factories < 58

    // 8. Anti-Spy FoW Lock: claude-1 leaves m1, CANNOT join empireIdx 1 in m1, CAN join empireIdx 1 in m2!
    let leave_res: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret1}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "params": { "name": "leave_match", "arguments": {} }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(leave_res["result"]["isError"].as_bool(), Some(false));

    let spy_attempt: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret1}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": { "name": "join_match", "arguments": { "matchId": m1_id, "empireIdx": 1 } }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        spy_attempt["result"]["isError"].as_bool(),
        Some(true),
        "Anti-Spy FoW Lock must reject switching empires in the same match"
    );

    let join_other_match: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret1}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/call",
            "params": { "name": "join_match", "arguments": { "matchId": m2_id, "empireIdx": 1 } }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(join_other_match["result"]["isError"].as_bool(), Some(false));

    // 9. GET /api/v1/mcp/seats MUST NEVER expose secret tokens
    let seats_text = client
        .get(format!("{base_url}/api/v1/mcp/seats"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(seats_text.contains("claude-1"));
    assert!(seats_text.contains("gemini-1"));
    assert!(
        !seats_text.contains(secret1),
        "Secret token leaked in /api/v1/mcp/seats!"
    );
    assert!(
        !seats_text.contains(secret2),
        "Secret token leaked in /api/v1/mcp/seats!"
    );

    state.shutdown_all().await;
}

#[tokio::test]
async fn e2e_lobby_autostart_on_all_distinct_agents_join() {
    let secret1 = "lobby_claude_secret_001";
    let secret2 = "lobby_gemini_secret_002";
    let logs_dir =
        std::env::temp_dir().join(format!("sistars_e2e_lobby1_{}", uuid::Uuid::new_v4()));
    let state = AppState::new_with_mcp(
        None,
        logs_dir,
        None,
        Some(&format!("claude-1:{secret1},gemini-1:{secret2}")),
    );
    let router = build_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let base_url = format!("http://{addr}");
    let ws_base_url = format!("ws://{addr}");
    let mcp_url = format!("{base_url}/mcp");
    let client = reqwest::Client::new();

    // 1. Create match with 6 races, openAgentSeats: [0, 1], lobbyTimeoutSec: 60
    let created: Value = client
        .post(format!("{base_url}/api/v1/matches"))
        .json(&json!({
            "seed": 910001,
            "galaxySize": "small",
            "executionMode": "realtimeAsync",
            "speedMult": 5.0,
            "openAgentSeats": [0, 1],
            "lobbyTimeoutSec": 60
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let match_id = created["matchId"].as_str().unwrap().to_string();
    assert_eq!(created["status"].as_str(), Some("WaitingForAgents"));
    assert_eq!(created["requiredAgentCount"].as_u64(), Some(2));
    assert_eq!(created["joinedAgentCount"].as_u64(), Some(0));
    assert!(created["lobbyDeadlineMs"].as_u64().is_some());

    // Connect spectator WS to verify lobbyState events
    let ws_url = format!("{ws_base_url}/ws/matches/{match_id}/spectate");
    let (mut ws_stream, _) = connect_async(&ws_url).await.unwrap();
    let _init = tokio::time::timeout(Duration::from_secs(2), ws_stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let initial_lobby_ws = tokio::time::timeout(Duration::from_secs(2), ws_stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let initial_lobby_json: Value =
        serde_json::from_str(initial_lobby_ws.to_text().unwrap()).unwrap();
    assert_eq!(initial_lobby_json["type"].as_str(), Some("lobbyState"));
    assert_eq!(
        initial_lobby_json["status"].as_str(),
        Some("WaitingForAgents")
    );

    // Verify simulation stays frozen at tick 0 while waiting in lobby
    tokio::time::sleep(Duration::from_millis(120)).await;
    let status_before: Value = client
        .get(format!("{base_url}/api/v1/matches/{match_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status_before["status"].as_str(), Some("WaitingForAgents"));
    assert_eq!(status_before["tick"].as_u64(), Some(0));

    // 2. Agent 1 (claude-1) joins empireIdx 0 -> joinedAgentCount == 1, still WaitingForAgents
    let j1: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret1}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "join_match", "arguments": { "matchId": match_id, "empireIdx": 0 } }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(j1["result"]["isError"].as_bool(), Some(false));
    let j1_payload: Value =
        serde_json::from_str(j1["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(j1_payload["joinedAgentCount"].as_u64(), Some(1));
    assert_eq!(j1_payload["matchStatus"].as_str(), Some("WaitingForAgents"));

    // 3. Agent 1 tries to also join empireIdx 1 -> REJECTED (same player cannot hold 2 seats)
    let j1_dup: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret1}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "join_match", "arguments": { "matchId": match_id, "empireIdx": 1 } }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(j1_dup["result"]["isError"].as_bool(), Some(true));

    // 4. Agent 2 (gemini-1) tries to join empireIdx 2 (bot-only seat) -> REJECTED
    let j2_bot: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret2}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "join_match", "arguments": { "matchId": match_id, "empireIdx": 2 } }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(j2_bot["result"]["isError"].as_bool(), Some(true));
    assert!(
        j2_bot["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("bot-only"),
        "Expected clear bot-only seat message, got: {j2_bot}"
    );

    // 5. Agent 2 joins empireIdx 1 -> IMMEDIATE AUTO-START (startReason: "allAgentsJoined")
    let j2_ok: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret2}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": { "name": "join_match", "arguments": { "matchId": match_id, "empireIdx": 1 } }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(j2_ok["result"]["isError"].as_bool(), Some(false));
    let j2_payload: Value =
        serde_json::from_str(j2_ok["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(j2_payload["matchStatus"].as_str(), Some("Running"));
    assert_eq!(j2_payload["startReason"].as_str(), Some("allAgentsJoined"));

    // Verify WebSocket receives lobbyState with status Running & startReason allAgentsJoined
    let mut saw_running_lobby_ws = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Some(Ok(msg))) =
            tokio::time::timeout(Duration::from_millis(300), ws_stream.next()).await
        {
            if let Ok(txt) = msg.to_text() {
                if let Ok(v) = serde_json::from_str::<Value>(txt) {
                    if v["type"].as_str() == Some("lobbyState")
                        && v["status"].as_str() == Some("Running")
                        && v["startReason"].as_str() == Some("allAgentsJoined")
                    {
                        saw_running_lobby_ws = true;
                        break;
                    }
                }
            }
        }
    }
    assert!(
        saw_running_lobby_ws,
        "Spectator WS must receive lobbyState transition to Running"
    );

    // Verify ticks actually advance now!
    tokio::time::sleep(Duration::from_millis(180)).await;
    let status_after: Value = client
        .get(format!("{base_url}/api/v1/matches/{match_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status_after["status"].as_str(), Some("Running"));
    assert!(
        status_after["tick"].as_u64().unwrap_or(0) > 0,
        "Ticks must advance after allAgentsJoined autostart, got {status_after}"
    );

    state.shutdown_all().await;
}

#[tokio::test]
async fn e2e_lobby_timeout_autostart_and_late_join() {
    let secret1 = "late_claude_secret_001";
    let secret2 = "late_gemini_secret_002";
    let logs_dir =
        std::env::temp_dir().join(format!("sistars_e2e_lobby2_{}", uuid::Uuid::new_v4()));
    let state = AppState::new_with_mcp(
        None,
        logs_dir,
        None,
        Some(&format!("claude-1:{secret1},gemini-1:{secret2}")),
    );
    let router = build_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let base_url = format!("http://{addr}");
    let mcp_url = format!("{base_url}/mcp");
    let client = reqwest::Client::new();

    // 1. Create match with openAgentSeats: [0, 1], short lobbyTimeoutSec: 2
    let created: Value = client
        .post(format!("{base_url}/api/v1/matches"))
        .json(&json!({
            "seed": 910002,
            "galaxySize": "small",
            "executionMode": "realtimeAsync",
            "speedMult": 10.0,
            "openAgentSeats": [0, 1],
            "lobbyTimeoutSec": 2
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let match_id = created["matchId"].as_str().unwrap().to_string();
    assert_eq!(created["status"].as_str(), Some("WaitingForAgents"));

    // Only Agent 1 joins during lobby; seat 1 remains unfilled
    let j1: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret1}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "join_match", "arguments": { "matchId": match_id, "empireIdx": 0 } }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(j1["result"]["isError"].as_bool(), Some(false));

    // 2. Wait ~2.3s for lobbyTimeout autostart + several simulation ticks
    tokio::time::sleep(Duration::from_millis(2400)).await;
    let mid_status: Value = client
        .get(format!("{base_url}/api/v1/matches/{match_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(mid_status["status"].as_str(), Some("Running"));
    assert_eq!(mid_status["startReason"].as_str(), Some("lobbyTimeout"));
    assert!(
        mid_status["tick"].as_u64().unwrap_or(0) >= 5,
        "Simulation must advance smoothly after lobbyTimeout, got {mid_status}"
    );

    // 3. Agent 2 performs late-join on open seat 1 while Running
    let j2: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret2}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "join_match", "arguments": { "matchId": match_id, "empireIdx": 1 } }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(j2["result"]["isError"].as_bool(), Some(false));

    // Agent 2 calls get_observation -> sees advanced tick > 0
    let obs2: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret2}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "get_observation", "arguments": {} }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let obs2_json: Value =
        serde_json::from_str(obs2["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(
        obs2_json["tick"].as_u64().unwrap_or(0) > 0,
        "Late-join agent must observe advanced tick > 0"
    );

    // Agent 2 submits orders -> accepted & applied
    let sub2: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret2}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "submit_orders",
                "arguments": {
                    "strategyMemory": {
                        "phase": "Late-Join Takeover",
                        "priorities": ["Scout sector 0"],
                        "rationale": "Taking over from Governor autopilot",
                        "diplomacyNotes": "None",
                        "pivotTriggers": []
                    },
                    "commands": [{ "scoutSector": { "sectorId": 0 } }]
                }
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(sub2["result"]["isError"].as_bool(), Some(false));
    let sub2_json: Value =
        serde_json::from_str(sub2["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(sub2_json["accepted"].as_bool(), Some(true));

    state.shutdown_all().await;
}

#[tokio::test]
async fn e2e_zero_agent_seats_starts_immediately() {
    let srv = spawn_test_server().await;
    let created: Value = srv
        .client
        .post(format!("{}/api/v1/matches", srv.base_url))
        .json(&json!({
            "seed": 910003,
            "galaxySize": "small",
            "openAgentSeats": [],
            "lobbyTimeoutSec": 60,
            "speedMult": 5.0
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let match_id = created["matchId"].as_str().unwrap().to_string();
    assert_eq!(created["status"].as_str(), Some("Running"));
    assert_eq!(created["requiredAgentCount"].as_u64(), Some(0));
    assert!(created["lobbyRemainingSec"].is_null());

    tokio::time::sleep(Duration::from_millis(160)).await;
    let status: Value = srv
        .client
        .get(format!("{}/api/v1/matches/{match_id}", srv.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["status"].as_str(), Some("Running"));
    assert!(status["tick"].as_u64().unwrap_or(0) > 0);
}

#[tokio::test]
async fn e2e_diplomacy_1_based_empire_id_and_feedback_in_arena() {
    let srv = spawn_test_server().await;
    let created: Value = srv
        .client
        .post(format!("{}/api/v1/matches", srv.base_url))
        .json(&json!({
            "seed": 910099,
            "galaxySize": "small",
            "executionMode": "realtimeAsync",
            "openAgentSeats": [0, 2, 3],
            "lobbyTimeoutSec": 0,
            "speedMult": 1.0
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let match_id = created["matchId"].as_str().unwrap().to_string();
    let tok0 = created["empireTokens"]["0"].as_str().unwrap().to_string();
    let tok2 = created["empireTokens"]["2"].as_str().unwrap().to_string();
    let tok3 = created["empireTokens"]["3"].as_str().unwrap().to_string();

    // Seat 0 (Humanoids, empireId=1) targets empireId=3 (Insectoids, seat idx 2)
    // and also tests rejected targetEmp=0 (neutral) and targetEmp=1 (self).
    let cable = "E2E diplomatic check for Insectoids";
    let resp: Value = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/commands",
            srv.base_url
        ))
        .bearer_auth(&tok0)
        .json(&json!({
            "commands": [
                { "setDiplomacy": { "targetEmp": 3, "stance": "Ceasefire" } },
                { "sendDipMessage": { "toEmp": 3, "text": cable } },
                { "setDiplomacy": { "targetEmp": 0, "stance": "War" } },
                { "setDiplomacy": { "targetEmp": 1, "stance": "War" } }
            ]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let fb = resp["commandFeedback"].as_array().unwrap();
    assert_eq!(fb.len(), 4);
    assert_eq!(fb[0]["status"].as_str(), Some("applied"));
    assert!(
        fb[0]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("Insectoids")
            && fb[0]["reason"]
                .as_str()
                .unwrap_or("")
                .contains("empireId=3"),
        "Expected Insectoids (empireId=3) in feedback, got: {}",
        fb[0]
    );
    assert_eq!(fb[1]["status"].as_str(), Some("applied"));
    assert!(
        fb[1]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("Insectoids")
            && fb[1]["reason"]
                .as_str()
                .unwrap_or("")
                .contains("empireId=3"),
        "Expected Insectoids (empireId=3) in cable feedback, got: {}",
        fb[1]
    );
    assert_eq!(fb[2]["status"].as_str(), Some("rejected"));
    assert_eq!(fb[3]["status"].as_str(), Some("rejected"));

    // Insectoids (seat 2, empireId=3) must observe Ceasefire + cable from Humanoids (empId=1)
    let obs_insectoids: Value = srv
        .client
        .get(format!(
            "{}/api/v1/matches/{match_id}/observation",
            srv.base_url
        ))
        .bearer_auth(&tok2)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let known_i = obs_insectoids["observation"]["knownEmpires"]
        .as_array()
        .unwrap();
    let humanoids_entry = known_i
        .iter()
        .find(|k| k["empId"].as_u64() == Some(1))
        .expect("Insectoids must see Humanoids (empId=1)");
    assert_eq!(humanoids_entry["stance"].as_str(), Some("Ceasefire"));
    assert_eq!(humanoids_entry["inbox"][0]["text"].as_str(), Some(cable));

    // Nucleids (seat 3, empireId=4) must NOT receive the cable or Ceasefire
    let obs_nucleids: Value = srv
        .client
        .get(format!(
            "{}/api/v1/matches/{match_id}/observation",
            srv.base_url
        ))
        .bearer_auth(&tok3)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let empty_vec = Vec::new();
    let known_n = obs_nucleids["observation"]["knownEmpires"]
        .as_array()
        .unwrap_or(&empty_vec);
    assert!(
        known_n
            .iter()
            .all(|k| k["inbox"].as_array().map(|a| a.is_empty()).unwrap_or(true)),
        "Nucleids (empireId=4) must not receive Insectoids' cable: {:?}",
        known_n
    );
}

#[tokio::test]
async fn e2e_hud_diplomacy_matrix_and_mcp_transition_from_finished() {
    let secret = "seat_secret_transition_test_0123";
    let logs_dir =
        std::env::temp_dir().join(format!("sistars_e2e_matrix_{}", uuid::Uuid::new_v4()));
    let state = AppState::new_with_mcp(None, logs_dir, None, Some(&format!("claude-1:{secret}")));
    let router = build_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let base_url = format!("http://{addr}");
    let ws_base_url = format!("ws://{addr}");
    let client = reqwest::Client::new();
    let mcp_url = format!("{base_url}/mcp");

    // 1. Create Match A (maxYear=2400.2 so it finishes quickly) and connect Spectator WS
    let created_a: Value = client
        .post(format!("{base_url}/api/v1/matches"))
        .json(&json!({
            "seed": 920001,
            "galaxySize": "small",
            "executionMode": "realtimeAsync",
            "openAgentSeats": [0, 1],
            "lobbyTimeoutSec": 0,
            "speedMult": 5.0,
            "tickDelayMs": 10
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let match_a = created_a["matchId"].as_str().unwrap().to_string();
    let tok1 = created_a["empireTokens"]["1"].as_str().unwrap().to_string();

    // Connect Spectator WS and check init.hud.diplomacyMatrix
    let (mut ws_stream, _) = connect_async(format!("{ws_base_url}/ws/matches/{match_a}/spectate"))
        .await
        .expect("WS connect failed");
    let first_msg = ws_stream.next().await.unwrap().unwrap();
    let init_json: Value = serde_json::from_str(first_msg.to_text().unwrap()).unwrap();
    assert_eq!(init_json["type"].as_str(), Some("init"));
    let init_matrix = init_json["hud"]["diplomacyMatrix"].as_array().unwrap();
    assert_eq!(init_matrix.len(), 6);
    assert_eq!(init_matrix[1][1].as_str(), Some("Self"));
    assert_eq!(init_matrix[1][2].as_str(), Some("War"));

    // Submit SetDiplomacy(targetEmp=3, Ceasefire) from seat 1 and verify diplomacyMatrix updates
    let sub_res = client
        .post(format!("{base_url}/api/v1/matches/{match_a}/commands"))
        .bearer_auth(&tok1)
        .json(&json!({
            "commands": [
                { "setDiplomacy": { "targetEmp": 3, "stance": "Ceasefire" } }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(sub_res.status(), StatusCode::OK);

    let summary_a: Value = client
        .get(format!("{base_url}/api/v1/matches/{match_a}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        summary_a["diplomacyMatrix"][1][2].as_str(),
        Some("Ceasefire")
    );
    assert_eq!(
        summary_a["diplomacyMatrix"][2][1].as_str(),
        Some("Ceasefire")
    );

    // Join Match A with persistent MCP seat
    let join_a: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "join_match",
                "arguments": { "matchId": match_a, "empireIdx": 0 }
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(join_a["result"]["isError"].as_bool(), Some(false));

    // Mark Match A as Finished directly on the server handle to simulate match end
    {
        let matches = state.matches.read().await;
        let handle_a = matches.get(&match_a).unwrap().clone();
        let mut guard = handle_a.inner.write().await;
        guard.status = sistars_server::models::MatchStatus::Finished;
        guard.sim.winner = Some(0);
        guard.sim.victory_reason = Some("PlanetDominance(62% of inhabited planets)".to_string());
    }

    // Verify get_observation on Finished match returns helpful `hint`
    let obs_finished: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "get_observation", "arguments": {} }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let obs_fin_json: Value = serde_json::from_str(
        obs_finished["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(obs_fin_json["status"].as_str(), Some("Finished"));
    assert!(
        obs_fin_json["hint"]
            .as_str()
            .unwrap_or("")
            .contains("join_match"),
        "Finished observation must include transition hint, got: {}",
        obs_fin_json
    );

    // 2. Create Match B and call join_match WITHOUT calling leave_match first!
    let created_b: Value = client
        .post(format!("{base_url}/api/v1/matches"))
        .json(&json!({
            "seed": 920002,
            "galaxySize": "small",
            "executionMode": "realtimeAsync",
            "openAgentSeats": [0],
            "lobbyTimeoutSec": 60,
            "speedMult": 1.0
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let match_b = created_b["matchId"].as_str().unwrap().to_string();

    let join_b: Value = client
        .post(&mcp_url)
        .header("Authorization", format!("Bearer {secret}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "join_match",
                "arguments": { "matchId": match_b, "empireIdx": 0 }
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(join_b["result"]["isError"].as_bool(), Some(false));
    let join_b_json: Value =
        serde_json::from_str(join_b["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(join_b_json["joined"].as_bool(), Some(true));
    assert_eq!(join_b_json["matchId"].as_str(), Some(match_b.as_str()));

    state.shutdown_all().await;
}

#[tokio::test]
async fn e2e_admin_abort_ends_match_without_winner() {
    let srv = spawn_test_server().await;
    let created: Value = srv
        .client
        .post(format!("{}/api/v1/matches", srv.base_url))
        .json(&json!({
            "seed": 910201,
            "galaxySize": "small",
            "executionMode": "realtimeAsync",
            "openAgentSeats": [0],
            "lobbyTimeoutSec": 0,
            "speedMult": 5.0
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let match_id = created["matchId"].as_str().unwrap().to_string();
    let tok0 = created["empireTokens"]["0"].as_str().unwrap().to_string();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let aborted: Value = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/control",
            srv.base_url
        ))
        .json(&json!({ "abort": true }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(aborted["status"].as_str(), Some("Aborted"));
    assert_eq!(aborted["endReason"].as_str(), Some("aborted"));
    assert!(aborted["winner"].is_null());

    // The loop stops: the tick counter must freeze and the status must stay Aborted.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let s1: Value = srv
        .client
        .get(format!("{}/api/v1/matches/{match_id}", srv.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let s2: Value = srv
        .client
        .get(format!("{}/api/v1/matches/{match_id}", srv.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(s2["status"].as_str(), Some("Aborted"));
    assert!(s2["winner"].is_null());
    assert_eq!(
        s1["tick"], s2["tick"],
        "aborted match must not keep ticking"
    );

    // Agents can no longer submit orders.
    let resp = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/commands",
            srv.base_url
        ))
        .bearer_auth(&tok0)
        .json(&json!({ "commands": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"].as_str(), Some("match_aborted"));

    // A second abort is a no-op.
    let again: Value = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/control",
            srv.base_url
        ))
        .json(&json!({ "abort": true }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(again["status"].as_str(), Some("Aborted"));
}

#[tokio::test]
async fn e2e_abort_and_delete_require_admin_token() {
    let srv = spawn_test_server_with_admin(Some("s3cret".to_string())).await;
    let created: Value = srv
        .client
        .post(format!("{}/api/v1/matches", srv.base_url))
        .header("x-admin-token", "s3cret")
        .json(&json!({
            "seed": 910202,
            "galaxySize": "small",
            "openAgentSeats": [0],
            "lobbyTimeoutSec": 0
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let match_id = created["matchId"].as_str().unwrap().to_string();
    let tok0 = created["empireTokens"]["0"].as_str().unwrap().to_string();

    // Neither an anonymous caller nor an agent's empire token may abort or delete.
    for auth in [None, Some(tok0.as_str())] {
        let mut req = srv
            .client
            .post(format!(
                "{}/api/v1/matches/{match_id}/control",
                srv.base_url
            ))
            .json(&json!({ "abort": true }));
        if let Some(t) = auth {
            req = req.bearer_auth(t);
        }
        assert_eq!(req.send().await.unwrap().status(), StatusCode::UNAUTHORIZED);

        let mut del = srv
            .client
            .delete(format!("{}/api/v1/matches/{match_id}", srv.base_url));
        if let Some(t) = auth {
            del = del.bearer_auth(t);
        }
        assert_eq!(del.send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
    }

    let aborted: Value = srv
        .client
        .post(format!(
            "{}/api/v1/matches/{match_id}/control",
            srv.base_url
        ))
        .header("x-admin-token", "s3cret")
        .json(&json!({ "abort": true }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(aborted["status"].as_str(), Some("Aborted"));

    let del = srv
        .client
        .delete(format!("{}/api/v1/matches/{match_id}", srv.base_url))
        .bearer_auth("s3cret")
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
}
