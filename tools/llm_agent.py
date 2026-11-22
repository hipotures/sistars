#!/usr/bin/env python3
"""
SI-Stars Lightweight LLM Agent Bridge (standard library only).

Connects any OpenAI-compatible `/v1/chat/completions` endpoint (OpenAI, Gemini,
Claude proxy, local vLLM/Ollama/Qwen) to a live `sistars-server` Arena match,
or runs a deterministic smart heuristic bot via `--mock` for end-to-end testing.
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


def http_json(
    method: str,
    url: str,
    headers: dict[str, str],
    payload: dict[str, Any] | None = None,
    timeout: float = 15.0,
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
    cleaned = text.strip()
    fence = re.search(r"```(?:json)?\s*(\{.*?\})\s*```", cleaned, flags=re.DOTALL)
    if fence:
        cleaned = fence.group(1)
    start = cleaned.find("{")
    end = cleaned.rfind("}")
    if start == -1 or end == -1 or end <= start:
        raise ValueError("No JSON object found in model output")
    return json.loads(cleaned[start : end + 1])


def sanitize_commands(raw_cmds: Any, current_policy: dict[str, Any]) -> list[dict[str, Any]]:
    """Validates and filters StrategicCommand items to strictly match the server schema."""
    if not isinstance(raw_cmds, list):
        return []
    valid: list[dict[str, Any]] = []
    for item in raw_cmds[:12]:
        if not isinstance(item, dict) or len(item) != 1:
            continue
        key, val = next(iter(item.items()))
        if key == "setSectorDirective" and isinstance(val, dict):
            sec = val.get("sector_id", val.get("sectorId"))
            mode = val.get("mode")
            if isinstance(sec, int) and sec >= 0 and mode in VALID_SECTOR_MODES:
                valid.append({"setSectorDirective": {"sector_id": sec, "mode": mode}})
        elif key == "setDiplomacy" and isinstance(val, dict):
            target = val.get("target_emp", val.get("targetEmp"))
            stance = val.get("stance")
            if isinstance(target, int) and target >= 0 and stance in VALID_STANCES:
                valid.append({"setDiplomacy": {"target_emp": target, "stance": stance}})
        elif key == "sendDipMessage" and isinstance(val, dict):
            target = val.get("to_emp", val.get("toEmp"))
            text = val.get("text")
            if isinstance(target, int) and target >= 0 and isinstance(text, str) and text.strip():
                valid.append({"sendDipMessage": {"to_emp": target, "text": text.strip()[:280]}})
        elif key == "buildHub" and isinstance(val, dict):
            planet_idx = val.get("planet_idx", val.get("planetIdx"))
            kind = val.get("kind")
            if isinstance(planet_idx, int) and planet_idx >= 0 and kind in VALID_HUB_KINDS:
                valid.append({"buildHub": {"planet_idx": planet_idx, "kind": kind}})
        elif key == "setPolicy" and isinstance(val, dict):
            merged = dict(current_policy)
            for pk, pv in val.items():
                if pk in merged:
                    merged[pk] = pv
            valid.append({"setPolicy": merged})
    return valid


def decide_mock(envelope: dict[str, Any], goal: str) -> tuple[str, list[dict[str, Any]]]:
    """Deterministic smart heuristic agent requiring no API key."""
    epoch = int(envelope.get("epoch", 0))
    obs = envelope.get("observation", {})
    emp = obs.get("empire", {})
    policy = dict(emp.get("policy", {}))
    sectors = obs.get("sectors", [])
    known_empires = obs.get("knownEmpires", [])
    own_planets = obs.get("keyPoints", {}).get("topOwnPlanets", [])

    commands: list[dict[str, Any]] = []
    notes: list[str] = []

    # 1. Dynamic economy & military doctrine shift based on game phase and contact
    if epoch == 0:
        policy["facShare"] = 0.50
        policy["shipShare"] = 0.24
        policy["techShare"] = 0.16
        policy["colonizeVsAttackBias"] = 0.80
        policy["minForceRatio"] = 1.25
        commands.append({"setPolicy": policy})
        notes.append("Opening industrial boom & rapid expansion policy.")
    elif known_empires and epoch % 3 == 0:
        policy["facShare"] = 0.36
        policy["shipShare"] = 0.38
        policy["techShare"] = 0.16
        policy["colonizeVsAttackBias"] = 0.35
        policy["minForceRatio"] = 1.30
        commands.append({"setPolicy": policy})
        notes.append("Contacted rivals detected; shifting budget to fleet production.")

    # 2. Sector directives: Expand into high-unexplored sectors, Attack enemy-occupied sectors
    best_expand = None
    best_attack = None
    for sec in sectors:
        sec_id = int(sec.get("sectorId", 0))
        unexp = int(sec.get("unexploredStars", 0)) + int(sec.get("neutralExploredPlanets", 0))
        foreign = sum(int(pair[1]) for pair in sec.get("foreignPlanetsByEmp", []) if len(pair) == 2)
        if foreign > 0 and (best_attack is None or foreign > best_attack[1]):
            best_attack = (sec_id, foreign)
        if unexp > 0 and (best_expand is None or unexp > best_expand[1]):
            best_expand = (sec_id, unexp)

    if best_expand is not None:
        commands.append({"setSectorDirective": {"sector_id": best_expand[0], "mode": "Expand"}})
        notes.append(f"Prioritizing Sector #{best_expand[0]} for expansion ({best_expand[1]} targets).")
    if best_attack is not None:
        commands.append({"setSectorDirective": {"sector_id": best_attack[0], "mode": "Attack"}})
        notes.append(f"Directing offensive operations into Sector #{best_attack[0]} ({best_attack[1]} rival worlds).")

    # 3. Build Starbase on top industrial world if unfortified
    if own_planets and epoch % 4 == 1:
        top = own_planets[0]
        if int(top.get("starbaseTier", 0)) < 2 and float(top.get("factories", 0.0)) >= 15.0:
            p_idx = int(top["planetIdx"])
            commands.append({"buildHub": {"planet_idx": p_idx, "kind": "Starbase"}})
            notes.append(f"Commissioning Starbase hub at Planet #{p_idx + 1}.")

    # 4. Diplomacy & cables when rivals are known
    for rival in known_empires[:1]:
        rival_id = int(rival.get("empId", 1))
        rival_idx = max(0, rival_id - 1)
        stance = rival.get("stance", "War")
        if epoch == 1:
            commands.append(
                {
                    "sendDipMessage": {
                        "to_emp": rival_idx,
                        "text": f"Greetings from {emp.get('name', 'Empire')} in year {emp.get('year', 2400):.1f}. Respect our frontier sectors.",
                    }
                }
            )
            notes.append(f"Sent diplomatic hail to {rival.get('name', 'rival')} (stance: {stance}).")

    reasoning = f"[MockBot | Goal: {goal[:48]}] " + (" ".join(notes) if notes else "Holding steady on Governor autopilot.")
    return reasoning, commands


def decide_llm(
    envelope: dict[str, Any],
    base_url: str,
    api_key: str,
    model: str,
    goal: str,
) -> tuple[str, list[dict[str, Any]]]:
    obs = envelope.get("observation", {})
    current_policy = obs.get("empire", {}).get("policy", {})

    system_prompt = (
        "You are the Grand Strategist AI of an interstellar empire in SI-Stars (4X strategy game).\n"
        f"Strategic Goal: {goal}\n"
        "You receive a Fog-of-War-filtered JSON observation every macro-epoch (1 game year).\n"
        "Respond ONLY with a single JSON object matching this exact schema:\n"
        "{\n"
        '  "reasoning_note": "Short 1-2 sentence private strategic rationale",\n'
        '  "commands": [\n'
        '    {"setPolicy": {"facShare": 0.45, "shipShare": 0.30, "techShare": 0.15, "colonizeVsAttackBias": 0.5, "minForceRatio": 1.3}},\n'
        '    {"setSectorDirective": {"sector_id": 0, "mode": "Expand|Fortify|Attack|Ignore"}},\n'
        '    {"setDiplomacy": {"target_emp": 1, "stance": "War|Neutral|Ceasefire|Alliance"}},\n'
        '    {"sendDipMessage": {"to_emp": 1, "text": "Short diplomatic cable"}},\n'
        '    {"buildHub": {"planet_idx": 0, "kind": "Starbase|Stargate"}}\n'
        "  ]\n"
        "}\n"
        "Rules: Omit unchanged policy fields or omit setPolicy if current policy is good. "
        "Keep commands concise (0-5 commands per epoch). Never fabricate empire or sector IDs."
    )

    user_prompt = json.dumps(
        {
            "epoch": envelope.get("epoch"),
            "year": envelope.get("year"),
            "observation": obs,
        },
        separators=(",", ":"),
    )

    endpoint = base_url.rstrip("/") + "/chat/completions"
    headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
    payload = {
        "model": model,
        "temperature": 0.3,
        "response_format": {"type": "json_object"},
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_prompt},
        ],
    }

    status, body = http_json("POST", endpoint, headers, payload, timeout=18.0)
    if status != 200:
        raise RuntimeError(f"LLM API returned HTTP {status}: {body}")

    content = body["choices"][0]["message"]["content"]
    parsed = extract_json_object(content)
    note = str(parsed.get("reasoning_note", parsed.get("reasoningNote", ""))).strip()[:400]
    cmds = sanitize_commands(parsed.get("commands", []), current_policy)
    return note or f"LLM ({model}) epoch decision applied.", cmds


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
        default="Achieve 60% galactic dominance through balanced expansion, tech, and diplomacy.",
        help="Strategic goal injected into the system prompt",
    )
    parser.add_argument(
        "--mock",
        action="store_true",
        help="Run deterministic heuristic bot without calling an external LLM API",
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

    print(
        f"[llm_agent] Connected to {server} | match={args.match} | mode={'MOCK' if args.mock else args.model}",
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

        try:
            if args.mock:
                reasoning_note, commands = decide_mock(env, args.goal)
            else:
                reasoning_note, commands = decide_llm(
                    env, args.base_url, args.api_key, args.model, args.goal
                )
        except Exception as exc:
            print(
                f"[llm_agent] Epoch {epoch} decision fallback (error: {exc})",
                file=sys.stderr,
                flush=True,
            )
            reasoning_note = f"Fallback to Governor autopilot due to agent error: {exc}"
            commands = []

        try:
            post_status, post_body = http_json(
                "POST",
                cmd_url,
                auth_headers,
                {
                    "epoch": epoch,
                    "commands": commands,
                    "reasoningNote": reasoning_note,
                },
                timeout=10.0,
            )
            if post_status == 200:
                last_submitted_epoch = epoch
                epochs_handled += 1
                print(
                    f"[llm_agent] Epoch {epoch} (yr {year:.1f}): submitted {len(commands)} cmd(s) | {reasoning_note}",
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
