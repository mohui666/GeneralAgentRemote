use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::{
    EngineOutput, JsonlEngine, MockScenario, request_id, requested_method, scenario_from_env,
};

const DEFAULT_MODEL: &str = "mock-codex-dynamic";
const DEFAULT_EFFORT: &str = "high";
const MOCK_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4, 0,
    0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 207, 240, 31, 0, 3, 3, 2,
    0, 239, 191, 105, 189, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

#[derive(Debug, Clone)]
struct ThreadRecord {
    id: String,
    cwd: String,
    title: String,
    preview: String,
    model: String,
    effort: String,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct ActiveTurn {
    thread_id: String,
    turn_id: String,
    scenario: MockScenario,
}

#[derive(Debug, Clone)]
struct PendingApproval {
    request_id: Value,
}

/// Codex CLI 0.150.1 stable app-server JSONL engine.
#[derive(Debug, Clone)]
pub struct MockCodexAppServer {
    scenario: MockScenario,
    initialized: bool,
    next_thread: u64,
    next_turn: u64,
    next_approval: u64,
    threads: BTreeMap<String, ThreadRecord>,
    active_turn: Option<ActiveTurn>,
    pending_approval: Option<PendingApproval>,
    materialize_images: bool,
}

impl Default for MockCodexAppServer {
    fn default() -> Self {
        Self::new(MockScenario::Complete)
    }
}

impl MockCodexAppServer {
    pub fn new(scenario: MockScenario) -> Self {
        Self {
            scenario,
            initialized: false,
            next_thread: 1,
            next_turn: 1,
            next_approval: 1,
            threads: BTreeMap::new(),
            active_turn: None,
            pending_approval: None,
            materialize_images: false,
        }
    }

    pub fn from_env() -> Result<Self> {
        let materialize_images = match std::env::var("AGENT_REMOTE_MOCK_WRITE_IMAGE") {
            Ok(value) => value == "1" || value.eq_ignore_ascii_case("true"),
            Err(std::env::VarError::NotPresent) => true,
            Err(error) => {
                return Err(error).context("AGENT_REMOTE_MOCK_WRITE_IMAGE is not valid Unicode");
            }
        };
        Ok(Self::new(scenario_from_env()?).with_image_materialization(materialize_images))
    }

    pub fn with_image_materialization(mut self, enabled: bool) -> Self {
        self.materialize_images = enabled;
        self
    }

    pub fn seed_thread(&mut self, cwd: impl Into<String>, title: impl Into<String>) -> String {
        let id = format!("mock-codex-thread-{}", self.next_thread);
        self.next_thread += 1;
        let title = title.into();
        self.threads.insert(
            id.clone(),
            ThreadRecord {
                id: id.clone(),
                cwd: cwd.into(),
                preview: title.clone(),
                title,
                model: DEFAULT_MODEL.to_owned(),
                effort: DEFAULT_EFFORT.to_owned(),
                updated_at: 1_788_192_000,
            },
        );
        id
    }

    fn response(id: Value, result: Value) -> EngineOutput {
        EngineOutput::Json(json!({"id": id, "result": result}))
    }

    fn error(id: Value, code: i64, message: impl Into<String>) -> EngineOutput {
        EngineOutput::Json(json!({
            "id": id,
            "error": {"code": code, "message": message.into()}
        }))
    }

    fn notification(method: &str, params: Value) -> EngineOutput {
        EngineOutput::Json(json!({"method": method, "params": params}))
    }

    fn thread_json(&self, record: &ThreadRecord, _include_turns: bool) -> Value {
        let active = self
            .active_turn
            .as_ref()
            .is_some_and(|turn| turn.thread_id == record.id);
        let status = if active {
            let active_flags = if self.pending_approval.is_some() {
                json!(["waitingOnApproval"])
            } else {
                json!([])
            };
            json!({"type": "active", "activeFlags": active_flags})
        } else {
            json!({"type": "idle"})
        };
        json!({
            "id": record.id,
            "sessionId": record.id,
            "forkedFromId": null,
            "parentThreadId": null,
            "preview": record.preview,
            "ephemeral": false,
            "section": null,
            "sectionEnteredAt": null,
            "projectId": null,
            "modelProvider": "openai",
            "createdAt": 1_788_192_000,
            "updatedAt": record.updated_at,
            "recencyAt": record.updated_at,
            "status": status,
            "path": null,
            "cwd": record.cwd,
            "cliVersion": "0.150.1",
            "source": "appServer",
            "threadSource": null,
            "agentNickname": null,
            "agentRole": null,
            "gitInfo": null,
            "name": record.title,
            "turns": []
        })
    }

