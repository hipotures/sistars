#!/usr/bin/env python3
"""
SI-Stars LLM Agent Bridge (standard library only).

Connects any OpenAI-compatible `/v1/chat/completions` endpoint (OpenAI, Gemini,
Claude proxy, local vLLM/Ollama/Qwen) to a live `sistars-server` Arena match,
supports `--mode stateless` (flat token cost + 100% system prefix caching),
`--mode thread --reset-every N` (multi-turn history caching with periodic reset),
`--manual` (interactive copy-paste chat mode), and `--mock` (deterministic bot).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from typing import Any


VALID_STANCES = {"War", "Neutral", "Ceasefire", "Alliance"}
VALID_SECTOR_MODES = {"Expand", "Fortify", "Attack", "Ignore"}
VALID_HUB_KINDS = {"Stargate", "Starbase"}

POLICY_FLOAT_FIELDS = {
    "facShare",
    "shipShare",
    "techShare",
    "mineShare",
    "colPopReq",
    "minShipyardPopRatio",
    "distExp",
    "qualExp",
    "cruiserShare",
    "bshipShare",
    "bomberShare",
    "interceptBias",
    "surplusDefShare",
    "surplusShipShare",
    "surplusAlchShare",
    "maxUpkeepRatio",
    "freightExportThreshold",
    "freightImportThreshold",
    "colonizeVsAttackBias",
    "minColonizeHab",
    "minForceRatio",
}
TECH_FIELDS = {"ene", "wep", "pro", "con", "ele", "bio"}
MINERAL_FIELDS = {"iron", "bora", "germ"}

PRT_DESCRIPTIONS: dict[str, str] = {
    "PRT_JOAT": (
        "Jack of All Trades (JOAT): Balanced generalist with built-in intrinsic scanners, "
        "higher max population (+20%), and no fatal weaknesses. Excels at adaptive macro pivots."
    ),
    "PRT_WM": (
        "War Monger (WM): +50% combat firepower and faster warships, but slower factory output "
        "and weaker planetary defenses. Must snowball through early military aggression."
    ),
    "PRT_HE": (
        "Hyper-Expansion (HE): Double population growth rate and cheap colonizers, but half max "
        "population per planet. Wins by flooding sectors rapidly and out-claiming neutrals."
    ),
    "PRT_IT": (
        "Interstellar Traveler (IT): Cheaper, longer-range Stargates with safe mass transit and "
        "wormhole stability. Dominates logistics and rapid multi-front force projection."
    ),
    "PRT_IS": (
        "Inner Strength (IS): Stronger planetary defenses, faster Starbase repair, and superior "
        "freighter/colonist resilience. Excels at fortifying chokepoints and winning wars of attrition."
    ),
    "PRT_SS": (
        "Super Stealth (SS): Intrinsic 75% cloaking on all ships, superior espionage/ECM, and "
        "safe ambush strikes. Excels at scouting deep behind Fog of War and surgical raids."
    ),
    "PRT_AR": (
        "Alternate Reality (AR): Orbital Starbase economy (no conventional planetary factories); "
        "population and Starbase tiers drive output. Protect orbital Starbases at all costs."
    ),
    "PRT_CA": (
        "Claim Adjuster (CA): Instant terraforming upon colonization and rapid habitability "
        "improvement. Turns marginal worlds into rich core hubs faster than any rival."
    ),
    "PRT_PP": (
        "Packet Physics (PP): Strong mineral logistics and long-range offensive bombardment "
        "synergy. Excels at industrial pressure and breaking fortified star systems."
    ),
    "PRT_SD": (
        "Space Demolition (SD): Area-denial and defensive fleet efficiency around owned hubs. "
        "Punishes overextended enemy offensives and locks down frontier sectors."
    ),
}


def build_system_prompt(empire_name: str, prt: str, goal: str, victory_goal: dict[str, Any]) -> str:
    """Builds a byte-stable SYSTEM_PROMPT once at startup for optimal provider prefix caching."""
    prt_desc = PRT_DESCRIPTIONS.get(
        prt,
        f"{prt}: Custom interstellar civilization; adapt economy, tech, and fleet doctrine to map conditions.",
    )
    p_dom = victory_goal.get("planetDominancePct", 60.0)
    r_dom = victory_goal.get("resourceDominancePct", 60.0)
    max_yr = victory_goal.get("maxYear", 2700.0)

    return (
        "You are the Grand Strategist AI commanding an interstellar empire in SI-Stars (4X strategy).\n"
        f"Empire: {empire_name} | Trait: {prt}\n"
        f"Racial Profile: {prt_desc}\n"
        f"Strategic Goal: {goal}\n"
        f"Victory Conditions: Control >= {p_dom}% of galaxy planets OR >= {r_dom}% of galaxy resources "
        f"(or lead at year {max_yr}).\n\n"
        "=== ARCHITECTURE & ROLE ===\n"
        "- Each macro-epoch equals 1 game year (default 20 simulation ticks).\n"
        "- A deterministic built-in Governor continuously handles all low-level micro (individual ship "
        "routing, freighters, colonizer dispatch, local combat, planetary build queues) strictly within "
        "your active policy sliders and sector directives.\n"
        "- You operate strictly at the Grand Strategy level: adjust policy sliders, assign sector "
        "directives, order key Starbase/Stargate hubs, and conduct diplomacy.\n"
        "- Fog of War is strictly enforced: you only see your own empire, explored sectors, scanned "
        "enemy assets, and diplomatic cables sent to you.\n\n"
        "=== OBSERVATION & POLICY GLOSSARY ===\n"
        "- galaxyOverview: totalPlanets, gridRows x gridCols sector grid, explored/unexplored counts, victoryGoal.\n"
        "- empire: resPerYear, minerals {iron, bora, germ}, upkeepRatio (fleet upkeep / resPerYear), "
        "planetCount, fleetCount, techLevels {ene, wep, pro, con, ele, bio}, policy.\n"
        "- sectors: non-empty grid sectors with sectorId, row, col, directive, unexploredStars, ownPlanets, "
        "ownResPerYear, neutralKnown, neutralPenScanned, avgNeutralHab (0.0-1.0), enemyPlanetsByEmp [[empId, count]], "
        "detectedEnemyFleetPower, avgIntelAgeYears.\n"
        "- ownHubs: top 10 own worlds (planetId, sectorId, pop, factories, defense, starbaseTier, hasGate, minerals).\n"
        "- enemyPlanets: strictly foreign planets (never neutrals) with planetId, sectorId, ownerEmp, starbaseTier, "
        "hasStargate, intelAgeYears (higher = staler Fog-of-War snapshot), penScanned (true = penetrating scanner "
        "revealed popEst/facEst/defEst; false = surface scanner only).\n"
        "- wormholes: [{wormholeId, fromSector, toSector}] where toSector is null until you discover the far endpoint.\n"
        "- detectedEnemyFleets: spotted enemy fleets with distFromNearestOwnLy (integer light-years).\n"
        "- Policy Sliders (setPolicy accepts PARTIAL patches — omit unchanged fields!):\n"
        "  * facShare, shipShare, techShare, mineShare: planetary budget shares (auto-normalized to sum to 1.0).\n"
        "  * colonizeVsAttackBias: 1.0 = max priority on colonizing neutrals, 0.0 = max priority on military war.\n"
        "  * minForceRatio: required fleet power advantage over known defense+Starbase before attacking "
        "(e.g. 1.25 = attack with +25% force advantage; range 0.5-4.0).\n"
        "  * cruiserShare, bshipShare, bomberShare: warship mix (Cruiser counters Bomber & intercepts; "
        "Battleship counters Cruiser; Bomber destroys planetary bunkers).\n"
        "  * techAlloc {ene, wep, pro, con, ele, bio}: research split across Energy, Weapons, Propulsion, "
        "Construction, Electronics, Biotechnology.\n"
        "  * mineralWeights {iron, bora, germ}: colonization/mining priority weights.\n"
        "  * maxUpkeepRatio: above this threshold, Governor pauses shipbuilding and scraps obsolete hulls.\n"
        "- Sector Directives (setSectorDirective): Expand (colonize/develop), Fortify (defend/garrison), "
        "Attack (offensive push), Ignore (deprioritize).\n"
        "- Diplomacy Stances (setDiplomacy): War, Neutral, Ceasefire, Alliance.\n\n"
        "=== REQUIRED JSON OUTPUT SCHEMA (STRICT camelCase) ===\n"
        "Respond ONLY with a single valid JSON object (no prose outside JSON):\n"
        "{\n"
        '  "strategyMemory": {\n'
        '    "phase": "Short phase label, e.g. Early Expansion | Fortify Frontier | Total War",\n'
        '    "priorities": ["1 to 4 concise operational priorities"],\n'
        '    "rationale": "1-3 sentences explaining WHY you chose this direction",\n'
        '    "diplomacyNotes": "Concise notes on neighbor relations and intentions",\n'
        '    "pivotTriggers": ["Concrete measurable triggers to pivot strategy in future epochs"]\n'
        "  },\n"
        '  "commands": [\n'
        '    {"setPolicy": {"facShare": 0.45, "shipShare": 0.30, "techShare": 0.15, "mineShare": 0.10, "colonizeVsAttackBias": 0.7, "minForceRatio": 1.25, "techAlloc": {"pro": 0.30, "ele": 0.25}}},\n'
        '    {"setSectorDirective": {"sectorId": 14, "mode": "Expand"}},\n'
        '    {"setDiplomacy": {"targetEmp": 2, "stance": "Neutral"}},\n'
        '    {"sendDipMessage": {"toEmp": 2, "text": "Short diplomatic cable"}},\n'
        '    {"buildHub": {"planetIdx": 0, "kind": "Stargate"}}\n'
        "  ]\n"
        "}\n"
        "Rules:\n"
        "1. Always return strategyMemory (it is persisted server-side and returned to you next epoch as previousStrategy).\n"
        "2. In setPolicy, include ONLY fields you want to change (or omit setPolicy completely if current policy is optimal).\n"
        "3. Keep commands concise (0-6 commands per epoch). Never fabricate sectorId, planetIdx, or empireId values."
    )


def build_epoch_payload(envelope: dict[str, Any]) -> dict[str, Any]:
    """Builds the compact per-epoch user message (no repeated rules or glossary)."""
    return {
        "epoch": int(envelope.get("epoch", 0)),
        "year": envelope.get("year", 2400.0),
        "previousStrategy": envelope.get("previousStrategy"),
        "delta": envelope.get("delta"),
        "state": envelope.get("observation", {}),
    }


def http_json(
    method: str,
    url: str,
    headers: dict[str, str],
    payload: dict[str, Any] | None = None,
    timeout: float = 20.0,
) -> tuple[int, Any]:
    data = json.dumps(payload).encode("utf-8") if payload is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    for k, v in headers.items():
        req.add_header(k, v)
    if data is not None:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read().decode("utf-8")
            return resp.status, json.loads(raw) if raw else {}
    except urllib.error.HTTPError as err:
        raw = err.read().decode("utf-8", errors="replace")
        try:
            parsed = json.loads(raw) if raw else {"error": str(err)}
        except json.JSONDecodeError:
            parsed = {"error": raw}
        return err.code, parsed


def extract_json_object(text: str) -> dict[str, Any]:
    """Extracts the first balanced JSON object from raw text or markdown code fences."""
    cleaned = text.strip()
    fence = re.search(r"```(?:json)?\s*(\{.*?\})\s*```", cleaned, flags=re.DOTALL)
    if fence:
        cleaned = fence.group(1).strip()

    start = cleaned.find("{")
    if start == -1:
        raise ValueError("No '{' JSON object start found in input")

    depth = 0
    in_str = False
    escaped = False
    end = -1
    for i in range(start, len(cleaned)):
        ch = cleaned[i]
        if in_str:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == '"':
                in_str = False
        else:
            if ch == '"':
                in_str = True
            elif ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    end = i
                    break

    if end == -1:
        raise ValueError("Unbalanced JSON braces (missing closing '}')")
    obj = json.loads(cleaned[start : end + 1])
    if not isinstance(obj, dict):
        raise ValueError("Top-level JSON value must be an object")
    return obj


def is_balanced_json_present(text: str) -> bool:
    """Returns True once `text` contains a complete balanced top-level `{...}` object."""
    start = text.find("{")
    if start == -1:
        return False
    depth = 0
    in_str = False
    escaped = False
    for i in range(start, len(text)):
        ch = text[i]
        if in_str:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == '"':
                in_str = False
        else:
            if ch == '"':
                in_str = True
            elif ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    return True
    return False


def sanitize_policy_patch(raw_patch: Any) -> dict[str, Any]:
    if not isinstance(raw_patch, dict):
        return {}
    out: dict[str, Any] = {}
    for k, v in raw_patch.items():
        if k in POLICY_FLOAT_FIELDS and isinstance(v, (int, float)):
            out[k] = float(v)
        elif k == "minGarrisonCount" and isinstance(v, int) and v >= 0:
            out[k] = v
        elif k == "techAlloc" and isinstance(v, dict):
            ta = {tk: float(tv) for tk, tv in v.items() if tk in TECH_FIELDS and isinstance(tv, (int, float))}
            if ta:
                out["techAlloc"] = ta
        elif k == "mineralWeights" and isinstance(v, dict):
            mw = {mk: float(mv) for mk, mv in v.items() if mk in MINERAL_FIELDS and isinstance(mv, (int, float))}
            if mw:
                out["mineralWeights"] = mw
    return out


def sanitize_commands(raw_cmds: Any) -> list[dict[str, Any]]:
    """Validates and filters StrategicCommand items to strictly match the camelCase server schema."""
    if not isinstance(raw_cmds, list):
        return []
    valid: list[dict[str, Any]] = []
    for item in raw_cmds[:12]:
        if not isinstance(item, dict) or len(item) != 1:
            continue
        key, val = next(iter(item.items()))
        if key == "setSectorDirective" and isinstance(val, dict):
            sec = val.get("sectorId", val.get("sector_id"))
            mode = val.get("mode")
            if isinstance(sec, int) and sec >= 0 and mode in VALID_SECTOR_MODES:
                valid.append({"setSectorDirective": {"sectorId": sec, "mode": mode}})
        elif key == "setDiplomacy" and isinstance(val, dict):
            target = val.get("targetEmp", val.get("target_emp"))
            stance = val.get("stance")
            if isinstance(target, int) and target >= 0 and stance in VALID_STANCES:
                valid.append({"setDiplomacy": {"targetEmp": target, "stance": stance}})
        elif key == "sendDipMessage" and isinstance(val, dict):
            target = val.get("toEmp", val.get("to_emp"))
            text = val.get("text")
            if isinstance(target, int) and target >= 0 and isinstance(text, str) and text.strip():
                valid.append({"sendDipMessage": {"toEmp": target, "text": text.strip()[:280]}})
        elif key == "buildHub" and isinstance(val, dict):
            planet_idx = val.get("planetIdx", val.get("planet_idx", val.get("planetId")))
            kind = val.get("kind")
            if isinstance(planet_idx, int) and planet_idx >= 0 and kind in VALID_HUB_KINDS:
                valid.append({"buildHub": {"planetIdx": planet_idx, "kind": kind}})
        elif key == "setPolicy" and isinstance(val, dict):
            patch = sanitize_policy_patch(val)
            if patch:
                valid.append({"setPolicy": patch})
    return valid


def sanitize_strategy_memory(raw: Any, fallback_note: str = "") -> dict[str, Any]:
    if isinstance(raw, dict):
        phase = str(raw.get("phase", "")).strip()[:80] or "Active Operations"
        priorities_raw = raw.get("priorities", [])
        priorities = (
            [str(p).strip()[:140] for p in priorities_raw[:6] if str(p).strip()]
            if isinstance(priorities_raw, list)
            else []
        )
        rationale = str(raw.get("rationale", "")).strip()[:450]
        diplomacy_notes = str(
            raw.get("diplomacyNotes", raw.get("diplomacy_notes", ""))
        ).strip()[:300]
        triggers_raw = raw.get("pivotTriggers", raw.get("pivot_triggers", []))
        pivot_triggers = (
            [str(t).strip()[:160] for t in triggers_raw[:6] if str(t).strip()]
            if isinstance(triggers_raw, list)
            else []
        )
        return {
            "phase": phase,
            "priorities": priorities,
            "rationale": rationale or fallback_note or "Maintaining current strategic posture.",
            "diplomacyNotes": diplomacy_notes or "No diplomatic updates.",
            "pivotTriggers": pivot_triggers,
        }
    return {
        "phase": "Active Operations",
        "priorities": [],
        "rationale": fallback_note or "Maintaining current strategic posture.",
        "diplomacyNotes": "No diplomatic updates.",
        "pivotTriggers": [],
    }


def decide_mock(envelope: dict[str, Any], goal: str) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """Deterministic smart heuristic agent emitting valid StrategyMemory + camelCase commands."""
    epoch = int(envelope.get("epoch", 0))
    obs = envelope.get("observation", {})
    sectors = obs.get("sectors", [])
    known_empires = obs.get("knownEmpires", [])
    own_hubs = obs.get("ownHubs", [])
    enemy_fleets = obs.get("detectedEnemyFleets", [])

    commands: list[dict[str, Any]] = []
    priorities: list[str] = []
    pivot_triggers: list[str] = []

    if epoch == 0:
        phase = "Early Expansion"
        commands.append(
            {
                "setPolicy": {
                    "facShare": 0.50,
                    "shipShare": 0.24,
                    "techShare": 0.16,
                    "mineShare": 0.10,
                    "colonizeVsAttackBias": 0.78,
                    "minForceRatio": 1.25,
                    "techAlloc": {"pro": 0.28, "ele": 0.22, "ene": 0.18, "con": 0.14, "wep": 0.10, "bio": 0.08},
                }
            }
        )
        priorities.append("Rapidly colonize nearby neutral worlds")
        priorities.append("Push Propulsion and Electronics for range and scanners")
        pivot_triggers.append("Shift shipShare >= 0.36 when enemy fleets or borders are detected")
    elif known_empires or enemy_fleets:
        phase = "Contested Expansion"
        if epoch % 3 == 0:
            commands.append(
                {
                    "setPolicy": {
                        "facShare": 0.36,
                        "shipShare": 0.40,
                        "techShare": 0.16,
                        "mineShare": 0.08,
                        "colonizeVsAttackBias": 0.40,
                        "minForceRatio": 1.30,
                    }
                }
            )
        priorities.append("Build combat fleet advantage and secure frontier sectors")
        pivot_triggers.append("Switch to Total War if enemy fleet power exceeds 150")
    else:
        phase = "Economic Snowball"
        priorities.append("Expand into unexplored and neutral-rich sectors")
        pivot_triggers.append("Fortify border upon first foreign contact")

    # Pick best expansion or attack sector
    best_attack_sec = None
    best_expand_sec = None
    best_expand_score = -1
    for sec in sectors:
        sid = int(sec.get("sectorId", 0))
        enemy_pairs = sec.get("enemyPlanetsByEmp", [])
        enemy_cnt = sum(int(p[1]) for p in enemy_pairs if isinstance(p, list) and len(p) == 2)
        if enemy_cnt > 0 and best_attack_sec is None:
            best_attack_sec = (sid, enemy_cnt)
        exp_score = int(sec.get("neutralKnown", 0)) * 3 + int(sec.get("unexploredStars", 0))
        if exp_score > best_expand_score:
            best_expand_score = exp_score
            best_expand_sec = (sid, exp_score)

    if best_attack_sec is not None:
        commands.append({"setSectorDirective": {"sectorId": best_attack_sec[0], "mode": "Attack"}})
        priorities.append(f"Press offensive in Sector #{best_attack_sec[0]} ({best_attack_sec[1]} enemy worlds)")
    elif best_expand_sec is not None and best_expand_score > 0:
        commands.append({"setSectorDirective": {"sectorId": best_expand_sec[0], "mode": "Expand"}})
        priorities.append(f"Prioritize Sector #{best_expand_sec[0]} for colonization")

    # Hub construction on top own hub without starbase
    if epoch >= 1 and epoch % 4 == 0 and own_hubs:
        top_hub = own_hubs[0]
        pid = int(top_hub.get("planetId", 0))
        if int(top_hub.get("starbaseTier", 0)) == 0:
            commands.append({"buildHub": {"planetIdx": pid, "kind": "Starbase"}})
            priorities.append(f"Construct Starbase at Planet #{pid}")

    # Diplomacy on contact
    dip_notes = "No foreign empires detected yet."
    if known_empires:
        rival = known_empires[0]
        target_id = int(rival.get("empId", 0))
        rival_name = rival.get("name", f"Empire #{target_id}")
        dip_notes = f"Tracking {rival_name} (stance={rival.get('stance')}, planets={rival.get('knownPlanetsCount')})."
        if target_id > 0 and epoch % 4 == 1:
            stance = "Ceasefire" if epoch < 8 else "War"
            commands.append({"setDiplomacy": {"targetEmp": target_id, "stance": stance}})
            commands.append(
                {
                    "sendDipMessage": {
                        "toEmp": target_id,
                        "text": f"Epoch {epoch}: proposing {stance} posture with {rival_name}.",
                    }
                }
            )

    strategy_memory = {
        "phase": phase,
        "priorities": priorities[:4],
        "rationale": f"[MockBot | {goal[:44]}] Executing {phase.lower()} with {len(commands)} strategic directive(s).",
        "diplomacyNotes": dip_notes,
        "pivotTriggers": pivot_triggers[:4],
    }
    return strategy_memory, commands


def decide_manual(
    envelope: dict[str, Any],
    system_prompt: str,
    printed_system_once: bool,
) -> tuple[dict[str, Any] | None, list[dict[str, Any]]]:
    """Interactive copy-paste chat mode with `full` and `skip` commands & resilient JSON parser."""
    epoch = int(envelope.get("epoch", 0))
    year = float(envelope.get("year", 2400.0))
    emp_name = envelope.get("observation", {}).get("empire", {}).get("name", "Empire")
    payload_str = json.dumps(build_epoch_payload(envelope), separators=(",", ":"))

    def print_package(include_system: bool) -> None:
        print("\n" + "=" * 76, flush=True)
        tag = "SYSTEM PROMPT + EPOCH JSON" if include_system else f"EPOCH {epoch} JSON"
        print(f"COPY BELOW TO YOUR LLM CHAT  |  {emp_name} | Epoch {epoch} (Year {year:.1f}) [{tag}]", flush=True)
        print("=" * 76, flush=True)
        if include_system:
            print(system_prompt, flush=True)
            print("\n=== CURRENT EPOCH DATA ===", flush=True)
        print(payload_str, flush=True)
        print("=" * 76, flush=True)
        print("END COPY", flush=True)
        print("=" * 76, flush=True)

    print_package(include_system=(not printed_system_once or epoch == 0))

    while True:
        print(
            "\nPaste LLM JSON response (auto-detects closing '}', or finish with empty line / END).\n"
            "Commands: type 'full' to reprint SYSTEM_PROMPT + Epoch JSON, or 'skip' / empty Enter to pass:",
            flush=True,
        )
        lines: list[str] = []
        while True:
            try:
                line = input()
            except EOFError:
                break
            stripped = line.strip()
            if not lines:
                if stripped.lower() == "full":
                    print_package(include_system=True)
                    lines = ["__REPROMPT__"]
                    break
                if stripped.lower() == "skip" or stripped == "":
                    lines = []
                    break
            if stripped == "END" or (stripped == "" and lines):
                break
            lines.append(line)
            joined = "\n".join(lines)
            if is_balanced_json_present(joined):
                # Auto-accept once balanced JSON is pasted (strip trailing code fence if any)
                break

        if lines == ["__REPROMPT__"]:
            continue

        raw = "\n".join(lines).strip()
        if not raw:
            prev = envelope.get("previousStrategy")
            return prev, []

        try:
            parsed = extract_json_object(raw)
            cmds = sanitize_commands(parsed.get("commands", []))
            fallback_note = str(parsed.get("reasoning_note", parsed.get("reasoningNote", ""))).strip()
            mem = sanitize_strategy_memory(
                parsed.get("strategyMemory", parsed.get("strategy_memory")),
                fallback_note=fallback_note,
            )
            return mem, cmds
        except Exception as exc:
            print(
                f"[llm_agent] JSON parse error: {exc}\n"
                "[llm_agent] Please paste a valid JSON object (or type 'skip' / 'full').",
                flush=True,
            )


def decide_llm(
    envelope: dict[str, Any],
    system_prompt: str,
    base_url: str,
    api_key: str,
    model: str,
    mode: str,
    reset_every: int,
    thread_messages: list[dict[str, str]],
    thread_turns: int,
) -> tuple[dict[str, Any], list[dict[str, Any]], list[dict[str, str]], int]:
    user_msg = json.dumps(build_epoch_payload(envelope), separators=(",", ":"))

    if mode == "thread":
        if not thread_messages or (reset_every > 0 and thread_turns >= reset_every):
            thread_messages = [{"role": "system", "content": system_prompt}]
            thread_turns = 0
        messages = [*thread_messages, {"role": "user", "content": user_msg}]
    else:
        messages = [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_msg},
        ]

    endpoint = base_url.rstrip("/") + "/chat/completions"
    headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
    payload = {
        "model": model,
        "temperature": 0.3,
        "response_format": {"type": "json_object"},
        "messages": messages,
    }

    status, body = http_json("POST", endpoint, headers, payload, timeout=25.0)
    if status != 200:
        raise RuntimeError(f"LLM API returned HTTP {status}: {body}")

    content = body["choices"][0]["message"]["content"]
    parsed = extract_json_object(content)
    fallback_note = str(parsed.get("reasoning_note", parsed.get("reasoningNote", ""))).strip()
    mem = sanitize_strategy_memory(
        parsed.get("strategyMemory", parsed.get("strategy_memory")),
        fallback_note=fallback_note or f"LLM ({model}) epoch decision applied.",
    )
    cmds = sanitize_commands(parsed.get("commands", []))

    if mode == "thread":
        thread_messages = [*messages, {"role": "assistant", "content": content}]
        thread_turns += 1

    return mem, cmds, thread_messages, thread_turns


def main() -> int:
    parser = argparse.ArgumentParser(description="SI-Stars LLM Agent Bridge")
    parser.add_argument("--server", default="http://127.0.0.1:8080", help="Arena server base URL")
    parser.add_argument("--match", required=True, help="Match ID")
    parser.add_argument("--token", required=True, help="Secret empire bearer token")
    parser.add_argument(
        "--base-url",
        default=os.environ.get("OPENAI_BASE_URL", "https://api.openai.com/v1"),
        help="OpenAI-compatible chat completions base URL",
    )
    parser.add_argument(
        "--api-key",
        default=os.environ.get("OPENAI_API_KEY", ""),
        help="API key for the LLM endpoint",
    )
    parser.add_argument("--model", default="gpt-4o-mini", help="Model name")
    parser.add_argument(
        "--goal",
        default="Achieve galactic dominance through balanced expansion, tech, and diplomacy.",
        help="Strategic goal injected once into SYSTEM_PROMPT",
    )
    parser.add_argument(
        "--mode",
        choices=["stateless", "thread"],
        default="stateless",
        help="API context mode: 'stateless' (prefix-cached system prompt, flat cost) or 'thread' (rolling history)",
    )
    parser.add_argument(
        "--reset-every",
        type=int,
        default=10,
        help="In --mode thread, reset conversation history every N epochs (preserving previousStrategy in Rust)",
    )
    parser.add_argument(
        "--mock",
        action="store_true",
        help="Run deterministic heuristic bot without calling an external LLM API",
    )
    parser.add_argument(
        "--manual",
        action="store_true",
        help="Interactive copy-paste mode: print SYSTEM_PROMPT + Epoch JSON and paste LLM reply back",
    )
    parser.add_argument(
        "--max-epochs",
        type=int,
        default=0,
        help="Optional maximum number of epochs to play before exiting (0 = play until match ends)",
    )
    parser.add_argument(
        "--poll-interval",
        type=float,
        default=0.25,
        help="Polling interval in seconds while waiting for the next epoch barrier",
    )
    args = parser.parse_args()

    server = args.server.rstrip("/")
    obs_url = f"{server}/api/v1/matches/{args.match}/observation"
    cmd_url = f"{server}/api/v1/matches/{args.match}/commands"
    auth_headers = {"Authorization": f"Bearer {args.token}"}

    epochs_handled = 0
    last_submitted_epoch: int | None = None
    system_prompt: str | None = None
    printed_system_once = False
    thread_messages: list[dict[str, str]] = []
    thread_turns = 0

    mode_label = (
        "MANUAL (copy/paste)"
        if args.manual
        else ("MOCK" if args.mock else f"{args.model} ({args.mode})")
    )
    print(
        f"[llm_agent] Connected to {server} | match={args.match} | mode={mode_label}",
        flush=True,
    )

    while True:
        try:
            status_code, env = http_json("GET", obs_url, auth_headers, timeout=10.0)
        except Exception as exc:
            print(f"[llm_agent] Network error fetching observation: {exc}", file=sys.stderr, flush=True)
            time.sleep(max(0.5, args.poll_interval))
            continue

        if status_code in (401, 403, 404):
            print(f"[llm_agent] Fatal HTTP {status_code}: {env}", file=sys.stderr, flush=True)
            return 1
        if status_code != 200:
            print(f"[llm_agent] Transient HTTP {status_code}: {env}", file=sys.stderr, flush=True)
            time.sleep(args.poll_interval)
            continue

        match_status = env.get("status")
        epoch = int(env.get("epoch", 0))
        year = float(env.get("year", 2400.0))
        already_submitted = bool(env.get("alreadySubmitted", False))

        if match_status == "Finished":
            winner = env.get("observation", {}).get("empire", {}).get("winnerEmp")
            print(
                f"[llm_agent] Match finished at epoch {epoch} (year {year:.1f}), winnerEmp={winner}",
                flush=True,
            )
            return 0

        if match_status not in ("WaitingForAgents", "Running") or already_submitted or last_submitted_epoch == epoch:
            time.sleep(args.poll_interval)
            continue

        if system_prompt is None:
            obs = env.get("observation", {})
            emp = obs.get("empire", {})
            vgoal = obs.get("galaxyOverview", {}).get("victoryGoal", {})
            system_prompt = build_system_prompt(
                str(emp.get("name", "Empire")),
                str(emp.get("prt", "PRT_JOAT")),
                args.goal,
                vgoal if isinstance(vgoal, dict) else {},
            )

        strategy_mem: dict[str, Any] | None = None
        reasoning_note: str | None = None
        try:
            if args.manual:
                strategy_mem, commands = decide_manual(env, system_prompt, printed_system_once)
                printed_system_once = True
            elif args.mock:
                strategy_mem, commands = decide_mock(env, args.goal)
            else:
                strategy_mem, commands, thread_messages, thread_turns = decide_llm(
                    env,
                    system_prompt,
                    args.base_url,
                    args.api_key,
                    args.model,
                    args.mode,
                    args.reset_every,
                    thread_messages,
                    thread_turns,
                )
        except Exception as exc:
            print(
                f"[llm_agent] Epoch {epoch} decision fallback (error: {exc})",
                file=sys.stderr,
                flush=True,
            )
            reasoning_note = f"Fallback to Governor autopilot due to agent error: {exc}"
            commands = []

        body_payload: dict[str, Any] = {
            "epoch": epoch,
            "commands": commands,
        }
        if strategy_mem is not None:
            body_payload["strategyMemory"] = strategy_mem
        elif reasoning_note is not None:
            body_payload["reasoningNote"] = reasoning_note

        try:
            post_status, post_body = http_json(
                "POST",
                cmd_url,
                auth_headers,
                body_payload,
                timeout=10.0,
            )
            if post_status == 200:
                last_submitted_epoch = epoch
                epochs_handled += 1
                summary_txt = (
                    f"[{strategy_mem.get('phase')}] {strategy_mem.get('rationale')}"
                    if strategy_mem
                    else (reasoning_note or "Pass")
                )
                print(
                    f"[llm_agent] Epoch {epoch} (yr {year:.1f}): submitted {len(commands)} cmd(s) | {summary_txt}",
                    flush=True,
                )
                if args.max_epochs > 0 and epochs_handled >= args.max_epochs:
                    print(
                        f"[llm_agent] Reached --max-epochs={args.max_epochs}; exiting cleanly.",
                        flush=True,
                    )
                    return 0
            else:
                print(
                    f"[llm_agent] POST /commands HTTP {post_status}: {post_body}",
                    file=sys.stderr,
                    flush=True,
                )
        except Exception as exc:
            print(f"[llm_agent] Network error posting commands: {exc}", file=sys.stderr, flush=True)

        time.sleep(args.poll_interval)


if __name__ == "__main__":
    raise SystemExit(main())
