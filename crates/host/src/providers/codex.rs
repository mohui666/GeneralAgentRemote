//! OpenAI Codex app-server adapter.
//!
//! The adapter intentionally implements the stable app-server surface only. The
//! wire decoder is tolerant, while each request we send has a small typed DTO.

use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex, RwLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use agent_remote_protocol::{
    AgentMessagePhase, ApprovalOption, ConversationId, ItemStatus, ModelOption, PlanStep,
    ProjectId, ProviderHealth, ProviderId, ProviderState, SessionOption, SessionOptionValue,
    SessionSummary,
};
use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    process::Command,
    sync::{Mutex as AsyncMutex, broadcast, oneshot},
};

use super::{
    AgentProvider, CommandAck, CreateSession, InterruptSession, NativeSession,
    ProviderCapabilities, ProviderEvent, ProviderEventKind, ResolveApproval, ResumeSession,
    SendMessage, SetSessionOption, SteerMessage,
};
use crate::storage::Project;

const CLIENT_NAME: &str = "agent_remote_messenger";
const CLIENT_TITLE: &str = "Agent Remote Messenger";
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

type DynWriter = Box<dyn AsyncWrite + Send + Unpin>;

#[derive(Clone)]
pub struct CodexProvider {
    shared: Arc<Shared>,
}

impl CodexProvider {
    pub fn new() -> Self {
        Self::with_executable("codex")
    }

    pub fn with_executable(executable: impl Into<OsString>) -> Self {
        let (events, _) = broadcast::channel(512);
        Self {
            shared: Arc::new(Shared {
                executable: executable.into(),
                events,
                connection: AsyncMutex::new(None),
                generation: AtomicU64::new(0),
                sessions: RwLock::new(HashMap::new()),
                active_turns: Mutex::new(HashMap::new()),
                pending_approvals: Mutex::new(HashMap::new()),
                message_states: Mutex::new(HashMap::new()),
                reasoning_states: Mutex::new(HashMap::new()),
                command_states: Mutex::new(HashMap::new()),
                emitted_images: Mutex::new(HashSet::new()),
                failed_turns: Mutex::new(HashSet::new()),
                model_cache: RwLock::new(Vec::new()),
            }),
        }
    }

    async fn connection(&self) -> Result<Arc<RpcWire>> {
        self.shared.connection().await
    }

    async fn ensure_loaded_session(
        &self,
        conversation_id: ConversationId,
        native_session_id: &str,
    ) -> Result<(Arc<RpcWire>, SessionContext)> {
        let wire = self.connection().await?;
        let context = self
            .shared
            .session(native_session_id)
            .ok_or_else(|| anyhow!("Codex thread is not registered with this host process"))?;
        if context.conversation_id != conversation_id {
            bail!("Codex thread belongs to a different conversation");
        }
        if context.loaded_generation != wire.generation {
            let response: ThreadOpenResponse = wire
                .request(
                    "thread/resume",
                    &ThreadResumeParams {
                        thread_id: native_session_id,
                        model: context.model.as_deref(),
                        cwd: path_string(&context.project_path),
                        approvals_reviewer: "user",
                    },
                )
                .await?;
            validate_thread_project(&response.thread, &context.project_path)?;
            self.shared.register_session(
                response.thread.id.clone(),
                SessionContext {
                    model: Some(response.model),
                    effort: response.reasoning_effort,
                    loaded_generation: wire.generation,
                    ..context.clone()
                },
            );
        }
        let context = self
            .shared
            .session(native_session_id)
            .ok_or_else(|| anyhow!("Codex thread disappeared while resuming"))?;
        Ok((wire, context))
    }

    async fn models_for_options(&self, project: &Project) -> Result<Vec<ModelOption>> {
        let cached = self
            .shared
            .model_cache
            .read()
            .expect("Codex model cache poisoned")
            .clone();
        if cached.is_empty() {
            self.list_models(project).await
        } else {
            Ok(cached)
        }
    }
}

