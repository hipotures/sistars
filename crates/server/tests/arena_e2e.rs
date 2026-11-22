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
    let logs_dir = std::env::temp_dir().join(format!(
        "sistars_test_logs_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let state = AppState::new(None, logs_dir);
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
        .get(format!("{}/api/v1/matches/{match1_id}/observation", srv.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(r_missing.status(), StatusCode::UNAUTHORIZED);

    // 2. Bogus token -> 401
    let r_bad = srv
        .client
        .get(format!("{}/api/v1/matches/{match1_id}/observation", srv.base_url))
        .bearer_auth("invalid_token_12345")
        .send()
        .await
        .unwrap();
    assert_eq!(r_bad.status(), StatusCode::UNAUTHORIZED);

    // 3. Valid token from Match 2 used against Match 1 -> 403 Forbidden
    let r_cross = srv
        .client
        .get(format!("{}/api/v1/matches/{match1_id}/observation", srv.base_url))
        .bearer_auth(&m2_tok0)
        .send()
        .await
        .unwrap();
    assert_eq!(r_cross.status(), StatusCode::FORBIDDEN);

    // 4. Token 0 returns strictly Empire 1 (seat 0) FoW; Token 1 returns strictly Empire 2 (seat 1) FoW
    let obs0: Value = srv
        .client
        .get(format!("{}/api/v1/matches/{match1_id}/observation", srv.base_url))
        .bearer_auth(&tok0)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let obs1: Value = srv
        .client
        .get(format!("{}/api/v1/matches/{match1_id}/observation", srv.base_url))
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
    assert_ne!(p0_idx, p1_idx, "Empires 0 and 1 must see their own distinct homeworlds");
    assert!(obs0["previousStrategy"].is_null(), "Epoch 0 previousStrategy must be null");
    assert!(obs0["delta"].is_null(), "Epoch 0 delta must be null");

    // 5. Private reasoning_note from Empire 0 MUST NEVER leak to Empire 1's observation
    let secret_note = "SECRET_OMEGA_BACKSTAB_PLAN_999";
    let sub0 = srv
        .client
        .post(format!("{}/api/v1/matches/{match1_id}/commands", srv.base_url))
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
        .post(format!("{}/api/v1/matches/{match1_id}/commands", srv.base_url))
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
        .get(format!("{}/api/v1/matches/{match1_id}/observation", srv.base_url))
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
        .post(format!("{}/api/v1/matches/{match_id}/commands", srv.base_url))
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
        .post(format!("{}/api/v1/matches/{match_id}/commands", srv.base_url))
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
        .post(format!("{}/api/v1/matches/{match_id}/commands", srv.base_url))
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
        .post(format!("{}/api/v1/matches/{match_id}/commands", srv.base_url))
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
                { "setDiplomacy": { "targetEmp": 1, "stance": "Ceasefire" } },
                { "sendDipMessage": { "toEmp": 1, "text": cable_text } }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res0.status(), StatusCode::OK);

    // Seat 1 sets a SectorDirective (camelCase)
    let res1 = srv
        .client
        .post(format!("{}/api/v1/matches/{match_id}/commands", srv.base_url))
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
        .get(format!("{}/api/v1/matches/{match_id}/observation", srv.base_url))
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
        .get(format!("{}/api/v1/matches/{match_id}/observation", srv.base_url))
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
        !serde_json::to_string(&obs1).unwrap().contains(secret_rationale),
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
                                if ev["strategy"]["rationale"].as_str() == Some(secret_rationale) =>
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

    assert!(saw_frame, "Spectator WS must receive live map render frames");
    assert!(
        saw_strategy,
        "Spectator WS must receive StrategyUpdate ArenaFeedEvent with full StrategyMemory"
    );
    assert!(saw_dip_msg, "Spectator WS must receive DipMessage ArenaFeedEvent");
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
        .get(format!("{}/api/v1/matches/{match_id}/system/0", srv.base_url))
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
        .get(format!("{}/api/v1/matches/{match_id}/planet/0", srv.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(planet["id"].as_u64(), Some(1));

    let reports: Value = srv
        .client
        .get(format!("{}/api/v1/matches/{match_id}/reports", srv.base_url))
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
            .post(format!("{}/api/v1/matches/{match_id}/commands", srv.base_url))
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
        .get(format!("{}/api/v1/matches/{match_id}/anomalies", srv.base_url))
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
    assert!(t1 > 0, "RealtimeAsync match must advance ticks without blocking on agents");

    // 2. Live pause via POST /control -> ticks MUST freeze
    let ctrl_pause: Value = srv
        .client
        .post(format!("{}/api/v1/matches/{match_id}/control", srv.base_url))
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
    assert_eq!(t_paused_a, t_paused_b, "Paused match must not advance ticks");

    // 3. stepTicks: 2 while paused -> advances by exactly 2 ticks and stays paused
    let _ = srv
        .client
        .post(format!("{}/api/v1/matches/{match_id}/control", srv.base_url))
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
        .post(format!("{}/api/v1/matches/{match_id}/control", srv.base_url))
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
        .post(format!("{}/api/v1/matches/{match_id}/commands", srv.base_url))
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
        .get(format!("{}/api/v1/matches/{match_id}/observation", srv.base_url))
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
    assert!((bias - 0.2).abs() < 0.02, "Expected patched bias 0.2, got {bias}");
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
        .post(format!("{}/api/v1/matches/{match_id}/commands", srv.base_url))
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
        .post(format!("{}/api/v1/matches/{match_id}/commands", srv.base_url))
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
        .get(format!("{}/api/v1/matches/{match_id}/observation", srv.base_url))
        .bearer_auth(&tok0)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let hub0 = &obs["observation"]["ownHubs"][0];
    assert!(hub0["maxPop"].as_f64().unwrap_or(0.0) > 10.0, "ownHubs[0] must include maxPop, got {hub0}");
    assert!(hub0["shipProdBuf"].is_number(), "ownHubs[0] must include shipProdBuf, got {hub0}");
    let pid = hub0["planetId"].as_u64().unwrap();

    // 2. Verify buildHub accepts BOTH `planetId` alias AND `planetIdx`
    let res = srv
        .client
        .post(format!("{}/api/v1/matches/{match_id}/commands", srv.base_url))
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
