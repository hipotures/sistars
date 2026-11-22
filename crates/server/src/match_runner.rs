use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rand::Rng;
use sistars_engine::config::SimConfig;
use sistars_engine::sim::Simulation;
use sistars_engine::tech::{classic_races, tech_store, LRT_LIST, PRT_LIST};
use sistars_engine::types::{StrategicCommand, TechCatalogView, VictoryConfig};
use tokio::sync::{broadcast, watch, Notify, RwLock};

use crate::models::{
    ArenaFeedEvent, ArenaFeedKind, CreateMatchRequest, EngineMetadata, MatchPublicSummary,
    MatchStatus, SpectatorWsMessage,
};

const MAX_FEED_HISTORY: usize = 250;

pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn generate_secret_token() -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 24] = rng.gen();
    let mut s = String::with_capacity(48);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

#[derive(Debug, Clone)]
pub struct QueuedDecision {
    pub commands: Vec<StrategicCommand>,
    pub reasoning_note: Option<String>,
}

pub struct MatchInner {
    pub match_id: String,
    pub sim: Simulation,
    pub status: MatchStatus,
    pub paused: bool,
    pub pending_step_ticks: u32,
    pub epoch: u64,
    pub ticks_per_epoch: u32,
    pub epoch_timeout_ms: u64,
    pub frame_stride_ticks: u32,
    pub tick_delay_ms: u64,
    pub agent_seats: Vec<usize>,
    pub pending_submissions: BTreeMap<usize, QueuedDecision>,
    pub epoch_deadline_ms: Option<u64>,
    pub feed_history: Vec<ArenaFeedEvent>,
    pub next_feed_id: u64,
    pub debug_audit: bool,
    pub audit_log_path: Option<PathBuf>,
    pub last_written_anomaly_idx: usize,
}

impl MatchInner {
    pub fn waiting_for_seats(&self) -> Vec<usize> {
        if self.status != MatchStatus::WaitingForAgents {
            return Vec::new();
        }
        self.agent_seats
            .iter()
            .copied()
            .filter(|seat| !self.pending_submissions.contains_key(seat))
            .collect()
    }

    pub fn submitted_seats(&self) -> Vec<usize> {
        self.pending_submissions.keys().copied().collect()
    }

    pub fn public_summary(&self) -> MatchPublicSummary {
        MatchPublicSummary {
            match_id: self.match_id.clone(),
            year: self.sim.year,
            tick: self.sim.ticks,
            epoch: self.epoch,
            status: self.status,
            paused: self.paused,
            tick_delay_ms: self.tick_delay_ms,
            winner: self.sim.winner,
            planet_count: self.sim.owner.len(),
            empire_count: self.sim.empires.len(),
            agent_seats: self.agent_seats.clone(),
            submitted_seats: self.submitted_seats(),
            waiting_for_seats: self.waiting_for_seats(),
            ticks_per_epoch: self.ticks_per_epoch,
            epoch_timeout_ms: self.epoch_timeout_ms,
            epoch_deadline_ms: self.epoch_deadline_ms,
            debug_audit: self.debug_audit,
            anomalies_count: self.sim.anomalies.len(),
        }
    }

    pub fn push_feed_event(
        &mut self,
        empire_idx: Option<usize>,
        kind: ArenaFeedKind,
        summary: String,
        detail: Option<String>,
    ) -> ArenaFeedEvent {
        let (empire_name, empire_color) = match empire_idx.and_then(|idx| self.sim.empires.get(idx))
        {
            Some(emp) => (emp.race.name.clone(), emp.race.color.clone()),
            None => ("ARENA".to_string(), "#38bdf8".to_string()),
        };
        let id = self.next_feed_id;
        self.next_feed_id += 1;
        let ev = ArenaFeedEvent {
            id,
            epoch: self.epoch,
            tick: self.sim.ticks,
            year: self.sim.year,
            empire_idx,
            empire_name,
            empire_color,
            kind,
            summary,
            detail,
        };
        self.feed_history.push(ev.clone());
        if self.feed_history.len() > MAX_FEED_HISTORY {
            self.feed_history.remove(0);
        }
        ev
    }