    fn turn_json(turn_id: &str, status: &str, error: Value) -> Value {
        json!({
            "id": turn_id,
            "items": [],
            "itemsView": "full",
            "status": status,
            "error": error,
            "startedAt": 1_788_192_000,
            "completedAt": if status == "inProgress" { Value::Null } else { json!(1_788_192_001) },
            "durationMs": if status == "inProgress" { Value::Null } else { json!(1000) }
        })
    }

    fn model_list() -> Value {
        json!({
            "data": [
                {
                    "id": DEFAULT_MODEL,
                    "model": DEFAULT_MODEL,
                    "upgrade": null,
                    "upgradeInfo": null,
                    "availabilityNux": null,
                    "displayName": "Mock Codex Dynamic",
                    "description": "Visible model supplied dynamically by the mock app-server",
                    "modelSpecialty": null,
                    "hidden": false,
                    "supportedReasoningEfforts": [
                        {"reasoningEffort": "minimal", "description": "Fast fixture response"},
                        {"reasoningEffort": "high", "description": "Detailed fixture response"},
                        {"reasoningEffort": "ultra", "description": "Maximum fixture response"}
                    ],
                    "defaultReasoningEffort": DEFAULT_EFFORT,
                    "inputModalities": ["text", "image"],
                    "supportsPersonality": false,
                    "multiAgentVersion": null,
                    "additionalSpeedTiers": [],
                    "serviceTiers": [],
                    "defaultServiceTier": null,
                    "isDefault": true
                },
                {
                    "id": "mock-codex-hidden",
                    "model": "mock-codex-hidden",
                    "upgrade": null,
                    "upgradeInfo": null,
                    "availabilityNux": null,
                    "displayName": "Mock Hidden",
                    "description": "Hidden fixture model",
                    "modelSpecialty": null,
                    "hidden": true,
                    "supportedReasoningEfforts": [
                        {"reasoningEffort": "medium", "description": "Hidden fixture effort"}
                    ],
                    "defaultReasoningEffort": "medium",
                    "inputModalities": ["text"],
                    "supportsPersonality": false,
                    "multiAgentVersion": null,
                    "additionalSpeedTiers": [],
                    "serviceTiers": [],
                    "defaultServiceTier": null,
                    "isDefault": false
                }
            ],
            "nextCursor": null
        })
    }

    fn validate_model_effort(
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Result<(), &'static str> {
        let model = model.unwrap_or(DEFAULT_MODEL);
        if model != DEFAULT_MODEL && model != "mock-codex-hidden" {
            return Err("unsupported mock Codex model");
        }
        let valid_effort = match model {
            DEFAULT_MODEL => matches!(effort, None | Some("minimal" | "high" | "ultra")),
            "mock-codex-hidden" => matches!(effort, None | Some("medium")),
            _ => false,
        };
        if !valid_effort {
            return Err("unsupported reasoning effort for mock Codex model");
        }
        Ok(())
    }

    fn start_or_resume_result(&self, record: &ThreadRecord) -> Value {
        json!({
            "thread": self.thread_json(record, false),
            "model": record.model,
            "modelProvider": "openai",
            "serviceTier": null,
            "cwd": record.cwd,
            "instructionSources": [],
            "approvalPolicy": "on-request",
            "approvalsReviewer": "user",
            "sandbox": {"type": "workspaceWrite", "writableRoots": [record.cwd], "networkAccess": false, "excludeTmpdirEnvVar": false, "excludeSlashTmp": false},
            "reasoningEffort": record.effort
        })
    }

