use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::{
    EngineOutput, JsonlEngine, MockScenario, request_id, requested_method, scenario_from_env,
};

const DEFAULT_MODEL: &str = "grok-4.6";
const DEFAULT_EFFORT: &str = "high";
const MOCK_PNG_BASE64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8z/8/AwMCAO+/ab0AAAAASUVORK5CYII=";

#[derive(Debug, Clone)]
struct SessionRecord {
    id: String,
    cwd: String,
    title: String,
    model: String,
    effort: String,
    updated_at: String,
}

#[derive(Debug, Clone)]
struct PendingPrompt {
    request_id: Value,
    session_id: String,
    scenario: MockScenario,
}

#[derive(Debug, Clone)]
struct PendingPermission {
    request_id: Value,
}

/// Grok Build 1.0.13 ACP v1 JSON-RPC engine, including its vendor model API.
#[derive(Debug, Clone)]
pub struct MockGrokAcp {
    scenario: MockScenario,
    initialized: bool,
    next_session: u64,
    next_permission: u64,
    sessions: BTreeMap<String, SessionRecord>,
    pending_prompt: Option<PendingPrompt>,
    pending_permission: Option<PendingPermission>,
}

impl Default for MockGrokAcp {
    fn default() -> Self {
        Self::new(MockScenario::Complete)
    }
}

impl MockGrokAcp {
    pub fn new(scenario: MockScenario) -> Self {
        Self {
            scenario,
            initialized: false,
            next_session: 1,
            next_permission: 1,
            sessions: BTreeMap::new(),
            pending_prompt: None,
            pending_permission: None,
        }
    }

    pub fn from_env() -> Result<Self> {
        Ok(Self::new(scenario_from_env()?))
    }

    pub fn seed_session(&mut self, cwd: impl Into<String>, title: impl Into<String>) -> String {
        let id = format!("mock-grok-session-{}", self.next_session);
        self.next_session += 1;
        self.sessions.insert(
            id.clone(),
            SessionRecord {
                id: id.clone(),
                cwd: cwd.into(),
                title: title.into(),
                model: DEFAULT_MODEL.to_owned(),
                effort: DEFAULT_EFFORT.to_owned(),
                updated_at: "2026-09-01T00:00:00Z".to_owned(),
            },
        );
        id
    }

