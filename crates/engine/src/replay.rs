//! Match replay format and deterministic re-simulation.
//!
//! A replay is a JSONL stream (gzip-compressed on disk): one `header` line, then time-stamped
//! action lines, then one `footer` line. Only the initial state and actions are stored; the
//! engine is deterministic, so re-running the actions reproduces the match.
//!
//! On replay, agent and system inputs are injected at their recorded tick, while governor
//! actions and state hashes are regenerated and compared with the log: the first mismatch
//! pinpoints where the current code diverges from the recorded match.

use serde::{Deserialize, Serialize};

use crate::config::SimConfig;
use crate::governor::GovernorAction;
use crate::sim::Simulation;
use crate::types::{CommandFeedback, RaceDef, StrategicCommand, StrategyMemory, VictoryConfig};

pub const REPLAY_FORMAT_VERSION: u32 = 1;
/// A state hash line is recorded every this many ticks.
pub const HASH_INTERVAL_TICKS: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplaySrc {
    Agent,
    Governor,
    System,
}

/// Everything needed to rebuild the initial simulation state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayHeader {
    pub format: u32,
    pub match_id: String,
    /// Git commit of the build that recorded the match (for a faithful replay via checkout).
    pub engine_commit: String,
    pub created_at_ms: u64,
    pub seed: u32,
    pub sim_config: SimConfig,
    pub races: Vec<RaceDef>,
    pub galaxy_size: Option<String>,
    pub galaxy_shape: Option<String>,
    pub victory_config: VictoryConfig,
    /// `trace_enabled` changes scan cadence, so it must match the original run.
    pub debug_audit: bool,
    pub scenario_name: String,
    pub execution_mode: String,
    pub ticks_per_epoch: u32,
    pub open_agent_seats: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayFooter {
    /// `victory` | `aborted` | `shutdown` | `deleted`
    pub end_reason: String,
    pub winner: Option<usize>,
    pub victory_reason: Option<String>,
    pub final_tick: u64,
    pub final_year: f32,
    pub final_hash: String,
    pub action_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ReplayLine {
    Header(Box<ReplayHeader>),
    /// Agent orders as applied via `Simulation::submit_commands`, with the engine's feedback.
    Commands {
        t: u64,
        src: ReplaySrc,
        emp: usize,
        cmds: Vec<StrategicCommand>,
        fb: Vec<CommandFeedback>,
    },
    /// Agent reasoning / strategy text (not derivable; used for feed and video banners).
    Note {
        t: u64,
        src: ReplaySrc,
        emp: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        strategy: Option<StrategyMemory>,
    },
    /// New value of `Simulation::external_agent_mask` (agent join / leave).
    AgentMask {
        t: u64,
        src: ReplaySrc,
        mask: u16,
    },
    Governor {
        t: u64,
        src: ReplaySrc,
        emp: usize,
        a: GovernorAction,
    },
    /// `Simulation::state_hash` after the tick counter reached `t`, as 16 hex digits.
    Hash {
        t: u64,
        src: ReplaySrc,
        h: String,
    },
    Footer(Box<ReplayFooter>),
}

pub fn format_hash(h: u64) -> String {
    format!("{h:016x}")
}

/// Builds the initial simulation exactly as the arena server does for a new match.
pub fn build_simulation(header: &ReplayHeader) -> Simulation {
    let mut sim = Simulation::new(header.sim_config.clone(), header.races.clone());
    if header.galaxy_size.is_some() || header.galaxy_shape.is_some() {
        let cur_size = format!("{:?}", sim.cfg.galaxy_size).to_lowercase();
        let cur_shape = format!("{:?}", sim.cfg.galaxy_shape).to_lowercase();
        let size = header
            .galaxy_size
            .as_deref()
            .unwrap_or(&cur_size)
            .to_string();
        let shape = header
            .galaxy_shape
            .as_deref()
            .unwrap_or(&cur_shape)
            .to_string();
        sim.set_galaxy_options(&size, &shape);
    }
    sim.init(header.seed, false);
    sim.victory_cfg = header.victory_config;
    if header.debug_audit {
        sim.trace_enabled = true;
        sim.auditor.prune_valid_events = true;
    }
    sim
}

/// Collects replay lines during a live match.
#[derive(Debug, Default)]
pub struct ReplayRecorder {
    pub lines: Vec<ReplayLine>,
    pub action_count: u64,
}

impl ReplayRecorder {
    pub fn new(header: ReplayHeader) -> Self {
        Self {
            lines: vec![ReplayLine::Header(Box::new(header))],
            action_count: 0,
        }
    }

    pub fn push(&mut self, line: ReplayLine) {
        if !matches!(line, ReplayLine::Hash { .. }) {
            self.action_count += 1;
        }
        self.lines.push(line);
    }

    /// Call right after every `step_tick`: moves logged governor actions into the replay and
    /// records a state hash on hash-interval ticks.
    pub fn after_tick(&mut self, sim: &mut Simulation) {
        if let Some(log) = sim.action_log.as_mut() {
            for (t, emp, a) in log.drain(..) {
                self.push(ReplayLine::Governor {
                    t,
                    src: ReplaySrc::Governor,
                    emp,
                    a,
                });
            }
        }
        if sim.ticks % HASH_INTERVAL_TICKS == 0 {
            self.push(ReplayLine::Hash {
                t: sim.ticks,
                src: ReplaySrc::System,
                h: format_hash(sim.state_hash()),
            });
        }
    }

    pub fn finish(&mut self, sim: &Simulation, end_reason: &str) {
        let footer = ReplayFooter {
            end_reason: end_reason.to_string(),
            winner: if end_reason == "aborted" {
                None
            } else {
                sim.winner
            },
            victory_reason: sim.victory_reason.clone(),
            final_tick: sim.ticks,
            final_year: sim.year,
            final_hash: format_hash(sim.state_hash()),
            action_count: self.action_count,
        };
        self.lines.push(ReplayLine::Footer(Box::new(footer)));
    }

    pub fn to_jsonl(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            if let Ok(s) = serde_json::to_string(line) {
                out.push_str(&s);
                out.push('\n');
            }
        }
        out
    }
}

