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
                    "enum": ["Expand", "Fortify", "Attack", "Ignore"]
                },
                "HubKind": {
                    "type": "string",
                    "enum": ["Stargate", "Starbase"]
                },
                "FleetClass": {
                    "type": "string",
                    "enum": ["col", "cruiser", "battleship", "bomber", "freight"]
                },
                "EmpirePolicy": {
                    "type": "object",
                    "description": "Macro-economic, colonization, and military doctrine executed continuously by the empire's low-level Governor.",
                    "properties": {
                        "facShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "shipShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "techShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "mineShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "colPopReq": { "type": "number", "minimum": 0.05, "maximum": 0.90 },
                        "minShipyardPopRatio": { "type": "number", "minimum": 0.05, "maximum": 0.90 },
                        "distExp": { "type": "number" },
                        "qualExp": { "type": "number" },
                        "cruiserShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "bshipShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "bomberShare": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "interceptBias": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "techAlloc": {
                            "type": "array",
                            "items": { "type": "number" },
                            "minItems": 6,
                            "maxItems": 6,
                            "description": "Research allocation across [ENE, WEP, PRO, CON, ELE, BIO]."
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
                            "type": "array",
                            "items": { "type": "number" },
                            "minItems": 3,
                            "maxItems": 3
                        },
                        "minForceRatio": { "type": "number", "minimum": 0.75, "maximum": 4.0 },
                        "minGarrisonCount": { "type": "integer", "minimum": 0 }
                    },
                    "required": ["facShare", "shipShare", "techShare", "colPopReq", "distExp", "qualExp", "cruiserShare", "bshipShare", "bomberShare", "interceptBias", "techAlloc"]
                },
                "StrategicCommand": {
                    "description": "High-level strategic directive submitted by an AI agent at an epoch boundary.",
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {
                                "setPolicy": { "$ref": "#/components/schemas/EmpirePolicy" }
                            },
                            "required": ["setPolicy"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "setDiplomacy": {
                                    "type": "object",
                                    "properties": {
                                        "target_emp": { "type": "integer", "minimum": 0 },
                                        "stance": { "$ref": "#/components/schemas/DiplomaticStance" }
                                    },
                                    "required": ["target_emp", "stance"]
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
                                        "sector_id": { "type": "integer", "minimum": 0 },
                                        "mode": { "$ref": "#/components/schemas/SectorMode" }
                                    },
                                    "required": ["sector_id", "mode"]
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
                                        "planet_idx": { "type": "integer", "minimum": 0 },
                                        "kind": { "$ref": "#/components/schemas/HubKind" }
                                    },
                                    "required": ["planet_idx", "kind"]
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
                                        "to_emp": { "type": "integer", "minimum": 0 },
                                        "text": { "type": "string" }
                                    },
                                    "required": ["to_emp", "text"]
                                }
                            },
                            "required": ["sendDipMessage"]
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
                "AgentEmpireSummary": {
                    "type": "object",
                    "properties": {
                        "empireId": { "type": "integer" },
                        "name": { "type": "string" },
                        "prt": { "type": "string" },
                        "year": { "type": "number" },
                        "tick": { "type": "integer" },
                        "resPerYear": { "type": "number" },
                        "surfIron": { "type": "number" },
                        "surfBora": { "type": "number" },
                        "surfGerm": { "type": "number" },
                        "upkeepRatio": { "type": "number" },
                        "planetCount": { "type": "integer" },
                        "fleetCount": { "type": "integer" },
                        "orbitingFleetCount": { "type": "integer" },
                        "techLevels": {
                            "type": "array",
                            "items": { "type": "number" },
                            "minItems": 6,
                            "maxItems": 6
                        },
                        "policy": { "$ref": "#/components/schemas/EmpirePolicy" },
                        "winnerEmp": { "type": "integer", "nullable": true }
                    },
                    "required": ["empireId", "name", "prt", "year", "tick", "resPerYear", "planetCount", "fleetCount", "techLevels", "policy"]
                },
                "SectorSummary": {
                    "type": "object",
                    "properties": {
                        "sectorId": { "type": "integer" },
                        "row": { "type": "integer" },
                        "col": { "type": "integer" },
                        "ownPlanets": { "type": "integer" },
                        "ownResPerYear": { "type": "number" },
                        "ownFactories": { "type": "number" },
                        "unexploredStars": { "type": "integer" },
                        "neutralExploredPlanets": { "type": "integer" },
                        "foreignPlanetsByEmp": {
                            "type": "array",
                            "items": {
                                "type": "array",
                                "items": { "type": "integer" },
                                "minItems": 2,
                                "maxItems": 2
                            }
                        },
                        "avgIntelAgeYears": { "type": "number" },
                        "detectedEnemyFleetPower": { "type": "number" },
                        "directive": { "$ref": "#/components/schemas/SectorMode" }
                    },
                    "required": ["sectorId", "row", "col", "ownPlanets", "unexploredStars", "directive"]
                },
                "AgentKeyPoints": {
                    "type": "object",
                    "properties": {
                        "topOwnPlanets": { "type": "array", "items": { "type": "object" } },
                        "knownForeignPlanets": { "type": "array", "items": { "type": "object" } },
                        "knownStargates": { "type": "array", "items": { "type": "object" } },
                        "discoveredWormholes": { "type": "array", "items": { "type": "object" } },
                        "detectedEnemyFleets": { "type": "array", "items": { "type": "object" } },
                        "visibleDebris": { "type": "array", "items": { "type": "object" } }
                    },
                    "required": ["topOwnPlanets", "knownForeignPlanets", "knownStargates", "discoveredWormholes", "detectedEnemyFleets", "visibleDebris"]
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
                    "required": ["empId", "name", "stance", "knownPlanetsCount", "inbox"]
                },
                "AgentObservation": {
                    "type": "object",
                    "description": "Fog-of-War-isolated intelligence report for a single empire.",
                    "properties": {
                        "empire": { "$ref": "#/components/schemas/AgentEmpireSummary" },
                        "sectors": {
                            "type": "array",
                            "items": { "$ref": "#/components/schemas/SectorSummary" }
                        },
                        "keyPoints": { "$ref": "#/components/schemas/AgentKeyPoints" },
                        "knownEmpires": {
                            "type": "array",
                            "items": { "$ref": "#/components/schemas/KnownEmpireSummary" }
                        },
                        "recentAlerts": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    },
                    "required": ["empire", "sectors", "keyPoints", "knownEmpires", "recentAlerts"]
                },
                "ObservationEnvelope": {
                    "type": "object",
                    "properties": {
                        "matchId": { "type": "string" },
                        "epoch": { "type": "integer" },
                        "tick": { "type": "integer" },
                        "year": { "type": "number" },
                        "status": { "$ref": "#/components/schemas/MatchStatus" },
                        "deadlineMs": { "type": "integer", "nullable": true },
                        "alreadySubmitted": { "type": "boolean" },
                        "observation": { "$ref": "#/components/schemas/AgentObservation" }
                    },
                    "required": ["matchId", "epoch", "tick", "year", "status", "alreadySubmitted", "observation"]
                },
                "SubmitCommandsRequest": {
                    "type": "object",
                    "properties": {
                        "epoch": { "type": "integer", "nullable": true },
                        "commands": {
                            "type": "array",
                            "items": { "$ref": "#/components/schemas/StrategicCommand" }
                        },
                        "reasoningNote": {
                            "type": "string",
                            "nullable": true,
                            "description": "Private chain-of-thought or commentary visible only in match logs and Spectator Mode (never exposed to other agents)."
                        }
                    },
                    "required": ["commands"]
                },
                "CreateMatchRequest": {
                    "type": "object",
                    "properties": {
                        "seed": { "type": "integer", "nullable": true },
                        "galaxySize": { "type": "string", "nullable": true },
                        "galaxyShape": { "type": "string", "nullable": true },
                        "ticksPerEpoch": { "type": "integer", "default": 20 },
                        "epochTimeoutMs": { "type": "integer", "default": 20000 },
                        "agentSeats": {
                            "type": "array",
                            "items": { "type": "integer" }
                        },
                        "debugAudit": { "type": "boolean", "default": false },
                        "frameStrideTicks": { "type": "integer", "default": 2 },
                        "tickDelayMs": { "type": "integer", "default": 0 }
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