    pub fn build_init_ws_message(&mut self) -> SpectatorWsMessage {
        let hud = self.sim.take_hud_snapshot();
        let store = tech_store();
        let catalog = TechCatalogView {
            entries: store.entries.clone(),
            prts: PRT_LIST.to_vec(),
            lrts: LRT_LIST.to_vec(),
            classic_races: classic_races(),
        };
        SpectatorWsMessage::Init {
            match_id: self.match_id.clone(),
            meta: EngineMetadata::for_world(self.sim.world_w, self.sim.world_h),
            catalog,
            config: self.sim.cfg.clone(),
            summary: self.public_summary(),
            hud,
            planets_buf: self.sim.planet_render_buf.clone(),
            fleets_buf: self.sim.fleet_render_buf.clone(),
            debris_buf: self.sim.debris_render_buf.clone(),
            feed_history: self.feed_history.clone(),
        }
    }

    pub fn build_frame_ws_message(&mut self, tps: f32) -> SpectatorWsMessage {
        let hud = self.sim.take_hud_snapshot();
        SpectatorWsMessage::Frame {
            epoch: self.epoch,
            tick: self.sim.ticks,
            status: self.status,
            winner: self.sim.winner,
            waiting_for_seats: self.waiting_for_seats(),
            deadline_ms: self.epoch_deadline_ms,
            hud,
            planets_buf: self.sim.planet_render_buf.clone(),
            fleets_buf: self.sim.fleet_render_buf.clone(),
            debris_buf: self.sim.debris_render_buf.clone(),
            tps,
        }
    }

    pub fn flush_live_auditor(&mut self) {
        if !self.debug_audit {
            return;
        }
        let _ = self.sim.drain_events();
        if self.sim.anomalies.len() > self.last_written_anomaly_idx {
            if let Some(ref path) = self.audit_log_path {
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
                    for anomaly in &self.sim.anomalies[self.last_written_anomaly_idx..] {
                        if let Ok(line) = serde_json::to_string(anomaly) {
                            let _ = writeln!(file, "{line}");
                        }
                    }
                }
            }
            self.last_written_anomaly_idx = self.sim.anomalies.len();
        }
    }
}

pub struct MatchHandle {
    pub inner: Arc<RwLock<MatchInner>>,
    pub barrier_notify: Arc<Notify>,
    pub control_notify: Arc<Notify>,
    pub ws_tx: broadcast::Sender<SpectatorWsMessage>,
    pub shutdown_tx: watch::Sender<bool>,
}