    fn extract_prompt(message: &Value) -> String {
        message
            .pointer("/params/input")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|input| input.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|input| input.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn item_started(thread_id: &str, turn_id: &str, item: Value) -> EngineOutput {
        Self::notification(
            "item/started",
            json!({
                "threadId": thread_id,
                "turnId": turn_id,
                "item": item,
                "startedAtMs": 1_788_192_000_000_i64
            }),
        )
    }

    fn item_completed(thread_id: &str, turn_id: &str, item: Value) -> EngineOutput {
        Self::notification(
            "item/completed",
            json!({
                "threadId": thread_id,
                "turnId": turn_id,
                "item": item,
                "completedAtMs": 1_788_192_001_000_i64
            }),
        )
    }

    fn agent_item(id: &str, text: &str, phase: &str) -> Value {
        json!({
            "type": "agentMessage",
            "id": id,
            "text": text,
            "phase": phase,
            "memoryCitation": null,
            "delivery": null
        })
    }

    fn command_item(cwd: &str, status: &str, exit_code: Value, output: Value) -> Value {
        json!({
            "type": "commandExecution",
            "id": "mock-command-1",
            "pluginId": null,
            "scriptPath": null,
            "command": "cargo test --quiet",
            "cwd": cwd,
            "processId": "mock-process-1",
            "source": "agent",
            "status": status,
            "commandActions": [],
            "aggregatedOutput": output,
            "exitCode": exit_code,
            "durationMs": if status == "inProgress" { Value::Null } else { json!(12) }
        })
    }

    fn file_item(cwd: &str, status: &str) -> Value {
        let path = PathBuf::from(cwd).join("src").join("mock.rs");
        json!({
            "type": "fileChange",
            "id": "mock-file-1",
            "changes": [{"path": path, "kind": "update", "diff": "@@ -1 +1 @@\n-old\n+new\n"}],
            "status": status
        })
    }

    fn mcp_item(status: &str) -> Value {
        json!({
            "type": "mcpToolCall",
            "id": "mock-tool-1",
            "server": "mock-tools",
            "tool": "inspect_project",
            "status": status,
            "arguments": {"depth": 1},
            "appContext": null,
            "pluginId": null,
            "readOnlyHint": true,
            "result": null,
            "error": null,
            "durationMs": if status == "inProgress" { Value::Null } else { json!(5) }
        })
    }

    fn image_path(&self, cwd: &str) -> Result<PathBuf> {
        let path = PathBuf::from(cwd).join(".agent-remote-mock-output.png");
        if self.materialize_images {
            std::fs::write(&path, MOCK_PNG).with_context(|| {
                format!("failed to write mock Codex image at {}", path.display())
            })?;
        }
        Ok(path)
    }

    fn initial_turn_events(
        &mut self,
        request_id: Value,
        thread_id: String,
        prompt: String,
        scenario: MockScenario,
    ) -> Result<Vec<EngineOutput>> {
        let turn_id = format!("mock-codex-turn-{}", self.next_turn);
        self.next_turn += 1;
        self.active_turn = Some(ActiveTurn {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            scenario,
        });
        let cwd = self
            .threads
            .get(&thread_id)
            .context("mock Codex turn references an unknown thread")?
            .cwd
            .clone();

        let mut output = vec![
            Self::response(
                request_id,
                json!({"turn": Self::turn_json(&turn_id, "inProgress", Value::Null)}),
            ),
            Self::notification(
                "turn/started",
                json!({
                    "threadId": thread_id,
                    "turn": Self::turn_json(&turn_id, "inProgress", Value::Null)
                }),
            ),
        ];

        let user_item = json!({
            "type": "userMessage",
            "id": "mock-user-1",
            "clientId": null,
            "content": [{"type": "text", "text": prompt, "text_elements": []}]
        });
        output.push(Self::item_started(&thread_id, &turn_id, user_item.clone()));
        output.push(Self::item_completed(&thread_id, &turn_id, user_item));
        output.push(Self::notification(
            "turn/plan/updated",
            json!({
                "threadId": thread_id,
                "turnId": turn_id,
                "explanation": "Mock plan",
                "plan": [
                    {"step": "Inspect the project", "status": "completed"},
                    {"step": "Run the mock workflow", "status": "inProgress"}
                ]
            }),
        ));

        let commentary =
            Self::agent_item("mock-commentary-1", "Inspecting the project.", "commentary");
        output.push(Self::item_started(&thread_id, &turn_id, commentary.clone()));
        output.push(Self::notification(
            "item/agentMessage/delta",
            json!({"threadId": thread_id, "turnId": turn_id, "itemId": "mock-commentary-1", "delta": "Inspecting "}),
        ));
        output.push(Self::notification(
            "item/agentMessage/delta",
            json!({"threadId": thread_id, "turnId": turn_id, "itemId": "mock-commentary-1", "delta": "the project."}),
        ));
        output.push(Self::item_completed(&thread_id, &turn_id, commentary));

        let tool = Self::mcp_item("inProgress");
        output.push(Self::item_started(&thread_id, &turn_id, tool));
        output.push(Self::notification(
            "item/mcpToolCall/progress",
            json!({"threadId": thread_id, "turnId": turn_id, "itemId": "mock-tool-1", "message": "Project inspected"}),
        ));
        output.push(Self::item_completed(
            &thread_id,
            &turn_id,
            Self::mcp_item("completed"),
        ));

        output.push(Self::item_started(
            &thread_id,
            &turn_id,
            Self::command_item(&cwd, "inProgress", Value::Null, Value::Null),
        ));

        if scenario == MockScenario::Crash {
            output.push(Self::notification(
                "item/commandExecution/outputDelta",
                json!({"threadId": thread_id, "turnId": turn_id, "itemId": "mock-command-1", "delta": "mock process is about to crash\n"}),
            ));
            output.push(EngineOutput::Exit(86));
            return Ok(output);
        }

        if scenario == MockScenario::Approval {
            let approval_id = json!(format!("mock-codex-approval-{}", self.next_approval));
            self.next_approval += 1;
            self.pending_approval = Some(PendingApproval {
                request_id: approval_id.clone(),
            });
            output.push(EngineOutput::Json(json!({
                "method": "item/commandExecution/requestApproval",
                "id": approval_id,
                "params": {
                    "kind": "command",
                    "threadId": thread_id,
                    "turnId": turn_id,
                    "itemId": "mock-command-1",
                    "startedAtMs": 1_788_192_000_250_i64,
                    "environmentId": null,
                    "reason": "Run the mock verification command",
                    "command": "cargo test --quiet",
                    "cwd": cwd,
                    "commandActions": [],
                    "proposedExecpolicyAmendment": null,
                    "proposedNetworkPolicyAmendments": null
                }
            })));
            return Ok(output);
        }

        output.extend(self.finish_active_turn(true)?);
        Ok(output)
    }

    fn finish_active_turn(&mut self, approved: bool) -> Result<Vec<EngineOutput>> {
        let active = self
            .active_turn
            .clone()
            .context("mock Codex has no active turn to finish")?;
        let record = self
            .threads
            .get(&active.thread_id)
            .context("active mock Codex thread disappeared")?;
        let thread_id = &active.thread_id;
        let turn_id = &active.turn_id;
        let mut output = Vec::new();

        let (command_status, exit_code, command_output) = if approved {
            ("completed", json!(0), json!("1 mock test passed\n"))
        } else {
            ("declined", Value::Null, json!("Command declined by user\n"))
        };
        if approved {
            output.push(Self::notification(
                "item/commandExecution/outputDelta",
                json!({"threadId": thread_id, "turnId": turn_id, "itemId": "mock-command-1", "delta": "1 mock test passed\n"}),
            ));
        }
        output.push(Self::item_completed(
            thread_id,
            turn_id,
            Self::command_item(&record.cwd, command_status, exit_code, command_output),
        ));

        let file_started = Self::file_item(&record.cwd, "inProgress");
        let file_completed = Self::file_item(&record.cwd, "completed");
        let changes = file_completed
            .get("changes")
            .cloned()
            .unwrap_or_else(|| json!([]));
        output.push(Self::item_started(thread_id, turn_id, file_started));
        output.push(Self::notification(
            "item/fileChange/patchUpdated",
            json!({"threadId": thread_id, "turnId": turn_id, "itemId": "mock-file-1", "changes": changes}),
        ));
        output.push(Self::item_completed(thread_id, turn_id, file_completed));

        let image_path = self.image_path(&record.cwd)?;
        let image = json!({"type": "imageView", "id": "mock-image-1", "path": image_path});
        output.push(Self::item_started(thread_id, turn_id, image.clone()));
        output.push(Self::item_completed(thread_id, turn_id, image));

        if active.scenario == MockScenario::Unknown {
            output.push(EngineOutput::Json(json!({
                "method": "mock/futureEvent",
                "params": {"threadId": thread_id, "turnId": turn_id, "futureField": {"version": 2}},
                "unknownTopLevel": true
            })));
        }

        if active.scenario == MockScenario::Failure {
            let error = json!({
                "message": "mock Codex turn failed",
                "codexErrorInfo": "other",
                "additionalDetails": "failure requested by test scenario"
            });
            output.push(Self::notification(
                "error",
                json!({"error": error, "willRetry": false, "threadId": thread_id, "turnId": turn_id}),
            ));
            output.push(Self::notification(
                "turn/completed",
                json!({"threadId": thread_id, "turn": Self::turn_json(turn_id, "failed", error)}),
            ));
            self.active_turn = None;
            return Ok(output);
        }

        let final_text = if approved {
            "Mock Codex completed the requested work."
        } else {
            "The command was declined; Mock Codex continued without it."
        };
        let final_item = Self::agent_item("mock-final-1", final_text, "final_answer");
        output.push(Self::item_started(thread_id, turn_id, final_item.clone()));
        for delta in ["Mock Codex ", "completed the requested work."] {
            output.push(Self::notification(
                "item/agentMessage/delta",
                json!({"threadId": thread_id, "turnId": turn_id, "itemId": "mock-final-1", "delta": delta}),
            ));
        }
        output.push(Self::item_completed(thread_id, turn_id, final_item));
        output.push(Self::notification(
            "turn/completed",
            json!({"threadId": thread_id, "turn": Self::turn_json(turn_id, "completed", Value::Null)}),
        ));
        self.active_turn = None;
        Ok(output)
    }

    fn handle_approval_response(&mut self, message: &Value) -> Result<Option<Vec<EngineOutput>>> {
        let Some(pending) = self.pending_approval.clone() else {
            return Ok(None);
        };
        if request_id(message).as_ref() != Some(&pending.request_id) {
            return Ok(None);
        }

        let decision = message
            .pointer("/result/decision")
            .and_then(Value::as_str)
            .unwrap_or("decline");
        let approved = matches!(decision, "accept" | "acceptForSession");
        let active = self
            .active_turn
            .clone()
            .context("approval response received without an active turn")?;
        self.pending_approval = None;
        let mut output = vec![Self::notification(
            "serverRequest/resolved",
            json!({"threadId": active.thread_id, "requestId": pending.request_id}),
        )];
        output.extend(self.finish_active_turn(approved)?);
        Ok(Some(output))
    }

    fn handle_method(&mut self, message: Value, method: &str) -> Result<Vec<EngineOutput>> {
        let id = request_id(&message).unwrap_or(Value::Null);
        if !self.initialized && !matches!(method, "initialize" | "initialized") {
            return Ok(vec![Self::error(
                id,
                -32002,
                "mock Codex app-server is not initialized",
            )]);
        }

        match method {
            "initialize" => {
                self.initialized = true;
                Ok(vec![Self::response(
                    id,
                    json!({
                        "userAgent": "codex_cli_rs/0.150.1 (Mock Agent Remote Testkit)",
                        "codexHome": "C:\\mock-codex-home",
                        "platformFamily": "windows",
                        "platformOs": "windows"
                    }),
                )])
            }
            "initialized" => Ok(Vec::new()),
            "account/read" => Ok(vec![Self::response(
                id,
                json!({"account": {"type": "apiKey"}, "requiresOpenaiAuth": true}),
            )]),
            "model/list" => {
                let include_hidden = message
                    .pointer("/params/includeHidden")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let mut models = Self::model_list();
                if !include_hidden
                    && let Some(data) = models.get_mut("data").and_then(Value::as_array_mut)
                {
                    data.retain(|model| model.get("hidden") == Some(&Value::Bool(false)));
                }
                Ok(vec![Self::response(id, models)])
            }
            "thread/start" => {
                let cwd = message
                    .pointer("/params/cwd")
                    .and_then(Value::as_str)
                    .unwrap_or("C:\\mock-project")
                    .to_owned();
                let model = message
                    .pointer("/params/model")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_MODEL);
                if let Err(reason) = Self::validate_model_effort(Some(model), None) {
                    return Ok(vec![Self::error(id, -32602, reason)]);
                }
                let thread_id = self.seed_thread(cwd, "Mock Codex session");
                self.threads
                    .get_mut(&thread_id)
                    .expect("seeded thread")
                    .model = model.to_owned();
                let record = self.threads.get(&thread_id).expect("seeded thread");
                let result = self.start_or_resume_result(record);
                Ok(vec![
                    Self::response(id, result),
                    Self::notification(
                        "thread/started",
                        json!({"thread": self.thread_json(record, false)}),
                    ),
                ])
            }
            "thread/list" => {
                let cwd_filter = message.pointer("/params/cwd");
                let matches_cwd = |record: &ThreadRecord| match cwd_filter {
                    Some(Value::String(cwd)) => &record.cwd == cwd,
                    Some(Value::Array(cwds)) => {
                        cwds.iter().any(|cwd| cwd.as_str() == Some(&record.cwd))
                    }
                    _ => true,
                };
                let data = self
                    .threads
                    .values()
                    .filter(|record| matches_cwd(record))
                    .map(|record| self.thread_json(record, false))
                    .collect::<Vec<_>>();
                Ok(vec![Self::response(
                    id,
                    json!({"data": data, "nextCursor": null, "backwardsCursor": null}),
                )])
            }
            "thread/read" => {
                let thread_id = message
                    .pointer("/params/threadId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let include_turns = message
                    .pointer("/params/includeTurns")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                match self.threads.get(thread_id) {
                    Some(record) => Ok(vec![Self::response(
                        id,
                        json!({"thread": self.thread_json(record, include_turns)}),
                    )]),
                    None => Ok(vec![Self::error(id, -32602, "unknown mock Codex thread")]),
                }
            }
            "thread/resume" => {
                let thread_id = message
                    .pointer("/params/threadId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let Some(record) = self.threads.get(thread_id) else {
                    return Ok(vec![Self::error(id, -32602, "unknown mock Codex thread")]);
                };
                if let Some(cwd) = message.pointer("/params/cwd").and_then(Value::as_str)
                    && cwd != record.cwd
                {
                    return Ok(vec![Self::error(
                        id,
                        -32602,
                        "thread cwd does not match mock project",
                    )]);
                }
                Ok(vec![Self::response(
                    id,
                    self.start_or_resume_result(record),
                )])
            }
            "turn/start" => {
                let thread_id = message
                    .pointer("/params/threadId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let Some(record) = self.threads.get(&thread_id) else {
                    return Ok(vec![Self::error(id, -32602, "unknown mock Codex thread")]);
                };
                if let Some(cwd) = message.pointer("/params/cwd").and_then(Value::as_str)
                    && cwd != record.cwd
                {
                    return Ok(vec![Self::error(
                        id,
                        -32602,
                        "turn cwd does not match mock project",
                    )]);
                }
                let model = message.pointer("/params/model").and_then(Value::as_str);
                let effort = message.pointer("/params/effort").and_then(Value::as_str);
                if let Err(reason) = Self::validate_model_effort(model, effort) {
                    return Ok(vec![Self::error(id, -32602, reason)]);
                }
                let prompt = Self::extract_prompt(&message);
                let scenario = MockScenario::from_prompt(&prompt, self.scenario);
                self.initial_turn_events(id, thread_id, prompt, scenario)
            }
            "turn/steer" => {
                let expected_turn_id = message
                    .pointer("/params/expectedTurnId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let Some(active) = self.active_turn.as_ref() else {
                    return Ok(vec![Self::error(id, -32602, "no active mock Codex turn")]);
                };
                if active.turn_id != expected_turn_id {
                    return Ok(vec![Self::error(
                        id,
                        -32602,
                        "expectedTurnId does not match active turn",
                    )]);
                }
                Ok(vec![
                    Self::response(id, json!({"turnId": active.turn_id})),
                    Self::notification(
                        "item/agentMessage/delta",
                        json!({"threadId": active.thread_id, "turnId": active.turn_id, "itemId": "mock-commentary-1", "delta": " Steering received."}),
                    ),
                ])
            }
            "turn/interrupt" => {
                let thread_id = message
                    .pointer("/params/threadId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let turn_id = message
                    .pointer("/params/turnId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let mut output = vec![Self::response(id, json!({}))];
                if self.active_turn.as_ref().is_some_and(|active| {
                    active.thread_id == thread_id && active.turn_id == turn_id
                }) {
                    output.push(Self::notification(
                        "turn/completed",
                        json!({"threadId": thread_id, "turn": Self::turn_json(turn_id, "interrupted", Value::Null)}),
                    ));
                    self.active_turn = None;
                    self.pending_approval = None;
                }
                Ok(output)
            }
            _ => Ok(vec![Self::error(
                id,
                -32601,
                format!("mock Codex method not found: {method}"),
            )]),
        }
    }
}

impl JsonlEngine for MockCodexAppServer {
    fn receive(&mut self, message: Value) -> Result<Vec<EngineOutput>> {
        if requested_method(&message).is_none()
            && let Some(output) = self.handle_approval_response(&message)?
        {
            return Ok(output);
        }

        let method = requested_method(&message).unwrap_or_default().to_owned();
        self.handle_method(message, &method)
    }
}