    fn response(id: Value, result: Value) -> EngineOutput {
        EngineOutput::Json(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    fn error(id: Value, code: i64, message: impl Into<String>) -> EngineOutput {
        EngineOutput::Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": code, "message": message.into()}
        }))
    }

    fn notification(method: &str, params: Value) -> EngineOutput {
        EngineOutput::Json(json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn request(id: Value, method: &str, params: Value) -> EngineOutput {
        EngineOutput::Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
    }

    fn model_state() -> Value {
        json!({
            "currentModelId": DEFAULT_MODEL,
            "availableModels": [
                {
                    "modelId": DEFAULT_MODEL,
                    "name": "Grok 4.6 Mock",
                    "description": "Dynamic frontier model advertised by the Grok mock",
                    "_meta": {
                        "reasoningEffort": DEFAULT_EFFORT,
                        "reasoningEfforts": [
                            {"id": "xhigh", "value": "xhigh", "label": "Extra High", "description": "Maximum mock effort", "default": false},
                            {"id": "high", "value": "high", "label": "High", "description": "Detailed mock effort", "default": true},
                            {"id": "medium", "value": "medium", "label": "Medium", "description": "Balanced mock effort", "default": false},
                            {"id": "low", "value": "low", "label": "Low", "description": "Fast mock effort", "default": false}
                        ],
                        "supportsReasoningEffort": true,
                        "totalContextTokens": 500000
                    }
                },
                {
                    "modelId": "grok-4.5",
                    "name": "Grok 4.5 Mock",
                    "description": "Second dynamic model advertised by the Grok mock",
                    "_meta": {
                        "reasoningEffort": "medium",
                        "reasoningEfforts": [
                            {"id": "high", "value": "high", "label": "High", "description": "Detailed mock effort", "default": false},
                            {"id": "medium", "value": "medium", "label": "Medium", "description": "Balanced mock effort", "default": true},
                            {"id": "low", "value": "low", "label": "Low", "description": "Fast mock effort", "default": false}
                        ],
                        "supportsReasoningEffort": true,
                        "totalContextTokens": 256000
                    }
                }
            ]
        })
    }

    fn initialize_result() -> Value {
        json!({
            "protocolVersion": 1,
            "agentCapabilities": {
                "loadSession": true,
                "promptCapabilities": {"image": false, "audio": false, "embeddedContext": true},
                "sessionCapabilities": {"list": {}, "resume": {}, "close": {}}
            },
            "authMethods": [],
            "agentInfo": {"name": "grok", "title": "Grok Build Mock", "version": "1.0.13"},
            "_meta": {
                "grokShell": true,
                "agentVersion": "1.0.13",
                "modelState": Self::model_state()
            }
        })
    }

    fn validate_model_effort(model: &str, effort: &str) -> Result<(), &'static str> {
        let valid = match model {
            DEFAULT_MODEL => matches!(effort, "xhigh" | "high" | "medium" | "low"),
            "grok-4.5" => matches!(effort, "high" | "medium" | "low"),
            _ => return Err("unsupported mock Grok model"),
        };
        if valid {
            Ok(())
        } else {
            Err("unsupported reasoning effort for mock Grok model")
        }
    }

    fn requested_model_effort(message: &Value) -> (&str, &str) {
        let model = message
            .pointer("/params/_meta/modelId")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_MODEL);
        let effort = message
            .pointer("/params/_meta/reasoningEffort")
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                if model == "grok-4.5" {
                    "medium"
                } else {
                    DEFAULT_EFFORT
                }
            });
        (model, effort)
    }

    fn session_meta(record: &SessionRecord) -> Value {
        let model_selected = |model: &str| model == record.model;
        let effort_selected = |effort: &str| effort == record.effort;
        json!({
            "x.ai/sessionConfig": {
                "options": [
                    {"id": DEFAULT_MODEL, "category": "model", "label": "Grok 4.6", "description": "Latest mock model", "selected": model_selected(DEFAULT_MODEL)},
                    {"id": "grok-4.5", "category": "model", "label": "Grok 4.5", "description": "Alternate mock model", "selected": model_selected("grok-4.5")},
                    {"id": "xhigh", "category": "mode", "label": "Extra High Effort", "description": "Highest effort and reasoning level", "selected": effort_selected("xhigh")},
                    {"id": "high", "category": "mode", "label": "High Effort", "description": "Higher implementation quality with extensive reasoning", "selected": effort_selected("high")},
                    {"id": "medium", "category": "mode", "label": "Medium Effort", "description": "Balanced effort with standard implementation and testing", "selected": effort_selected("medium")},
                    {"id": "low", "category": "mode", "label": "Low Effort", "description": "Quick, fast implementations", "selected": effort_selected("low")}
                ]
            },
            "x.ai/sessionDetail": {"modelId": record.model, "reasoningEffort": record.effort}
        })
    }

    fn session_result(record: &SessionRecord, include_id: bool) -> Value {
        let mut result = json!({"_meta": Self::session_meta(record)});
        if include_id {
            result["sessionId"] = json!(record.id);
        }
        result
    }

    fn update(session_id: &str, update: Value) -> EngineOutput {
        Self::notification(
            "session/update",
            json!({"sessionId": session_id, "update": update}),
        )
    }

    fn text_from_prompt(message: &Value) -> String {
        message
            .pointer("/params/prompt")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|content| content.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|content| content.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn tool_call(
        tool_call_id: &str,
        title: &str,
        kind: &str,
        status: &str,
        raw_input: Value,
    ) -> Value {
        json!({
            "sessionUpdate": "tool_call",
            "toolCallId": tool_call_id,
            "title": title,
            "kind": kind,
            "status": status,
            "content": [],
            "locations": [],
            "rawInput": raw_input
        })
    }

    fn prompt_prefix(session_id: &str) -> Vec<EngineOutput> {
        vec![
            Self::update(
                session_id,
                json!({
                    "sessionUpdate": "plan",
                    "entries": [
                        {"content": "Inspect the project", "priority": "high", "status": "in_progress"},
                        {"content": "Complete the mock task", "priority": "medium", "status": "pending"}
                    ]
                }),
            ),
            Self::update(
                session_id,
                json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "Inspecting the project."}, "messageId": "mock-thought-1"}),
            ),
            Self::update(
                session_id,
                Self::tool_call(
                    "mock-grok-command-1",
                    "Run verification command",
                    "execute",
                    "in_progress",
                    json!({"command": "cargo test --quiet"}),
                ),
            ),
        ]
    }

    fn finish_prompt(&mut self, allowed: bool) -> Result<Vec<EngineOutput>> {
        let pending = self
            .pending_prompt
            .clone()
            .context("mock Grok has no pending prompt to finish")?;
        let record = self
            .sessions
            .get(&pending.session_id)
            .context("active mock Grok session disappeared")?;
        let session_id = &pending.session_id;
        let mut output = Vec::new();

        output.push(Self::update(
            session_id,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "mock-grok-command-1",
                "status": if allowed { "completed" } else { "failed" },
                "content": [{"type": "content", "content": {"type": "text", "text": if allowed { "1 mock test passed" } else { "Command rejected by user" }}}],
                "rawOutput": {"exitCode": if allowed { json!(0) } else { Value::Null }}
            }),
        ));

        let file_path = PathBuf::from(&record.cwd).join("src").join("mock.rs");
        output.push(Self::update(
            session_id,
            Self::tool_call(
                "mock-grok-file-1",
                "Update mock file",
                "edit",
                "in_progress",
                json!({"path": file_path}),
            ),
        ));
        output.push(Self::update(
            session_id,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "mock-grok-file-1",
                "status": "completed",
                "locations": [{"path": file_path, "line": 1}],
                "content": [
                    {"type": "diff", "path": file_path, "oldText": "old\n", "newText": "new\n"},
                    {"type": "content", "content": {"type": "image", "mimeType": "image/png", "data": MOCK_PNG_BASE64}}
                ]
            }),
        ));
        output.push(Self::update(
            session_id,
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "image", "mimeType": "image/png", "data": MOCK_PNG_BASE64}, "messageId": "mock-image-1"}),
        ));

        if pending.scenario == MockScenario::Unknown {
            output.push(EngineOutput::Json(json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": session_id,
                    "update": {"sessionUpdate": "future_mock_update", "futureField": [1, 2, 3]}
                },
                "futureTopLevel": true
            })));
        }

        if pending.scenario == MockScenario::Failure {
            output.push(Self::update(
                session_id,
                json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "mock-grok-file-1",
                    "status": "failed",
                    "content": [{"type": "content", "content": {"type": "text", "text": "mock Grok prompt failed"}}]
                }),
            ));
            output.push(Self::error(
                pending.request_id,
                -32000,
                "mock Grok prompt failed",
            ));
            self.pending_prompt = None;
            return Ok(output);
        }

        let (first_delta, second_delta) = if allowed {
            ("Mock Grok ", "completed the requested work.")
        } else {
            (
                "The command was rejected; ",
                "Mock Grok continued without it.",
            )
        };
        for delta in [first_delta, second_delta] {
            output.push(Self::update(
                session_id,
                json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": delta}, "messageId": "mock-final-1"}),
            ));
        }
        output.push(Self::update(
            session_id,
            json!({
                "sessionUpdate": "plan",
                "entries": [
                    {"content": "Inspect the project", "priority": "high", "status": "completed"},
                    {"content": "Complete the mock task", "priority": "medium", "status": "completed"}
                ]
            }),
        ));
        output.push(Self::response(
            pending.request_id,
            json!({"stopReason": "end_turn"}),
        ));
        self.pending_prompt = None;
        Ok(output)
    }

    fn handle_permission_response(&mut self, message: &Value) -> Result<Option<Vec<EngineOutput>>> {
        let Some(pending_permission) = self.pending_permission.clone() else {
            return Ok(None);
        };
        if request_id(message).as_ref() != Some(&pending_permission.request_id) {
            return Ok(None);
        }

        let option_id = message
            .pointer("/result/outcome/optionId")
            .and_then(Value::as_str);
        let allowed = matches!(option_id, Some("allow-once" | "allow-always"));
        self.pending_permission = None;
        Ok(Some(self.finish_prompt(allowed)?))
    }

    fn start_prompt(
        &mut self,
        request_id: Value,
        session_id: String,
        scenario: MockScenario,
    ) -> Result<Vec<EngineOutput>> {
        self.pending_prompt = Some(PendingPrompt {
            request_id,
            session_id: session_id.clone(),
            scenario,
        });
        let mut output = Self::prompt_prefix(&session_id);

        if scenario == MockScenario::Crash {
            output.push(Self::update(
                &session_id,
                json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Mock Grok is about to crash."}, "messageId": "mock-crash-1"}),
            ));
            output.push(EngineOutput::Exit(87));
            return Ok(output);
        }

        if scenario == MockScenario::Approval {
            let permission_id = json!(format!("mock-grok-permission-{}", self.next_permission));
            self.next_permission += 1;
            self.pending_permission = Some(PendingPermission {
                request_id: permission_id.clone(),
            });
            output.push(Self::request(
                permission_id,
                "session/request_permission",
                json!({
                    "sessionId": session_id,
                    "toolCall": {
                        "toolCallId": "mock-grok-command-1",
                        "title": "Run verification command",
                        "kind": "execute",
                        "status": "pending",
                        "rawInput": {"command": "cargo test --quiet"}
                    },
                    "options": [
                        {"optionId": "allow-once", "name": "Allow once", "kind": "allow_once"},
                        {"optionId": "allow-always", "name": "Always allow", "kind": "allow_always"},
                        {"optionId": "reject-once", "name": "Reject", "kind": "reject_once"},
                        {"optionId": "reject-always", "name": "Always reject", "kind": "reject_always"}
                    ]
                }),
            ));
            return Ok(output);
        }

        output.extend(self.finish_prompt(true)?);
        Ok(output)
    }

    fn handle_method(&mut self, message: Value, method: &str) -> Result<Vec<EngineOutput>> {
        let id = request_id(&message).unwrap_or(Value::Null);
        if !self.initialized && method != "initialize" {
            return Ok(vec![Self::error(
                id,
                -32002,
                "mock Grok ACP agent is not initialized",
            )]);
        }

        match method {
            "initialize" => {
                self.initialized = true;
                Ok(vec![Self::response(id, Self::initialize_result())])
            }
            "authenticate" => Ok(vec![Self::response(id, json!({}))]),
            "session/new" => {
                let cwd = message
                    .pointer("/params/cwd")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let (model, effort) = Self::requested_model_effort(&message);
                if let Err(reason) = Self::validate_model_effort(model, effort) {
                    return Ok(vec![Self::error(id, -32602, reason)]);
                }
                let session_id = self.seed_session(cwd, "Mock Grok session");
                let record = self.sessions.get_mut(&session_id).expect("seeded session");
                record.model = model.to_owned();
                record.effort = effort.to_owned();
                Ok(vec![Self::response(id, Self::session_result(record, true))])
            }
            "session/list" => {
                let cwd = message.pointer("/params/cwd").and_then(Value::as_str);
                let sessions = self
                    .sessions
                    .values()
                    .filter(|session| cwd.is_none_or(|cwd| cwd == session.cwd))
                    .map(|session| {
                        json!({
                            "sessionId": session.id,
                            "cwd": session.cwd,
                            "title": session.title,
                            "updatedAt": session.updated_at
                        })
                    })
                    .collect::<Vec<_>>();
                Ok(vec![Self::response(
                    id,
                    json!({"sessions": sessions, "nextCursor": null}),
                )])
            }
            "session/load" | "session/resume" => {
                let session_id = message
                    .pointer("/params/sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let cwd = message
                    .pointer("/params/cwd")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let Some(record) = self.sessions.get(session_id) else {
                    return Ok(vec![Self::error(id, -32602, "unknown mock Grok session")]);
                };
                if cwd != record.cwd {
                    return Ok(vec![Self::error(
                        id,
                        -32602,
                        "session cwd does not match mock project",
                    )]);
                }
                let mut output = Vec::new();
                if method == "session/load" {
                    output.push(Self::update(
                        session_id,
                        json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "Loaded mock history"}, "messageId": "mock-history-1"}),
                    ));
                    output.push(Self::update(
                        session_id,
                        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Mock history restored"}, "messageId": "mock-history-2"}),
                    ));
                }
                output.push(Self::response(id, Self::session_result(record, false)));
                Ok(output)
            }
            "session/set_config_option" => Ok(vec![Self::error(
                id,
                -32601,
                "Grok Build 1.0.13 does not implement session/set_config_option",
            )]),
            "session/set_model" => {
                let session_id = message
                    .pointer("/params/sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let model = message
                    .pointer("/params/modelId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let effort = message
                    .pointer("/params/_meta/reasoningEffort")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_EFFORT);
                if let Err(reason) = Self::validate_model_effort(model, effort) {
                    return Ok(vec![Self::error(id, -32602, reason)]);
                }
                let Some(record) = self.sessions.get_mut(session_id) else {
                    return Ok(vec![Self::error(id, -32602, "unknown mock Grok session")]);
                };
                record.model = model.to_owned();
                record.effort = effort.to_owned();
                Ok(vec![Self::response(id, json!({}))])
            }
            "_x.ai/interject" => {
                let session_id = message
                    .pointer("/params/sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let text = message
                    .pointer("/params/text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                Ok(vec![
                    Self::response(id, json!({"status": "queued"})),
                    Self::update(
                        session_id,
                        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": format!("Interjection received: {text}")}, "messageId": "mock-interject-1"}),
                    ),
                ])
            }
            "session/prompt" => {
                let session_id = message
                    .pointer("/params/sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if !self.sessions.contains_key(&session_id) {
                    return Ok(vec![Self::error(id, -32602, "unknown mock Grok session")]);
                }
                let prompt = Self::text_from_prompt(&message);
                let scenario = MockScenario::from_prompt(&prompt, self.scenario);
                self.start_prompt(id, session_id, scenario)
            }
            "session/cancel" => {
                let session_id = message
                    .pointer("/params/sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let Some(pending) = self.pending_prompt.clone() else {
                    return Ok(Vec::new());
                };
                if pending.session_id != session_id {
                    return Ok(Vec::new());
                }
                self.pending_permission = None;
                self.pending_prompt = None;
                Ok(vec![
                    Self::update(
                        session_id,
                        json!({"sessionUpdate": "tool_call_update", "toolCallId": "mock-grok-command-1", "status": "failed", "content": [{"type": "content", "content": {"type": "text", "text": "Cancelled"}}]}),
                    ),
                    Self::response(pending.request_id, json!({"stopReason": "cancelled"})),
                ])
            }
            _ => Ok(vec![Self::error(
                id,
                -32601,
                format!("mock Grok ACP method not found: {method}"),
            )]),
        }
    }
}

impl JsonlEngine for MockGrokAcp {
    fn receive(&mut self, message: Value) -> Result<Vec<EngineOutput>> {
        if requested_method(&message).is_none()
            && let Some(output) = self.handle_permission_response(&message)?
        {
            return Ok(output);
        }

        let method = requested_method(&message).unwrap_or_default().to_owned();
        self.handle_method(message, &method)
    }
}
