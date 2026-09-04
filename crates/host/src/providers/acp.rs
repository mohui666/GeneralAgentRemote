//! Shared ACP v1 adapter for coding-agent CLIs.

use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex as StdMutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

use agent_client_protocol::{
    AcpAgent, AcpAgentConfig, Agent, Client, ConnectionTo, JsonRpcMessage, JsonRpcNotification,
    JsonRpcRequest, JsonRpcResponse, Responder, UntypedMessage,
    schema::{
        ProtocolVersion,
        v1::{
            AuthenticateRequest, BooleanConfigOptionCapabilities, CancelNotification,
            ClientCapabilities, ClientSessionCapabilities, CloseSessionRequest, ContentBlock,
            CreateTerminalRequest, CreateTerminalResponse, FileSystemCapabilities, Implementation,
            InitializeRequest, InitializeResponse, KillTerminalRequest, KillTerminalResponse,
            ListSessionsRequest, LoadSessionRequest, Meta, NewSessionRequest, PermissionOptionId,
            PlanEntryStatus, PromptRequest, ReadTextFileRequest, ReadTextFileResponse,
            ReleaseTerminalRequest, ReleaseTerminalResponse, RequestPermissionOutcome,
            RequestPermissionRequest, RequestPermissionResponse, ResumeSessionRequest,
            SelectedPermissionOutcome, SessionConfigKind, SessionConfigOption,
            SessionConfigOptionCategory, SessionConfigOptionValue,
            SessionConfigOptionsCapabilities, SessionConfigSelectOptions, SessionId,
            SessionModeState, SessionUpdate, SetSessionConfigOptionRequest, SetSessionModeRequest,
            StopReason, TerminalExitStatus, TerminalOutputRequest, TerminalOutputResponse,
            TextContent, ToolCall, ToolCallContent, ToolCallStatus, WaitForTerminalExitRequest,
            WaitForTerminalExitResponse, WriteTextFileRequest, WriteTextFileResponse,
        },
    },
};
use agent_remote_protocol::{
    AgentMessagePhase, ApprovalOption, ConversationId, EffortOption, ItemStatus, ModelOption,
    PlanStep, ProjectId, ProviderHealth, ProviderId, ProviderState, SessionOption,
    SessionOptionValue, SessionSummary, TimelineItemKind,
};
use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::{Mutex as AsyncMutex, broadcast, mpsc, oneshot, watch},
};
use uuid::Uuid;

use super::{
    AgentProvider, CommandAck, CreateSession, InterruptSession, NativeSession,
    ProviderCapabilities, ProviderEvent, ProviderEventKind, ProviderHistoryBarrier,
    ProviderHistoryPage, ReadSessionHistory, ResolveApproval, ResumeSession, SendMessage,
    SessionOptionsSnapshot, SetSessionOption, SteerMessage,
};
use crate::storage::{Project, now_ms};

const GROK_EXTENSION_VERSION: &str = "1.0.13";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AcpFlavor {
    Standard,
    Grok,
}

#[derive(Debug, Clone)]
pub struct AcpProviderConfig {
    provider: ProviderId,
    display_name: &'static str,
    executable: PathBuf,
    agent_args: Vec<String>,
    version_args: Vec<String>,
    auth_method: Option<&'static str>,
    flavor: AcpFlavor,
}

impl AcpProviderConfig {
    fn new(
        provider: ProviderId,
        display_name: &'static str,
        executable: impl Into<PathBuf>,
        agent_args: &[&str],
        version_args: &[&str],
    ) -> Self {
        Self {
            provider,
            display_name,
            executable: executable.into(),
            agent_args: agent_args.iter().map(|value| (*value).to_owned()).collect(),
            version_args: version_args
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            auth_method: None,
            flavor: AcpFlavor::Standard,
        }
    }

    pub fn grok() -> Self {
        Self {
            flavor: AcpFlavor::Grok,
            ..Self::new(
                ProviderId::Grok,
                "Grok",
                configured_executable("AGENT_REMOTE_GROK_BIN", "grok"),
                &["--no-auto-update", "agent", "stdio"],
                &["--no-auto-update", "--version"],
            )
        }
    }

    pub fn claude() -> Self {
        Self::new(
            ProviderId::ClaudeCode,
            "Claude Code",
            configured_executable("AGENT_REMOTE_CLAUDE_CODE_ACP_BIN", "claude-agent-acp"),
            &[],
            &["--version"],
        )
    }

    pub fn gemini() -> Self {
        Self::new(
            ProviderId::GeminiCli,
            "Gemini CLI",
            configured_executable("AGENT_REMOTE_GEMINI_BIN", "gemini"),
            &["--acp"],
            &["--version"],
        )
    }

    pub fn copilot() -> Self {
        Self::new(
            ProviderId::CopilotCli,
            "GitHub Copilot",
            configured_executable("AGENT_REMOTE_COPILOT_BIN", "copilot"),
            &["--acp", "--stdio", "--no-auto-update"],
            &["--version", "--no-auto-update"],
        )
    }

    pub fn opencode() -> Self {
        Self::new(
            ProviderId::OpenCode,
            "OpenCode",
            configured_executable("AGENT_REMOTE_OPENCODE_BIN", "opencode"),
            &["acp"],
            &["--version"],
        )
    }

    pub fn cursor() -> Self {
        let mut config = Self::new(
            ProviderId::Cursor,
            "Cursor Agent",
            configured_executable("AGENT_REMOTE_CURSOR_BIN", "agent"),
            &["acp"],
            &["--version"],
        );
        config.auth_method = Some("cursor_login");
        config
    }

    pub fn cline() -> Self {
        Self::new(
            ProviderId::Cline,
            "Cline",
            configured_executable("AGENT_REMOTE_CLINE_BIN", "cline"),
            &["--acp"],
            &["--version"],
        )
    }

    pub fn goose() -> Self {
        Self::new(
            ProviderId::Goose,
            "Goose",
            configured_executable("AGENT_REMOTE_GOOSE_BIN", "goose"),
            &["acp"],
            &["--version"],
        )
    }

    pub fn junie() -> Self {
        Self::new(
            ProviderId::Junie,
            "JetBrains Junie",
            configured_executable("AGENT_REMOTE_JUNIE_BIN", "junie"),
            &["--acp=true"],
            &["--version"],
        )
    }

    pub fn qwen() -> Self {
        Self::new(
            ProviderId::QwenCode,
            "Qwen Code",
            configured_executable("AGENT_REMOTE_QWEN_BIN", "qwen"),
            &["--acp"],
            &["--version"],
        )
    }

    pub fn kimi() -> Self {
        Self::new(
            ProviderId::KimiCli,
            "Kimi CLI",
            configured_executable("AGENT_REMOTE_KIMI_BIN", "kimi"),
            &["acp"],
            &["--version"],
        )
    }

    pub fn kiro() -> Self {
        Self::new(
            ProviderId::KiroCli,
            "Kiro CLI",
            configured_executable("AGENT_REMOTE_KIRO_BIN", "kiro-cli"),
            &["acp"],
            &["--version"],
        )
    }

    pub fn vibe() -> Self {
        Self::new(
            ProviderId::MistralVibe,
            "Mistral Vibe",
            configured_executable("AGENT_REMOTE_VIBE_BIN", "vibe-acp"),
            &[],
            &["--version"],
        )
    }

    pub fn qoder() -> Self {
        Self::new(
            ProviderId::QoderCli,
            "Qoder CLI",
            configured_executable("AGENT_REMOTE_QODER_BIN", "qoder"),
            &["--acp"],
            &["--version"],
        )
    }
}

fn configured_executable(variable: &str, fallback: &str) -> PathBuf {
    env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(fallback))
}

fn matching_auth_method(response: &InitializeResponse, configured: Option<&str>) -> Option<String> {
    let configured = configured?;
    response
        .auth_methods
        .iter()
        .any(|method| method.id().0.as_ref() == configured)
        .then(|| configured.to_owned())
}

#[derive(Clone)]
pub struct AcpProvider {
    shared: Arc<Shared>,
}

struct Shared {
    config: AcpProviderConfig,
    events: broadcast::Sender<ProviderEvent>,
    connections: AsyncMutex<HashMap<ProjectId, Arc<ProjectConnection>>>,
    sessions: RwLock<HashMap<(ProjectId, String), SessionBinding>>,
    negotiated: RwLock<Option<Negotiated>>,
    health: RwLock<Option<ProviderHealth>>,
    permissions: StdMutex<HashMap<String, PendingPermission>>,
    terminals: AsyncMutex<HashMap<String, Arc<TerminalProcess>>>,
    tool_calls: StdMutex<HashMap<(ProjectId, String, String), ToolCall>>,
}

struct ProjectConnection {
    connection: ConnectionTo<Agent>,
    negotiated: Negotiated,
    shutdown: StdMutex<Option<oneshot::Sender<()>>>,
}

impl Drop for ProjectConnection {
    fn drop(&mut self) {
        if let Some(shutdown) = self
            .shutdown
            .lock()
            .expect("ACP shutdown mutex poisoned")
            .take()
        {
            let _ = shutdown.send(());
        }
    }
}

#[derive(Debug, Clone)]
struct Negotiated {
    version: Option<String>,
    grok_shell: bool,
    supports_list: bool,
    supports_load: bool,
    supports_resume: bool,
    supports_close: bool,
    models: Vec<GrokModel>,
    current_model: Option<String>,
}

impl Negotiated {
    fn supports_extensions(&self) -> bool {
        self.grok_shell && self.version.as_deref() == Some(GROK_EXTENSION_VERSION)
    }

