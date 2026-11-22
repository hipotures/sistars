use serde_json::{json, Value};

/// Builds a complete OpenAPI 3.0.3 specification for the SI-Stars Arena Server,
/// including full schemas for `AgentObservation`, `EmpirePolicy`, and `StrategicCommand`.
pub fn build_openapi_spec() -> Value {
    json!({
        "openapi": "3.0.3",
        "info": {
            "title": "SI-Stars AI Arena API",
            "version": "1.0.0",
            "description": "Authoritative 4X strategy simulation arena server with Fog-of-War-isolated agent API, Macro-Epoch Barrier, and WebSocket Spectator Mode."
        },
        "servers": [
            { "url": "http://localhost:8080", "description": "Local Arena Server" }
        ],
        "components": {
            "securitySchemes": {
                "EmpireBearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "Secret per-empire token issued by POST /api/v1/matches. Hard-bound to a single (match_id, empire_idx)."
                },
                "AdminBearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "Optional admin token enforced only when SISTARS_ADMIN_TOKEN is set."
                }
            },
            "schemas": {
                "ExecutionMode": {
                    "type": "string",
                    "enum": ["realtimeAsync", "lockstepEpoch"],
                    "description": "realtimeAsync = smooth RTS with non-blocking async agents (default); lockstepEpoch = turn-based epoch barrier."
                },
                "MatchStatus": {
                    "type": "string",
                    "enum": ["WaitingForAgents", "Running", "Finished"]
                },
                "DiplomaticStance": {
                    "type": "string",
                    "enum": ["War", "Neutral", "Ceasefire", "Alliance"]
                },
                "SectorMode": {
                    "type": "string",
                    "enum": ["Balanced", "Expand", "Scout", "Fortify", "Attack", "Ignore"]
                },
                "HubKind": {
                    "type": "string",
                    "enum": ["Stargate", "Starbase"]
                },
                "FleetClass": {
                    "type": "string",
                    "enum": ["col", "cruiser", "battleship", "bomber", "freight"]
                },
                "NamedTechLevels": {
                    "type": "object",
                    "properties": {
                        "ene": { "type": "number" },
                        "wep": { "type": "number" },
                        "pro": { "type": "number" },
                        "con": { "type": "number" },
                        "ele": { "type": "number" },
                        "bio": { "type": "number" }
                    },
                    "required": ["ene", "wep", "pro", "con", "ele", "bio"]
                },
                "NamedMinerals": {
                    "type": "object",
                    "properties": {
                        "iron": { "type": "number" },
                        "bora": { "type": "number" },
                        "germ": { "type": "number" }
                    },
                    "required": ["iron", "bora", "germ"]
                },
                "AgentPolicyView": {
                    "type": "object",
                    "description": "Clean view of active strategic policy sliders executed continuously by the empire's low-level Governor.",
                    "properties": {
                        "facShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "shipShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "techShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "mineShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "colPopReq": { "type": "number", "minimum": 0.05, "maximum": 0.95 },
                        "minShipyardPopRatio": { "type": "number", "minimum": 0.05, "maximum": 0.95 },
                        "distExp": { "type": "number" },
                        "qualExp": { "type": "number" },
                        "cruiserShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "bshipShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "bomberShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "interceptBias": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "techAlloc": { "$ref": "#/components/schemas/NamedTechLevels" },
                        "surplusDefShare": { "type": "number" },
                        "surplusShipShare": { "type": "number" },
                        "surplusAlchShare": { "type": "number" },
                        "maxUpkeepRatio": { "type": "number" },
                        "freightExportThreshold": { "type": "number" },
                        "freightImportThreshold": { "type": "number" },
                        "colonizeVsAttackBias": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "minColonizeHab": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "mineralWeights": { "$ref": "#/components/schemas/NamedMinerals" },
                        "minForceRatio": { "type": "number", "minimum": 0.5, "maximum": 4.0 },
                        "minGarrisonCount": { "type": "integer", "minimum": 0 }
                    },
                    "required": ["facShare", "shipShare", "techShare", "mineShare", "cruiserShare", "bshipShare", "bomberShare", "techAlloc", "mineralWeights"]
                },
                "AgentPolicyPatch": {
                    "type": "object",
                    "description": "Partial merge patch for active empire policy. Omitted fields remain unchanged.",
                    "properties": {
                        "facShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "shipShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "techShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "mineShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "colPopReq": { "type": "number", "minimum": 0.05, "maximum": 0.95 },
                        "minShipyardPopRatio": { "type": "number", "minimum": 0.05, "maximum": 0.95 },
                        "distExp": { "type": "number" },
                        "qualExp": { "type": "number" },
                        "cruiserShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "bshipShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "bomberShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "interceptBias": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
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
                        "surplusDefShare": { "type": "number" },
                        "surplusShipShare": { "type": "number" },
                        "surplusAlchShare": { "type": "number" },
                        "maxUpkeepRatio": { "type": "number" },
                        "freightExportThreshold": { "type": "number" },
                        "freightImportThreshold": { "type": "number" },
                        "colonizeVsAttackBias": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "minColonizeHab": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "mineralWeights": {
                            "type": "object",
                            "properties": {
                                "iron": { "type": "number" },
                                "bora": { "type": "number" },
                                "germ": { "type": "number" }
                            }
                        },
                        "minForceRatio": { "type": "number", "minimum": 0.5, "maximum": 4.0 },
                        "minGarrisonCount": { "type": "integer", "minimum": 0 }
                    }
                },
                "StrategyMemory": {
                    "type": "object",
                    "description": "Private strategic continuity memory persisted server-side per empire across epochs.",
                    "properties": {
                        "phase": { "type": "string" },
                        "priorities": { "type": "array", "items": { "type": "string" } },
                        "rationale": { "type": "string" },
                        "diplomacyNotes": { "type": "string" },
                        "pivotTriggers": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["phase", "priorities", "rationale", "diplomacyNotes", "pivotTriggers"]
                },
                "EpochDelta": {
                    "type": "object",
                    "description": "Deterministic epoch-over-epoch change metrics computed server-side for the empire.",
                    "properties": {
                        "resPerYearDelta": { "type": "number" },
                        "planetsDelta": { "type": "integer" },
                        "fleetsDelta": { "type": "integer" },
                        "upkeepRatioDelta": { "type": "number" },
                        "mineralsDelta": { "$ref": "#/components/schemas/NamedMinerals" },
                        "techLevelsDelta": { "$ref": "#/components/schemas/NamedTechLevels" },
                        "newContacts": { "type": "array", "items": { "type": "integer" } },
                        "planetsGained": { "type": "array", "items": { "type": "integer" } },
                        "planetsLost": { "type": "array", "items": { "type": "integer" } },
                        "newMessagesCount": { "type": "integer" },
                        "newAlertsCount": { "type": "integer" }
                    },
                    "required": ["resPerYearDelta", "planetsDelta", "fleetsDelta", "upkeepRatioDelta", "mineralsDelta", "techLevelsDelta", "newMessagesCount", "newAlertsCount"]
                },
                "StrategicCommand": {
                    "description": "High-level strategic directive submitted by an AI agent at an epoch boundary (100% camelCase).",
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {
                                "setPolicy": { "$ref": "#/components/schemas/AgentPolicyPatch" }
                            },
                            "required": ["setPolicy"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "setDiplomacy": {
                                    "type": "object",
                                    "properties": {
                                        "targetEmp": { "type": "integer", "minimum": 0 },
                                        "stance": { "$ref": "#/components/schemas/DiplomaticStance" }
                                    },
                                    "required": ["targetEmp", "stance"]
                                }
                            },
                            "required": ["setDiplomacy"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "setSectorDirective": {
                                    "type": "object",
                                    "properties": {
                                        "sectorId": { "type": "integer", "minimum": 0 },
                                        "mode": { "$ref": "#/components/schemas/SectorMode" }
                                    },
                                    "required": ["sectorId", "mode"]
                                }
                            },
                            "required": ["setSectorDirective"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "buildHub": {
                                    "type": "object",
                                    "properties": {
                                        "planetIdx": {
                                            "type": "integer",
                                            "minimum": 0,
                                            "description": "Planet index (also accepts alias `planetId`)."
                                        },
                                        "kind": { "$ref": "#/components/schemas/HubKind" }
                                    },
                                    "required": ["planetIdx", "kind"]
                                }
                            },
                            "required": ["buildHub"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "sendDipMessage": {
                                    "type": "object",
                                    "properties": {
                                        "toEmp": { "type": "integer", "minimum": 0 },
                                        "text": { "type": "string" }
                                    },
                                    "required": ["toEmp", "text"]
                                }
                            },
                            "required": ["sendDipMessage"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "scoutSector": {
                                    "type": "object",
                                    "properties": {
                                        "sectorId": { "type": "integer", "minimum": 0 }
                                    },
                                    "required": ["sectorId"]
                                }
                            },
                            "required": ["scoutSector"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "probeWormhole": {
                                    "type": "object",
                                    "properties": {
                                        "wormholeId": { "type": "integer", "minimum": 0 }
                                    },
                                    "required": ["wormholeId"]
                                }
                            },
                            "required": ["probeWormhole"]
                        }
                    ]
                },
                "DipMessage": {
                    "type": "object",
                    "properties": {
                        "tick": { "type": "integer" },
                        "year": { "type": "number" },
                        "fromEmp": { "type": "integer" },
                        "toEmp": { "type": "integer" },
                        "text": { "type": "string" }
                    },
                    "required": ["tick", "year", "fromEmp", "toEmp", "text"]
                },
                "GalaxyOverview": {
                    "type": "object",
                    "properties": {
                        "totalPlanets": { "type": "integer" },
                        "gridRows": { "type": "integer" },
                        "gridCols": { "type": "integer" },
                        "exploredPlanets": { "type": "integer" },
                        "unexploredPlanets": { "type": "integer" },
                        "ownPlanets": { "type": "integer" },
                        "neutralKnownPlanets": { "type": "integer" },
                        "enemyKnownPlanets": { "type": "integer" },
                        "victoryGoal": {
                            "type": "object",
                            "properties": {
                                "planetDominancePct": { "type": "number" },
                                "resourceDominancePct": { "type": "number" },
                                "maxYear": { "type": "number" }
                            },
                            "required": ["planetDominancePct", "resourceDominancePct", "maxYear"]
                        }
                    },
                    "required": ["totalPlanets", "gridRows", "gridCols", "exploredPlanets", "unexploredPlanets", "ownPlanets", "neutralKnownPlanets", "enemyKnownPlanets", "victoryGoal"]
                },
                "AgentEmpireSummary": {
                    "type": "object",
                    "properties": {
                        "empireId": { "type": "integer" },
                        "name": { "type": "string" },
                        "prt": { "type": "string" },
                        "year": { "type": "number" },
                        "tick": { "type": "integer" },
                        "resPerYear": { "type": "number" },
                        "minerals": { "$ref": "#/components/schemas/NamedMinerals" },
                        "upkeepRatio": { "type": "number" },
                        "planetCount": { "type": "integer" },
                        "fleetCount": { "type": "integer" },
                        "orbitingFleetCount": { "type": "integer" },
                        "techLevels": { "$ref": "#/components/schemas/NamedTechLevels" },
                        "policy": { "$ref": "#/components/schemas/AgentPolicyView" },
                        "winnerEmp": { "type": "integer", "nullable": true }
                    },
                    "required": ["empireId", "name", "prt", "year", "tick", "resPerYear", "minerals", "planetCount", "fleetCount", "techLevels", "policy"]
                },
                "SectorSummary": {
                    "type": "object",
                    "properties": {
                        "sectorId": { "type": "integer" },
                        "row": { "type": "integer" },
                        "col": { "type": "integer" },
                        "directive": { "$ref": "#/components/schemas/SectorMode" },
                        "unexploredStars": { "type": "integer" },
                        "ownPlanets": { "type": "integer" },
                        "ownResPerYear": { "type": "number" },
                        "neutralKnown": { "type": "integer" },
                        "neutralPenScanned": { "type": "integer" },
                        "avgNeutralHab": { "type": "number", "nullable": true },
                        "enemyPlanetsByEmp": {
                            "type": "array",
                            "items": {
                                "type": "array",
                                "items": { "type": "integer" },
                                "minItems": 2,
                                "maxItems": 2
                            }
                        },
                        "detectedEnemyFleetPower": { "type": "number" },
                        "avgIntelAgeYears": { "type": "number" }
                    },
                    "required": ["sectorId", "row", "col", "directive"]
                },
                "KnownEmpireSummary": {
                    "type": "object",
                    "properties": {
                        "empId": { "type": "integer" },
                        "name": { "type": "string" },
                        "stance": { "$ref": "#/components/schemas/DiplomaticStance" },
                        "knownPlanetsCount": { "type": "integer" },
                        "estimatedFactories": { "type": "number" },
                        "estimatedPop": { "type": "number" },
                        "detectedFleetPower": { "type": "number" },
                        "inbox": {
                            "type": "array",
                            "items": { "$ref": "#/components/schemas/DipMessage" }
                        }
                    },
                    "required": ["empId", "name", "stance", "knownPlanetsCount"]
                },
                "AgentObservation": {
                    "type": "object",
                    "description": "Fog-of-War-isolated, topology-only intelligence report for a single empire (no raw x/y coordinates).",
                    "properties": {
                        "galaxyOverview": { "$ref": "#/components/schemas/GalaxyOverview" },
                        "empire": { "$ref": "#/components/schemas/AgentEmpireSummary" },
                        "sectors": {
                            "type": "array",
                            "items": { "$ref": "#/components/schemas/SectorSummary" }
                        },
                        "ownHubs": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "planetId": { "type": "integer" },
                                    "sectorId": { "type": "integer" },
                                    "pop": { "type": "number" },
                                    "maxPop": { "type": "number" },
                                    "factories": { "type": "number" },
                                    "defense": { "type": "number" },
                                    "starbaseTier": { "type": "integer" },
                                    "hasGate": { "type": "boolean" },
                                    "shipProdBuf": { "type": "number" },
                                    "minerals": { "$ref": "#/components/schemas/NamedMinerals" }
                                }
                            }
                        },
                        "ownFleetsSummary": { "type": "object" },
                        "pendingHubOrders": { "type": "array", "items": { "type": "object" } },
                        "activeExplorationOrders": { "type": "array", "items": { "type": "object" } },
                        "enemyPlanets": { "type": "array", "items": { "type": "object" } },
                        "wormholes": { "type": "array", "items": { "type": "object" } },
                        "stargates": { "type": "array", "items": { "type": "object" } },
                        "detectedEnemyFleets": { "type": "array", "items": { "type": "object" } },
                        "visibleDebris": { "type": "array", "items": { "type": "object" } },
                        "knownEmpires": {
                            "type": "array",
                            "items": { "$ref": "#/components/schemas/KnownEmpireSummary" }
                        },
                        "recentAlerts": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    },
                    "required": ["galaxyOverview", "empire", "sectors"]
                },
                "ObservationEnvelope": {
                    "type": "object",
                    "properties": {
                        "matchId": { "type": "string" },
                        "epoch": { "type": "integer" },
                        "tick": { "type": "integer" },
                        "year": { "type": "number" },
                        "status": { "$ref": "#/components/schemas/MatchStatus" },
                        "executionMode": { "$ref": "#/components/schemas/ExecutionMode" },
                        "paused": { "type": "boolean" },
                        "speedMult": { "type": "number" },
                        "epochTimeoutMs": { "type": "integer" },
                        "deadlineMs": { "type": "integer", "nullable": true },
                        "alreadySubmitted": { "type": "boolean" },
                        "previousStrategy": {
                            "nullable": true,
                            "allOf": [{ "$ref": "#/components/schemas/StrategyMemory" }]
                        },
                        "delta": {
                            "nullable": true,
                            "allOf": [{ "$ref": "#/components/schemas/EpochDelta" }]
                        },
                        "observation": { "$ref": "#/components/schemas/AgentObservation" }
                    },
                    "required": ["matchId", "epoch", "tick", "year", "status", "alreadySubmitted", "observation"]
                },
                "SubmitCommandsRequest": {
                    "type": "object",
                    "properties": {
                        "epoch": { "type": "integer", "nullable": true },
                        "strategyMemory": {
                            "nullable": true,
                            "allOf": [{ "$ref": "#/components/schemas/StrategyMemory" }]
                        },
                        "commands": {
                            "type": "array",
                            "items": { "$ref": "#/components/schemas/StrategicCommand" }
                        },
                        "reasoningNote": {
                            "type": "string",
                            "nullable": true,
                            "description": "Optional fallback commentary if strategyMemory is omitted."
                        }
                    },
                    "required": ["commands"]
                },
                "MatchControlRequest": {
                    "type": "object",
                    "description": "Live clock & execution control patch for a running Arena match.",
                    "properties": {
                        "paused": { "type": "boolean", "nullable": true },
                        "speedMult": { "type": "number", "minimum": 0.05, "maximum": 20.0, "nullable": true },
                        "executionMode": { "$ref": "#/components/schemas/ExecutionMode" },
                        "stepTicks": { "type": "integer", "minimum": 1, "maximum": 100, "nullable": true }
                    }
                },
                "CreateMatchRequest": {
                    "type": "object",
                    "properties": {
                        "seed": { "type": "integer", "nullable": true },
                        "galaxySize": { "type": "string", "nullable": true },
                        "galaxyShape": { "type": "string", "nullable": true },
                        "executionMode": { "$ref": "#/components/schemas/ExecutionMode" },
                        "speedMult": { "type": "number", "default": 1.0, "minimum": 0.05, "maximum": 20.0 },
                        "startPaused": { "type": "boolean", "default": false },
                        "ticksPerEpoch": { "type": "integer", "default": 20 },
                        "epochTimeoutMs": { "type": "integer", "default": 20000, "description": "0 = wait indefinitely (no timeout) in LockstepEpoch mode." },
                        "agentSeats": {
                            "type": "array",
                            "items": { "type": "integer" }
                        },
                        "debugAudit": { "type": "boolean", "default": false },
                        "frameStrideTicks": { "type": "integer", "default": 1 },
                        "tickDelayMs": { "type": "integer", "nullable": true }
                    }
                },
                "MatchPublicSummary": {
                    "type": "object",
                    "properties": {
                        "matchId": { "type": "string" },
                        "year": { "type": "number" },
                        "tick": { "type": "integer" },
                        "epoch": { "type": "integer" },
                        "status": { "$ref": "#/components/schemas/MatchStatus" },
                        "executionMode": { "$ref": "#/components/schemas/ExecutionMode" },
                        "paused": { "type": "boolean" },
                        "speedMult": { "type": "number" },
                        "winner": { "type": "integer", "nullable": true },
                        "planetCount": { "type": "integer" },
                        "empireCount": { "type": "integer" },
                        "agentSeats": { "type": "array", "items": { "type": "integer" } },
                        "submittedSeats": { "type": "array", "items": { "type": "integer" } },
                        "waitingForSeats": { "type": "array", "items": { "type": "integer" } },
                        "ticksPerEpoch": { "type": "integer" },
                        "epochTimeoutMs": { "type": "integer" },
                        "epochDeadlineMs": { "type": "integer", "nullable": true },
                        "debugAudit": { "type": "boolean" },
                        "anomaliesCount": { "type": "integer" }
                    },
                    "required": ["matchId", "year", "tick", "epoch", "status", "planetCount", "empireCount", "agentSeats"]
                }
            }
        },
        "paths": {
            "/openapi.json": {
                "get": {
                    "summary": "OpenAPI 3.0 specification",
                    "responses": { "200": { "description": "OpenAPI JSON document" } }
                }
            },
            "/mcp": {
                "post": {
                    "summary": "Persistent Model Context Protocol (MCP — Streamable HTTP) endpoint with header auth (Authorization: Bearer <token> or X-Api-Key)",
                    "description": "Exposes 6 tools: list_matches, join_match, leave_match, get_briefing, get_observation, submit_orders. Always returns 403 Forbidden (never 401) for invalid/missing tokens.",
                    "responses": {
                        "200": { "description": "JSON-RPC 2.0 response" },
                        "202": { "description": "Accepted JSON-RPC notification (empty body)" },
                        "403": { "description": "Missing or invalid token (no WWW-Authenticate header)" }
                    }
                }
            },
            "/mcp/{token}": {
                "post": {
                    "summary": "Model Context Protocol (MCP — Streamable HTTP) JSON-RPC 2.0 endpoint for Claude Custom Connectors",
                    "description": "Accepts per-match empire tokens or persistent seat tokens (SISTARS_MCP_SEATS). Exposes tools: get_briefing, get_observation, submit_orders. Returns 403 Forbidden (never 401) for invalid tokens and 202 Accepted for JSON-RPC notifications.",
                    "parameters": [{ "name": "token", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": {
                        "200": { "description": "JSON-RPC 2.0 response" },
                        "202": { "description": "Accepted JSON-RPC notification (empty body)" },
                        "403": { "description": "Invalid or unknown MCP token (no WWW-Authenticate header)" }
                    }
                },
                "delete": {
                    "summary": "Stateless MCP session termination",
                    "parameters": [{ "name": "token", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": { "204": { "description": "Session terminated" } }
                }
            },
            "/api/v1/mcp/seats": {
                "get": {
                    "summary": "List configured persistent MCP seats and their public binding status (zero secrets exposed)",
                    "responses": { "200": { "description": "Array of McpSeatPublicInfo" } }
                }
            },
            "/api/v1/mcp/seats/{seat_name}/bind": {
                "post": {
                    "summary": "Rebind a persistent MCP seat (configured via SISTARS_MCP_SEATS) to a live match and empire index",
                    "parameters": [{ "name": "seat_name", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": {
                        "200": { "description": "Seat bound to matchId and empireIdx" },
                        "404": { "description": "Seat or match not found" }
                    }
                }
            },
            "/api/v1/matches": {
                "get": {
                    "summary": "List active and finished matches (public metadata only)",
                    "responses": {
                        "200": {
                            "description": "Array of MatchPublicSummary",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "array",
                                        "items": { "$ref": "#/components/schemas/MatchPublicSummary" }
                                    }
                                }
                            }
                        }
                    }
                },
                "post": {
                    "summary": "Create a new Arena match and issue per-empire bearer tokens",
                    "security": [{ "AdminBearerAuth": [] }],
                    "requestBody": {
                        "required": false,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/CreateMatchRequest" }
                            }
                        }
                    },
                    "responses": {
                        "201": { "description": "Match created with empire_tokens and spectator_ws_url" },
                        "401": { "description": "Missing or invalid SISTARS_ADMIN_TOKEN" }
                    }
                }
            },
            "/api/v1/matches/{id}": {
                "get": {
                    "summary": "Get public match summary (no secret FoW data)",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": {
                        "200": {
                            "description": "MatchPublicSummary",
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/MatchPublicSummary" }
                                }
                            }
                        },
                        "404": { "description": "Match not found" }
                    }
                },
                "delete": {
                    "summary": "Shut down and delete a match from the server",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": {
                        "204": { "description": "Match deleted" },
                        "404": { "description": "Match not found" }
                    }
                }
            },
            "/api/v1/matches/{id}/control": {
                "post": {
                    "summary": "Live-control match pause, speedMult, executionMode, or single-step ticks",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/MatchControlRequest" }
                            }
                        }
                    },
                    "responses": {
                        "200": {
                            "description": "Updated MatchPublicSummary",
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/MatchPublicSummary" }
                                }
                            }
                        },
                        "404": { "description": "Match not found" }
                    }
                }
            },
            "/api/v1/matches/{id}/observation": {
                "get": {
                    "summary": "Get Fog-of-War-isolated AgentObservation for the empire bound to the bearer token",
                    "security": [{ "EmpireBearerAuth": [] }],
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": {
                        "200": {
                            "description": "ObservationEnvelope",
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/ObservationEnvelope" }
                                }
                            }
                        },
                        "401": { "description": "Missing or unknown bearer token" },
                        "403": { "description": "Token belongs to a different match" }
                    }
                }
            },
            "/api/v1/matches/{id}/commands": {
                "post": {
                    "summary": "Submit strategic commands and optional private reasoning_note for the current epoch",
                    "security": [{ "EmpireBearerAuth": [] }],
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/SubmitCommandsRequest" }
                            }
                        }
                    },
                    "responses": {
                        "200": { "description": "Commands queued for current epoch" },
                        "401": { "description": "Missing or unknown bearer token" },
                        "403": { "description": "Token belongs to a different match" },
                        "409": { "description": "Epoch mismatch or match not accepting commands" }
                    }
                }
            },
            "/api/v1/matches/{id}/system/{planet_idx}": {
                "get": {
                    "summary": "Spectator on-demand deterministic 3D star system view",
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "planet_idx", "in": "path", "required": true, "schema": { "type": "integer" } }
                    ],
                    "responses": { "200": { "description": "StarSystemView JSON" }, "404": { "description": "Not found" } }
                }
            },
            "/api/v1/matches/{id}/planet/{planet_idx}": {
                "get": {
                    "summary": "Spectator on-demand planet inspection view",
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "planet_idx", "in": "path", "required": true, "schema": { "type": "integer" } }
                    ],
                    "responses": { "200": { "description": "PlanetView JSON" }, "404": { "description": "Not found" } }
                }
            },
            "/api/v1/matches/{id}/planet/{planet_idx}/fleets": {
                "get": {
                    "summary": "Spectator on-demand list of fleets orbiting a planet",
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "planet_idx", "in": "path", "required": true, "schema": { "type": "integer" } }
                    ],
                    "responses": { "200": { "description": "Array of Fleet JSON" }, "404": { "description": "Not found" } }
                }
            },
            "/api/v1/matches/{id}/fleet/by-id/{fleet_id}": {
                "get": {
                    "summary": "Spectator on-demand fleet details by stable fleet ID",
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "fleet_id", "in": "path", "required": true, "schema": { "type": "integer" } }
                    ],
                    "responses": { "200": { "description": "Fleet JSON or null" }, "404": { "description": "Match not found" } }
                }
            },
            "/api/v1/matches/{id}/fleet/by-index/{idx}": {
                "get": {
                    "summary": "Spectator on-demand in-flight fleet details by render buffer index",
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "idx", "in": "path", "required": true, "schema": { "type": "integer" } }
                    ],
                    "responses": { "200": { "description": "Fleet JSON or null" }, "404": { "description": "Match not found" } }
                }
            },
            "/api/v1/matches/{id}/debris/{idx}": {
                "get": {
                    "summary": "Spectator on-demand debris field details",
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "idx", "in": "path", "required": true, "schema": { "type": "integer" } }
                    ],
                    "responses": { "200": { "description": "DebrisField JSON or null" }, "404": { "description": "Match not found" } }
                }
            },
            "/api/v1/matches/{id}/reports": {
                "get": {
                    "summary": "Spectator on-demand full ReportsSnapshot (planets, fleets, debris)",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": { "200": { "description": "ReportsSnapshot JSON" }, "404": { "description": "Match not found" } }
                }
            },
            "/api/v1/matches/{id}/power-history": {
                "get": {
                    "summary": "Spectator on-demand empire power history series",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": { "200": { "description": "2D array of empire Res/yr history" }, "404": { "description": "Match not found" } }
                }
            },
            "/api/v1/matches/{id}/export-state": {
                "get": {
                    "summary": "Spectator JSON export of config & races",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": { "200": { "description": "JSON state string" }, "404": { "description": "Match not found" } }
                }
            },
            "/api/v1/matches/{id}/anomalies": {
                "get": {
                    "summary": "LiveAuditor anomaly report for a match",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": { "200": { "description": "LiveAuditor anomalies JSON" }, "404": { "description": "Match not found" } }
                }
            },
            "/ws/matches/{id}/spectate": {
                "get": {
                    "summary": "WebSocket Spectator / God Mode stream (initial map, live frames, and ArenaFeedEvent stream)",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": { "101": { "description": "Switching Protocols (WebSocket)" } }
                }
            }
        }
    })
}