impl Default for CodexProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentProvider for CodexProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_session_list: true,
            supports_resume: true,
            supports_steer: true,
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<ProviderEvent> {
        self.shared.events.subscribe()
    }

    async fn health(&self) -> ProviderHealth {
        let mut version_command = Command::new(&self.shared.executable);
        version_command
            .arg("--version")
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        version_command.creation_flags(0x0800_0000);
        let version_output = version_command.output().await;
        let version = match version_output {
            Ok(output) if output.status.success() => {
                let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                (!value.is_empty()).then_some(value)
            }
            Ok(_) => {
                return provider_health(
                    ProviderState::Crashed,
                    None,
                    Some("Codex executable returned an error"),
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return provider_health(
                    ProviderState::NotInstalled,
                    None,
                    Some("Codex executable was not found on PATH"),
                );
            }
            Err(_) => {
                return provider_health(
                    ProviderState::Offline,
                    None,
                    Some("Codex executable could not be started"),
                );
            }
        };

        let result = async {
            let wire = self.connection().await?;
            wire.request::<_, AccountReadResponse>(
                "account/read",
                &AccountReadParams {
                    refresh_token: false,
                },
            )
            .await
        }
        .await;
        match result {
            Ok(account) if account.account.is_some() || !account.requires_openai_auth => {
                provider_health(ProviderState::Ready, version, None)
            }
            Ok(_) => provider_health(
                ProviderState::NotAuthenticated,
                version,
                Some("Codex requires authentication"),
            ),
            Err(_) => provider_health(
                ProviderState::ProtocolIncompatible,
                version,
                Some("Codex app-server initialization or account/read failed"),
            ),
        }
    }

    async fn list_models(&self, _project: &Project) -> Result<Vec<ModelOption>> {
        let wire = self.connection().await?;
        let mut cursor = None;
        let mut models = Vec::new();
        loop {
            let response: ModelListResponse = wire
                .request(
                    "model/list",
                    &ModelListParams {
                        cursor: cursor.as_deref(),
                        limit: Some(100),
                        include_hidden: false,
                    },
                )
                .await?;
            models.extend(
                response
                    .data
                    .into_iter()
                    .filter(|model| !model.hidden)
                    .map(|model| ModelOption {
                        id: model.model,
                        display_name: model.display_name,
                        effort_options: model
                            .supported_reasoning_efforts
                            .into_iter()
                            .map(|effort| agent_remote_protocol::EffortOption {
                                id: effort.reasoning_effort.clone(),
                                display_name: effort.reasoning_effort,
                            })
                            .collect(),
                        default_effort: Some(model.default_reasoning_effort),
                    }),
            );
            cursor = response.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        *self
            .shared
            .model_cache
            .write()
            .expect("Codex model cache poisoned") = models.clone();
        Ok(models)
    }

    async fn list_sessions(&self, project: &Project) -> Result<Vec<SessionSummary>> {
        let wire = self.connection().await?;
        let project_path = path_string(&project.canonical_path);
        let mut cursor = None;
        let mut sessions = Vec::new();
        loop {
            let response: ThreadListResponse = wire
                .request(
                    "thread/list",
                    &ThreadListParams {
                        cursor: cursor.as_deref(),
                        limit: Some(100),
                        sort_key: "updated_at",
                        sort_direction: "desc",
                        source_kinds: &["cli", "vscode", "appServer"],
                        archived: false,
                        cwd: &project_path,
                    },
                )
                .await?;
            for thread in response.data {
                if !thread_matches_project(&thread, &project.canonical_path) {
                    continue;
                }
                let title = thread
                    .name
                    .filter(|name| !name.trim().is_empty())
                    .or_else(|| (!thread.preview.trim().is_empty()).then_some(thread.preview))
                    .unwrap_or_else(|| "Codex session".to_owned());
                sessions.push(SessionSummary {
                    native_session_id: thread.id,
                    title,
                    updated_at_ms: thread.updated_at.saturating_mul(1_000),
                });
            }
            cursor = response.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        Ok(sessions)
    }

    async fn create_session(&self, request: CreateSession) -> Result<NativeSession> {
        let wire = self.connection().await?;
        let response: ThreadOpenResponse = wire
            .request(
                "thread/start",
                &ThreadStartParams {
                    model: request.model.as_deref(),
                    cwd: path_string(&request.project.canonical_path),
                    approvals_reviewer: "user",
                    service_name: CLIENT_NAME,
                },
            )
            .await?;
        validate_thread_project(&response.thread, &request.project.canonical_path)?;
        let selected_model = Some(response.model.clone());
        let selected_effort = request.effort.clone().or(response.reasoning_effort.clone());
        self.shared.register_session(
            response.thread.id.clone(),
            SessionContext {
                project_id: request.project.id,
                conversation_id: request.conversation_id,
                project_path: request.project.canonical_path.clone(),
                model: selected_model.clone(),
                effort: selected_effort.clone(),
                loaded_generation: wire.generation,
            },
        );
        let models = self.models_for_options(&request.project).await?;
        Ok(native_session(
            response.thread,
            selected_model,
            selected_effort,
            &models,
        ))
    }

    async fn resume_session(&self, request: ResumeSession) -> Result<NativeSession> {
        if self
            .shared
            .session(&request.native_session_id)
            .is_some_and(|context| context.conversation_id != request.conversation_id)
        {
            bail!("Codex thread is already attached to a different conversation");
        }
        let wire = self.connection().await?;
        let read: ThreadReadResponse = wire
            .request(
                "thread/read",
                &ThreadReadParams {
                    thread_id: &request.native_session_id,
                    include_turns: false,
                },
            )
            .await?;
        validate_thread_project(&read.thread, &request.project.canonical_path)?;
        let response: ThreadOpenResponse = wire
            .request(
                "thread/resume",
                &ThreadResumeParams {
                    thread_id: &request.native_session_id,
                    model: request.model.as_deref(),
                    cwd: path_string(&request.project.canonical_path),
                    approvals_reviewer: "user",
                },
            )
            .await?;
        validate_thread_project(&response.thread, &request.project.canonical_path)?;
        let selected_model = Some(response.model.clone());
        let selected_effort = request.effort.clone().or(response.reasoning_effort.clone());
        self.shared.register_session(
            response.thread.id.clone(),
            SessionContext {
                project_id: request.project.id,
                conversation_id: request.conversation_id,
                project_path: request.project.canonical_path.clone(),
                model: selected_model.clone(),
                effort: selected_effort.clone(),
                loaded_generation: wire.generation,
            },
        );
        let models = self.models_for_options(&request.project).await?;
        Ok(native_session(
            response.thread,
            selected_model,
            selected_effort,
            &models,
        ))
    }

    async fn send_message(&self, request: SendMessage) -> Result<CommandAck> {
        let (wire, context) = self
            .ensure_loaded_session(request.conversation_id, &request.native_session_id)
            .await?;
        let response: TurnStartResponse = wire
            .request(
                "turn/start",
                &TurnStartParams {
                    thread_id: &request.native_session_id,
                    client_user_message_id: Some(ConversationId::new().to_string()),
                    input: vec![TextInput {
                        kind: "text",
                        text: request.text,
                    }],
                    cwd: path_string(&context.project_path),
                    approvals_reviewer: "user",
                    model: request.model.as_deref().or(context.model.as_deref()),
                    effort: request.effort.as_deref().or(context.effort.as_deref()),
                },
            )
            .await?;
        self.shared
            .active_turns
            .lock()
            .expect("Codex active turn state poisoned")
            .insert(request.native_session_id, response.turn.id);
        Ok(CommandAck)
    }

    async fn steer(&self, request: SteerMessage) -> Result<CommandAck> {
        let (wire, _) = self
            .ensure_loaded_session(request.conversation_id, &request.native_session_id)
            .await?;
        let active_turn = self
            .shared
            .active_turns
            .lock()
            .expect("Codex active turn state poisoned")
            .get(&request.native_session_id)
            .cloned()
            .ok_or_else(|| anyhow!("Codex thread has no active steerable turn"))?;
        let response: TurnSteerResponse = wire
            .request(
                "turn/steer",
                &TurnSteerParams {
                    thread_id: &request.native_session_id,
                    client_user_message_id: Some(ConversationId::new().to_string()),
                    input: vec![TextInput {
                        kind: "text",
                        text: request.text,
                    }],
                    expected_turn_id: &active_turn,
                },
            )
            .await?;
        if response.turn_id != active_turn {
            bail!("Codex steered a different active turn");
        }
        Ok(CommandAck)
    }

    async fn interrupt(&self, request: InterruptSession) -> Result<CommandAck> {
        let (wire, _) = self
            .ensure_loaded_session(request.conversation_id, &request.native_session_id)
            .await?;
        let turn_id = self
            .shared
            .active_turns
            .lock()
            .expect("Codex active turn state poisoned")
            .get(&request.native_session_id)
            .cloned()
            .ok_or_else(|| anyhow!("Codex thread has no active turn"))?;
        let _: EmptyResponse = wire
            .request(
                "turn/interrupt",
                &TurnInterruptParams {
                    thread_id: &request.native_session_id,
                    turn_id: &turn_id,
                },
            )
            .await?;
        Ok(CommandAck)
    }

    async fn resolve_approval(&self, request: ResolveApproval) -> Result<CommandAck> {
        if request.option_id != "accept" && request.option_id != "decline" {
            bail!("unsupported Codex approval option");
        }
        let pending = self
            .shared
            .pending_approvals
            .lock()
            .expect("Codex approval state poisoned")
            .remove(&request.provider_request_id)
            .ok_or_else(|| anyhow!("Codex approval is no longer pending"))?;
        if pending.conversation_id != request.conversation_id {
            bail!("Codex approval belongs to a different conversation");
        }
        let accepted = request.option_id == "accept";
        let result = match pending.kind {
            PendingApprovalKind::Command => {
                json!({"decision": if accepted { "accept" } else { "decline" }})
            }
            PendingApprovalKind::FileChange => {
                json!({"decision": if accepted { "accept" } else { "decline" }})
            }
            PendingApprovalKind::Permissions { requested } => {
                json!({
                    "permissions": if accepted { requested } else { json!({}) },
                    "scope": "turn"
                })
            }
            PendingApprovalKind::LegacyCommand | PendingApprovalKind::LegacyPatch => {
                if accepted {
                    json!({"decision":"approved"})
                } else {
                    json!({"decision":{"denied":{"rejection":"Declined remotely"}}})
                }
            }
        };
        pending.wire.respond_result(pending.rpc_id, result).await?;
        Ok(CommandAck)
    }

    async fn set_session_option(&self, request: SetSessionOption) -> Result<CommandAck> {
        let mut sessions = self
            .shared
            .sessions
            .write()
            .expect("Codex session state poisoned");
        let context = sessions
            .get_mut(&request.native_session_id)
            .ok_or_else(|| anyhow!("Codex thread is not registered"))?;
        if context.conversation_id != request.conversation_id {
            bail!("Codex thread belongs to a different conversation");
        }
        match request.option_id.as_str() {
            "model" => context.model = Some(request.value),
            "reasoning_effort" | "thought_level" => context.effort = Some(request.value),
            _ => bail!("unsupported Codex session option"),
        }
        Ok(CommandAck)
    }
}

struct Shared {
    executable: OsString,
    events: broadcast::Sender<ProviderEvent>,
    connection: AsyncMutex<Option<Arc<RpcWire>>>,
    generation: AtomicU64,
    sessions: RwLock<HashMap<String, SessionContext>>,
    active_turns: Mutex<HashMap<String, String>>,
    pending_approvals: Mutex<HashMap<String, PendingApproval>>,
    message_states: Mutex<HashMap<ItemKey, MessageState>>,
    reasoning_states: Mutex<HashMap<ReasoningKey, String>>,
    command_states: Mutex<HashMap<ItemKey, CommandState>>,
    emitted_images: Mutex<HashSet<ItemKey>>,
    failed_turns: Mutex<HashSet<(String, String)>>,
    model_cache: RwLock<Vec<ModelOption>>,
}

impl Shared {
    async fn connection(self: &Arc<Self>) -> Result<Arc<RpcWire>> {
        let mut slot = self.connection.lock().await;
        if let Some(wire) = slot.as_ref()
            && !wire.closed.load(Ordering::Acquire)
        {
            return Ok(Arc::clone(wire));
        }

        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let wire = self.spawn_wire(generation).await?;
        let _: InitializeResponse = wire
            .request(
                "initialize",
                &InitializeParams {
                    client_info: ClientInfo {
                        name: CLIENT_NAME,
                        title: CLIENT_TITLE,
                        version: CLIENT_VERSION,
                    },
                    capabilities: InitializeCapabilities {
                        experimental_api: false,
                        request_attestation: false,
                    },
                },
            )
            .await
            .context("initialize Codex app-server")?;
        wire.notify_initialized().await?;
        *slot = Some(Arc::clone(&wire));
        Ok(wire)
    }

    async fn spawn_wire(self: &Arc<Self>, generation: u64) -> Result<Arc<RpcWire>> {
        let mut command = Command::new(&self.executable);
        command
            .arg("app-server")
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        let mut child = command
            .spawn()
            .context("start `codex app-server --stdio`")?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Codex app-server stdin was unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Codex app-server stdout was unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("Codex app-server stderr was unavailable"))?;
        let wire = RpcWire::start(stdout, stdin, Arc::downgrade(self), generation);

        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                // Stderr can contain account, filesystem, and command diagnostics. Record
                // only that a diagnostic occurred, never its contents.
                tracing::debug!(diagnostic_bytes = line.len(), "Codex app-server diagnostic");
            }
        });

        let weak = Arc::downgrade(self);
        let monitored_wire = Arc::downgrade(&wire);
        tokio::spawn(async move {
            let status = child.wait().await;
            if let (Some(shared), Some(wire)) = (weak.upgrade(), monitored_wire.upgrade()) {
                let message = match status {
                    Ok(status) => format!("Codex app-server exited with {status}"),
                    Err(_) => "Codex app-server process could not be monitored".to_owned(),
                };
                shared.disconnect(wire, message).await;
            }
        });
        Ok(wire)
    }

    async fn disconnect(self: &Arc<Self>, wire: Arc<RpcWire>, message: String) {
        if wire.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        wire.fail_pending("Codex app-server disconnected");
        {
            let mut slot = self.connection.lock().await;
            if slot
                .as_ref()
                .is_some_and(|current| current.generation == wire.generation)
            {
                *slot = None;
            }
        }
        self.active_turns
            .lock()
            .expect("Codex active turn state poisoned")
            .clear();
        self.pending_approvals
            .lock()
            .expect("Codex approval state poisoned")
            .retain(|_, pending| pending.wire.generation != wire.generation);
        let contexts: Vec<_> = self
            .sessions
            .write()
            .expect("Codex session state poisoned")
            .values_mut()
            .filter_map(|context| {
                if context.loaded_generation == wire.generation {
                    context.loaded_generation = 0;
                    Some(context.clone())
                } else {
                    None
                }
            })
            .collect();
        for context in contexts {
            self.emit_context(
                &context,
                ProviderEventKind::Crashed {
                    message: message.clone(),
                },
            );
        }
    }

    fn session(&self, thread_id: &str) -> Option<SessionContext> {
        self.sessions
            .read()
            .expect("Codex session state poisoned")
            .get(thread_id)
            .cloned()
    }

    fn register_session(&self, thread_id: String, context: SessionContext) {
        self.sessions
            .write()
            .expect("Codex session state poisoned")
            .insert(thread_id, context);
    }

    fn emit(&self, thread_id: &str, kind: ProviderEventKind) {
        if let Some(context) = self.session(thread_id) {
            self.emit_context(&context, kind);
        }
    }

    fn emit_context(&self, context: &SessionContext, kind: ProviderEventKind) {
        let _ = self.events.send(ProviderEvent {
            provider: ProviderId::Codex,
            project_id: context.project_id,
            conversation_id: context.conversation_id,
            kind,
        });
    }

    async fn handle_server_request(
        self: &Arc<Self>,
        wire: Arc<RpcWire>,
        id: RpcId,
        method: String,
        params: Value,
    ) {
        let parsed = match method.as_str() {
            "item/commandExecution/requestApproval" => {
                serde_json::from_value::<CommandApprovalParams>(params).map(|request| {
                    let prompt = approval_prompt(
                        request.reason.as_deref(),
                        request.command.as_deref(),
                        request.cwd.as_deref(),
                    );
                    (request.thread_id, PendingApprovalKind::Command, prompt)
                })
            }
            "item/fileChange/requestApproval" => {
                serde_json::from_value::<FileApprovalParams>(params).map(|request| {
                    (
                        request.thread_id,
                        PendingApprovalKind::FileChange,
                        request
                            .reason
                            .unwrap_or_else(|| "Allow the proposed file changes?".to_owned()),
                    )
                })
            }
            "item/permissions/requestApproval" => {
                serde_json::from_value::<PermissionApprovalParams>(params).map(|request| {
                    let prompt = request.reason.unwrap_or_else(|| {
                        "Grant the requested permissions for this turn?".to_owned()
                    });
                    (
                        request.thread_id,
                        PendingApprovalKind::Permissions {
                            requested: request.permissions,
                        },
                        prompt,
                    )
                })
            }
            "execCommandApproval" => serde_json::from_value::<LegacyExecApprovalParams>(params)
                .map(|request| {
                    let command = request.command.join(" ");
                    let prompt = approval_prompt(
                        request.reason.as_deref(),
                        Some(&command),
                        Some(&request.cwd),
                    );
                    (
                        request.conversation_id,
                        PendingApprovalKind::LegacyCommand,
                        prompt,
                    )
                }),
            "applyPatchApproval" => serde_json::from_value::<LegacyPatchApprovalParams>(params)
                .map(|request| {
                    (
                        request.conversation_id,
                        PendingApprovalKind::LegacyPatch,
                        request
                            .reason
                            .unwrap_or_else(|| "Allow the proposed file changes?".to_owned()),
                    )
                }),
            _ => {
                let _ = wire
                    .respond_error(id, -32601, "Server request is not supported by this client")
                    .await;
                return;
            }
        };

        let Ok((thread_id, kind, prompt)) = parsed else {
            let _ = wire
                .respond_error(id, -32602, "Malformed app-server request")
                .await;
            return;
        };
        let Some(context) = self.session(&thread_id) else {
            let _ = wire
                .respond_error(id, -32600, "Thread is not registered with this client")
                .await;
            return;
        };
        let provider_request_id = id.opaque();
        self.pending_approvals
            .lock()
            .expect("Codex approval state poisoned")
            .insert(
                provider_request_id.clone(),
                PendingApproval {
                    rpc_id: id,
                    conversation_id: context.conversation_id,
                    kind,
                    wire,
                },
            );
        self.emit_context(
            &context,
            ProviderEventKind::Approval {
                provider_request_id,
                prompt,
                options: vec![
                    ApprovalOption {
                        id: "accept".to_owned(),
                        label: "Allow once".to_owned(),
                    },
                    ApprovalOption {
                        id: "decline".to_owned(),
                        label: "Deny".to_owned(),
                    },
                ],
            },
        );
    }

    fn handle_notification(&self, method: &str, params: Value) {
        match method {
            "turn/started" => {
                if let Ok(event) = serde_json::from_value::<TurnNotification>(params) {
                    self.active_turns
                        .lock()
                        .expect("Codex active turn state poisoned")
                        .insert(event.thread_id, event.turn.id);
                }
            }
            "turn/completed" => {
                if let Ok(event) = serde_json::from_value::<TurnNotification>(params) {
                    for item in event.turn.items {
                        self.handle_item(&event.thread_id, &event.turn.id, item, true);
                    }
                    let active_matches = self
                        .active_turns
                        .lock()
                        .expect("Codex active turn state poisoned")
                        .get(&event.thread_id)
                        .is_some_and(|turn| turn == &event.turn.id);
                    if active_matches {
                        self.active_turns
                            .lock()
                            .expect("Codex active turn state poisoned")
                            .remove(&event.thread_id);
                    }
                    match event.turn.status.as_str() {
                        "completed" => self.emit(&event.thread_id, ProviderEventKind::Completed),
                        "interrupted" => {
                            self.emit(&event.thread_id, ProviderEventKind::Interrupted)
                        }
                        "failed" => {
                            let key = (event.thread_id.clone(), event.turn.id.clone());
                            if self
                                .failed_turns
                                .lock()
                                .expect("Codex failed turn state poisoned")
                                .insert(key)
                            {
                                self.emit(
                                    &event.thread_id,
                                    ProviderEventKind::Failed {
                                        code: "codex_turn_failed".to_owned(),
                                        message: event
                                            .turn
                                            .error
                                            .and_then(|error| error.message)
                                            .unwrap_or_else(|| "Codex turn failed".to_owned()),
                                    },
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            "item/started" => {
                if let Ok(event) = serde_json::from_value::<ItemNotification>(params) {
                    self.handle_item(&event.thread_id, &event.turn_id, event.item, false);
                }
            }
            "item/completed" => {
                if let Ok(event) = serde_json::from_value::<ItemNotification>(params) {
                    self.handle_item(&event.thread_id, &event.turn_id, event.item, true);
                }
            }
            "item/agentMessage/delta" => {
                if let Ok(event) = serde_json::from_value::<TextDeltaNotification>(params) {
                    let key = ItemKey::new(&event.thread_id, &event.item_id);
                    let phase = {
                        let mut states = self
                            .message_states
                            .lock()
                            .expect("Codex message state poisoned");
                        let state = states.entry(key).or_insert_with(|| MessageState {
                            phase: AgentMessagePhase::Commentary,
                            text: String::new(),
                        });
                        state.text.push_str(&event.delta);
                        state.phase
                    };
                    self.emit(
                        &event.thread_id,
                        ProviderEventKind::AgentTextDelta {
                            provider_item_id: event.item_id,
                            phase,
                            delta: event.delta,
                        },
                    );
                }
            }
            "item/reasoning/summaryTextDelta" => {
                if let Ok(event) = serde_json::from_value::<ReasoningDeltaNotification>(params) {
                    self.reasoning_states
                        .lock()
                        .expect("Codex reasoning state poisoned")
                        .entry(ReasoningKey::new(
                            &event.thread_id,
                            &event.item_id,
                            event.summary_index,
                        ))
                        .or_default()
                        .push_str(&event.delta);
                    self.emit(
                        &event.thread_id,
                        ProviderEventKind::AgentTextDelta {
                            provider_item_id: format!(
                                "reasoning:{}:{}",
                                event.item_id, event.summary_index
                            ),
                            phase: AgentMessagePhase::ReasoningSummary,
                            delta: event.delta,
                        },
                    );
                }
            }
            // Raw reasoning is intentionally not forwarded.
            "item/reasoning/textDelta" | "item/reasoning/summaryPartAdded" => {}
            "turn/plan/updated" => {
                if let Ok(event) = serde_json::from_value::<PlanNotification>(params) {
                    self.emit(
                        &event.thread_id,
                        ProviderEventKind::Plan {
                            provider_item_id: format!("plan:{}", event.turn_id),
                            steps: event
                                .plan
                                .into_iter()
                                .map(|step| PlanStep {
                                    text: step.step,
                                    status: map_status(&step.status, ItemStatus::Pending),
                                })
                                .collect(),
                        },
                    );
                }
            }
            "item/commandExecution/outputDelta" => {
                if let Ok(event) = serde_json::from_value::<TextDeltaNotification>(params) {
                    let key = ItemKey::new(&event.thread_id, &event.item_id);
                    let state = {
                        let mut states = self
                            .command_states
                            .lock()
                            .expect("Codex command state poisoned");
                        let Some(state) = states.get_mut(&key) else {
                            return;
                        };
                        state.output.push_str(&event.delta);
                        state.clone()
                    };
                    self.emit_command(&event.thread_id, &event.item_id, state, ItemStatus::Running);
                }
            }
            "item/fileChange/patchUpdated" => {
                if let Ok(event) = serde_json::from_value::<FilePatchNotification>(params) {
                    self.emit_file_changes(
                        &event.thread_id,
                        &event.item_id,
                        &event.changes,
                        ItemStatus::Running,
                    );
                }
            }
            "item/mcpToolCall/progress" => {
                if let Ok(event) = serde_json::from_value::<McpProgressNotification>(params) {
                    self.emit(
                        &event.thread_id,
                        ProviderEventKind::ToolCall {
                            provider_item_id: event.item_id,
                            name: "MCP tool".to_owned(),
                            status: ItemStatus::Running,
                            input_summary: None,
                            output_summary: Some(event.message),
                        },
                    );
                }
            }
            "error" => {
                if let Ok(event) = serde_json::from_value::<ErrorNotification>(params)
                    && !event.will_retry
                {
                    let key = (event.thread_id.clone(), event.turn_id.clone());
                    if self
                        .failed_turns
                        .lock()
                        .expect("Codex failed turn state poisoned")
                        .insert(key)
                    {
                        self.emit(
                            &event.thread_id,
                            ProviderEventKind::Failed {
                                code: "codex_error".to_owned(),
                                message: event.error.message,
                            },
                        );
                    }
                }
            }
            "serverRequest/resolved" => {
                if let Ok(event) = serde_json::from_value::<RequestResolvedNotification>(params) {
                    self.pending_approvals
                        .lock()
                        .expect("Codex approval state poisoned")
                        .remove(&event.request_id.opaque());
                }
            }
            // Unknown stable additions and experimental notifications are ignored.
            _ => {}
        }
    }

    fn handle_item(&self, thread_id: &str, turn_id: &str, item: Value, completed: bool) {
        let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
        match kind {
            "agentMessage" => {
                if let Ok(item) = serde_json::from_value::<AgentMessageItem>(item) {
                    let key = ItemKey::new(thread_id, &item.id);
                    let item_phase = map_phase(item.phase.as_deref());
                    let delta = {
                        let mut states = self
                            .message_states
                            .lock()
                            .expect("Codex message state poisoned");
                        let state = states.entry(key).or_insert_with(|| MessageState {
                            phase: item_phase,
                            text: String::new(),
                        });
                        if state.text.is_empty() {
                            state.phase = item_phase;
                        }
                        if completed && item.text.starts_with(&state.text) {
                            let suffix = item.text[state.text.len()..].to_owned();
                            state.text = item.text;
                            suffix
                        } else {
                            String::new()
                        }
                    };
                    if !delta.is_empty() {
                        let phase = self
                            .message_states
                            .lock()
                            .expect("Codex message state poisoned")
                            .get(&ItemKey::new(thread_id, &item.id))
                            .map_or(item_phase, |state| state.phase);
                        self.emit(
                            thread_id,
                            ProviderEventKind::AgentTextDelta {
                                provider_item_id: item.id,
                                phase,
                                delta,
                            },
                        );
                    }
                }
            }
            "reasoning" if completed => {
                if let Ok(item) = serde_json::from_value::<ReasoningItem>(item) {
                    for (index, summary) in item.summary.into_iter().enumerate() {
                        let key = ReasoningKey::new(thread_id, &item.id, index as u64);
                        let suffix = {
                            let mut states = self
                                .reasoning_states
                                .lock()
                                .expect("Codex reasoning state poisoned");
                            let state = states.entry(key).or_default();
                            if summary.starts_with(state.as_str()) {
                                let suffix = summary[state.len()..].to_owned();
                                *state = summary;
                                suffix
                            } else {
                                String::new()
                            }
                        };
                        if !suffix.is_empty() {
                            self.emit(
                                thread_id,
                                ProviderEventKind::AgentTextDelta {
                                    provider_item_id: format!("reasoning:{}:{index}", item.id),
                                    phase: AgentMessagePhase::ReasoningSummary,
                                    delta: suffix,
                                },
                            );
                        }
                    }
                }
            }
            "commandExecution" => {
                if let Ok(item) = serde_json::from_value::<CommandItem>(item) {
                    let status = map_status(
                        item.status.as_deref().unwrap_or(if completed {
                            "completed"
                        } else {
                            "inProgress"
                        }),
                        if completed {
                            ItemStatus::Completed
                        } else {
                            ItemStatus::Running
                        },
                    );
                    let state = CommandState {
                        command: item.command,
                        cwd: item.cwd,
                        output: item.aggregated_output.unwrap_or_default(),
                        exit_code: item.exit_code,
                    };
                    self.command_states
                        .lock()
                        .expect("Codex command state poisoned")
                        .insert(ItemKey::new(thread_id, &item.id), state.clone());
                    self.emit_command(thread_id, &item.id, state, status);
                }
            }
            "fileChange" => {
                if let Ok(item) = serde_json::from_value::<FileChangeItem>(item) {
                    let status = map_status(
                        &item.status,
                        if completed {
                            ItemStatus::Completed
                        } else {
                            ItemStatus::Running
                        },
                    );
                    self.emit_file_changes(thread_id, &item.id, &item.changes, status);
                }
            }
            "mcpToolCall" | "dynamicToolCall" | "collabAgentToolCall" => {
                if let Ok(item) = serde_json::from_value::<GenericToolItem>(item) {
                    let name = match (item.server, item.tool) {
                        (Some(server), Some(tool)) => format!("{server}/{tool}"),
                        (_, Some(tool)) => tool,
                        _ => "Codex tool".to_owned(),
                    };
                    self.emit(
                        thread_id,
                        ProviderEventKind::ToolCall {
                            provider_item_id: item.id,
                            name,
                            status: map_status(
                                item.status.as_deref().unwrap_or(if completed {
                                    "completed"
                                } else {
                                    "inProgress"
                                }),
                                if completed {
                                    ItemStatus::Completed
                                } else {
                                    ItemStatus::Running
                                },
                            ),
                            input_summary: item.arguments.as_ref().map(compact_json),
                            output_summary: item
                                .error
                                .as_ref()
                                .or(item.result.as_ref())
                                .map(compact_json),
                        },
                    );
                }
            }
            "webSearch" => {
                if let Ok(item) = serde_json::from_value::<WebSearchItem>(item) {
                    self.emit(
                        thread_id,
                        ProviderEventKind::ToolCall {
                            provider_item_id: item.id,
                            name: "web_search".to_owned(),
                            status: if completed {
                                ItemStatus::Completed
                            } else {
                                ItemStatus::Running
                            },
                            input_summary: (!item.query.is_empty()).then_some(item.query),
                            output_summary: item.results.as_ref().map(compact_json),
                        },
                    );
                }
            }
            "imageView" if completed => {
                if let Ok(item) = serde_json::from_value::<ImageViewItem>(item) {
                    let key = ItemKey::new(thread_id, &item.id);
                    if self
                        .emitted_images
                        .lock()
                        .expect("Codex image state poisoned")
                        .insert(key)
                    {
                        self.emit(
                            thread_id,
                            ProviderEventKind::ImagePath {
                                path: PathBuf::from(item.path),
                                controlled_temp_roots: Vec::new(),
                                alt: "Image viewed by Codex".to_owned(),
                            },
                        );
                    }
                }
            }
            "imageGeneration" if completed => {
                if let Ok(item) = serde_json::from_value::<ImageGenerationItem>(item) {
                    let key = ItemKey::new(thread_id, &item.id);
                    if !self
                        .emitted_images
                        .lock()
                        .expect("Codex image state poisoned")
                        .insert(key)
                    {
                        return;
                    }
                    if item.status == "completed" && !item.result.is_empty() {
                        match BASE64_STANDARD.decode(item.result.trim()) {
                            Ok(bytes) => self.emit(
                                thread_id,
                                ProviderEventKind::ImageBytes {
                                    bytes,
                                    mime_type: "image/png".to_owned(),
                                    alt: item
                                        .revised_prompt
                                        .unwrap_or_else(|| "Image generated by Codex".to_owned()),
                                },
                            ),
                            Err(_) => self.emit(
                                thread_id,
                                ProviderEventKind::ToolCall {
                                    provider_item_id: item.id,
                                    name: "image_generation".to_owned(),
                                    status: ItemStatus::Failed,
                                    input_summary: item.revised_prompt,
                                    output_summary: Some(
                                        "Codex returned invalid generated-image data".to_owned(),
                                    ),
                                },
                            ),
                        }
                    } else {
                        self.emit(
                            thread_id,
                            ProviderEventKind::ToolCall {
                                provider_item_id: item.id,
                                name: "image_generation".to_owned(),
                                status: ItemStatus::Failed,
                                input_summary: item.revised_prompt,
                                output_summary: item.failure.as_ref().map(compact_json),
                            },
                        );
                    }
                }
            }
            "contextCompaction" | "subAgentActivity" => {
                if let Some(id) = item.get("id").and_then(Value::as_str) {
                    self.emit(
                        thread_id,
                        ProviderEventKind::ToolCall {
                            provider_item_id: id.to_owned(),
                            name: kind.to_owned(),
                            status: if completed {
                                ItemStatus::Completed
                            } else {
                                ItemStatus::Running
                            },
                            input_summary: None,
                            output_summary: None,
                        },
                    );
                }
            }
            _ => {
                let _ = turn_id;
            }
        }
    }

    fn emit_command(
        &self,
        thread_id: &str,
        item_id: &str,
        state: CommandState,
        status: ItemStatus,
    ) {
        self.emit(
            thread_id,
            ProviderEventKind::Command {
                provider_item_id: item_id.to_owned(),
                command: state.command,
                relative_cwd: Some(state.cwd),
                status,
                exit_code: state.exit_code,
                output: (!state.output.is_empty()).then_some(state.output),
            },
        );
    }

    fn emit_file_changes(
        &self,
        thread_id: &str,
        item_id: &str,
        changes: &[FileUpdateChange],
        status: ItemStatus,
    ) {
        for (index, change) in changes.iter().enumerate() {
            self.emit(
                thread_id,
                ProviderEventKind::FileChange {
                    provider_item_id: format!("{item_id}:{index}:{}", change.path),
                    relative_path: change.path.clone(),
                    change_kind: file_change_kind(&change.kind),
                    status,
                },
            );
        }
    }
}

struct RpcWire {
    writer: AsyncMutex<DynWriter>,
    pending: Mutex<HashMap<RpcId, oneshot::Sender<std::result::Result<Value, RpcFailure>>>>,
    next_id: AtomicU64,
    closed: AtomicBool,
    generation: u64,
}

impl RpcWire {
    fn start<R, W>(reader: R, writer: W, shared: Weak<Shared>, generation: u64) -> Arc<Self>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let wire = Arc::new(Self {
            writer: AsyncMutex::new(Box::new(writer)),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            closed: AtomicBool::new(false),
            generation,
        });
        let reader_wire = Arc::downgrade(&wire);
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        if line.trim().is_empty() {
                            continue;
                        }
                        let Ok(envelope) = serde_json::from_str::<IncomingEnvelope>(&line) else {
                            tracing::warn!("Codex app-server emitted malformed JSONL");
                            continue;
                        };
                        let Some(wire) = reader_wire.upgrade() else {
                            break;
                        };
                        if envelope.method.is_none() {
                            if let Some(id) = envelope.id {
                                let result = match envelope.error {
                                    Some(error) => Err(RpcFailure::Server(error)),
                                    None => Ok(envelope.result.unwrap_or(Value::Null)),
                                };
                                if let Some(sender) = wire
                                    .pending
                                    .lock()
                                    .expect("Codex RPC pending state poisoned")
                                    .remove(&id)
                                {
                                    let _ = sender.send(result);
                                }
                            }
                            continue;
                        }
                        let Some(shared) = shared.upgrade() else {
                            break;
                        };
                        let method = envelope.method.unwrap_or_default();
                        let params = envelope.params.unwrap_or_else(|| json!({}));
                        if let Some(id) = envelope.id {
                            shared
                                .handle_server_request(Arc::clone(&wire), id, method, params)
                                .await;
                        } else {
                            shared.handle_notification(&method, params);
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
            if let (Some(shared), Some(wire)) = (shared.upgrade(), reader_wire.upgrade()) {
                shared
                    .disconnect(wire, "Codex app-server stdout closed".to_owned())
                    .await;
            }
        });
        wire
    }

    async fn request<P, R>(&self, method: &'static str, params: &P) -> Result<R>
    where
        P: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        if self.closed.load(Ordering::Acquire) {
            bail!("Codex app-server connection is closed");
        }
        let id = RpcId::Number(self.next_id.fetch_add(1, Ordering::Relaxed) as i64);
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .expect("Codex RPC pending state poisoned")
            .insert(id.clone(), sender);
        let request = OutgoingRequest {
            method,
            id: &id,
            params,
        };
        if let Err(error) = self.write_json(&request).await {
            self.pending
                .lock()
                .expect("Codex RPC pending state poisoned")
                .remove(&id);
            return Err(error);
        }
        let value = receiver
            .await
            .context("Codex app-server response channel closed")?
            .map_err(|error| anyhow!(error.to_string()))?;
        serde_json::from_value(value).with_context(|| format!("decode Codex response for {method}"))
    }

    async fn notify_initialized(&self) -> Result<()> {
        self.write_json(&InitializedNotification {
            method: "initialized",
        })
        .await
    }

    async fn respond_result(&self, id: RpcId, result: Value) -> Result<()> {
        self.write_json(&OutgoingResult {
            id: &id,
            result: &result,
        })
        .await
    }

    async fn respond_error(&self, id: RpcId, code: i64, message: &'static str) -> Result<()> {
        self.write_json(&OutgoingError {
            id: &id,
            error: OutgoingErrorBody { code, message },
        })
        .await
    }

    async fn write_json<T: Serialize + ?Sized>(&self, value: &T) -> Result<()> {
        let mut line = serde_json::to_vec(value).context("encode Codex JSONL request")?;
        line.push(b'\n');
        let mut writer = self.writer.lock().await;
        writer
            .write_all(&line)
            .await
            .context("write Codex JSONL request")?;
        writer.flush().await.context("flush Codex JSONL request")
    }

    fn fail_pending(&self, message: &'static str) {
        let pending = std::mem::take(
            &mut *self
                .pending
                .lock()
                .expect("Codex RPC pending state poisoned"),
        );
        for (_, sender) in pending {
            let _ = sender.send(Err(RpcFailure::Disconnected(message)));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
enum RpcId {
    Number(i64),
    String(String),
}

impl RpcId {
    fn opaque(&self) -> String {
        match self {
            Self::Number(value) => format!("n:{value}"),
            Self::String(value) => format!("s:{value}"),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IncomingEnvelope {
    #[serde(default)]
    id: Option<RpcId>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<RpcError>,
    #[allow(dead_code)]
    #[serde(default)]
    emitted_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
    #[allow(dead_code)]
    #[serde(default)]
    data: Option<Value>,
}

#[derive(Debug)]
enum RpcFailure {
    Server(RpcError),
    Disconnected(&'static str),
}

impl std::fmt::Display for RpcFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Server(error) => write!(formatter, "Codex RPC {}: {}", error.code, error.message),
            Self::Disconnected(message) => formatter.write_str(message),
        }
    }
}

#[derive(Serialize)]
struct OutgoingRequest<'a, P: ?Sized> {
    method: &'static str,
    id: &'a RpcId,
    params: &'a P,
}

#[derive(Serialize)]
struct InitializedNotification {
    method: &'static str,
}

#[derive(Serialize)]
struct OutgoingResult<'a> {
    id: &'a RpcId,
    result: &'a Value,
}

#[derive(Serialize)]
struct OutgoingError<'a> {
    id: &'a RpcId,
    error: OutgoingErrorBody,
}

#[derive(Serialize)]
struct OutgoingErrorBody {
    code: i64,
    message: &'static str,
}

#[derive(Debug, Clone)]
struct SessionContext {
    project_id: ProjectId,
    conversation_id: ConversationId,
    project_path: PathBuf,
    model: Option<String>,
    effort: Option<String>,
    loaded_generation: u64,
}

struct PendingApproval {
    rpc_id: RpcId,
    conversation_id: ConversationId,
    kind: PendingApprovalKind,
    wire: Arc<RpcWire>,
}

enum PendingApprovalKind {
    Command,
    FileChange,
    Permissions { requested: Value },
    LegacyCommand,
    LegacyPatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ItemKey {
    thread_id: String,
    item_id: String,
}

impl ItemKey {
    fn new(thread_id: &str, item_id: &str) -> Self {
        Self {
            thread_id: thread_id.to_owned(),
            item_id: item_id.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ReasoningKey {
    thread_id: String,
    item_id: String,
    summary_index: u64,
}

impl ReasoningKey {
    fn new(thread_id: &str, item_id: &str, summary_index: u64) -> Self {
        Self {
            thread_id: thread_id.to_owned(),
            item_id: item_id.to_owned(),
            summary_index,
        }
    }
}

#[derive(Clone)]
struct MessageState {
    phase: AgentMessagePhase,
    text: String,
}

#[derive(Clone)]
struct CommandState {
    command: String,
    cwd: String,
    output: String,
    exit_code: Option<i32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InitializeParams<'a> {
    client_info: ClientInfo<'a>,
    capabilities: InitializeCapabilities,
}

#[derive(Serialize)]
struct ClientInfo<'a> {
    name: &'a str,
    title: &'a str,
    version: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InitializeCapabilities {
    experimental_api: bool,
    request_attestation: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitializeResponse {
    #[allow(dead_code)]
    user_agent: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountReadParams {
    refresh_token: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountReadResponse {
    account: Option<Value>,
    requires_openai_auth: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelListParams<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    include_hidden: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelListResponse {
    data: Vec<CodexModel>,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexModel {
    model: String,
    display_name: String,
    #[serde(default)]
    hidden: bool,
    supported_reasoning_efforts: Vec<CodexEffort>,
    default_reasoning_effort: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexEffort {
    reasoning_effort: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadListParams<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    sort_key: &'static str,
    sort_direction: &'static str,
    source_kinds: &'a [&'static str],
    archived: bool,
    cwd: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadListResponse {
    data: Vec<CodexThread>,
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexThread {
    id: String,
    #[serde(default)]
    preview: String,
    cwd: String,
    #[serde(default)]
    updated_at: i64,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadReadParams<'a> {
    thread_id: &'a str,
    include_turns: bool,
}

#[derive(Deserialize)]
struct ThreadReadResponse {
    thread: CodexThread,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadStartParams<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
    cwd: String,
    approvals_reviewer: &'static str,
    service_name: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadResumeParams<'a> {
    thread_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
    cwd: String,
    approvals_reviewer: &'static str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadOpenResponse {
    thread: CodexThread,
    model: String,
    #[serde(default)]
    reasoning_effort: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TurnStartParams<'a> {
    thread_id: &'a str,
    client_user_message_id: Option<String>,
    input: Vec<TextInput>,
    cwd: String,
    approvals_reviewer: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    effort: Option<&'a str>,
}

#[derive(Serialize)]
struct TextInput {
    #[serde(rename = "type")]
    kind: &'static str,
    text: String,
}

#[derive(Deserialize)]
struct TurnStartResponse {
    turn: CodexTurn,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TurnSteerParams<'a> {
    thread_id: &'a str,
    client_user_message_id: Option<String>,
    input: Vec<TextInput>,
    expected_turn_id: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TurnSteerResponse {
    turn_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TurnInterruptParams<'a> {
    thread_id: &'a str,
    turn_id: &'a str,
}

#[derive(Deserialize)]
struct EmptyResponse {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TurnNotification {
    thread_id: String,
    turn: CodexTurn,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexTurn {
    id: String,
    #[serde(default)]
    items: Vec<Value>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    error: Option<TurnError>,
}

#[derive(Deserialize)]
struct TurnError {
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItemNotification {
    thread_id: String,
    turn_id: String,
    item: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TextDeltaNotification {
    thread_id: String,
    #[allow(dead_code)]
    turn_id: String,
    item_id: String,
    delta: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReasoningDeltaNotification {
    thread_id: String,
    #[allow(dead_code)]
    turn_id: String,
    item_id: String,
    delta: String,
    summary_index: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlanNotification {
    thread_id: String,
    turn_id: String,
    plan: Vec<CodexPlanStep>,
}

#[derive(Deserialize)]
struct CodexPlanStep {
    step: String,
    status: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FilePatchNotification {
    thread_id: String,
    #[allow(dead_code)]
    turn_id: String,
    item_id: String,
    changes: Vec<FileUpdateChange>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpProgressNotification {
    thread_id: String,
    #[allow(dead_code)]
    turn_id: String,
    item_id: String,
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ErrorNotification {
    error: ErrorBody,
    will_retry: bool,
    thread_id: String,
    turn_id: String,
}

#[derive(Deserialize)]
struct ErrorBody {
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestResolvedNotification {
    #[allow(dead_code)]
    thread_id: String,
    request_id: RpcId,
}

#[derive(Deserialize)]
struct AgentMessageItem {
    id: String,
    text: String,
    #[serde(default)]
    phase: Option<String>,
}

#[derive(Deserialize)]
struct ReasoningItem {
    id: String,
    #[serde(default)]
    summary: Vec<String>,
    // Deliberately no `content` field: raw reasoning must not leave the host.
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommandItem {
    id: String,
    command: String,
    cwd: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    aggregated_output: Option<String>,
    #[serde(default)]
    exit_code: Option<i32>,
}

#[derive(Deserialize)]
struct FileChangeItem {
    id: String,
    changes: Vec<FileUpdateChange>,
    status: String,
}

#[derive(Deserialize)]
struct FileUpdateChange {
    path: String,
    kind: Value,
}

#[derive(Deserialize)]
struct GenericToolItem {
    id: String,
    #[serde(default)]
    server: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    arguments: Option<Value>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<Value>,
}

#[derive(Deserialize)]
struct WebSearchItem {
    id: String,
    #[serde(default)]
    query: String,
    #[serde(default)]
    results: Option<Value>,
}

#[derive(Deserialize)]
struct ImageViewItem {
    id: String,
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageGenerationItem {
    id: String,
    status: String,
    #[serde(default)]
    revised_prompt: Option<String>,
    #[serde(default)]
    result: String,
    #[serde(default)]
    failure: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommandApprovalParams {
    thread_id: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileApprovalParams {
    thread_id: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PermissionApprovalParams {
    thread_id: String,
    #[serde(default)]
    reason: Option<String>,
    permissions: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyExecApprovalParams {
    conversation_id: String,
    command: Vec<String>,
    cwd: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyPatchApprovalParams {
    conversation_id: String,
    #[serde(default)]
    reason: Option<String>,
}

fn provider_health(
    state: ProviderState,
    version: Option<String>,
    detail: Option<&str>,
) -> ProviderHealth {
    ProviderHealth {
        provider: ProviderId::Codex,
        state,
        version,
        detail: detail.map(str::to_owned),
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn thread_matches_project(thread: &CodexThread, project: &Path) -> bool {
    let thread_path = PathBuf::from(&thread.cwd);
    match (thread_path.canonicalize(), project.canonicalize()) {
        (Ok(thread), Ok(project)) => thread == project,
        _ => {
            #[cfg(windows)]
            {
                thread.cwd.eq_ignore_ascii_case(&path_string(project))
            }
            #[cfg(not(windows))]
            {
                thread.cwd == path_string(project)
            }
        }
    }
}

fn validate_thread_project(thread: &CodexThread, project: &Path) -> Result<()> {
    if !thread_matches_project(thread, project) {
        bail!("Codex thread belongs to a different project path");
    }
    Ok(())
}

fn native_session(
    thread: CodexThread,
    selected_model: Option<String>,
    selected_effort: Option<String>,
    models: &[ModelOption],
) -> NativeSession {
    let title = thread
        .name
        .filter(|name| !name.trim().is_empty())
        .or_else(|| (!thread.preview.trim().is_empty()).then_some(thread.preview))
        .unwrap_or_else(|| "New Codex session".to_owned());
    let mut session_options = Vec::new();
    if let Some(model_id) = selected_model.as_ref() {
        session_options.push(SessionOption {
            id: "model".to_owned(),
            display_name: "Model".to_owned(),
            category: Some("Codex".to_owned()),
            current_value: model_id.clone(),
            values: models
                .iter()
                .map(|model| SessionOptionValue {
                    value: model.id.clone(),
                    display_name: model.display_name.clone(),
                })
                .collect(),
        });
        if let Some(model) = models.iter().find(|model| &model.id == model_id) {
            let current_effort = selected_effort
                .clone()
                .or_else(|| model.default_effort.clone())
                .unwrap_or_default();
            session_options.push(SessionOption {
                id: "reasoning_effort".to_owned(),
                display_name: "Reasoning effort".to_owned(),
                category: Some("Codex".to_owned()),
                current_value: current_effort,
                values: model
                    .effort_options
                    .iter()
                    .map(|effort| SessionOptionValue {
                        value: effort.id.clone(),
                        display_name: effort.display_name.clone(),
                    })
                    .collect(),
            });
        }
    }
    NativeSession {
        native_session_id: thread.id,
        title,
        selected_model,
        selected_effort,
        session_options,
    }
}

fn map_phase(phase: Option<&str>) -> AgentMessagePhase {
    match phase {
        Some("final_answer") => AgentMessagePhase::Final,
        _ => AgentMessagePhase::Commentary,
    }
}

fn map_status(status: &str, fallback: ItemStatus) -> ItemStatus {
    match status {
        "pending" => ItemStatus::Pending,
        "inProgress" | "in_progress" | "running" => ItemStatus::Running,
        "completed" => ItemStatus::Completed,
        "failed" => ItemStatus::Failed,
        "declined" => ItemStatus::Declined,
        "interrupted" => ItemStatus::Interrupted,
        _ => fallback,
    }
}

fn file_change_kind(kind: &Value) -> String {
    kind.as_str()
        .or_else(|| kind.get("type").and_then(Value::as_str))
        .unwrap_or("update")
        .to_owned()
}

fn compact_json(value: &Value) -> String {
    const LIMIT: usize = 8 * 1024;
    let mut result = match value {
        Value::String(value) => value.clone(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "<unavailable>".to_owned()),
    };
    if result.len() > LIMIT {
        result.truncate(LIMIT);
        result.push('…');
    }
    result
}

fn approval_prompt(reason: Option<&str>, command: Option<&str>, cwd: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(reason) = reason.filter(|value| !value.trim().is_empty()) {
        parts.push(reason.to_owned());
    }
    if let Some(command) = command.filter(|value| !value.trim().is_empty()) {
        parts.push(format!("Command: {command}"));
    }
    if let Some(cwd) = cwd.filter(|value| !value.trim().is_empty()) {
        parts.push(format!("Working directory: {cwd}"));
    }
    if parts.is_empty() {
        "Allow this Codex action?".to_owned()
    } else {
        parts.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncWriteExt, DuplexStream, duplex, split};

    fn test_provider() -> CodexProvider {
        CodexProvider::with_executable("unused-in-duplex-tests")
    }

    fn test_project() -> Project {
        Project {
            id: ProjectId::new(),
            display_name: "test".to_owned(),
            canonical_path: std::env::current_dir().expect("current directory"),
            enabled_providers: vec![ProviderId::Codex],
        }
    }

    async fn install_duplex(provider: &CodexProvider) -> DuplexStream {
        let (client, server) = duplex(128 * 1024);
        let (reader, writer) = split(client);
        let wire = RpcWire::start(reader, writer, Arc::downgrade(&provider.shared), 1);
        *provider.shared.connection.lock().await = Some(wire);
        server
    }

    async fn read_json(reader: &mut BufReader<tokio::io::ReadHalf<DuplexStream>>) -> Value {
        let mut line = String::new();
        reader.read_line(&mut line).await.expect("read JSONL");
        serde_json::from_str(&line).expect("valid JSONL")
    }

    #[tokio::test]
    async fn initialize_is_followed_by_initialized_without_experimental_opt_in() {
        let provider = test_provider();
        let (client, server) = duplex(32 * 1024);
        let (client_read, client_write) = split(client);
        let wire = RpcWire::start(
            client_read,
            client_write,
            Arc::downgrade(&provider.shared),
            1,
        );
        let (server_read, mut server_write) = split(server);
        let mut server_read = BufReader::new(server_read);
        let mock = tokio::spawn(async move {
            let initialize = read_json(&mut server_read).await;
            assert_eq!(initialize["method"], "initialize");
            assert_eq!(
                initialize["params"]["capabilities"]["experimentalApi"],
                false
            );
            assert_eq!(
                initialize["params"]["capabilities"]["requestAttestation"],
                false
            );
            let id = initialize["id"].clone();
            server_write
                .write_all(
                    format!(
                        "{}\n",
                        json!({"id":id,"result":{
                            "userAgent":"codex/0.150.1","codexHome":"hidden",
                            "platformFamily":"windows","platformOs":"windows"
                        }})
                    )
                    .as_bytes(),
                )
                .await
                .expect("write initialize response");
            let initialized = read_json(&mut server_read).await;
            assert_eq!(initialized, json!({"method":"initialized"}));
        });
        let _: InitializeResponse = wire
            .request(
                "initialize",
                &InitializeParams {
                    client_info: ClientInfo {
                        name: CLIENT_NAME,
                        title: CLIENT_TITLE,
                        version: CLIENT_VERSION,
                    },
                    capabilities: InitializeCapabilities {
                        experimental_api: false,
                        request_attestation: false,
                    },
                },
            )
            .await
            .expect("initialize response");
        wire.notify_initialized()
            .await
            .expect("initialized notification");
        mock.await.expect("mock server");
    }

    #[tokio::test]
    async fn paginated_models_keep_dynamic_effort_order() {
        let provider = test_provider();
        let server = install_duplex(&provider).await;
        let (server_read, mut server_write) = split(server);
        let mut server_read = BufReader::new(server_read);
        let mock = tokio::spawn(async move {
            let first = read_json(&mut server_read).await;
            assert_eq!(first["method"], "model/list");
            assert_eq!(first["params"]["includeHidden"], false);
            let first_id = first["id"].clone();
            server_write
                .write_all(
                    format!(
                        "{}\n",
                        json!({"id":first_id,"result":{"data":[{
                            "model":"model-a","displayName":"Model A","hidden":false,
                            "supportedReasoningEfforts":[
                                {"reasoningEffort":"ultra-new"},
                                {"reasoningEffort":"low"}
                            ],"defaultReasoningEffort":"ultra-new"
                        }],"nextCursor":"page-2"}})
                    )
                    .as_bytes(),
                )
                .await
                .expect("write first model page");
            let second = read_json(&mut server_read).await;
            assert_eq!(second["params"]["cursor"], "page-2");
            let second_id = second["id"].clone();
            server_write
                .write_all(
                    format!(
                        "{}\n",
                        json!({"id":second_id,"result":{"data":[{
                            "model":"model-b","displayName":"Model B","hidden":false,
                            "supportedReasoningEfforts":[{"reasoningEffort":"custom"}],
                            "defaultReasoningEffort":"custom"
                        }],"nextCursor":null}})
                    )
                    .as_bytes(),
                )
                .await
                .expect("write second model page");
        });
        let models = provider
            .list_models(&test_project())
            .await
            .expect("list models");
        mock.await.expect("mock server");
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "model-a");
        assert_eq!(
            models[0]
                .effort_options
                .iter()
                .map(|effort| effort.id.as_str())
                .collect::<Vec<_>>(),
            ["ultra-new", "low"]
        );
    }

    #[tokio::test]
    async fn maps_visible_deltas_plan_and_ignores_raw_reasoning_and_unknown_events() {
        let provider = test_provider();
        let mut events = provider.subscribe();
        let project = test_project();
        let conversation_id = ConversationId::new();
        provider.shared.register_session(
            "thread-1".to_owned(),
            SessionContext {
                project_id: project.id,
                conversation_id,
                project_path: project.canonical_path,
                model: None,
                effort: None,
                loaded_generation: 1,
            },
        );
        provider.shared.handle_notification(
            "item/started",
            json!({
                "threadId":"thread-1","turnId":"turn-1","startedAtMs":1,
                "item":{"type":"agentMessage","id":"message-1","text":"","phase":"final_answer"}
            }),
        );
        provider.shared.handle_notification(
            "item/agentMessage/delta",
            json!({"threadId":"thread-1","turnId":"turn-1","itemId":"message-1","delta":"done"}),
        );
        provider.shared.handle_notification(
            "item/reasoning/textDelta",
            json!({"threadId":"thread-1","turnId":"turn-1","itemId":"reason-1","delta":"secret","contentIndex":0}),
        );
        provider.shared.handle_notification(
            "item/reasoning/summaryTextDelta",
            json!({"threadId":"thread-1","turnId":"turn-1","itemId":"reason-1","delta":"checked tests","summaryIndex":0}),
        );
        provider.shared.handle_notification(
            "turn/plan/updated",
            json!({"threadId":"thread-1","turnId":"turn-1","explanation":null,"plan":[{"step":"Run tests","status":"inProgress"}]}),
        );
        provider
            .shared
            .handle_notification("future/event", json!({"unexpected":true}));

        let first = events.recv().await.expect("agent delta");
        assert!(matches!(
            first.kind,
            ProviderEventKind::AgentTextDelta {
                phase: AgentMessagePhase::Final,
                ref delta,
                ..
            } if delta == "done"
        ));
        let second = events.recv().await.expect("reasoning summary");
        assert!(matches!(
            second.kind,
            ProviderEventKind::AgentTextDelta {
                phase: AgentMessagePhase::ReasoningSummary,
                ref delta,
                ..
            } if delta == "checked tests"
        ));
        let third = events.recv().await.expect("plan");
        assert!(matches!(
            third.kind,
            ProviderEventKind::Plan { ref steps, .. }
                if steps.len() == 1 && steps[0].status == ItemStatus::Running
        ));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn approval_is_correlated_and_replied_to_with_server_request_id() {
        let provider = test_provider();
        let mut events = provider.subscribe();
        let project = test_project();
        let conversation_id = ConversationId::new();
        provider.shared.register_session(
            "thread-approval".to_owned(),
            SessionContext {
                project_id: project.id,
                conversation_id,
                project_path: project.canonical_path,
                model: None,
                effort: None,
                loaded_generation: 1,
            },
        );
        let mut server = install_duplex(&provider).await;
        server
            .write_all(
                concat!(
                    "{\"method\":\"item/commandExecution/requestApproval\",\"id\":77,",
                    "\"params\":{\"threadId\":\"thread-approval\",\"turnId\":\"turn-1\",",
                    "\"itemId\":\"item-1\",\"startedAtMs\":1,\"kind\":\"command\",",
                    "\"environmentId\":null,\"command\":\"cargo test\",\"cwd\":\".\"}}\n"
                )
                .as_bytes(),
            )
            .await
            .expect("send approval request");
        let event = events.recv().await.expect("approval event");
        let provider_request_id = match event.kind {
            ProviderEventKind::Approval {
                provider_request_id,
                options,
                ..
            } => {
                assert_eq!(options[0].id, "accept");
                provider_request_id
            }
            _ => panic!("expected approval"),
        };
        provider
            .resolve_approval(ResolveApproval {
                conversation_id,
                provider_request_id,
                option_id: "accept".to_owned(),
            })
            .await
            .expect("resolve approval");
        let mut response = String::new();
        BufReader::new(server)
            .read_line(&mut response)
            .await
            .expect("read approval response");
        let response: Value = serde_json::from_str(&response).expect("valid approval response");
        assert_eq!(response["id"], 77);
        assert_eq!(response["result"]["decision"], "accept");
    }

    #[tokio::test]
    async fn maps_image_view_and_generated_image_items() {
        let provider = test_provider();
        let mut events = provider.subscribe();
        let project = test_project();
        let conversation_id = ConversationId::new();
        provider.shared.register_session(
            "thread-images".to_owned(),
            SessionContext {
                project_id: project.id,
                conversation_id,
                project_path: project.canonical_path.clone(),
                model: None,
                effort: None,
                loaded_generation: 1,
            },
        );
        provider.shared.handle_notification(
            "item/completed",
            json!({
                "threadId":"thread-images","turnId":"turn-1","completedAtMs":1,
                "item":{"type":"imageView","id":"view-1","path":path_string(&project.canonical_path.join("shot.png"))}
            }),
        );
        provider.shared.handle_notification(
            "item/completed",
            json!({
                "threadId":"thread-images","turnId":"turn-1","completedAtMs":2,
                "item":{
                    "type":"imageGeneration","id":"generated-1","status":"completed",
                    "revisedPrompt":"a small image","result":BASE64_STANDARD.encode([1_u8,2,3]),
                    "failure":null
                }
            }),
        );
        let viewed = events.recv().await.expect("image view event");
        assert!(matches!(viewed.kind, ProviderEventKind::ImagePath { .. }));
        let generated = events.recv().await.expect("generated image event");
        assert!(matches!(
            generated.kind,
            ProviderEventKind::ImageBytes { ref bytes, ref mime_type, .. }
                if bytes == &[1, 2, 3] && mime_type == "image/png"
        ));
    }
}