impl MatchHandle {
    pub fn spawn(
        match_id: String,
        req: CreateMatchRequest,
        logs_dir: PathBuf,
    ) -> (Arc<Self>, HashMap<usize, String>, Vec<usize>) {
        let mut cfg = req.sim_config.unwrap_or_else(SimConfig::default);
        if let Some(mods) = req.scenario_modifiers {
            cfg.scenario_modifiers = mods;
        }
        let races = req.races.unwrap_or_else(classic_races);
        let mut sim = Simulation::new(cfg, races);

        if req.galaxy_size.is_some() || req.galaxy_shape.is_some() {
            let cur_size = format!("{:?}", sim.cfg.galaxy_size).to_lowercase();
            let cur_shape = format!("{:?}", sim.cfg.galaxy_shape).to_lowercase();
            let size_str = req.galaxy_size.as_deref().unwrap_or(&cur_size);
            let shape_str = req.galaxy_shape.as_deref().unwrap_or(&cur_shape);
            sim.set_galaxy_options(size_str, shape_str);
        }

        let seed = req
            .seed
            .unwrap_or_else(|| rand::thread_rng().gen_range(100_000..999_999));
        sim.init(seed, false);

        sim.victory_cfg = req.victory_config.unwrap_or_else(VictoryConfig::default);

        let debug_audit = req.debug_audit.unwrap_or(false);
        let audit_log_path = if debug_audit {
            sim.trace_enabled = true;
            sim.auditor.prune_valid_events = true;
            Some(logs_dir.join(format!("match_{match_id}_anomalies.jsonl")))
        } else {
            None
        };

        sim.fill_render_buffers();

        let num_empires = sim.empires.len();
        let mut agent_seats: Vec<usize> = req
            .agent_seats
            .unwrap_or_default()
            .into_iter()
            .filter(|&s| s < num_empires)
            .collect();
        agent_seats.sort_unstable();
        agent_seats.dedup();

        // Issue unique bearer tokens for every empire seat (or specifically for agent_seats + all empires so admin can attach to any seat).
        let mut empire_tokens = HashMap::new();
        for seat in 0..num_empires {
            if agent_seats.is_empty() || agent_seats.contains(&seat) {
                empire_tokens.insert(seat, generate_secret_token());
            }
        }

        let ticks_per_epoch = req.ticks_per_epoch.unwrap_or(20).clamp(1, 500);
        let epoch_timeout_ms = req.epoch_timeout_ms.unwrap_or(20_000).clamp(50, 600_000);
        let frame_stride_ticks = req.frame_stride_ticks.unwrap_or(2).max(1);
        let tick_delay_ms = req.tick_delay_ms.unwrap_or(0).min(1_000);
        let start_paused = req.start_paused.unwrap_or(false);

        let initial_status = if agent_seats.is_empty() {
            MatchStatus::Running
        } else {
            MatchStatus::WaitingForAgents
        };
        // Epoch 0 never times out: when external agent seats are selected, the match waits
        // indefinitely for every selected manager to connect and submit its initial Epoch 0 direction.
        let initial_deadline = None;

        let inner = Arc::new(RwLock::new(MatchInner {
            match_id,
            sim,
            status: initial_status,
            paused: start_paused,
            pending_step_ticks: 0,
            epoch: 0,
            ticks_per_epoch,
            epoch_timeout_ms,
            frame_stride_ticks,
            tick_delay_ms,
            agent_seats: agent_seats.clone(),
            pending_submissions: BTreeMap::new(),
            epoch_deadline_ms: initial_deadline,
            feed_history: Vec::new(),
            next_feed_id: 1,
            debug_audit,
            audit_log_path,
            last_written_anomaly_idx: 0,
        }));

        let barrier_notify = Arc::new(Notify::new());
        let control_notify = Arc::new(Notify::new());
        let (ws_tx, _) = broadcast::channel(256);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let handle = Arc::new(Self {
            inner: inner.clone(),
            barrier_notify: barrier_notify.clone(),
            control_notify: control_notify.clone(),
            ws_tx: ws_tx.clone(),
            shutdown_tx,
        });

        tokio::spawn(run_match_loop(
            inner,
            barrier_notify,
            control_notify,
            ws_tx,
            shutdown_rx,
        ));

        (handle, empire_tokens, agent_seats)
    }
}