    fn provider_capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_session_list: self.supports_list,
            supports_resume: self.supports_resume || self.supports_load,
            supports_history: self.supports_load,
            supports_incremental_sync: false,
            supports_rename: false,
            supports_steer: self.supports_extensions(),
        }
    }

    fn public_models(&self) -> Vec<ModelOption> {
        self.models.iter().map(GrokModel::public).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GrokModel {
    id: String,
    name: String,
    efforts: Vec<GrokEffort>,
    default_effort: Option<String>,
}

impl GrokModel {
    fn public(&self) -> ModelOption {
        ModelOption {
            id: self.id.clone(),
            display_name: self.name.clone(),
            effort_options: self
                .efforts
                .iter()
                .map(|effort| EffortOption {
                    id: effort.id.clone(),
                    display_name: effort.label.clone(),
                })
                .collect(),
            default_effort: self.default_effort.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GrokEffort {
    id: String,
    label: String,
}

#[derive(Debug, Clone)]
struct SessionBinding {
    conversation_id: ConversationId,
    model: Option<String>,
    effort: Option<String>,
    session_options: Vec<SessionOption>,
    acp_options: Vec<SessionConfigOption>,
    modes: Option<SessionModeState>,
    prompt_in_flight: bool,
    replaying: bool,
    turn_index: u64,
    user_item_id: String,
    agent_item_id: String,
    thought_item_id: String,
    replay_user_text: String,
    replay_agent_text: String,
    replay_thought_text: String,
    replay_has_response: bool,
    replay_image_index: u64,
}

#[derive(Clone, Copy)]
enum ReplayTextKind {
    User,
    Agent,
    Thought,
}

fn start_replay_turn(binding: &mut SessionBinding, session_id: &str) {
    binding.turn_index += 1;
    let prefix = format!("history:{session_id}:{}", binding.turn_index);
    binding.user_item_id = format!("{prefix}:user");
    binding.agent_item_id = format!("{prefix}:agent");
    binding.thought_item_id = format!("{prefix}:thought");
    binding.replay_has_response = false;
}

fn drain_replay_items(binding: &mut SessionBinding) -> Vec<(String, TimelineItemKind)> {
    let mut items = Vec::with_capacity(3);
    let user = std::mem::take(&mut binding.replay_user_text);
    if !user.is_empty() {
        items.push((
            binding.user_item_id.clone(),
            TimelineItemKind::UserMessage { text: user },
        ));
    }
    let thought = std::mem::take(&mut binding.replay_thought_text);
    if !thought.is_empty() {
        items.push((
            binding.thought_item_id.clone(),
            TimelineItemKind::AgentMessage {
                phase: AgentMessagePhase::ReasoningSummary,
                text: thought,
            },
        ));
    }
    let agent = std::mem::take(&mut binding.replay_agent_text);
    if !agent.is_empty() {
        items.push((
            binding.agent_item_id.clone(),
            TimelineItemKind::AgentMessage {
                phase: AgentMessagePhase::Final,
                text: agent,
            },
        ));
    }
    binding.replay_has_response = false;
    items
}

struct PendingPermission {
    conversation_id: ConversationId,
    session_id: String,
    allowed_options: Vec<String>,
    responder: Responder<RequestPermissionResponse>,
}

struct TerminalProcess {
    project_id: ProjectId,
    session_id: String,
    output: Arc<StdMutex<TerminalOutputBuffer>>,
    kill: mpsc::UnboundedSender<()>,
    exit: watch::Receiver<Option<std::result::Result<ProcessExit, String>>>,
}

#[derive(Debug, Default)]
struct TerminalOutputBuffer {
    text: String,
    limit: Option<usize>,
    truncated: bool,
}

#[derive(Debug, Clone)]
struct ProcessExit {
    exit_code: Option<u32>,
    signal: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSessionNotification {
    session_id: SessionId,
    update: Value,
    #[serde(default, rename = "_meta")]
    meta: Option<Meta>,
}

impl JsonRpcMessage for RawSessionNotification {
    fn matches_method(method: &str) -> bool {
        method == "session/update"
    }

    fn method(&self) -> &str {
        "session/update"
    }

    fn to_untyped_message(
        &self,
    ) -> std::result::Result<UntypedMessage, agent_client_protocol::Error> {
        UntypedMessage::new(self.method(), self)
    }

    fn parse_message(
        method: &str,
        params: &impl Serialize,
    ) -> std::result::Result<Self, agent_client_protocol::Error> {
        if !Self::matches_method(method) {
            return Err(agent_client_protocol::Error::method_not_found());
        }
        let value = serde_json::to_value(params)
            .map_err(agent_client_protocol::Error::into_internal_error)?;
        serde_json::from_value(value)
            .map_err(|error| agent_client_protocol::Error::invalid_params().data(error.to_string()))
    }
}

impl JsonRpcNotification for RawSessionNotification {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetModelRequest {
    session_id: SessionId,
    model_id: String,
    #[serde(rename = "_meta")]
    meta: Meta,
}

impl JsonRpcMessage for SetModelRequest {
    fn matches_method(method: &str) -> bool {
        method == "session/set_model"
    }

    fn method(&self) -> &str {
        "session/set_model"
    }

    fn to_untyped_message(
        &self,
    ) -> std::result::Result<UntypedMessage, agent_client_protocol::Error> {
        UntypedMessage::new(self.method(), self)
    }

    fn parse_message(
        method: &str,
        params: &impl Serialize,
    ) -> std::result::Result<Self, agent_client_protocol::Error> {
        parse_custom_message(Self::matches_method, method, params)
    }
}

impl JsonRpcRequest for SetModelRequest {
    type Response = Value;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InterjectRequest {
    session_id: SessionId,
    text: String,
    interjection_id: String,
}

impl JsonRpcMessage for InterjectRequest {
    fn matches_method(method: &str) -> bool {
        method == "_x.ai/interject"
    }

    fn method(&self) -> &str {
        "_x.ai/interject"
    }

    fn to_untyped_message(
        &self,
    ) -> std::result::Result<UntypedMessage, agent_client_protocol::Error> {
        UntypedMessage::new(self.method(), self)
    }

    fn parse_message(
        method: &str,
        params: &impl Serialize,
    ) -> std::result::Result<Self, agent_client_protocol::Error> {
        parse_custom_message(Self::matches_method, method, params)
    }
}

impl JsonRpcRequest for InterjectRequest {
    type Response = Value;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
struct CursorAskQuestionRequest(Value);

impl JsonRpcMessage for CursorAskQuestionRequest {
    fn matches_method(method: &str) -> bool {
        method == "cursor/ask_question"
    }

    fn method(&self) -> &str {
        "cursor/ask_question"
    }

    fn to_untyped_message(
        &self,
    ) -> std::result::Result<UntypedMessage, agent_client_protocol::Error> {
        UntypedMessage::new(self.method(), self)
    }

    fn parse_message(
        method: &str,
        params: &impl Serialize,
    ) -> std::result::Result<Self, agent_client_protocol::Error> {
        parse_custom_message(Self::matches_method, method, params)
    }
}

impl JsonRpcRequest for CursorAskQuestionRequest {
    type Response = CursorCancelledResponse;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
struct CursorCreatePlanRequest(Value);

impl JsonRpcMessage for CursorCreatePlanRequest {
    fn matches_method(method: &str) -> bool {
        method == "cursor/create_plan"
    }

    fn method(&self) -> &str {
        "cursor/create_plan"
    }

    fn to_untyped_message(
        &self,
    ) -> std::result::Result<UntypedMessage, agent_client_protocol::Error> {
        UntypedMessage::new(self.method(), self)
    }

    fn parse_message(
        method: &str,
        params: &impl Serialize,
    ) -> std::result::Result<Self, agent_client_protocol::Error> {
        parse_custom_message(Self::matches_method, method, params)
    }
}

impl JsonRpcRequest for CursorCreatePlanRequest {
    type Response = CursorCancelledResponse;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonRpcResponse)]
struct CursorCancelledResponse {
    outcome: CursorCancelledOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CursorCancelledOutcome {
    outcome: String,
}

impl CursorCancelledResponse {
    fn cancelled() -> Self {
        Self {
            outcome: CursorCancelledOutcome {
                outcome: "cancelled".to_owned(),
            },
        }
    }
}

fn parse_custom_message<T: serde::de::DeserializeOwned>(
    matches: impl FnOnce(&str) -> bool,
    method: &str,
    params: &impl Serialize,
) -> std::result::Result<T, agent_client_protocol::Error> {
    if !matches(method) {
        return Err(agent_client_protocol::Error::method_not_found());
    }
    let value =
        serde_json::to_value(params).map_err(agent_client_protocol::Error::into_internal_error)?;
    serde_json::from_value(value)
        .map_err(|error| agent_client_protocol::Error::invalid_params().data(error.to_string()))
}

impl Default for AcpProvider {
    fn default() -> Self {
        Self::grok()
    }
}

impl AcpProvider {
    pub fn grok() -> Self {
        Self::from_config(AcpProviderConfig::grok())
    }

    pub fn claude() -> Self {
        Self::from_config(AcpProviderConfig::claude())
    }

    pub fn gemini() -> Self {
        Self::from_config(AcpProviderConfig::gemini())
    }

    pub fn copilot() -> Self {
        Self::from_config(AcpProviderConfig::copilot())
    }

    pub fn opencode() -> Self {
        Self::from_config(AcpProviderConfig::opencode())
    }

    pub fn cursor() -> Self {
        Self::from_config(AcpProviderConfig::cursor())
    }

    pub fn cline() -> Self {
        Self::from_config(AcpProviderConfig::cline())
    }

    pub fn goose() -> Self {
        Self::from_config(AcpProviderConfig::goose())
    }

    pub fn junie() -> Self {
        Self::from_config(AcpProviderConfig::junie())
    }

    pub fn qwen() -> Self {
        Self::from_config(AcpProviderConfig::qwen())
    }

    pub fn kimi() -> Self {
        Self::from_config(AcpProviderConfig::kimi())
    }

    pub fn kiro() -> Self {
        Self::from_config(AcpProviderConfig::kiro())
    }

    pub fn vibe() -> Self {
        Self::from_config(AcpProviderConfig::vibe())
    }

    pub fn qoder() -> Self {
        Self::from_config(AcpProviderConfig::qoder())
    }

    pub fn new() -> Self {
        Self::grok()
    }

    pub fn from_config(config: AcpProviderConfig) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            shared: Arc::new(Shared {
                config,
                events,
                connections: AsyncMutex::new(HashMap::new()),
                sessions: RwLock::new(HashMap::new()),
                negotiated: RwLock::new(None),
                health: RwLock::new(None),
                permissions: StdMutex::new(HashMap::new()),
                terminals: AsyncMutex::new(HashMap::new()),
                tool_calls: StdMutex::new(HashMap::new()),
            }),
        }
    }

    pub fn with_executable(executable: impl Into<PathBuf>) -> Self {
        let mut config = AcpProviderConfig::grok();
        config.executable = executable.into();
        Self::from_config(config)
    }

    /// Loads an existing session and replays its history. The shared provider trait uses
    /// `session/resume` when available; this explicit entry point preserves ACP's load semantics.
    pub async fn load_session(&self, request: ResumeSession) -> Result<NativeSession> {
        self.open_existing_session(request, true).await
    }

    /// Closes a live ACP session when the agent advertised `session/close`.
    pub async fn close_session(
        &self,
        conversation_id: ConversationId,
        native_session_id: &str,
    ) -> Result<()> {
        let (project_id, connection, _) = self
            .connection_for_bound_session(native_session_id, conversation_id)
            .await?;
        if !connection.negotiated.supports_close {
            bail!(
                "{} did not advertise session/close",
                self.shared.config.display_name
            );
        }
        self.shared.cancel_permissions(native_session_id)?;
        connection
            .connection
            .send_request(CloseSessionRequest::new(SessionId::new(
                native_session_id.to_owned(),
            )))
            .block_task()
            .await?;
        self.shared
            .sessions
            .write()
            .expect("ACP session map poisoned")
            .remove(&(project_id, native_session_id.to_owned()));
        self.shared
            .release_session_terminals(native_session_id)
            .await;
        Ok(())
    }

    async fn connection_for_project(&self, project: &Project) -> Result<Arc<ProjectConnection>> {
        self.shared.connection_for_project(project).await
    }

    async fn connection_for_bound_session(
        &self,
        native_session_id: &str,
        conversation_id: ConversationId,
    ) -> Result<(ProjectId, Arc<ProjectConnection>, SessionBinding)> {
        let (project_id, binding) = self
            .shared
            .sessions
            .read()
            .expect("ACP session map poisoned")
            .iter()
            .find_map(|((project_id, session_id), binding)| {
                (session_id == native_session_id && binding.conversation_id == conversation_id)
                    .then(|| (*project_id, binding.clone()))
            })
            .ok_or_else(|| anyhow!("ACP session {native_session_id} is not active"))?;
        let connection = self
            .shared
            .connections
            .lock()
            .await
            .get(&project_id)
            .cloned()
            .ok_or_else(|| anyhow!("ACP connection for session {native_session_id} is closed"))?;
        if connection.connection.is_incoming_closed() {
            bail!("ACP connection for session {native_session_id} is closed");
        }
        Ok((project_id, connection, binding))
    }

    async fn open_existing_session(
        &self,
        request: ResumeSession,
        replay: bool,
    ) -> Result<NativeSession> {
        let connection = self.connection_for_project(&request.project).await?;
        let existing_binding = self
            .shared
            .session_binding(request.project.id, &request.native_session_id);
        if let Some(binding) = &existing_binding
            && binding.conversation_id != request.conversation_id
        {
            bail!(
                "ACP session {} is already bound to another conversation",
                request.native_session_id
            );
        }
        let (model, effort) = if self.shared.config.flavor == AcpFlavor::Standard {
            (
                request.model.clone().or_else(|| {
                    existing_binding
                        .as_ref()
                        .and_then(|binding| binding.model.clone())
                }),
                request.effort.clone().or_else(|| {
                    existing_binding
                        .as_ref()
                        .and_then(|binding| binding.effort.clone())
                }),
            )
        } else {
            select_model_and_effort(
                &connection.negotiated,
                request.model.as_deref().or(existing_binding
                    .as_ref()
                    .and_then(|binding| binding.model.as_deref())),
                request.effort.as_deref().or(existing_binding
                    .as_ref()
                    .and_then(|binding| binding.effort.as_deref())),
            )?
        };
        let session_id = SessionId::new(request.native_session_id.clone());
        let replay_previous = if replay {
            self.shared.prepare_history_replay(
                request.project.id,
                &request.native_session_id,
                request.conversation_id,
                model.clone(),
                effort.clone(),
            )?
        } else {
            self.shared.bind_session(
                request.project.id,
                &request.native_session_id,
                request.conversation_id,
                model.clone(),
                effort.clone(),
            );
            None
        };

        let meta = match self.shared.config.flavor {
            AcpFlavor::Grok => selection_meta(model.as_deref(), effort.as_deref()),
            AcpFlavor::Standard => Meta::new(),
        };
        let result = if replay {
            if !connection.negotiated.supports_load {
                bail!("ACP agent did not advertise session/load");
            }
            connection
                .connection
                .send_request(
                    LoadSessionRequest::new(session_id, request.project.canonical_path.clone())
                        .meta(meta),
                )
                .block_task()
                .await
                .map(|response| (response.config_options.unwrap_or_default(), response.modes))
        } else if connection.negotiated.supports_resume {
            connection
                .connection
                .send_request(
                    ResumeSessionRequest::new(session_id, request.project.canonical_path.clone())
                        .meta(meta),
                )
                .block_task()
                .await
                .map(|response| (response.config_options.unwrap_or_default(), response.modes))
        } else if connection.negotiated.supports_load {
            connection
                .connection
                .send_request(
                    LoadSessionRequest::new(session_id, request.project.canonical_path.clone())
                        .meta(meta),
                )
                .block_task()
                .await
                .map(|response| (response.config_options.unwrap_or_default(), response.modes))
        } else {
            bail!("ACP agent advertised neither session/resume nor session/load");
        };

        let (acp_options, modes) = match result {
            Ok(state) => state,
            Err(error) => {
                if replay {
                    self.shared.restore_history_binding(
                        request.project.id,
                        &request.native_session_id,
                        replay_previous,
                    );
                } else {
                    self.shared
                        .sessions
                        .write()
                        .expect("ACP session map poisoned")
                        .remove(&(request.project.id, request.native_session_id.clone()));
                }
                return Err(anyhow!(error));
            }
        };
        if replay {
            self.shared
                .set_replaying(request.project.id, &request.native_session_id, false);
        }
        if !acp_options.is_empty() {
            self.shared.update_acp_options(
                request.project.id,
                &request.native_session_id,
                acp_options.clone(),
            );
        }
        if let Some(modes) = modes.as_ref() {
            self.shared.update_modes(
                request.project.id,
                &request.native_session_id,
                modes.clone(),
            );
        }

        let mut native = native_session(
            request.native_session_id,
            request.project.display_name,
            model,
            effort,
            &connection.negotiated.models,
        );
        if !acp_options.is_empty() || modes.is_some() {
            native.session_options = public_session_options(&acp_options, modes.as_ref());
            (native.selected_model, native.selected_effort) =
                selected_model_and_effort(&native.session_options);
        } else if let Some(binding) = existing_binding {
            native.session_options = binding.session_options;
            native.selected_model = binding.model;
            native.selected_effort = binding.effort;
        }
        Ok(native)
    }
}

#[async_trait]
impl AgentProvider for AcpProvider {
    fn id(&self) -> ProviderId {
        self.shared.config.provider
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.shared
            .negotiated
            .read()
            .expect("ACP capabilities poisoned")
            .as_ref()
            .map(Negotiated::provider_capabilities)
            .unwrap_or(ProviderCapabilities {
                supports_session_list: false,
                supports_resume: false,
                supports_history: false,
                supports_incremental_sync: false,
                supports_rename: false,
                supports_steer: false,
            })
    }

    fn subscribe(&self) -> broadcast::Receiver<ProviderEvent> {
        self.shared.events.subscribe()
    }

    async fn health(&self) -> ProviderHealth {
        if let Some(health) = self
            .shared
            .health
            .read()
            .expect("ACP health poisoned")
            .clone()
            && matches!(
                health.state,
                ProviderState::Ready | ProviderState::ProtocolIncompatible
            )
        {
            return health;
        }

        let output = Command::new(&self.shared.config.executable)
            .args(&self.shared.config.version_args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;
        let health = match output {
            Ok(output) if output.status.success() => ProviderHealth {
                provider: self.id(),
                state: ProviderState::Ready,
                version: parse_cli_version(&String::from_utf8_lossy(&output.stdout)),
                detail: Some(format!(
                    "{} ACP will start when a project is opened",
                    self.shared.config.display_name
                )),
            },
            Ok(output) => ProviderHealth {
                provider: self.id(),
                state: ProviderState::Crashed,
                version: None,
                detail: Some(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => ProviderHealth {
                provider: self.id(),
                state: ProviderState::NotInstalled,
                version: None,
                detail: Some(format!(
                    "{} was not found",
                    self.shared.config.executable.display()
                )),
            },
            Err(error) => ProviderHealth {
                provider: self.id(),
                state: ProviderState::Crashed,
                version: None,
                detail: Some(error.to_string()),
            },
        };
        *self.shared.health.write().expect("ACP health poisoned") = Some(health.clone());
        health
    }

    async fn list_models(&self, project: &Project) -> Result<Vec<ModelOption>> {
        let connection = self.connection_for_project(project).await?;
        Ok(connection.negotiated.public_models())
    }

    async fn list_sessions(&self, project: &Project) -> Result<Vec<SessionSummary>> {
        let connection = self.connection_for_project(project).await?;
        if !connection.negotiated.supports_list {
            bail!(
                "{} did not advertise session/list",
                self.shared.config.display_name
            );
        }
        let mut sessions = Vec::new();
        let mut cursor = None;
        loop {
            let response = connection
                .connection
                .send_request(
                    ListSessionsRequest::new()
                        .cwd(project.canonical_path.clone())
                        .cursor(cursor.clone()),
                )
                .block_task()
                .await?;
            sessions.extend(response.sessions.into_iter().map(|session| SessionSummary {
                native_session_id: session.session_id.to_string(),
                title: session.title.unwrap_or_else(|| {
                    format!("Untitled {} session", self.shared.config.display_name)
                }),
                updated_at_ms: acp_timestamp_ms(session.updated_at.as_deref()),
            }));
            cursor = response.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        Ok(sessions)
    }

    async fn read_session_history(
        &self,
        request: ReadSessionHistory,
    ) -> Result<ProviderHistoryPage> {
        if let Some(binding) = self
            .shared
            .session_binding(request.project.id, &request.native_session_id)
        {
            if binding.conversation_id != request.conversation_id {
                bail!(
                    "ACP session {} is already bound to another conversation",
                    request.native_session_id
                );
            }
            if binding.prompt_in_flight {
                bail!(
                    "ACP session {} has an active turn; history sync must be retried",
                    request.native_session_id
                );
            }
        }
        self.load_session(ResumeSession {
            conversation_id: request.conversation_id,
            project: request.project,
            native_session_id: request.native_session_id,
            model: None,
            effort: None,
        })
        .await?;
        Ok(ProviderHistoryPage {
            items: Vec::new(),
            next_cursor: None,
            full_read_fallback: true,
        })
    }

    async fn flush_history_events(
        &self,
        project_id: ProjectId,
        conversation_id: ConversationId,
    ) -> Result<()> {
        let barrier = Arc::new(ProviderHistoryBarrier::default());
        self.shared
            .events
            .send(ProviderEvent {
                provider: self.id(),
                project_id,
                conversation_id,
                kind: ProviderEventKind::HistoryBarrier {
                    barrier: Arc::clone(&barrier),
                },
            })
            .map_err(|_| anyhow!("ACP history barrier has no active event pump"))?;
        barrier.wait().await
    }

    async fn create_session(&self, request: CreateSession) -> Result<NativeSession> {
        let connection = self.connection_for_project(&request.project).await?;
        let (model, effort) = if self.shared.config.flavor == AcpFlavor::Standard {
            (request.model.clone(), request.effort.clone())
        } else {
            select_model_and_effort(
                &connection.negotiated,
                request.model.as_deref(),
                request.effort.as_deref(),
            )?
        };
        let response = connection
            .connection
            .send_request(
                NewSessionRequest::new(request.project.canonical_path.clone()).meta(
                    match self.shared.config.flavor {
                        AcpFlavor::Grok => selection_meta(model.as_deref(), effort.as_deref()),
                        AcpFlavor::Standard => Meta::new(),
                    },
                ),
            )
            .block_task()
            .await?;
        let native_session_id = response.session_id.to_string();
        let acp_options = response.config_options.unwrap_or_default();
        let modes = response.modes;
        self.shared.bind_session(
            request.project.id,
            &native_session_id,
            request.conversation_id,
            model.clone(),
            effort.clone(),
        );
        self.shared
            .update_acp_options(request.project.id, &native_session_id, acp_options.clone());
        if let Some(modes) = modes.as_ref() {
            self.shared
                .update_modes(request.project.id, &native_session_id, modes.clone());
        }
        let mut native = native_session(
            native_session_id,
            request.project.display_name,
            model,
            effort,
            &connection.negotiated.models,
        );
        if !acp_options.is_empty() || modes.is_some() {
            native.session_options = public_session_options(&acp_options, modes.as_ref());
            (native.selected_model, native.selected_effort) =
                selected_model_and_effort(&native.session_options);
        }
        Ok(native)
    }

    async fn resume_session(&self, request: ResumeSession) -> Result<NativeSession> {
        self.open_existing_session(request, false).await
    }

    async fn send_message(&self, request: SendMessage) -> Result<CommandAck> {
        if self
            .shared
            .sessions
            .read()
            .expect("ACP session map poisoned")
            .iter()
            .all(|((_, session_id), binding)| {
                session_id != &request.native_session_id
                    || binding.conversation_id != request.conversation_id
            })
        {
            self.resume_session(ResumeSession {
                conversation_id: request.conversation_id,
                project: request.project.clone(),
                native_session_id: request.native_session_id.clone(),
                model: request.model.clone(),
                effort: request.effort.clone(),
            })
            .await?;
        }
        let (project_id, connection, binding) = self
            .connection_for_bound_session(&request.native_session_id, request.conversation_id)
            .await?;
        if self.shared.config.flavor == AcpFlavor::Standard {
            if request.model.is_some() && request.model != binding.model {
                bail!("model changes must use the Provider session option");
            }
            if request.effort.is_some() && request.effort != binding.effort {
                bail!("effort changes must use the Provider session option");
            }
        } else {
            let requested_model = request.model.as_deref().or(binding.model.as_deref());
            let requested_effort = request.effort.as_deref().or(binding.effort.as_deref());
            let (model, effort) =
                select_model_and_effort(&connection.negotiated, requested_model, requested_effort)?;
            if model != binding.model || effort != binding.effort {
                set_grok_model(
                    &connection,
                    &request.native_session_id,
                    model.as_deref(),
                    effort.as_deref(),
                )
                .await?;
                self.shared
                    .update_selection(project_id, &request.native_session_id, model, effort);
            }
        }

        if !self
            .shared
            .start_turn(project_id, &request.native_session_id)
        {
            bail!("ACP session {} is busy", request.native_session_id);
        }
        let connection_to_agent = connection.connection.clone();
        let shared = Arc::clone(&self.shared);
        let session_id = request.native_session_id.clone();
        let conversation_id = request.conversation_id;
        let pending_response = connection_to_agent.send_request(PromptRequest::new(
            SessionId::new(session_id.clone()),
            vec![ContentBlock::Text(TextContent::new(request.text))],
        ));
        tokio::spawn(async move {
            let result = pending_response.block_task().await;
            shared.finish_turn(project_id, &session_id);
            let kind = match result {
                Ok(response) => stop_reason_event(response.stop_reason),
                Err(error) => ProviderEventKind::Failed {
                    provider_item_id: None,
                    code: format!("{}_prompt_failed", provider_slug(shared.config.provider)),
                    message: error.to_string(),
                },
            };
            shared.emit(project_id, conversation_id, kind);
        });
        Ok(CommandAck)
    }

    async fn steer(&self, request: SteerMessage) -> Result<CommandAck> {
        let (_, connection, _) = self
            .connection_for_bound_session(&request.native_session_id, request.conversation_id)
            .await?;
        if !connection.negotiated.supports_extensions() {
            bail!("this Grok version does not safely advertise steering");
        }
        connection
            .connection
            .send_request(InterjectRequest {
                session_id: SessionId::new(request.native_session_id),
                text: request.text,
                interjection_id: Uuid::new_v4().to_string(),
            })
            .block_task()
            .await?;
        Ok(CommandAck)
    }

    async fn interrupt(&self, request: InterruptSession) -> Result<CommandAck> {
        let (_, connection, _) = self
            .connection_for_bound_session(&request.native_session_id, request.conversation_id)
            .await?;
        self.shared.cancel_permissions(&request.native_session_id)?;
        connection
            .connection
            .send_notification(CancelNotification::new(SessionId::new(
                request.native_session_id,
            )))?;
        Ok(CommandAck)
    }

    async fn resolve_approval(&self, request: ResolveApproval) -> Result<CommandAck> {
        let pending = {
            let mut permissions = self
                .shared
                .permissions
                .lock()
                .expect("ACP permission map poisoned");
            let pending = permissions
                .get(&request.provider_request_id)
                .ok_or_else(|| anyhow!("ACP permission request is no longer pending"))?;
            if pending.conversation_id != request.conversation_id {
                bail!("permission request does not belong to this conversation");
            }
            if !pending
                .allowed_options
                .iter()
                .any(|id| id == &request.option_id)
            {
                bail!("permission option {} was not offered", request.option_id);
            }
            permissions
                .remove(&request.provider_request_id)
                .expect("permission was checked while locked")
        };
        pending.responder.respond(RequestPermissionResponse::new(
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                PermissionOptionId::new(request.option_id),
            )),
        ))?;
        Ok(CommandAck)
    }

    async fn set_session_option(&self, request: SetSessionOption) -> Result<CommandAck> {
        let (project_id, connection, binding) = self
            .connection_for_bound_session(&request.native_session_id, request.conversation_id)
            .await?;
        if self.shared.config.flavor == AcpFlavor::Standard {
            if request.option_id == "mode"
                && let Some(modes) = binding.modes.as_ref()
                && !binding
                    .acp_options
                    .iter()
                    .any(|option| option.id.to_string() == request.option_id)
            {
                if !modes
                    .available_modes
                    .iter()
                    .any(|mode| mode.id.to_string() == request.value)
                {
                    bail!("ACP session mode {} is not available", request.value);
                }
                connection
                    .connection
                    .send_request(SetSessionModeRequest::new(
                        SessionId::new(request.native_session_id.clone()),
                        request.value.clone(),
                    ))
                    .block_task()
                    .await?;
                self.shared.update_current_mode(
                    project_id,
                    &request.native_session_id,
                    &request.value,
                );
                return Ok(CommandAck);
            }
            let value =
                config_option_value(&binding.acp_options, &request.option_id, &request.value)?;
            let response = connection
                .connection
                .send_request(SetSessionConfigOptionRequest::new(
                    SessionId::new(request.native_session_id.clone()),
                    request.option_id,
                    value,
                ))
                .block_task()
                .await?;
            self.shared.update_acp_options(
                project_id,
                &request.native_session_id,
                response.config_options,
            );
            return Ok(CommandAck);
        }
        let (model, effort) = match request.option_id.as_str() {
            "model" => (Some(request.value), binding.effort),
            "reasoning_effort" | "thought_level" => (binding.model, Some(request.value)),
            other => bail!("Grok session option {other} is not supported"),
        };
        let (model, effort) =
            select_model_and_effort(&connection.negotiated, model.as_deref(), effort.as_deref())?;
        set_grok_model(
            &connection,
            &request.native_session_id,
            model.as_deref(),
            effort.as_deref(),
        )
        .await?;
        self.shared
            .update_selection(project_id, &request.native_session_id, model, effort);
        Ok(CommandAck)
    }

    fn session_options_snapshot(
        &self,
        conversation_id: ConversationId,
        native_session_id: &str,
    ) -> Option<SessionOptionsSnapshot> {
        if self.shared.config.flavor != AcpFlavor::Standard {
            return None;
        }
        self.shared
            .sessions
            .read()
            .expect("ACP session map poisoned")
            .iter()
            .find(|((_, session_id), binding)| {
                session_id == native_session_id && binding.conversation_id == conversation_id
            })
            .map(|(_, binding)| SessionOptionsSnapshot {
                selected_model: binding.model.clone(),
                selected_effort: binding.effort.clone(),
                session_options: binding.session_options.clone(),
            })
    }
}

fn acp_timestamp_ms(value: Option<&str>) -> i64 {
    value
        .and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok())
        .and_then(|timestamp| {
            (timestamp.unix_timestamp_nanos() / 1_000_000)
                .try_into()
                .ok()
        })
        .unwrap_or_else(now_ms)
}

impl Shared {
    async fn connection_for_project(
        self: &Arc<Self>,
        project: &Project,
    ) -> Result<Arc<ProjectConnection>> {
        let mut connections = self.connections.lock().await;
        if let Some(connection) = connections.get(&project.id)
            && !connection.connection.is_incoming_closed()
        {
            return Ok(Arc::clone(connection));
        }
        connections.remove(&project.id);
        let connection = self.start_connection(project.clone()).await?;
        connections.insert(project.id, Arc::clone(&connection));
        Ok(connection)
    }

    async fn start_connection(
        self: &Arc<Self>,
        project: Project,
    ) -> Result<Arc<ProjectConnection>> {
        let agent = AcpAgent::new(
            AcpAgentConfig::new(self.config.executable.clone())
                .args(self.config.agent_args.clone()),
        );
        let (ready_tx, ready_rx) =
            oneshot::channel::<std::result::Result<(ConnectionTo<Agent>, Negotiated), String>>();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let initialized = Arc::new(AtomicBool::new(false));

        let update_state = Arc::clone(self);
        let update_project_id = project.id;
        let permission_state = Arc::clone(self);
        let permission_project_id = project.id;
        let read_state = Arc::clone(self);
        let read_project = project.clone();
        let write_state = Arc::clone(self);
        let write_project = project.clone();
        let create_terminal_state = Arc::clone(self);
        let create_terminal_project = project.clone();
        let terminal_output_state = Arc::clone(self);
        let wait_terminal_state = Arc::clone(self);
        let kill_terminal_state = Arc::clone(self);
        let release_terminal_state = Arc::clone(self);
        let task_state = Arc::clone(self);
        let task_project_id = project.id;
        let task_initialized = Arc::clone(&initialized);
        let connection_initialized = Arc::clone(&initialized);

        tokio::spawn(async move {
            let client_name = format!("agent-remote-{}", provider_slug(task_state.config.provider));
            let task_flavor = task_state.config.flavor;
            let task_auth_method = task_state.config.auth_method;
            let result = Client
                .builder()
                .name(client_name)
                .on_receive_notification(
                    async move |notification: RawSessionNotification, _connection| {
                        update_state.handle_session_update(update_project_id, notification);
                        Ok(())
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                .on_receive_request(
                    async move |request: RequestPermissionRequest, responder, _connection| {
                        permission_state.handle_permission_request(
                            permission_project_id,
                            request,
                            responder,
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_request: CursorAskQuestionRequest, responder, _connection| {
                        responder.respond(CursorCancelledResponse::cancelled())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_request: CursorCreatePlanRequest, responder, _connection| {
                        responder.respond(CursorCancelledResponse::cancelled())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: ReadTextFileRequest, responder, connection| {
                        let state = Arc::clone(&read_state);
                        let project = read_project.clone();
                        connection.spawn(async move {
                            let response = state
                                .read_text_file(&project, request)
                                .await
                                .map_err(acp_handler_error);
                            responder.respond_with_result(response)?;
                            Ok(())
                        })?;
                        Ok(())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: WriteTextFileRequest, responder, connection| {
                        let state = Arc::clone(&write_state);
                        let project = write_project.clone();
                        connection.spawn(async move {
                            let response = state
                                .write_text_file(&project, request)
                                .await
                                .map_err(acp_handler_error);
                            responder.respond_with_result(response)?;
                            Ok(())
                        })?;
                        Ok(())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: CreateTerminalRequest, responder, connection| {
                        let state = Arc::clone(&create_terminal_state);
                        let project = create_terminal_project.clone();
                        connection.spawn(async move {
                            let response = state
                                .create_terminal(&project, request)
                                .await
                                .map_err(acp_handler_error);
                            responder.respond_with_result(response)?;
                            Ok(())
                        })?;
                        Ok(())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: TerminalOutputRequest, responder, connection| {
                        let state = Arc::clone(&terminal_output_state);
                        connection.spawn(async move {
                            let response = state
                                .terminal_output(request)
                                .await
                                .map_err(acp_handler_error);
                            responder.respond_with_result(response)?;
                            Ok(())
                        })?;
                        Ok(())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: WaitForTerminalExitRequest, responder, connection| {
                        let state = Arc::clone(&wait_terminal_state);
                        connection.spawn(async move {
                            let response = state
                                .wait_for_terminal_exit(request)
                                .await
                                .map_err(acp_handler_error);
                            responder.respond_with_result(response)?;
                            Ok(())
                        })?;
                        Ok(())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: KillTerminalRequest, responder, connection| {
                        let state = Arc::clone(&kill_terminal_state);
                        connection.spawn(async move {
                            let response = state
                                .kill_terminal(request)
                                .await
                                .map_err(acp_handler_error);
                            responder.respond_with_result(response)?;
                            Ok(())
                        })?;
                        Ok(())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: ReleaseTerminalRequest, responder, connection| {
                        let state = Arc::clone(&release_terminal_state);
                        connection.spawn(async move {
                            let response = state
                                .release_terminal(request)
                                .await
                                .map_err(acp_handler_error);
                            responder.respond_with_result(response)?;
                            Ok(())
                        })?;
                        Ok(())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_with(agent, async move |connection| {
                    let initialize = InitializeRequest::new(ProtocolVersion::V1)
                        .client_capabilities(
                            ClientCapabilities::new()
                                .fs(FileSystemCapabilities::new()
                                    .read_text_file(true)
                                    .write_text_file(true))
                                .terminal(true)
                                .session(
                                    ClientSessionCapabilities::new().config_options(
                                        SessionConfigOptionsCapabilities::new()
                                            .boolean(BooleanConfigOptionCapabilities::new()),
                                    ),
                                ),
                        )
                        .client_info(
                            Implementation::new(
                                "agent-remote-messenger",
                                env!("CARGO_PKG_VERSION"),
                            )
                            .title("Agent Remote Messenger"),
                        );
                    let response = match connection.send_request(initialize).block_task().await {
                        Ok(response) => response,
                        Err(error) => {
                            let _ = ready_tx.send(Err(error.to_string()));
                            return Err(error);
                        }
                    };
                    let negotiated = match negotiate(&response, task_flavor) {
                        Ok(negotiated) => negotiated,
                        Err(error) => {
                            let message = error.to_string();
                            let _ = ready_tx.send(Err(message.clone()));
                            return Err(
                                agent_client_protocol::Error::invalid_request().data(message)
                            );
                        }
                    };
                    if let Some(method_id) = matching_auth_method(&response, task_auth_method)
                        && let Err(error) = connection
                            .send_request(AuthenticateRequest::new(method_id))
                            .block_task()
                            .await
                    {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return Err(error);
                    }
                    connection_initialized.store(true, Ordering::Release);
                    let _ = ready_tx.send(Ok((connection.clone(), negotiated)));
                    let _ = shutdown_rx.await;
                    Ok(())
                })
                .await;

            if let Err(error) = result {
                *task_state.health.write().expect("ACP health poisoned") = Some(
                    classify_connection_error(task_state.config.provider, &error),
                );
                if task_initialized.load(Ordering::Acquire) {
                    task_state.emit_crash_for_project(task_project_id, error.to_string());
                }
            }
            task_state.cleanup_project(task_project_id).await;
        });

        let (connection, negotiated) = ready_rx
            .await
            .map_err(|_| {
                anyhow!(
                    "{} ACP process stopped during initialization",
                    self.config.display_name
                )
            })?
            .map_err(|message| anyhow!(message))?;
        *self.negotiated.write().expect("ACP capabilities poisoned") = Some(negotiated.clone());
        *self.health.write().expect("ACP health poisoned") = Some(ProviderHealth {
            provider: self.config.provider,
            state: ProviderState::Ready,
            version: negotiated.version.clone(),
            detail: Some("ACP v1 initialized".to_owned()),
        });
        Ok(Arc::new(ProjectConnection {
            connection,
            negotiated,
            shutdown: StdMutex::new(Some(shutdown_tx)),
        }))
    }

    fn session_binding(&self, project_id: ProjectId, session_id: &str) -> Option<SessionBinding> {
        self.sessions
            .read()
            .expect("ACP session map poisoned")
            .get(&(project_id, session_id.to_owned()))
            .cloned()
    }

    fn prepare_history_replay(
        &self,
        project_id: ProjectId,
        session_id: &str,
        conversation_id: ConversationId,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<Option<SessionBinding>> {
        let key = (project_id, session_id.to_owned());
        let mut sessions = self.sessions.write().expect("ACP session map poisoned");
        if let Some(binding) = sessions.get_mut(&key) {
            if binding.conversation_id != conversation_id {
                bail!("ACP session {session_id} is already bound to another conversation");
            }
            if binding.prompt_in_flight {
                bail!("ACP session {session_id} has an active turn; history sync must be retried");
            }
            if binding.replaying {
                bail!("ACP session {session_id} history replay is already in progress");
            }

            let previous_binding = binding.clone();
            binding.model = model;
            binding.effort = effort;
            binding.replaying = true;
            binding.turn_index = 0;
            binding.replay_user_text.clear();
            binding.replay_agent_text.clear();
            binding.replay_thought_text.clear();
            binding.replay_has_response = false;
            binding.replay_image_index = 0;
            return Ok(Some(previous_binding));
        }

        let prefix = format!("history:{session_id}:0");
        sessions.insert(
            key,
            SessionBinding {
                conversation_id,
                model,
                effort,
                session_options: Vec::new(),
                acp_options: Vec::new(),
                modes: None,
                prompt_in_flight: false,
                replaying: true,
                turn_index: 0,
                user_item_id: format!("{prefix}:user"),
                agent_item_id: format!("{prefix}:agent"),
                thought_item_id: format!("{prefix}:thought"),
                replay_user_text: String::new(),
                replay_agent_text: String::new(),
                replay_thought_text: String::new(),
                replay_has_response: false,
                replay_image_index: 0,
            },
        );
        Ok(None)
    }

    fn restore_history_binding(
        &self,
        project_id: ProjectId,
        session_id: &str,
        previous_binding: Option<SessionBinding>,
    ) {
        let key = (project_id, session_id.to_owned());
        let mut sessions = self.sessions.write().expect("ACP session map poisoned");
        if let Some(binding) = previous_binding {
            sessions.insert(key, binding);
        } else {
            sessions.remove(&key);
        }
    }

    fn bind_session(
        &self,
        project_id: ProjectId,
        session_id: &str,
        conversation_id: ConversationId,
        model: Option<String>,
        effort: Option<String>,
    ) {
        let prefix = format!("history:{session_id}:0");
        self.sessions
            .write()
            .expect("ACP session map poisoned")
            .insert(
                (project_id, session_id.to_owned()),
                SessionBinding {
                    conversation_id,
                    model,
                    effort,
                    session_options: Vec::new(),
                    acp_options: Vec::new(),
                    modes: None,
                    prompt_in_flight: false,
                    replaying: false,
                    turn_index: 0,
                    user_item_id: format!("{prefix}:user"),
                    agent_item_id: format!("{prefix}:agent"),
                    thought_item_id: format!("{prefix}:thought"),
                    replay_user_text: String::new(),
                    replay_agent_text: String::new(),
                    replay_thought_text: String::new(),
                    replay_has_response: false,
                    replay_image_index: 0,
                },
            );
    }

    fn set_replaying(&self, project_id: ProjectId, session_id: &str, replaying: bool) {
        let completed = {
            let mut sessions = self.sessions.write().expect("ACP session map poisoned");
            let Some(binding) = sessions.get_mut(&(project_id, session_id.to_owned())) else {
                return;
            };
            binding.replaying = replaying;
            if replaying {
                binding.turn_index = 0;
                binding.replay_user_text.clear();
                binding.replay_agent_text.clear();
                binding.replay_thought_text.clear();
                binding.replay_has_response = false;
                binding.replay_image_index = 0;
                None
            } else {
                Some((binding.conversation_id, drain_replay_items(binding)))
            }
        };
        if let Some((conversation_id, items)) = completed {
            self.emit_replay_items(project_id, conversation_id, items);
        }
    }

    fn start_turn(&self, project_id: ProjectId, session_id: &str) -> bool {
        if let Some(binding) = self
            .sessions
            .write()
            .expect("ACP session map poisoned")
            .get_mut(&(project_id, session_id.to_owned()))
        {
            if binding.replaying || binding.prompt_in_flight {
                return false;
            }
            binding.prompt_in_flight = true;
            binding.turn_index += 1;
            let prefix = format!("history:{session_id}:{}", binding.turn_index);
            binding.user_item_id = format!("{prefix}:user");
            binding.agent_item_id = format!("{prefix}:agent");
            binding.thought_item_id = format!("{prefix}:thought");
            binding.replay_user_text.clear();
            binding.replay_agent_text.clear();
            binding.replay_thought_text.clear();
            binding.replay_has_response = false;
            return true;
        }
        false
    }

    fn finish_turn(&self, project_id: ProjectId, session_id: &str) {
        if let Some(binding) = self
            .sessions
            .write()
            .expect("ACP session map poisoned")
            .get_mut(&(project_id, session_id.to_owned()))
        {
            binding.prompt_in_flight = false;
        }
    }

    fn update_selection(
        &self,
        project_id: ProjectId,
        session_id: &str,
        model: Option<String>,
        effort: Option<String>,
    ) {
        if let Some(binding) = self
            .sessions
            .write()
            .expect("ACP session map poisoned")
            .get_mut(&(project_id, session_id.to_owned()))
        {
            binding.model = model;
            binding.effort = effort;
        }
    }

    fn update_acp_options(
        &self,
        project_id: ProjectId,
        session_id: &str,
        acp_options: Vec<SessionConfigOption>,
    ) {
        if let Some(binding) = self
            .sessions
            .write()
            .expect("ACP session map poisoned")
            .get_mut(&(project_id, session_id.to_owned()))
        {
            let options = public_session_options(&acp_options, binding.modes.as_ref());
            let (model, effort) = selected_model_and_effort(&options);
            binding.model = model;
            binding.effort = effort;
            binding.session_options = options;
            binding.acp_options = acp_options;
        }
    }

    fn update_modes(&self, project_id: ProjectId, session_id: &str, modes: SessionModeState) {
        if let Some(binding) = self
            .sessions
            .write()
            .expect("ACP session map poisoned")
            .get_mut(&(project_id, session_id.to_owned()))
        {
            binding.modes = Some(modes);
            binding.session_options =
                public_session_options(&binding.acp_options, binding.modes.as_ref());
        }
    }

    fn update_current_mode(&self, project_id: ProjectId, session_id: &str, mode_id: &str) {
        if let Some(binding) = self
            .sessions
            .write()
            .expect("ACP session map poisoned")
            .get_mut(&(project_id, session_id.to_owned()))
            && let Some(modes) = binding.modes.as_mut()
        {
            modes.current_mode_id = mode_id.to_owned().into();
            binding.session_options =
                public_session_options(&binding.acp_options, binding.modes.as_ref());
        }
    }

    fn emit_session_options(&self, project_id: ProjectId, session_id: &str) {
        let state = self
            .sessions
            .read()
            .expect("ACP session map poisoned")
            .get(&(project_id, session_id.to_owned()))
            .map(|binding| {
                (
                    binding.conversation_id,
                    binding.session_options.clone(),
                    binding.model.clone(),
                    binding.effort.clone(),
                )
            });
        if let Some((conversation_id, session_options, selected_model, selected_effort)) = state {
            self.emit(
                project_id,
                conversation_id,
                ProviderEventKind::SessionOptionsChanged {
                    session_options,
                    selected_model,
                    selected_effort,
                },
            );
        }
    }

    fn emit(
        &self,
        project_id: ProjectId,
        conversation_id: ConversationId,
        kind: ProviderEventKind,
    ) {
        let _ = self.events.send(ProviderEvent {
            provider: self.config.provider,
            project_id,
            conversation_id,
            kind,
        });
    }

    fn emit_crash_for_project(&self, project_id: ProjectId, message: String) {
        let conversations: Vec<_> = self
            .sessions
            .read()
            .expect("ACP session map poisoned")
            .iter()
            .filter_map(|((bound_project, _), binding)| {
                (*bound_project == project_id).then_some(binding.conversation_id)
            })
            .collect();
        for conversation_id in conversations {
            self.emit(
                project_id,
                conversation_id,
                ProviderEventKind::Crashed {
                    message: message.clone(),
                },
            );
        }
    }

    fn handle_permission_request(
        &self,
        project_id: ProjectId,
        request: RequestPermissionRequest,
        responder: Responder<RequestPermissionResponse>,
    ) -> std::result::Result<(), agent_client_protocol::Error> {
        let session_id = request.session_id.to_string();
        let Some(binding) = self
            .sessions
            .read()
            .expect("ACP session map poisoned")
            .get(&(project_id, session_id.clone()))
            .cloned()
        else {
            return responder.respond(RequestPermissionResponse::new(
                RequestPermissionOutcome::Cancelled,
            ));
        };
        if request.options.is_empty() {
            return responder.respond(RequestPermissionResponse::new(
                RequestPermissionOutcome::Cancelled,
            ));
        }

        let provider_request_id = Uuid::new_v4().to_string();
        let options: Vec<_> = request
            .options
            .iter()
            .map(|option| ApprovalOption {
                id: option.option_id.to_string(),
                label: option.name.clone(),
            })
            .collect();
        let allowed_options = options.iter().map(|option| option.id.clone()).collect();
        let prompt = request.tool_call.fields.title.clone().unwrap_or_else(|| {
            format!(
                "{} requests permission to run a tool",
                self.config.display_name
            )
        });
        self.permissions
            .lock()
            .expect("ACP permission map poisoned")
            .insert(
                provider_request_id.clone(),
                PendingPermission {
                    conversation_id: binding.conversation_id,
                    session_id,
                    allowed_options,
                    responder,
                },
            );
        self.emit(
            project_id,
            binding.conversation_id,
            ProviderEventKind::Approval {
                provider_request_id,
                prompt,
                options,
            },
        );
        Ok(())
    }

    fn cancel_permissions(&self, session_id: &str) -> Result<()> {
        let pending: Vec<_> = {
            let mut permissions = self
                .permissions
                .lock()
                .expect("ACP permission map poisoned");
            let ids: Vec<_> = permissions
                .iter()
                .filter_map(|(id, pending)| {
                    (pending.session_id == session_id).then_some(id.clone())
                })
                .collect();
            ids.into_iter()
                .filter_map(|id| permissions.remove(&id))
                .collect()
        };
        for pending in pending {
            pending.responder.respond(RequestPermissionResponse::new(
                RequestPermissionOutcome::Cancelled,
            ))?;
        }
        Ok(())
    }

    fn ensure_bound_session(&self, project_id: ProjectId, session_id: &SessionId) -> Result<()> {
        if self
            .sessions
            .read()
            .expect("ACP session map poisoned")
            .contains_key(&(project_id, session_id.to_string()))
        {
            Ok(())
        } else {
            bail!("unknown ACP session {}", session_id)
        }
    }

    async fn read_text_file(
        &self,
        project: &Project,
        request: ReadTextFileRequest,
    ) -> Result<ReadTextFileResponse> {
        self.ensure_bound_session(project.id, &request.session_id)?;
        if request.line == Some(0) {
            bail!("ACP read line numbers are 1-based");
        }
        let path = confined_existing_path(project, &request.path)?;
        let content = tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("read {}", path.display()))?;
        Ok(ReadTextFileResponse::new(select_lines(
            &content,
            request.line,
            request.limit,
        )))
    }

    async fn write_text_file(
        &self,
        project: &Project,
        request: WriteTextFileRequest,
    ) -> Result<WriteTextFileResponse> {
        self.ensure_bound_session(project.id, &request.session_id)?;
        let path = confined_write_path(project, &request.path)?;
        tokio::fs::write(&path, request.content)
            .await
            .with_context(|| format!("write {}", path.display()))?;
        Ok(WriteTextFileResponse::new())
    }

    async fn create_terminal(
        &self,
        project: &Project,
        request: CreateTerminalRequest,
    ) -> Result<CreateTerminalResponse> {
        self.ensure_bound_session(project.id, &request.session_id)?;
        let cwd = confined_existing_path(
            project,
            request.cwd.as_deref().unwrap_or(&project.canonical_path),
        )?;
        if !cwd.is_dir() {
            bail!("terminal working directory is not a directory");
        }

        let mut command = Command::new(&request.command);
        command
            .args(&request.args)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for variable in request.env {
            command.env(variable.name, variable.value);
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("start terminal command {}", request.command))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("terminal stdout was unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("terminal stderr was unavailable"))?;
        let limit = request
            .output_byte_limit
            .map(|limit| usize::try_from(limit).unwrap_or(usize::MAX));
        let output = Arc::new(StdMutex::new(TerminalOutputBuffer {
            text: String::new(),
            limit,
            truncated: false,
        }));
        let stdout_reader = tokio::spawn(read_terminal_stream(stdout, Arc::clone(&output)));
        let stderr_reader = tokio::spawn(read_terminal_stream(stderr, Arc::clone(&output)));

        let (kill_tx, mut kill_rx) = mpsc::unbounded_channel();
        let (exit_tx, exit_rx) = watch::channel(None);
        tokio::spawn(async move {
            let status = tokio::select! {
                status = child.wait() => status,
                _ = kill_rx.recv() => {
                    let _ = child.kill().await;
                    child.wait().await
                }
            };
            let status = status
                .map(|status| ProcessExit {
                    exit_code: status.code().map(|code| code as u32),
                    signal: None,
                })
                .map_err(|error| error.to_string());
            let _ = stdout_reader.await;
            let _ = stderr_reader.await;
            let _ = exit_tx.send(Some(status));
        });

        let terminal_id = Uuid::new_v4().to_string();
        self.terminals.lock().await.insert(
            terminal_id.clone(),
            Arc::new(TerminalProcess {
                project_id: project.id,
                session_id: request.session_id.to_string(),
                output,
                kill: kill_tx,
                exit: exit_rx,
            }),
        );
        Ok(CreateTerminalResponse::new(terminal_id))
    }

    async fn terminal_for(
        &self,
        session_id: &SessionId,
        terminal_id: &str,
    ) -> Result<Arc<TerminalProcess>> {
        let terminal = self
            .terminals
            .lock()
            .await
            .get(terminal_id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown or released terminal {terminal_id}"))?;
        if terminal.session_id != session_id.to_string() {
            bail!("terminal does not belong to session {session_id}");
        }
        self.ensure_bound_session(terminal.project_id, session_id)?;
        Ok(terminal)
    }

    async fn terminal_output(
        &self,
        request: TerminalOutputRequest,
    ) -> Result<TerminalOutputResponse> {
        let terminal = self
            .terminal_for(&request.session_id, &request.terminal_id.to_string())
            .await?;
        let output = terminal
            .output
            .lock()
            .expect("terminal output mutex poisoned");
        let mut response = TerminalOutputResponse::new(output.text.clone(), output.truncated);
        if let Some(status) = terminal.exit.borrow().clone() {
            response =
                response.exit_status(process_exit_status(status.map_err(anyhow::Error::msg)?));
        }
        Ok(response)
    }

    async fn wait_for_terminal_exit(
        &self,
        request: WaitForTerminalExitRequest,
    ) -> Result<WaitForTerminalExitResponse> {
        let terminal = self
            .terminal_for(&request.session_id, &request.terminal_id.to_string())
            .await?;
        let mut exit = terminal.exit.clone();
        loop {
            if let Some(status) = exit.borrow().clone() {
                return Ok(WaitForTerminalExitResponse::new(process_exit_status(
                    status.map_err(anyhow::Error::msg)?,
                )));
            }
            exit.changed()
                .await
                .map_err(|_| anyhow!("terminal exit monitor closed"))?;
        }
    }

    async fn kill_terminal(&self, request: KillTerminalRequest) -> Result<KillTerminalResponse> {
        let terminal = self
            .terminal_for(&request.session_id, &request.terminal_id.to_string())
            .await?;
        let _ = terminal.kill.send(());
        wait_for_exit(&terminal).await?;
        Ok(KillTerminalResponse::new())
    }

    async fn release_terminal(
        &self,
        request: ReleaseTerminalRequest,
    ) -> Result<ReleaseTerminalResponse> {
        let terminal_id = request.terminal_id.to_string();
        let terminal = self.terminal_for(&request.session_id, &terminal_id).await?;
        self.terminals.lock().await.remove(&terminal_id);
        let _ = terminal.kill.send(());
        wait_for_exit(&terminal).await?;
        Ok(ReleaseTerminalResponse::new())
    }

    async fn release_session_terminals(&self, session_id: &str) {
        let terminals = {
            let mut active = self.terminals.lock().await;
            let ids: Vec<_> = active
                .iter()
                .filter_map(|(id, terminal)| {
                    (terminal.session_id == session_id).then_some(id.clone())
                })
                .collect();
            ids.into_iter()
                .filter_map(|id| active.remove(&id))
                .collect::<Vec<_>>()
        };
        for terminal in terminals {
            let _ = terminal.kill.send(());
        }
    }

    async fn cleanup_project(&self, project_id: ProjectId) {
        let session_ids: Vec<_> = self
            .sessions
            .read()
            .expect("ACP session map poisoned")
            .keys()
            .filter_map(|(bound_project, session_id)| {
                (*bound_project == project_id).then_some(session_id.clone())
            })
            .collect();
        for session_id in &session_ids {
            let _ = self.cancel_permissions(session_id);
        }
        self.sessions
            .write()
            .expect("ACP session map poisoned")
            .retain(|(bound_project, _), _| *bound_project != project_id);

        let terminals = {
            let mut active = self.terminals.lock().await;
            let ids: Vec<_> = active
                .iter()
                .filter_map(|(id, terminal)| {
                    (terminal.project_id == project_id).then_some(id.clone())
                })
                .collect();
            ids.into_iter()
                .filter_map(|id| active.remove(&id))
                .collect::<Vec<_>>()
        };
        for terminal in terminals {
            let _ = terminal.kill.send(());
        }
    }
}

fn negotiate(response: &InitializeResponse, flavor: AcpFlavor) -> Result<Negotiated> {
    if response.protocol_version != ProtocolVersion::V1 {
        bail!(
            "agent selected unsupported ACP protocol version {:?}",
            response.protocol_version
        );
    }
    let meta = response.meta.as_ref();
    let version = meta
        .and_then(|meta| meta.get("agentVersion"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            response
                .agent_info
                .as_ref()
                .map(|info| info.version.clone())
        });
    let grok_shell = flavor == AcpFlavor::Grok
        && meta
            .and_then(|meta| meta.get("grokShell"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let (models, current_model) = meta
        .and_then(|meta| meta.get("modelState"))
        .map(parse_model_state)
        .unwrap_or_default();
    let capabilities = &response.agent_capabilities;
    Ok(Negotiated {
        version,
        grok_shell,
        supports_list: capabilities.session_capabilities.list.is_some(),
        supports_load: capabilities.load_session,
        supports_resume: capabilities.session_capabilities.resume.is_some(),
        supports_close: capabilities.session_capabilities.close.is_some(),
        models,
        current_model,
    })
}

fn parse_model_state(value: &Value) -> (Vec<GrokModel>, Option<String>) {
    let current_model = value
        .get("currentModelId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let models = value
        .get("availableModels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let id = model.get("modelId")?.as_str()?.to_owned();
            let name = model
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(&id)
                .to_owned();
            let model_meta = model.get("_meta").and_then(Value::as_object);
            let configured_effort = model_meta
                .and_then(|meta| meta.get("reasoningEffort"))
                .and_then(Value::as_str);
            let effort_values = model_meta
                .and_then(|meta| meta.get("reasoningEfforts"))
                .and_then(Value::as_array);
            let efforts: Vec<_> = effort_values
                .into_iter()
                .flatten()
                .filter_map(|effort| {
                    let id = effort
                        .get("id")
                        .or_else(|| effort.get("value"))?
                        .as_str()?
                        .to_owned();
                    let label = effort
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or(&id)
                        .to_owned();
                    Some(GrokEffort { id, label })
                })
                .collect();
            let default_effort = effort_values
                .into_iter()
                .flatten()
                .find(|effort| {
                    effort
                        .get("default")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                })
                .and_then(|effort| {
                    effort
                        .get("id")
                        .or_else(|| effort.get("value"))
                        .and_then(Value::as_str)
                })
                .or(configured_effort)
                .map(str::to_owned);
            Some(GrokModel {
                id,
                name,
                efforts,
                default_effort,
            })
        })
        .collect();
    (models, current_model)
}

fn select_model_and_effort(
    negotiated: &Negotiated,
    selected_model: Option<&str>,
    selected_effort: Option<&str>,
) -> Result<(Option<String>, Option<String>)> {
    if negotiated.models.is_empty() {
        if selected_model.is_some() || selected_effort.is_some() {
            bail!("Grok did not report structured model metadata");
        }
        return Ok((None, None));
    }
    let model_id = selected_model
        .map(str::to_owned)
        .or_else(|| negotiated.current_model.clone())
        .or_else(|| negotiated.models.first().map(|model| model.id.clone()))
        .expect("non-empty model list");
    let model = negotiated
        .models
        .iter()
        .find(|model| model.id == model_id)
        .ok_or_else(|| anyhow!("model {model_id} was not reported by Grok"))?;
    let effort = selected_effort
        .map(str::to_owned)
        .or_else(|| model.default_effort.clone());
    if let Some(effort) = effort.as_deref()
        && !model.efforts.iter().any(|candidate| candidate.id == effort)
    {
        bail!("effort {effort} is not supported by model {model_id}");
    }
    Ok((Some(model_id), effort))
}

fn session_options(
    models: &[GrokModel],
    selected_model: Option<&str>,
    selected_effort: Option<&str>,
) -> Vec<SessionOption> {
    let mut options = Vec::new();
    if let Some(selected_model) = selected_model {
        options.push(SessionOption {
            id: "model".to_owned(),
            display_name: "Model".to_owned(),
            category: Some("model".to_owned()),
            current_value: selected_model.to_owned(),
            values: models
                .iter()
                .map(|model| SessionOptionValue {
                    value: model.id.clone(),
                    display_name: model.name.clone(),
                })
                .collect(),
        });
    }
    if let (Some(selected_model), Some(selected_effort)) = (selected_model, selected_effort)
        && let Some(model) = models.iter().find(|model| model.id == selected_model)
    {
        options.push(SessionOption {
            id: "reasoning_effort".to_owned(),
            display_name: "Reasoning effort".to_owned(),
            category: Some("thought_level".to_owned()),
            current_value: selected_effort.to_owned(),
            values: model
                .efforts
                .iter()
                .map(|effort| SessionOptionValue {
                    value: effort.id.clone(),
                    display_name: effort.label.clone(),
                })
                .collect(),
        });
    }
    options
}

fn public_session_options(
    options: &[SessionConfigOption],
    modes: Option<&SessionModeState>,
) -> Vec<SessionOption> {
    let mut public = options
        .iter()
        .map(|option| {
            let (current_value, values) = match &option.kind {
                SessionConfigKind::Select(select) => {
                    let values = match &select.options {
                        SessionConfigSelectOptions::Ungrouped(options) => options
                            .iter()
                            .map(|value| SessionOptionValue {
                                value: value.value.to_string(),
                                display_name: value.name.clone(),
                            })
                            .collect(),
                        SessionConfigSelectOptions::Grouped(groups) => groups
                            .iter()
                            .flat_map(|group| group.options.iter())
                            .map(|value| SessionOptionValue {
                                value: value.value.to_string(),
                                display_name: value.name.clone(),
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                    (select.current_value.to_string(), values)
                }
                SessionConfigKind::Boolean(boolean) => (
                    boolean.current_value.to_string(),
                    vec![
                        SessionOptionValue {
                            value: "true".to_owned(),
                            display_name: "On".to_owned(),
                        },
                        SessionOptionValue {
                            value: "false".to_owned(),
                            display_name: "Off".to_owned(),
                        },
                    ],
                ),
                _ => (String::new(), Vec::new()),
            };
            SessionOption {
                id: option.id.to_string(),
                display_name: option.name.clone(),
                category: option.category.as_ref().map(session_option_category),
                current_value,
                values,
            }
        })
        .filter(|option| !option.current_value.is_empty() && !option.values.is_empty())
        .collect::<Vec<_>>();
    if !public
        .iter()
        .any(|option| option.category.as_deref() == Some("mode"))
        && let Some(modes) = modes
        && !modes.available_modes.is_empty()
    {
        public.push(SessionOption {
            id: "mode".to_owned(),
            display_name: "Mode".to_owned(),
            category: Some("mode".to_owned()),
            current_value: modes.current_mode_id.to_string(),
            values: modes
                .available_modes
                .iter()
                .map(|mode| SessionOptionValue {
                    value: mode.id.to_string(),
                    display_name: mode.name.clone(),
                })
                .collect(),
        });
    }
    public
}

fn session_option_category(category: &SessionConfigOptionCategory) -> String {
    match category {
        SessionConfigOptionCategory::Mode => "mode".to_owned(),
        SessionConfigOptionCategory::Model => "model".to_owned(),
        SessionConfigOptionCategory::ModelConfig => "model_config".to_owned(),
        SessionConfigOptionCategory::ThoughtLevel => "thought_level".to_owned(),
        SessionConfigOptionCategory::Other(value) => value.clone(),
        _ => "other".to_owned(),
    }
}

fn selected_model_and_effort(options: &[SessionOption]) -> (Option<String>, Option<String>) {
    let selected = |category: &str| {
        options
            .iter()
            .find(|option| option.category.as_deref() == Some(category))
            .map(|option| option.current_value.clone())
    };
    (selected("model"), selected("thought_level"))
}

fn config_option_value(
    options: &[SessionConfigOption],
    option_id: &str,
    value: &str,
) -> Result<SessionConfigOptionValue> {
    let option = options
        .iter()
        .find(|option| option.id.to_string() == option_id)
        .ok_or_else(|| anyhow!("ACP session option {option_id} is not available"))?;
    match &option.kind {
        SessionConfigKind::Select(select) => {
            let found = match &select.options {
                SessionConfigSelectOptions::Ungrouped(values) => values
                    .iter()
                    .any(|candidate| candidate.value.to_string() == value),
                SessionConfigSelectOptions::Grouped(groups) => groups.iter().any(|group| {
                    group
                        .options
                        .iter()
                        .any(|candidate| candidate.value.to_string() == value)
                }),
                _ => false,
            };
            if !found {
                bail!("ACP session option {option_id} does not offer {value}");
            }
            Ok(SessionConfigOptionValue::value_id(value.to_owned()))
        }
        SessionConfigKind::Boolean(_) => match value {
            "true" => Ok(SessionConfigOptionValue::boolean(true)),
            "false" => Ok(SessionConfigOptionValue::boolean(false)),
            _ => bail!("ACP boolean session option {option_id} requires true or false"),
        },
        _ => bail!("ACP session option {option_id} has an unsupported value type"),
    }
}

fn native_session(
    native_session_id: String,
    title: String,
    model: Option<String>,
    effort: Option<String>,
    models: &[GrokModel],
) -> NativeSession {
    NativeSession {
        native_session_id,
        title,
        selected_model: model.clone(),
        selected_effort: effort.clone(),
        session_options: session_options(models, model.as_deref(), effort.as_deref()),
    }
}

fn selection_meta(model: Option<&str>, effort: Option<&str>) -> Meta {
    let mut meta = Meta::new();
    if let Some(model) = model {
        meta.insert("modelId".to_owned(), Value::String(model.to_owned()));
    }
    if let Some(effort) = effort {
        meta.insert(
            "reasoningEffort".to_owned(),
            Value::String(effort.to_owned()),
        );
    }
    meta
}

async fn set_grok_model(
    connection: &ProjectConnection,
    session_id: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<()> {
    if !connection.negotiated.supports_extensions() {
        bail!(
            "live model changes require Grok Build {}",
            GROK_EXTENSION_VERSION
        );
    }
    let model = model.ok_or_else(|| anyhow!("Grok did not report a selected model"))?;
    let mut meta = Meta::new();
    if let Some(effort) = effort {
        meta.insert(
            "reasoningEffort".to_owned(),
            Value::String(effort.to_owned()),
        );
    }
    connection
        .connection
        .send_request(SetModelRequest {
            session_id: SessionId::new(session_id.to_owned()),
            model_id: model.to_owned(),
            meta,
        })
        .block_task()
        .await?;
    Ok(())
}

fn stop_reason_event(stop_reason: StopReason) -> ProviderEventKind {
    match stop_reason {
        StopReason::Cancelled => ProviderEventKind::Interrupted,
        StopReason::EndTurn
        | StopReason::MaxTokens
        | StopReason::MaxTurnRequests
        | StopReason::Refusal => ProviderEventKind::Completed,
        _ => ProviderEventKind::Completed,
    }
}

fn tool_status(status: ToolCallStatus) -> ItemStatus {
    match status {
        ToolCallStatus::Pending => ItemStatus::Pending,
        ToolCallStatus::InProgress => ItemStatus::Running,
        ToolCallStatus::Completed => ItemStatus::Completed,
        ToolCallStatus::Failed => ItemStatus::Failed,
        _ => ItemStatus::Pending,
    }
}

fn json_summary(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn parse_cli_version(stdout: &str) -> Option<String> {
    stdout
        .split_whitespace()
        .find(|token| token.chars().next().is_some_and(|ch| ch.is_ascii_digit()))
        .map(|token| {
            token
                .trim_matches(|ch: char| ch == '(' || ch == ')')
                .to_owned()
        })
}

fn classify_connection_error(
    provider: ProviderId,
    error: &agent_client_protocol::Error,
) -> ProviderHealth {
    let message = error.to_string();
    let lower = message.to_ascii_lowercase();
    let state = if lower.contains("login")
        || lower.contains("not authenticated")
        || lower.contains("unauthenticated")
        || lower.contains("credential")
    {
        ProviderState::NotAuthenticated
    } else if lower.contains("protocol version") {
        ProviderState::ProtocolIncompatible
    } else {
        ProviderState::Crashed
    };
    ProviderHealth {
        provider,
        state,
        version: None,
        detail: Some(message),
    }
}

fn provider_slug(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Codex => "codex",
        ProviderId::Grok => "grok",
        ProviderId::ClaudeCode => "claude-code",
        ProviderId::GeminiCli => "gemini-cli",
        ProviderId::CopilotCli => "copilot-cli",
        ProviderId::OpenCode => "opencode",
        ProviderId::Cursor => "cursor",
        ProviderId::Cline => "cline",
        ProviderId::Goose => "goose",
        ProviderId::Junie => "junie",
        ProviderId::QwenCode => "qwen-code",
        ProviderId::KimiCli => "kimi-cli",
        ProviderId::KiroCli => "kiro-cli",
        ProviderId::MistralVibe => "mistral-vibe",
        ProviderId::QoderCli => "qoder-cli",
    }
}

fn acp_handler_error(error: anyhow::Error) -> agent_client_protocol::Error {
    agent_client_protocol::Error::invalid_params().data(error.to_string())
}

fn confined_existing_path(project: &Project, requested: &Path) -> Result<PathBuf> {
    if !requested.is_absolute() {
        bail!("ACP file and terminal paths must be absolute");
    }
    project.resolve_existing(requested)
}

fn confined_write_path(project: &Project, requested: &Path) -> Result<PathBuf> {
    if !requested.is_absolute() {
        bail!("ACP write paths must be absolute");
    }
    if requested.exists() {
        return project.resolve_existing(requested);
    }
    project.resolve_for_write(requested)
}

fn select_lines(content: &str, line: Option<u32>, limit: Option<u32>) -> String {
    let start = line.unwrap_or(1).saturating_sub(1) as usize;
    let limit = limit.map(|value| value as usize).unwrap_or(usize::MAX);
    content
        .split_inclusive('\n')
        .skip(start)
        .take(limit)
        .collect()
}

async fn read_terminal_stream(
    mut stream: impl tokio::io::AsyncRead + Unpin,
    output: Arc<StdMutex<TerminalOutputBuffer>>,
) {
    let mut bytes = [0_u8; 4096];
    loop {
        match stream.read(&mut bytes).await {
            Ok(0) => break,
            Ok(read) => output
                .lock()
                .expect("terminal output mutex poisoned")
                .push(&String::from_utf8_lossy(&bytes[..read])),
            Err(error) => {
                tracing::debug!(%error, "failed to read ACP terminal output");
                break;
            }
        }
    }
}

impl TerminalOutputBuffer {
    fn push(&mut self, value: &str) {
        self.text.push_str(value);
        let Some(limit) = self.limit else {
            return;
        };
        if self.text.len() <= limit {
            return;
        }
        self.truncated = true;
        let mut cut = self.text.len() - limit;
        while cut < self.text.len() && !self.text.is_char_boundary(cut) {
            cut += 1;
        }
        self.text.drain(..cut);
    }
}

fn process_exit_status(exit: ProcessExit) -> TerminalExitStatus {
    TerminalExitStatus::new()
        .exit_code(exit.exit_code)
        .signal(exit.signal)
}

async fn wait_for_exit(terminal: &TerminalProcess) -> Result<ProcessExit> {
    let mut exit = terminal.exit.clone();
    loop {
        if let Some(status) = exit.borrow().clone() {
            return status.map_err(anyhow::Error::msg);
        }
        exit.changed()
            .await
            .map_err(|_| anyhow!("terminal exit monitor closed"))?;
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use std::{fs, sync::Arc};

    use agent_client_protocol::schema::v1::{
        AgentCapabilities, AuthMethod, AuthMethodAgent, InitializeResponse, PermissionOption,
        PermissionOptionKind, RequestPermissionRequest, SessionConfigBoolean, SessionConfigSelect,
        SessionConfigSelectGroup, SessionConfigSelectOption, SessionListCapabilities, SessionMode,
        SessionResumeCapabilities, SetSessionConfigOptionResponse, ToolCallUpdate,
        ToolCallUpdateFields,
    };
    use agent_client_protocol::{Agent, Channel, Client};
    use tokio::sync::oneshot;

    use super::*;

    fn model_state() -> Value {
        serde_json::json!({
            "currentModelId": "grok-4.6",
            "availableModels": [
                {
                    "modelId": "grok-4.6",
                    "name": "Grok 4.6",
                    "_meta": {
                        "reasoningEffort": "high",
                        "reasoningEfforts": [
                            {"id": "xhigh", "label": "Extra High", "default": false},
                            {"id": "high", "label": "High", "default": true},
                            {"id": "medium", "label": "Medium", "default": false},
                            {"id": "low", "label": "Low", "default": false}
                        ]
                    }
                },
                {
                    "modelId": "grok-4.5",
                    "name": "Grok 4.5",
                    "_meta": {
                        "reasoningEffort": "high",
                        "reasoningEfforts": [
                            {"id": "high", "label": "High", "default": true},
                            {"id": "medium", "label": "Medium", "default": false},
                            {"id": "low", "label": "Low", "default": false}
                        ]
                    }
                }
            ]
        })
    }

    fn negotiated() -> Negotiated {
        let (models, current_model) = parse_model_state(&model_state());
        Negotiated {
            version: Some(GROK_EXTENSION_VERSION.to_owned()),
            grok_shell: true,
            supports_list: true,
            supports_load: true,
            supports_resume: true,
            supports_close: true,
            models,
            current_model,
        }
    }

    #[test]
    fn grok_extensions_require_the_grok_profile() {
        let mut meta = Meta::new();
        meta.insert(
            "agentVersion".to_owned(),
            Value::String(GROK_EXTENSION_VERSION.to_owned()),
        );
        meta.insert("grokShell".to_owned(), Value::Bool(true));
        let response = InitializeResponse::new(ProtocolVersion::V1).meta(meta);

        assert!(
            negotiate(&response, AcpFlavor::Grok)
                .expect("Grok negotiation")
                .supports_extensions()
        );
        assert!(
            !negotiate(&response, AcpFlavor::Standard)
                .expect("standard ACP negotiation")
                .supports_extensions()
        );
    }

    fn project(path: &Path) -> Project {
        Project {
            id: ProjectId::new(),
            display_name: "test".to_owned(),
            canonical_path: path.canonicalize().expect("canonical test path"),
            enabled_providers: vec![ProviderId::Grok],
        }
    }

    #[test]
    fn maps_dynamic_models_and_model_dependent_efforts() {
        let negotiated = negotiated();
        let models = negotiated.public_models();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "grok-4.6");
        assert_eq!(models[0].default_effort.as_deref(), Some("high"));
        assert!(
            models[0]
                .effort_options
                .iter()
                .any(|effort| effort.id == "xhigh")
        );
        assert!(
            !models[1]
                .effort_options
                .iter()
                .any(|effort| effort.id == "xhigh")
        );
        assert!(select_model_and_effort(&negotiated, Some("grok-4.5"), Some("xhigh")).is_err());

        let options = session_options(&negotiated.models, Some("grok-4.5"), Some("high"));
        assert_eq!(options[0].id, "model");
        assert_eq!(options[1].id, "reasoning_effort");
        assert_eq!(options[1].category.as_deref(), Some("thought_level"));
        assert_eq!(options[1].values.len(), 3);
    }

    #[test]
    fn standard_acp_profiles_use_their_documented_launch_arguments() {
        let profiles = [
            (
                AcpProviderConfig::cursor(),
                ProviderId::Cursor,
                "Cursor Agent",
                vec!["acp"],
                Some("cursor_login"),
            ),
            (
                AcpProviderConfig::cline(),
                ProviderId::Cline,
                "Cline",
                vec!["--acp"],
                None,
            ),
            (
                AcpProviderConfig::goose(),
                ProviderId::Goose,
                "Goose",
                vec!["acp"],
                None,
            ),
            (
                AcpProviderConfig::junie(),
                ProviderId::Junie,
                "JetBrains Junie",
                vec!["--acp=true"],
                None,
            ),
            (
                AcpProviderConfig::qwen(),
                ProviderId::QwenCode,
                "Qwen Code",
                vec!["--acp"],
                None,
            ),
            (
                AcpProviderConfig::kimi(),
                ProviderId::KimiCli,
                "Kimi CLI",
                vec!["acp"],
                None,
            ),
            (
                AcpProviderConfig::kiro(),
                ProviderId::KiroCli,
                "Kiro CLI",
                vec!["acp"],
                None,
            ),
            (
                AcpProviderConfig::vibe(),
                ProviderId::MistralVibe,
                "Mistral Vibe",
                vec![],
                None,
            ),
            (
                AcpProviderConfig::qoder(),
                ProviderId::QoderCli,
                "Qoder CLI",
                vec!["--acp"],
                None,
            ),
        ];

        for (profile, provider, display_name, agent_args, auth_method) in profiles {
            assert_eq!(profile.provider, provider);
            assert_eq!(profile.display_name, display_name);
            assert_eq!(profile.agent_args, agent_args);
            assert_eq!(profile.version_args, ["--version"]);
            assert_eq!(profile.auth_method, auth_method);
            assert!(matches!(profile.flavor, AcpFlavor::Standard));
        }
    }

    #[test]
    fn cursor_authentication_selects_only_the_configured_advertised_method() {
        let response = InitializeResponse::new(ProtocolVersion::V1).auth_methods(vec![
            AuthMethod::Agent(AuthMethodAgent::new("other_login", "Other login")),
            AuthMethod::Agent(AuthMethodAgent::new("cursor_login", "Cursor login")),
        ]);

        assert_eq!(
            matching_auth_method(&response, Some("cursor_login")).as_deref(),
            Some("cursor_login")
        );
        assert_eq!(matching_auth_method(&response, Some("missing_login")), None);
        assert_eq!(matching_auth_method(&response, None), None);
    }

    #[test]
    fn cursor_blocking_requests_receive_the_cancelled_outcome_shape() {
        assert!(CursorAskQuestionRequest::matches_method(
            "cursor/ask_question"
        ));
        assert!(CursorCreatePlanRequest::matches_method(
            "cursor/create_plan"
        ));
        assert_eq!(
            serde_json::to_value(CursorCancelledResponse::cancelled()).unwrap(),
            serde_json::json!({"outcome": {"outcome": "cancelled"}})
        );
    }

    #[test]
    fn maps_standard_acp_config_options_and_modes() {
        let model = SessionConfigOption::new(
            "model",
            "Model",
            SessionConfigKind::Select(SessionConfigSelect::new(
                "model-a",
                vec![
                    SessionConfigSelectOption::new("model-a", "Model A"),
                    SessionConfigSelectOption::new("model-b", "Model B"),
                ],
            )),
        )
        .category(SessionConfigOptionCategory::Model);
        let effort = SessionConfigOption::new(
            "effort",
            "Reasoning effort",
            SessionConfigKind::Select(SessionConfigSelect::new(
                "high",
                vec![SessionConfigSelectGroup::new(
                    "reasoning",
                    "Reasoning",
                    vec![
                        SessionConfigSelectOption::new("low", "Low"),
                        SessionConfigSelectOption::new("high", "High"),
                    ],
                )],
            )),
        )
        .category(SessionConfigOptionCategory::ThoughtLevel);
        let brave = SessionConfigOption::new(
            "brave",
            "Brave mode",
            SessionConfigKind::Boolean(SessionConfigBoolean::new(true)),
        );
        let modes = SessionModeState::new(
            "code",
            vec![
                SessionMode::new("code", "Code"),
                SessionMode::new("plan", "Plan"),
            ],
        );
        let raw = vec![model, effort, brave];
        let options = public_session_options(&raw, Some(&modes));

        assert_eq!(
            selected_model_and_effort(&options),
            (Some("model-a".to_owned()), Some("high".to_owned()),)
        );
        assert_eq!(
            options
                .iter()
                .find(|option| option.id == "effort")
                .unwrap()
                .values
                .len(),
            2
        );
        assert_eq!(
            options
                .iter()
                .find(|option| option.id == "mode")
                .unwrap()
                .current_value,
            "code"
        );
        assert!(matches!(
            config_option_value(&raw, "brave", "false").unwrap(),
            SessionConfigOptionValue::Boolean { value: false }
        ));
        assert!(config_option_value(&raw, "model", "missing").is_err());
    }

    #[test]
    fn standard_acp_profile_keeps_events_in_its_provider_scope() {
        let provider = AcpProvider::opencode();
        let project_id = ProjectId::new();
        let conversation_id = ConversationId::new();
        provider
            .shared
            .bind_session(project_id, "shared-id", conversation_id, None, None);
        let mut events = provider.subscribe();

        provider.shared.handle_session_update(
            project_id,
            RawSessionNotification {
                session_id: SessionId::new("shared-id"),
                update: serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "hello"}
                }),
                meta: None,
            },
        );

        let event = events.try_recv().expect("OpenCode event");
        assert_eq!(event.provider, ProviderId::OpenCode);
        assert_eq!(event.project_id, project_id);
        assert_eq!(event.conversation_id, conversation_id);
    }

    #[tokio::test]
    async fn standard_acp_profile_sets_typed_config_options() {
        let provider = AcpProvider::opencode();
        let project_id = ProjectId::new();
        let conversation_id = ConversationId::new();
        let session_id = "standard-config";
        let initial = SessionConfigOption::new(
            "model",
            "Model",
            SessionConfigKind::Select(SessionConfigSelect::new(
                "model-a",
                vec![
                    SessionConfigSelectOption::new("model-a", "Model A"),
                    SessionConfigSelectOption::new("model-b", "Model B"),
                ],
            )),
        )
        .category(SessionConfigOptionCategory::Model);
        let updated = SessionConfigOption::new(
            "model",
            "Model",
            SessionConfigKind::Select(SessionConfigSelect::new(
                "model-b",
                vec![
                    SessionConfigSelectOption::new("model-a", "Model A"),
                    SessionConfigSelectOption::new("model-b", "Model B"),
                ],
            )),
        )
        .category(SessionConfigOptionCategory::Model);
        provider
            .shared
            .bind_session(project_id, session_id, conversation_id, None, None);
        provider
            .shared
            .update_acp_options(project_id, session_id, vec![initial]);

        let (client_transport, agent_transport) = Channel::duplex();
        let (ready_tx, ready_rx) = oneshot::channel();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_task = tokio::spawn(async move {
            Client
                .builder()
                .connect_with(client_transport, async move |connection| {
                    let _ = ready_tx.send(connection.clone());
                    let _ = shutdown_rx.await;
                    Ok(())
                })
                .await
        });
        let (request_tx, mut request_rx) = mpsc::unbounded_channel();
        let agent_task = tokio::spawn(async move {
            Agent
                .builder()
                .on_receive_request(
                    async move |request: SetSessionConfigOptionRequest, responder, _connection| {
                        let _ = request_tx.send(request);
                        responder
                            .respond(SetSessionConfigOptionResponse::new(vec![updated.clone()]))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_with(agent_transport, async move |_connection| {
                    std::future::pending::<()>().await;
                    #[allow(unreachable_code)]
                    Ok(())
                })
                .await
        });
        let connection = ready_rx.await.expect("client connection");
        provider.shared.connections.lock().await.insert(
            project_id,
            Arc::new(ProjectConnection {
                connection,
                negotiated: Negotiated {
                    version: Some("test".to_owned()),
                    grok_shell: false,
                    supports_list: true,
                    supports_load: true,
                    supports_resume: true,
                    supports_close: true,
                    models: Vec::new(),
                    current_model: None,
                },
                shutdown: StdMutex::new(Some(shutdown_tx)),
            }),
        );

        provider
            .set_session_option(SetSessionOption {
                conversation_id,
                native_session_id: session_id.to_owned(),
                option_id: "model".to_owned(),
                value: "model-b".to_owned(),
            })
            .await
            .expect("set standard model config");
        let request = request_rx.recv().await.expect("set-config request");
        assert_eq!(request.config_id.to_string(), "model");
        assert_eq!(request.value.as_value_id().unwrap().to_string(), "model-b");
        assert_eq!(
            provider
                .shared
                .session_binding(project_id, session_id)
                .unwrap()
                .model
                .as_deref(),
            Some("model-b")
        );

        provider.shared.connections.lock().await.remove(&project_id);
        let _ = client_task.await;
        agent_task.abort();
    }

    #[test]
    fn confines_file_paths_to_the_project() {
        let project_root = tempfile::tempdir().expect("project tempdir");
        let outside_root = tempfile::tempdir().expect("outside tempdir");
        let inside = project_root.path().join("inside.txt");
        let outside = outside_root.path().join("outside.txt");
        fs::write(&inside, "inside").expect("write inside fixture");
        fs::write(&outside, "outside").expect("write outside fixture");
        let project = project(project_root.path());

        assert_eq!(
            confined_existing_path(&project, &inside)
                .expect("inside path")
                .canonicalize()
                .expect("canonical inside"),
            inside.canonicalize().expect("canonical fixture")
        );
        assert!(confined_existing_path(&project, &outside).is_err());
        assert!(confined_write_path(&project, &outside_root.path().join("new.txt")).is_err());
        assert!(confined_existing_path(&project, Path::new("inside.txt")).is_err());
    }

    #[test]
    fn emits_image_bytes_and_ignores_unknown_updates() {
        let provider = AcpProvider::new();
        let project_id = ProjectId::new();
        let conversation_id = ConversationId::new();
        provider.shared.bind_session(
            project_id,
            "s1",
            conversation_id,
            Some("grok-4.6".to_owned()),
            Some("high".to_owned()),
        );
        let mut events = provider.subscribe();
        provider.shared.handle_session_update(
            project_id,
            RawSessionNotification {
                session_id: SessionId::new("s1"),
                update: serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {
                        "type": "image",
                        "mimeType": "image/png",
                        "data": BASE64.encode([1_u8, 2, 3])
                    }
                }),
                meta: None,
            },
        );
        let event = events.try_recv().expect("image event");
        match event.kind {
            ProviderEventKind::ImageBytes {
                bytes, mime_type, ..
            } => {
                assert_eq!(bytes, vec![1, 2, 3]);
                assert_eq!(mime_type, "image/png");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        provider.shared.handle_session_update(
            project_id,
            RawSessionNotification {
                session_id: SessionId::new("s1"),
                update: serde_json::json!({"sessionUpdate": "future_update", "value": 1}),
                meta: None,
            },
        );
        assert!(matches!(
            events.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        assert!(matches!(
            stop_reason_event(StopReason::Cancelled),
            ProviderEventKind::Interrupted
        ));
    }

    #[test]
    fn replay_text_chunks_are_coalesced_into_stable_turn_items() {
        let provider = AcpProvider::new();
        let project_id = ProjectId::new();
        let conversation_id = ConversationId::new();
        provider
            .shared
            .bind_session(project_id, "history", conversation_id, None, None);
        let mut events = provider.subscribe();

        let replay = |provider: &AcpProvider| {
            provider.shared.set_replaying(project_id, "history", true);
            for (session_update, text) in [
                ("user_message_chunk", "hel"),
                ("user_message_chunk", "lo"),
                ("agent_thought_chunk", "summary"),
                ("agent_message_chunk", "ans"),
                ("agent_message_chunk", "wer"),
            ] {
                provider.shared.handle_session_update(
                    project_id,
                    RawSessionNotification {
                        session_id: SessionId::new("history"),
                        update: serde_json::json!({
                            "sessionUpdate": session_update,
                            "content": {"type": "text", "text": text}
                        }),
                        meta: None,
                    },
                );
            }
            provider.shared.set_replaying(project_id, "history", false);
        };
        let collect = |events: &mut broadcast::Receiver<ProviderEvent>| {
            (0..3)
                .map(|_| {
                    let event = events.try_recv().expect("coalesced history item");
                    match event.kind {
                        ProviderEventKind::HistoryItem {
                            provider_item_id,
                            kind,
                        } => (provider_item_id, kind),
                        other => panic!("unexpected replay event: {other:?}"),
                    }
                })
                .collect::<Vec<_>>()
        };

        replay(&provider);
        let first = collect(&mut events);
        assert!(matches!(
            &first[0].1,
            TimelineItemKind::UserMessage { text } if text == "hello"
        ));
        assert!(matches!(
            &first[1].1,
            TimelineItemKind::AgentMessage { phase: AgentMessagePhase::ReasoningSummary, text }
                if text == "summary"
        ));
        assert!(matches!(
            &first[2].1,
            TimelineItemKind::AgentMessage { phase: AgentMessagePhase::Final, text }
                if text == "answer"
        ));

        replay(&provider);
        let second = collect(&mut events);
        assert_eq!(
            first.iter().map(|item| &item.0).collect::<Vec<_>>(),
            second.iter().map(|item| &item.0).collect::<Vec<_>>()
        );
        assert!(matches!(
            events.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn history_read_keeps_active_turn_chunks_live() {
        let project_root = tempfile::tempdir().expect("project tempdir");
        let project = project(project_root.path());
        let provider = AcpProvider::new();
        let conversation_id = ConversationId::new();
        let session_id = "active-session";
        provider.shared.bind_session(
            project.id,
            session_id,
            conversation_id,
            Some("grok-4.6".to_owned()),
            Some("high".to_owned()),
        );
        assert!(provider.shared.start_turn(project.id, session_id));
        let live_item_id = provider
            .shared
            .session_binding(project.id, session_id)
            .expect("active binding")
            .agent_item_id;
        let mut events = provider.subscribe();

        let error = provider
            .read_session_history(ReadSessionHistory {
                conversation_id,
                project: project.clone(),
                native_session_id: session_id.to_owned(),
                cursor: None,
                limit: 200,
            })
            .await
            .expect_err("active history sync must be retried");
        assert!(error.to_string().contains("must be retried"));
        let binding = provider
            .shared
            .session_binding(project.id, session_id)
            .expect("active binding remains");
        assert!(binding.prompt_in_flight);
        assert!(!binding.replaying);
        assert_eq!(binding.agent_item_id, live_item_id);

        provider.shared.handle_session_update(
            project.id,
            RawSessionNotification {
                session_id: SessionId::new(session_id),
                update: serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "still live"}
                }),
                meta: None,
            },
        );
        let event = events.try_recv().expect("live agent chunk");
        assert!(matches!(
            event.kind,
            ProviderEventKind::AgentTextDelta {
                provider_item_id,
                phase: AgentMessagePhase::Final,
                delta,
            } if provider_item_id == live_item_id && delta == "still live"
        ));
        assert!(matches!(
            events.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn duplex_permission_resolution_and_cancel_notification() {
        let provider = AcpProvider::new();
        let project_id = ProjectId::new();
        let conversation_id = ConversationId::new();
        let session_id = "duplex-session";
        provider.shared.bind_session(
            project_id,
            session_id,
            conversation_id,
            Some("grok-4.6".to_owned()),
            Some("high".to_owned()),
        );
        let mut events = provider.subscribe();

        let (client_transport, agent_transport) = Channel::duplex();
        let (client_ready_tx, client_ready_rx) = oneshot::channel();
        let (client_shutdown_tx, client_shutdown_rx) = oneshot::channel();
        let client_state = Arc::clone(&provider.shared);
        let client_task = tokio::spawn(async move {
            Client
                .builder()
                .on_receive_request(
                    async move |request: RequestPermissionRequest, responder, _connection| {
                        client_state.handle_permission_request(project_id, request, responder)
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_with(client_transport, async move |connection| {
                    connection
                        .send_request(InitializeRequest::new(ProtocolVersion::V1))
                        .block_task()
                        .await?;
                    let _ = client_ready_tx.send(connection.clone());
                    let _ = client_shutdown_rx.await;
                    Ok(())
                })
                .await
        });

        let (init_tx, init_rx) = oneshot::channel();
        let init_tx = Arc::new(StdMutex::new(Some(init_tx)));
        let init_handler_tx = Arc::clone(&init_tx);
        let (start_permission_tx, start_permission_rx) = oneshot::channel();
        let (permission_outcome_tx, permission_outcome_rx) = oneshot::channel();
        let (set_model_tx, set_model_rx) = oneshot::channel();
        let set_model_tx = Arc::new(StdMutex::new(Some(set_model_tx)));
        let set_model_handler_tx = Arc::clone(&set_model_tx);
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let cancel_tx = Arc::new(StdMutex::new(Some(cancel_tx)));
        let cancel_handler_tx = Arc::clone(&cancel_tx);
        let agent_task = tokio::spawn(async move {
            Agent
                .builder()
                .on_receive_request(
                    async move |request: InitializeRequest, responder, _connection| {
                        if let Some(tx) = init_handler_tx.lock().expect("init mutex").take() {
                            let _ = tx.send(());
                        }
                        responder.respond(
                            InitializeResponse::new(request.protocol_version).agent_capabilities(
                                AgentCapabilities::new()
                                    .load_session(true)
                                    .session_capabilities(
                                    agent_client_protocol::schema::v1::SessionCapabilities::new()
                                        .list(SessionListCapabilities::new())
                                        .resume(SessionResumeCapabilities::new()),
                                ),
                            ),
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_notification(
                    async move |_request: CancelNotification, _connection| {
                        if let Some(tx) = cancel_handler_tx.lock().expect("cancel mutex").take() {
                            let _ = tx.send(());
                        }
                        Ok(())
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                .on_receive_request(
                    async move |request: SetModelRequest, responder, _connection| {
                        if let Some(tx) =
                            set_model_handler_tx.lock().expect("set model mutex").take()
                        {
                            let _ = tx.send(request);
                        }
                        responder.respond(Value::Object(Default::default()))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_with(agent_transport, async move |connection| {
                    let _ = init_rx.await;
                    let _ = start_permission_rx.await;
                    let response = connection
                        .send_request(RequestPermissionRequest::new(
                            SessionId::new(session_id),
                            ToolCallUpdate::new(
                                "tool-1",
                                ToolCallUpdateFields::new().title("Run command"),
                            ),
                            vec![PermissionOption::new(
                                "allow-once",
                                "Allow once",
                                PermissionOptionKind::AllowOnce,
                            )],
                        ))
                        .block_task()
                        .await?;
                    let _ = permission_outcome_tx.send(response.outcome);
                    std::future::pending::<()>().await;
                    #[allow(unreachable_code)]
                    Ok(())
                })
                .await
        });

        let connection = client_ready_rx.await.expect("duplex client initialized");
        provider.shared.connections.lock().await.insert(
            project_id,
            Arc::new(ProjectConnection {
                connection,
                negotiated: negotiated(),
                shutdown: StdMutex::new(Some(client_shutdown_tx)),
            }),
        );
        start_permission_tx.send(()).expect("start permission");

        let approval = events.recv().await.expect("approval event");
        let provider_request_id = match approval.kind {
            ProviderEventKind::Approval {
                provider_request_id,
                options,
                ..
            } => {
                assert_eq!(options[0].id, "allow-once");
                provider_request_id
            }
            other => panic!("unexpected event: {other:?}"),
        };
        provider
            .resolve_approval(ResolveApproval {
                conversation_id,
                provider_request_id,
                option_id: "allow-once".to_owned(),
            })
            .await
            .expect("resolve permission");
        assert!(matches!(
            permission_outcome_rx.await.expect("permission result"),
            RequestPermissionOutcome::Selected(_)
        ));

        provider
            .set_session_option(SetSessionOption {
                conversation_id,
                native_session_id: session_id.to_owned(),
                option_id: "reasoning_effort".to_owned(),
                value: "medium".to_owned(),
            })
            .await
            .expect("set reasoning effort");
        let set_model = set_model_rx.await.expect("legacy set_model request");
        assert_eq!(set_model.model_id, "grok-4.6");
        assert_eq!(
            set_model.meta.get("reasoningEffort"),
            Some(&Value::String("medium".to_owned()))
        );

        provider
            .interrupt(InterruptSession {
                conversation_id,
                native_session_id: session_id.to_owned(),
            })
            .await
            .expect("send cancel notification");
        cancel_rx.await.expect("agent received cancel");

        provider.shared.connections.lock().await.remove(&project_id);
        let _ = client_task.await;
        agent_task.abort();
    }
}

impl Shared {
    fn append_replay_text(
        &self,
        project_id: ProjectId,
        session_id: &str,
        kind: ReplayTextKind,
        delta: &str,
    ) {
        let completed = {
            let mut sessions = self.sessions.write().expect("ACP session map poisoned");
            let Some(binding) = sessions.get_mut(&(project_id, session_id.to_owned())) else {
                return;
            };
            let mut completed = Vec::new();
            if matches!(kind, ReplayTextKind::User)
                && (binding.turn_index == 0 || binding.replay_has_response)
            {
                if binding.turn_index > 0 {
                    completed = drain_replay_items(binding);
                }
                start_replay_turn(binding, session_id);
            } else if binding.turn_index == 0 {
                start_replay_turn(binding, session_id);
            }
            match kind {
                ReplayTextKind::User => binding.replay_user_text.push_str(delta),
                ReplayTextKind::Agent => {
                    binding.replay_has_response = true;
                    binding.replay_agent_text.push_str(delta);
                }
                ReplayTextKind::Thought => {
                    binding.replay_has_response = true;
                    binding.replay_thought_text.push_str(delta);
                }
            }
            (binding.conversation_id, completed)
        };
        self.emit_replay_items(project_id, completed.0, completed.1);
    }

    fn emit_replay_items(
        &self,
        project_id: ProjectId,
        conversation_id: ConversationId,
        items: Vec<(String, TimelineItemKind)>,
    ) {
        for (provider_item_id, kind) in items {
            self.emit(
                project_id,
                conversation_id,
                ProviderEventKind::HistoryItem {
                    provider_item_id,
                    kind,
                },
            );
        }
    }

    fn handle_session_update(&self, project_id: ProjectId, notification: RawSessionNotification) {
        let session_id = notification.session_id.to_string();
        let Some(binding) = self
            .sessions
            .read()
            .expect("ACP session map poisoned")
            .get(&(project_id, session_id.clone()))
            .cloned()
        else {
            tracing::debug!(%session_id, provider = %self.config.provider, "ignored update for an unbound ACP session");
            return;
        };
        let Ok(update) = serde_json::from_value::<SessionUpdate>(notification.update) else {
            // ACP enums are non-exhaustive in Rust but serde rejects a future discriminator.
            // Keeping the outer notification raw lets this adapter safely ignore it.
            tracing::debug!(%session_id, provider = %self.config.provider, "ignored unknown ACP session/update variant");
            return;
        };

        match update {
            SessionUpdate::AgentMessageChunk(chunk) if binding.replaying => match chunk.content {
                ContentBlock::Text(text) => self.append_replay_text(
                    project_id,
                    &session_id,
                    ReplayTextKind::Agent,
                    &text.text,
                ),
                content => self.emit_replay_content(
                    project_id,
                    &session_id,
                    binding.conversation_id,
                    AgentMessagePhase::Final,
                    &content,
                    &format!("{} image", self.config.display_name),
                ),
            },
            SessionUpdate::AgentMessageChunk(chunk) => self.emit_content(
                project_id,
                binding.conversation_id,
                &binding.agent_item_id,
                AgentMessagePhase::Final,
                &chunk.content,
                &format!("{} image", self.config.display_name),
            ),
            SessionUpdate::AgentThoughtChunk(chunk) if binding.replaying => match chunk.content {
                ContentBlock::Text(text) => self.append_replay_text(
                    project_id,
                    &session_id,
                    ReplayTextKind::Thought,
                    &text.text,
                ),
                content => self.emit_replay_content(
                    project_id,
                    &session_id,
                    binding.conversation_id,
                    AgentMessagePhase::ReasoningSummary,
                    &content,
                    &format!("{} reasoning image", self.config.display_name),
                ),
            },
            SessionUpdate::AgentThoughtChunk(chunk) => self.emit_content(
                project_id,
                binding.conversation_id,
                &binding.thought_item_id,
                AgentMessagePhase::ReasoningSummary,
                &chunk.content,
                &format!("{} reasoning image", self.config.display_name),
            ),
            SessionUpdate::ToolCall(tool_call) => {
                self.record_tool_call(project_id, &session_id, binding.conversation_id, tool_call)
            }
            SessionUpdate::ToolCallUpdate(update) => {
                self.update_tool_call(project_id, &session_id, binding.conversation_id, update)
            }
            SessionUpdate::Plan(plan) => self.emit(
                project_id,
                binding.conversation_id,
                ProviderEventKind::Plan {
                    provider_item_id: format!("plan:{session_id}:{}", binding.turn_index),
                    steps: plan
                        .entries
                        .into_iter()
                        .map(|entry| PlanStep {
                            text: entry.content,
                            status: match entry.status {
                                PlanEntryStatus::Pending => ItemStatus::Pending,
                                PlanEntryStatus::InProgress => ItemStatus::Running,
                                PlanEntryStatus::Completed => ItemStatus::Completed,
                                _ => ItemStatus::Pending,
                            },
                        })
                        .collect(),
                },
            ),
            SessionUpdate::UserMessageChunk(chunk) if binding.replaying => {
                if let ContentBlock::Text(text) = chunk.content {
                    self.append_replay_text(
                        project_id,
                        &session_id,
                        ReplayTextKind::User,
                        &text.text,
                    );
                }
            }
            SessionUpdate::ConfigOptionUpdate(update) => {
                self.update_acp_options(project_id, &session_id, update.config_options);
                self.emit_session_options(project_id, &session_id);
            }
            SessionUpdate::CurrentModeUpdate(update) => {
                self.update_current_mode(
                    project_id,
                    &session_id,
                    &update.current_mode_id.to_string(),
                );
                self.emit_session_options(project_id, &session_id);
            }
            SessionUpdate::UserMessageChunk(_)
            | SessionUpdate::AvailableCommandsUpdate(_)
            | SessionUpdate::SessionInfoUpdate(_)
            | SessionUpdate::UsageUpdate(_) => {}
            _ => {}
        }
    }

    fn emit_replay_content(
        &self,
        project_id: ProjectId,
        session_id: &str,
        conversation_id: ConversationId,
        phase: AgentMessagePhase,
        content: &ContentBlock,
        image_alt: &str,
    ) {
        let provider_item_id = {
            let mut sessions = self.sessions.write().expect("ACP session map poisoned");
            sessions
                .get_mut(&(project_id, session_id.to_owned()))
                .map(|binding| {
                    binding.replay_image_index += 1;
                    format!("history:{session_id}:image:{}", binding.replay_image_index)
                })
        };
        self.emit_content_with_image_id(
            project_id,
            conversation_id,
            "replay-content",
            phase,
            content,
            image_alt,
            provider_item_id.as_deref(),
        );
    }

    fn emit_content(
        &self,
        project_id: ProjectId,
        conversation_id: ConversationId,
        provider_item_id: &str,
        phase: AgentMessagePhase,
        content: &ContentBlock,
        image_alt: &str,
    ) {
        self.emit_content_with_image_id(
            project_id,
            conversation_id,
            provider_item_id,
            phase,
            content,
            image_alt,
            None,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_content_with_image_id(
        &self,
        project_id: ProjectId,
        conversation_id: ConversationId,
        provider_item_id: &str,
        phase: AgentMessagePhase,
        content: &ContentBlock,
        image_alt: &str,
        image_provider_item_id: Option<&str>,
    ) {
        match content {
            ContentBlock::Text(text) => self.emit(
                project_id,
                conversation_id,
                ProviderEventKind::AgentTextDelta {
                    provider_item_id: provider_item_id.to_owned(),
                    phase,
                    delta: text.text.clone(),
                },
            ),
            ContentBlock::Image(image) => match BASE64.decode(&image.data) {
                Ok(bytes) => self.emit(
                    project_id,
                    conversation_id,
                    ProviderEventKind::ImageBytes {
                        provider_item_id: image_provider_item_id.map(str::to_owned),
                        bytes,
                        mime_type: image.mime_type.clone(),
                        alt: image_alt.to_owned(),
                    },
                ),
                Err(error) => self.emit(
                    project_id,
                    conversation_id,
                    ProviderEventKind::Failed {
                        provider_item_id: None,
                        code: "grok_invalid_image".to_owned(),
                        message: error.to_string(),
                    },
                ),
            },
            ContentBlock::Resource(resource) => {
                if let Ok(value) = serde_json::to_value(resource)
                    && let (Some(blob), Some(mime_type)) = (
                        value.pointer("/resource/blob").and_then(Value::as_str),
                        value.pointer("/resource/mimeType").and_then(Value::as_str),
                    )
                    && mime_type.starts_with("image/")
                {
                    match BASE64.decode(blob) {
                        Ok(bytes) => self.emit(
                            project_id,
                            conversation_id,
                            ProviderEventKind::ImageBytes {
                                provider_item_id: image_provider_item_id.map(str::to_owned),
                                bytes,
                                mime_type: mime_type.to_owned(),
                                alt: image_alt.to_owned(),
                            },
                        ),
                        Err(error) => self.emit(
                            project_id,
                            conversation_id,
                            ProviderEventKind::Failed {
                                provider_item_id: None,
                                code: "grok_invalid_image".to_owned(),
                                message: error.to_string(),
                            },
                        ),
                    }
                }
            }
            ContentBlock::Audio(_) | ContentBlock::ResourceLink(_) => {}
            _ => {}
        }
    }

    fn record_tool_call(
        &self,
        project_id: ProjectId,
        session_id: &str,
        conversation_id: ConversationId,
        tool_call: ToolCall,
    ) {
        let tool_id = tool_call.tool_call_id.to_string();
        self.tool_calls
            .lock()
            .expect("ACP tool map poisoned")
            .insert(
                (project_id, session_id.to_owned(), tool_id),
                tool_call.clone(),
            );
        self.emit_tool_call(project_id, conversation_id, tool_call);
    }

    fn update_tool_call(
        &self,
        project_id: ProjectId,
        session_id: &str,
        conversation_id: ConversationId,
        update: agent_client_protocol::schema::v1::ToolCallUpdate,
    ) {
        let tool_id = update.tool_call_id.to_string();
        let tool_call = {
            let mut tools = self.tool_calls.lock().expect("ACP tool map poisoned");
            let tool = tools
                .entry((project_id, session_id.to_owned(), tool_id.clone()))
                .or_insert_with(|| {
                    ToolCall::new(tool_id, format!("{} tool", self.config.display_name))
                });
            tool.update(update.fields);
            tool.clone()
        };
        self.emit_tool_call(project_id, conversation_id, tool_call);
    }

    fn emit_tool_call(
        &self,
        project_id: ProjectId,
        conversation_id: ConversationId,
        tool_call: ToolCall,
    ) {
        let input_summary = tool_call.raw_input.as_ref().map(json_summary);
        let mut output_summary = tool_call.raw_output.as_ref().map(json_summary);
        if output_summary.is_none() {
            let text: Vec<_> = tool_call
                .content
                .iter()
                .filter_map(|content| match content {
                    ToolCallContent::Content(content) => match &content.content {
                        ContentBlock::Text(text) => Some(text.text.clone()),
                        _ => None,
                    },
                    _ => None,
                })
                .collect();
            if !text.is_empty() {
                output_summary = Some(text.join("\n"));
            }
        }
        self.emit(
            project_id,
            conversation_id,
            ProviderEventKind::ToolCall {
                provider_item_id: format!("tool:{}", tool_call.tool_call_id),
                name: tool_call.title.clone(),
                status: tool_status(tool_call.status),
                input_summary,
                output_summary,
            },
        );

        for content in &tool_call.content {
            match content {
                ToolCallContent::Content(content) => self.emit_content(
                    project_id,
                    conversation_id,
                    &format!("tool-content:{}", tool_call.tool_call_id),
                    AgentMessagePhase::Commentary,
                    &content.content,
                    &tool_call.title,
                ),
                ToolCallContent::Diff(diff) => self.emit(
                    project_id,
                    conversation_id,
                    ProviderEventKind::FileChange {
                        provider_item_id: format!(
                            "tool-diff:{}:{}",
                            tool_call.tool_call_id,
                            diff.path.display()
                        ),
                        relative_path: diff.path.display().to_string(),
                        change_kind: if diff.old_text.is_some() {
                            "modified".to_owned()
                        } else {
                            "created".to_owned()
                        },
                        status: tool_status(tool_call.status),
                    },
                ),
                ToolCallContent::Terminal(_) => {}
                _ => {}
            }
        }
    }
}