pub fn parse_jsonl(text: &str) -> Result<Vec<ReplayLine>, String> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("line {}: {e}", i + 1)))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Divergence {
    GovernorAction {
        t: u64,
        emp: usize,
        expected: Option<GovernorAction>,
        actual: Option<GovernorAction>,
    },
    Hash {
        t: u64,
        expected: String,
        actual: String,
    },
    FinalHash {
        t: u64,
        expected: String,
        actual: String,
    },
}

impl Divergence {
    pub fn tick(&self) -> u64 {
        match self {
            Divergence::GovernorAction { t, .. }
            | Divergence::Hash { t, .. }
            | Divergence::FinalHash { t, .. } => *t,
        }
    }

    /// One-line human-readable description (feed, UI, CLI).
    pub fn describe(&self) -> String {
        match self {
            Divergence::GovernorAction {
                t,
                emp,
                expected,
                actual,
            } => format!(
                "tick {t}, empire #{}: governor action differs (recorded {}, replayed {})",
                emp + 1,
                expected
                    .as_ref()
                    .map_or("nothing".to_string(), |a| format!("{a:?}")),
                actual
                    .as_ref()
                    .map_or("nothing".to_string(), |a| format!("{a:?}")),
            ),
            Divergence::Hash {
                t,
                expected,
                actual,
            } => format!(
                "state hash differs after tick {t} (recorded {expected}, replayed {actual})"
            ),
            Divergence::FinalHash {
                t,
                expected,
                actual,
            } => format!(
                "final state hash differs at tick {t} (recorded {expected}, replayed {actual})"
            ),
        }
    }
}

/// Steps through a replay alongside a simulation: injects recorded inputs before each tick and
/// compares regenerated governor actions / state hashes after it. After the first divergence
/// comparison stops, but inputs keep being injected ("what if" continuation).
pub struct ReplayCursor {
    lines: Vec<ReplayLine>,
    idx: usize,
    checking: bool,
    header: ReplayHeader,
    footer: Option<ReplayFooter>,
    total_ticks: u64,
    expected_gov: Vec<(usize, GovernorAction)>,
    expected_hash: Option<String>,
    pub divergence: Option<Divergence>,
    pub governor_actions_checked: u64,
    pub hashes_checked: u64,
}