async fn run_match_loop(
    inner: Arc<RwLock<MatchInner>>,
    barrier_notify: Arc<Notify>,
    control_notify: Arc<Notify>,
    ws_tx: broadcast::Sender<SpectatorWsMessage>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        // If the match is paused (and no manual single-step is queued), stay completely frozen
        // before starting any epoch or agent timeout countdown.
        loop {
            let (is_paused, has_step, is_done) = {
                let guard = inner.read().await;
                (
                    guard.paused,
                    guard.pending_step_ticks > 0,
                    guard.sim.winner.is_some(),
                )
            };
            if !is_paused || has_step || is_done {
                break;
            }
            tokio::select! {
                _ = control_notify.notified() => {}
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        return;
                    }
                }
            }
        }

        // Phase A: Macro-Epoch Decision Barrier for external agent seats
        let (has_agents, wait_indefinitely, timeout_ms, already_finished) = {
            let mut guard = inner.write().await;
            if guard.sim.winner.is_some() {
                guard.status = MatchStatus::Finished;
                guard.epoch_deadline_ms = None;
                let summary = guard.public_summary();
                let _ = ws_tx.send(SpectatorWsMessage::Status { summary });
                (false, false, 0, true)
            } else if !guard.agent_seats.is_empty() {
                guard.status = MatchStatus::WaitingForAgents;
                let is_initial_connect = guard.epoch == 0;
                let timeout = guard.epoch_timeout_ms;
                if is_initial_connect {
                    // Never time out before initial connection & first direction acceptance on Epoch 0
                    guard.epoch_deadline_ms = None;
                } else if guard.epoch_deadline_ms.is_none() {
                    guard.epoch_deadline_ms = Some(now_unix_ms() + timeout);
                }
                let summary = guard.public_summary();
                let _ = ws_tx.send(SpectatorWsMessage::Status { summary });
                (true, is_initial_connect, timeout, false)
            } else {
                guard.status = MatchStatus::Running;
                guard.epoch_deadline_ms = None;
                (false, false, 0, false)
            }
        };

        if already_finished {
            break;
        }

        if has_agents {
            let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
            loop {
                let all_submitted = {
                    let guard = inner.read().await;
                    guard
                        .agent_seats
                        .iter()
                        .all(|s| guard.pending_submissions.contains_key(s))
                };
                if all_submitted {
                    break;
                }
                if wait_indefinitely {
                    tokio::select! {
                        _ = barrier_notify.notified() => {}
                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() {
                                return;
                            }
                        }
                    }
                } else {
                    tokio::select! {
                        _ = barrier_notify.notified() => {}
                        _ = tokio::time::sleep_until(deadline) => {
                            break;
                        }
                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() {
                                return;
                            }
                        }
                    }
                }
            }
        }

        if *shutdown_rx.borrow() {
            break;
        }

        // Phase B: Apply queued agent decisions & emit Spectator ArenaFeedEvents
        let (ticks_per_epoch, frame_stride, tick_delay_ms) = {
            let mut guard = inner.write().await;
            let submissions = std::mem::take(&mut guard.pending_submissions);
            let mut new_feed_events = Vec::new();

            for (emp_idx, decision) in submissions {
                if let Some(note) = decision.reasoning_note {
                    let trimmed = note.trim();
                    if !trimmed.is_empty() {
                        let ev = guard.push_feed_event(
                            Some(emp_idx),
                            ArenaFeedKind::Reasoning,
                            trimmed.to_string(),
                            None,
                        );
                        new_feed_events.push(ev);
                    }
                }

                for cmd in &decision.commands {
                    let ev = match cmd {
                        StrategicCommand::SetPolicy(pol) => guard.push_feed_event(
                            Some(emp_idx),
                            ArenaFeedKind::PolicyUpdate,
                            format!(
                                "Updated policy (fac {:.0}% / ship {:.0}% / tech {:.0}%, minForce {:.2}x)",
                                pol.fac_share * 100.0,
                                pol.ship_share * 100.0,
                                pol.tech_share * 100.0,
                                pol.min_force_ratio
                            ),
                            None,
                        ),
                        StrategicCommand::SetDiplomacy { target_emp, stance } => {
                            let target_name = guard
                                .sim
                                .empires
                                .get(*target_emp)
                                .or_else(|| target_emp.checked_sub(1).and_then(|i| guard.sim.empires.get(i)))
                                .map(|e| e.race.name.clone())
                                .unwrap_or_else(|| format!("Empire #{target_emp}"));
                            guard.push_feed_event(
                                Some(emp_idx),
                                ArenaFeedKind::DiplomacyChange,
                                format!("Set diplomatic stance with {target_name} -> {stance:?}"),
                                None,
                            )
                        }
                        StrategicCommand::SetSectorDirective { sector_id, mode } => {
                            guard.push_feed_event(
                                Some(emp_idx),
                                ArenaFeedKind::SectorDirective,
                                format!("Sector #{sector_id} directive -> {mode:?}"),
                                None,
                            )
                        }
                        StrategicCommand::BuildHub { planet_idx, kind } => guard.push_feed_event(
                            Some(emp_idx),
                            ArenaFeedKind::BuildHub,
                            format!("Ordered {kind:?} hub at Planet #{}", planet_idx + 1),
                            None,
                        ),
                        StrategicCommand::SendDipMessage { to_emp, text } => {
                            let target_name = guard
                                .sim
                                .empires
                                .get(*to_emp)
                                .or_else(|| to_emp.checked_sub(1).and_then(|i| guard.sim.empires.get(i)))
                                .map(|e| e.race.name.clone())
                                .unwrap_or_else(|| format!("Empire #{to_emp}"));
                            guard.push_feed_event(
                                Some(emp_idx),
                                ArenaFeedKind::DipMessage,
                                format!("Diplomatic cable to {target_name}: \"{text}\""),
                                None,
                            )
                        }
                    };
                    new_feed_events.push(ev);
                }

                guard.sim.submit_commands(emp_idx, decision.commands);
            }

            guard.status = MatchStatus::Running;
            guard.epoch_deadline_ms = None;
            let summary = guard.public_summary();

            for ev in new_feed_events {
                let _ = ws_tx.send(SpectatorWsMessage::Feed { event: ev });
            }
            let _ = ws_tx.send(SpectatorWsMessage::Status { summary });

            (
                guard.ticks_per_epoch,
                guard.frame_stride_ticks,
                guard.tick_delay_ms,
            )
        };

        // Phase C: Step simulation for `ticks_per_epoch` ticks (halting immediately on victory, honoring live pause/speed/step)
        let epoch_start = Instant::now();
        let mut last_ws_broadcast = Instant::now() - Duration::from_secs(1);
        let mut finished = false;
        let mut ticks_done = 0u32;

        for step_i in 0..ticks_per_epoch {
            if *shutdown_rx.borrow() {
                return;
            }

            // Honor server-side pause & single-step controls cleanly
            loop {
                let (is_paused, has_step) = {
                    let guard = inner.read().await;
                    (guard.paused, guard.pending_step_ticks > 0)
                };
                if !is_paused || has_step {
                    break;
                }
                tokio::select! {
                    _ = control_notify.notified() => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            return;
                        }
                    }
                }
            }

            let (frame_opt, cur_delay_ms) = {
                let mut guard = inner.write().await;
                let was_stepped = if guard.pending_step_ticks > 0 {
                    guard.pending_step_ticks -= 1;
                    true
                } else {
                    false
                };
                guard.sim.step_tick(false);
                ticks_done += 1;

                let cur_stride = guard.frame_stride_ticks.max(1);
                let cur_delay = guard.tick_delay_ms;
                let planet_count = guard.sim.owner.len();
                let min_ws_interval_ms: u64 = if planet_count >= 50_000 {
                    250
                } else if planet_count >= 5_000 {
                    80
                } else {
                    33
                };
                let ws_due = last_ws_broadcast.elapsed() >= Duration::from_millis(min_ws_interval_ms);
                let is_victory = guard.sim.winner.is_some();
                let is_last = step_i + 1 == ticks_per_epoch;
                let should_broadcast = was_stepped
                    || is_victory
                    || (ws_due && (is_last || ((step_i + 1) % cur_stride == 0)));

                if is_victory {
                    finished = true;
                    guard.status = MatchStatus::Finished;
                    let winner_idx = guard.sim.winner;
                    let winner_name = winner_idx
                        .and_then(|w| guard.sim.empires.get(w))
                        .map(|e| e.race.name.clone())
                        .unwrap_or_else(|| "Unknown".to_string());
                    let win_year = guard.sim.year;
                    let ev = guard.push_feed_event(
                        winner_idx,
                        ArenaFeedKind::MatchFinished,
                        format!("{winner_name} achieved galactic victory in year {win_year:.1}!"),
                        None,
                    );
                    let _ = ws_tx.send(SpectatorWsMessage::Feed { event: ev });
                }

                let frame = if should_broadcast && ws_tx.receiver_count() > 0 {
                    last_ws_broadcast = Instant::now();
                    let elapsed = epoch_start.elapsed().as_secs_f32().max(0.001);
                    let tps = if guard.paused {
                        0.0
                    } else {
                        ticks_done as f32 / elapsed
                    };
                    Some(guard.build_frame_ws_message(tps))
                } else {
                    None
                };
                (frame, cur_delay)
            };

            if let Some(frame) = frame_opt {
                let _ = ws_tx.send(frame);
            }

            if finished {
                break;
            }

            if cur_delay_ms > 0 {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(cur_delay_ms)) => {}
                    _ = control_notify.notified() => {}
                }
            } else if step_i % 4 == 3 {
                tokio::task::yield_now().await;
            }
        }
        let _ = (frame_stride, tick_delay_ms);

        // Phase D: Post-epoch LiveAuditor flush & epoch increment
        {
            let mut guard = inner.write().await;
            guard.flush_live_auditor();
            if finished || guard.sim.winner.is_some() {
                guard.status = MatchStatus::Finished;
                guard.epoch_deadline_ms = None;
                let summary = guard.public_summary();
                let _ = ws_tx.send(SpectatorWsMessage::Status { summary });
                break;
            }
            guard.epoch += 1;
            if !guard.agent_seats.is_empty() {
                guard.status = MatchStatus::WaitingForAgents;
                guard.epoch_deadline_ms = Some(now_unix_ms() + guard.epoch_timeout_ms);
            }
            let summary = guard.public_summary();
            let _ = ws_tx.send(SpectatorWsMessage::Status { summary });
        }

        // For autonomous (no-agent) matches with 0 tick delay, yield briefly between epochs
        if !has_agents && tick_delay_ms == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