impl ReplayCursor {
    pub fn new(lines: Vec<ReplayLine>) -> Result<Self, String> {
        let header = match lines.first() {
            Some(ReplayLine::Header(h)) => (**h).clone(),
            _ => return Err("replay does not start with a header line".to_string()),
        };
        let footer = lines.iter().rev().find_map(|l| match l {
            ReplayLine::Footer(f) => Some((**f).clone()),
            _ => None,
        });
        let total_ticks = footer
            .as_ref()
            .map(|f| f.final_tick)
            .or_else(|| lines.iter().filter_map(line_tick).max())
            .unwrap_or(0);
        Ok(Self {
            lines,
            idx: 1,
            checking: true,
            header,
            footer,
            total_ticks,
            expected_gov: Vec::new(),
            expected_hash: None,
            divergence: None,
            governor_actions_checked: 0,
            hashes_checked: 0,
        })
    }

    pub fn header(&self) -> &ReplayHeader {
        &self.header
    }

    pub fn footer(&self) -> Option<&ReplayFooter> {
        self.footer.as_ref()
    }

    /// Last tick of the recording (where playback ends).
    pub fn total_ticks(&self) -> u64 {
        self.total_ticks
    }

    /// Disables comparisons (e.g. when tracing was forced on and divergence is expected).
    pub fn set_checking(&mut self, on: bool) {
        self.checking = on;
    }

    /// Call before `step_tick`: applies agent orders and agent-mask changes recorded at the
    /// current tick and returns the input lines (Commands / Note / AgentMask) for feed display.
    /// Returned `Commands` lines carry the feedback of the current engine, not the recorded one.
    pub fn apply_inputs(&mut self, sim: &mut Simulation) -> Vec<ReplayLine> {
        let t = sim.ticks;
        let mut applied = Vec::new();
        self.expected_gov.clear();
        self.expected_hash = None;
        while self.idx < self.lines.len() {
            let line = &self.lines[self.idx];
            let Some(lt) = line_tick(line) else {
                self.idx += 1;
                continue;
            };
            // Governor/hash lines of tick t (hash stamped t+1) follow the inputs of tick t.
            if lt > t + 1 || (lt == t + 1 && !matches!(line, ReplayLine::Hash { .. })) {
                break;
            }
            if lt < t {
                // Inputs from ticks already passed (e.g. cursor created mid-way): skip.
                self.idx += 1;
                continue;
            }
            match line {
                ReplayLine::Commands { emp, cmds, src, .. } if lt == t => {
                    // Return the feedback the current engine gave (may differ after divergence).
                    let fb = sim.submit_commands(*emp, cmds.clone());
                    applied.push(ReplayLine::Commands {
                        t,
                        src: *src,
                        emp: *emp,
                        cmds: cmds.clone(),
                        fb,
                    });
                }
                ReplayLine::AgentMask { mask, .. } if lt == t => {
                    sim.external_agent_mask = *mask;
                    applied.push(line.clone());
                }
                ReplayLine::Note { .. } if lt == t => applied.push(line.clone()),
                ReplayLine::Governor { emp, a, .. } if lt == t => {
                    self.expected_gov.push((*emp, a.clone()));
                }
                ReplayLine::Hash { h, .. } if lt == t + 1 => {
                    self.expected_hash = Some(h.clone());
                }
                _ => {}
            }
            self.idx += 1;
        }
        applied
    }

    /// Call after `step_tick`: drains the simulation's governor action log and compares it and
    /// the recorded state hash. Returns the divergence the first time one is detected.
    pub fn check_after_tick(&mut self, sim: &mut Simulation) -> Option<Divergence> {
        let t = sim.ticks.saturating_sub(1);
        let actual_gov: Vec<(usize, GovernorAction)> = sim
            .action_log
            .as_mut()
            .map(|l| l.drain(..).map(|(_, e, a)| (e, a)).collect())
            .unwrap_or_default();
        if !self.checking {
            return None;
        }
        let n = self.expected_gov.len().max(actual_gov.len());
        for i in 0..n {
            let exp = self.expected_gov.get(i);
            let act = actual_gov.get(i);
            if exp != act {
                return self.diverged(Divergence::GovernorAction {
                    t,
                    emp: exp.or(act).map(|(e, _)| *e).unwrap_or(0),
                    expected: exp.map(|(_, a)| a.clone()),
                    actual: act.map(|(_, a)| a.clone()),
                });
            }
        }
        self.governor_actions_checked += actual_gov.len() as u64;
        if let Some(exp) = self.expected_hash.take() {
            let act = format_hash(sim.state_hash());
            self.hashes_checked += 1;
            if exp != act {
                return self.diverged(Divergence::Hash {
                    t: sim.ticks,
                    expected: exp,
                    actual: act,
                });
            }
        }
        if let Some(f) = self.footer.as_ref() {
            if sim.ticks == f.final_tick {
                let act = format_hash(sim.state_hash());
                if act != f.final_hash {
                    let expected = f.final_hash.clone();
                    return self.diverged(Divergence::FinalHash {
                        t: sim.ticks,
                        expected,
                        actual: act,
                    });
                }
            }
        }
        None
    }

    fn diverged(&mut self, d: Divergence) -> Option<Divergence> {
        self.checking = false;
        self.divergence = Some(d.clone());
        Some(d)
    }
}

#[derive(Debug)]
pub struct VerifyReport {
    pub header: ReplayHeader,
    pub footer: Option<ReplayFooter>,
    pub ticks_run: u64,
    pub governor_actions_checked: u64,
    pub hashes_checked: u64,
    pub divergence: Option<Divergence>,
    pub sim: Simulation,
}

/// Re-simulates a replay with the current engine. Stops at the first divergence, at
/// `until_tick`, or at the recorded final tick. `force_trace` enables tracing / LiveAuditor
/// even if the original match ran without it (this changes scan cadence, so divergence is
/// then expected and governor/hash checks are skipped).
pub fn verify(
    lines: &[ReplayLine],
    until_tick: Option<u64>,
    force_trace: bool,
) -> Result<VerifyReport, String> {
    let mut cursor = ReplayCursor::new(lines.to_vec())?;
    let header = cursor.header().clone();
    let mut sim = build_simulation(&header);
    if force_trace {
        sim.trace_enabled = true;
        sim.auditor.prune_valid_events = true;
        cursor.set_checking(header.debug_audit);
    }
    sim.action_log = Some(Vec::new());
    let stop = until_tick.map_or(cursor.total_ticks(), |u| u.min(cursor.total_ticks()));

    while sim.ticks < stop {
        cursor.apply_inputs(&mut sim);
        sim.step_tick(false);
        let _ = sim.drain_events();
        if cursor.check_after_tick(&mut sim).is_some() {
            break;
        }
    }

    Ok(VerifyReport {
        header,
        footer: cursor.footer().cloned(),
        ticks_run: sim.ticks,
        governor_actions_checked: cursor.governor_actions_checked,
        hashes_checked: cursor.hashes_checked,
        divergence: cursor.divergence.clone(),
        sim,
    })
}

fn line_tick(line: &ReplayLine) -> Option<u64> {
    match line {
        ReplayLine::Commands { t, .. }
        | ReplayLine::Note { t, .. }
        | ReplayLine::AgentMask { t, .. }
        | ReplayLine::Governor { t, .. }
        | ReplayLine::Hash { t, .. } => Some(*t),
        ReplayLine::Header(_) | ReplayLine::Footer(_) => None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn write_gz(path: &std::path::Path, recorder: &ReplayRecorder) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("gz.tmp");
    {
        let file = std::fs::File::create(&tmp)?;
        let mut enc = flate2::write::GzEncoder::new(
            std::io::BufWriter::new(file),
            flate2::Compression::default(),
        );
        for line in &recorder.lines {
            serde_json::to_writer(&mut enc, line)?;
            enc.write_all(b"\n")?;
        }
        enc.finish()?.flush()?;
    }
    std::fs::rename(tmp, path)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn read_file(path: &std::path::Path) -> Result<Vec<ReplayLine>, String> {
    use std::io::Read;
    let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = if raw.starts_with(&[0x1f, 0x8b]) {
        let mut s = String::new();
        flate2::read::MultiGzDecoder::new(&raw[..])
            .read_to_string(&mut s)
            .map_err(|e| format!("{}: gzip: {e}", path.display()))?;
        s
    } else {
        String::from_utf8(raw).map_err(|e| format!("{}: {e}", path.display()))?
    };
    parse_jsonl(&text)
}
