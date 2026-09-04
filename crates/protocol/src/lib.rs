use std::fmt;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;
use uuid::Uuid;

pub const PROTOCOL_VERSION: u16 = 3;
pub const RELAY_PROTOCOL_VERSION: u16 = 1;

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

uuid_id!(HostId);
uuid_id!(DeviceId);
uuid_id!(ProjectId);
uuid_id!(ConversationId);
uuid_id!(TimelineItemId);
uuid_id!(AttachmentId);
uuid_id!(CommandId);
uuid_id!(ApprovalId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRisk {
    Standard,
    Elevated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionModeOption {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub risk: PermissionRisk,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AttachmentCapability {
    pub allowed_mime_types: Vec<String>,
    pub max_count: u16,
    pub max_bytes: u64,
    pub max_total_bytes: u64,
}

impl AttachmentCapability {
    pub fn supported(&self) -> bool {
        self.max_count > 0
            && self.max_bytes > 0
            && self.max_total_bytes > 0
            && !self.allowed_mime_types.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientAttachment {
    pub id: AttachmentId,
    pub file_name: String,
    pub mime_type: String,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Codex,
    Grok,
    ClaudeCode,
    GeminiCli,
    CopilotCli,
    OpenCode,
    Cursor,
    Cline,
    Goose,
    Junie,
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codex => formatter.write_str("Codex"),
            Self::Grok => formatter.write_str("Grok"),
            Self::ClaudeCode => formatter.write_str("Claude Code"),
            Self::GeminiCli => formatter.write_str("Gemini CLI"),
            Self::CopilotCli => formatter.write_str("GitHub Copilot"),
            Self::OpenCode => formatter.write_str("OpenCode"),
            Self::Cursor => formatter.write_str("Cursor Agent"),
            Self::Cline => formatter.write_str("Cline"),
            Self::Goose => formatter.write_str("Goose"),
            Self::Junie => formatter.write_str("JetBrains Junie"),
        }
    }
}

impl ProviderId {
    pub const ALL: [Self; 10] = [
        Self::Codex,
        Self::Grok,
        Self::ClaudeCode,
        Self::GeminiCli,
        Self::CopilotCli,
        Self::OpenCode,
        Self::Cursor,
        Self::Cline,
        Self::Goose,
        Self::Junie,
    ];

    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::ClaudeCode => "claude_code",
            Self::GeminiCli => "gemini_cli",
            Self::CopilotCli => "copilot_cli",
            Self::OpenCode => "open_code",
            Self::Cursor => "cursor",
            Self::Cline => "cline",
            Self::Goose => "goose",
            Self::Junie => "junie",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderState {
    NotInstalled,
    NotAuthenticated,
    Starting,
    Ready,
    Crashed,
    ProtocolIncompatible,
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHealth {
    pub provider: ProviderId,
    pub state: ProviderState,
    pub version: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffortOption {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelOption {
    pub id: String,
    pub display_name: String,
    pub effort_options: Vec<EffortOption>,
    pub default_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionOption {
    pub id: String,
    pub display_name: String,
    pub category: Option<String>,
    pub current_value: String,
    pub values: Vec<SessionOptionValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionOptionValue {
    pub value: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: ProjectId,
    pub display_name: String,
    pub short_path: String,
    pub enabled_providers: Vec<ProviderId>,
    pub valid: bool,
    pub last_activity_at_ms: Option<i64>,
    pub conversation_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub native_session_id: String,
    pub title: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationState {
    Idle,
    Running,
    NeedsApproval,
    Completed,
    Failed,
    Interrupted,
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConversationTitleSource {
    #[default]
    Fallback,
    Generated,
    Provider,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendTraceStage {
    HostReceived,
    ProviderReceived,
    FirstProviderEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: ConversationId,
    pub revision: u64,
    pub provider: ProviderId,
    pub project_id: ProjectId,
    pub native_session_id: String,
    pub title: String,
    #[serde(default)]
    pub title_source: ConversationTitleSource,
    #[serde(default)]
    pub title_updated_at_ms: i64,
    pub selected_model: Option<String>,
    pub selected_effort: Option<String>,
    pub state: ConversationState,
    pub session_options: Vec<SessionOption>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessagePhase {
    Commentary,
    ReasoningSummary,
    Final,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Declined,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressKind {
    Command,
    Tool,
    WebSearch,
    Test,
    File,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub text: String,
    pub status: ItemStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalOption {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineItemKind {
    UserMessage {
        text: String,
    },
    AgentMessage {
        phase: AgentMessagePhase,
        text: String,
    },
    Progress {
        kind: ProgressKind,
        label: String,
        status: ItemStatus,
        detail: Option<String>,
    },
    Plan {
        steps: Vec<PlanStep>,
    },
    ToolCall {
        name: String,
        status: ItemStatus,
        input_summary: Option<String>,
        output_summary: Option<String>,
    },
    Command {
        command: String,
        relative_cwd: Option<String>,
        status: ItemStatus,
        exit_code: Option<i32>,
        output: Option<String>,
    },
    FileChange {
        relative_path: String,
        change_kind: String,
        status: ItemStatus,
    },
    Approval {
        approval_id: ApprovalId,
        prompt: String,
        options: Vec<ApprovalOption>,
        resolved_option: Option<String>,
    },
    Image {
        attachment_id: AttachmentId,
        alt: String,
    },
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineItem {
    pub id: TimelineItemId,
    pub conversation_id: ConversationId,
    pub revision: u64,
    pub created_at_ms: i64,
    pub kind: TimelineItemKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelinePageCursor {
    pub created_at_ms: i64,
    pub item_id: TimelineItemId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentMetadata {
    pub id: AttachmentId,
    pub conversation_id: ConversationId,
    pub mime_type: String,
    pub byte_len: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapability {
    pub provider: ProviderId,
    pub project_id: ProjectId,
    pub health: ProviderHealth,
    pub models: Vec<ModelOption>,
    pub supports_session_list: bool,
    pub supports_history: bool,
    pub supports_incremental_sync: bool,
    pub supports_rename: bool,
    pub supports_steer: bool,
    pub permission_modes: Vec<PermissionModeOption>,
    pub default_permission_mode: Option<String>,
    pub attachments: AttachmentCapability,
    pub sessions: Vec<SessionSummary>,
    pub limitation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub host_id: HostId,
    pub host_name: String,
    pub projects: Vec<ProjectSummary>,
    pub provider_capabilities: Vec<ProviderCapability>,
    pub conversations: Vec<Conversation>,
    pub timeline: Vec<TimelineItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientCommand {
    Pair {
        host_id: HostId,
        pair_token: String,
        device_name: String,
    },
    Authenticate {
        host_id: HostId,
        device_id: DeviceId,
        device_token: String,
    },
    GetSnapshot,
    RefreshProjects {
        provider: ProviderId,
    },
    SyncProject {
        command_id: CommandId,
        project_id: ProjectId,
        provider: ProviderId,
    },
    CreateConversation {
        command_id: CommandId,
        project_id: ProjectId,
        provider: ProviderId,
        native_session_id: Option<String>,
        model: Option<String>,
        effort: Option<String>,
    },
    StartConversation {
        command_id: CommandId,
        #[serde(default)]
        attempt: u32,
        conversation_id: ConversationId,
        project_id: ProjectId,
        provider: ProviderId,
        model: Option<String>,
        effort: Option<String>,
        permission_mode: Option<String>,
        #[serde(default)]
        client_message_id: Option<String>,
        text: String,
        #[serde(default)]
        attachments: Vec<ClientAttachment>,
    },
    SendMessage {
        command_id: CommandId,
        #[serde(default)]
        attempt: u32,
        conversation_id: ConversationId,
        #[serde(default)]
        client_message_id: Option<String>,
        text: String,
        #[serde(default)]
        attachments: Vec<ClientAttachment>,
    },
    Steer {
        command_id: CommandId,
        conversation_id: ConversationId,
        text: String,
    },
    Interrupt {
        command_id: CommandId,
        conversation_id: ConversationId,
    },
    ResolveApproval {
        command_id: CommandId,
        approval_id: ApprovalId,
        option_id: String,
    },
    SetSessionOption {
        command_id: CommandId,
        conversation_id: ConversationId,
        option_id: String,
        value: String,
    },
    RenameConversation {
        command_id: CommandId,
        conversation_id: ConversationId,
        title: String,
    },
    GetConversationPage {
        conversation_id: ConversationId,
        before: Option<TimelinePageCursor>,
        limit: u32,
    },
    GetAttachment {
        attachment_id: AttachmentId,
    },
}

impl ClientCommand {
    pub fn command_id(&self) -> Option<CommandId> {
        match self {
            Self::CreateConversation { command_id, .. }
            | Self::StartConversation { command_id, .. }
            | Self::SyncProject { command_id, .. }
            | Self::SendMessage { command_id, .. }
            | Self::Steer { command_id, .. }
            | Self::Interrupt { command_id, .. }
            | Self::ResolveApproval { command_id, .. }
            | Self::SetSessionOption { command_id, .. }
            | Self::RenameConversation { command_id, .. } => Some(*command_id),
            Self::Pair { .. }
            | Self::Authenticate { .. }
            | Self::GetSnapshot
            | Self::RefreshProjects { .. }
            | Self::GetConversationPage { .. }
            | Self::GetAttachment { .. } => None,
        }
    }

    pub fn attempt(&self) -> u32 {
        match self {
            Self::StartConversation { attempt, .. } | Self::SendMessage { attempt, .. } => *attempt,
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Paired {
        host_id: HostId,
        device_id: DeviceId,
        device_token: String,
    },
    Authenticated {
        host_id: HostId,
        device_id: DeviceId,
    },
    Snapshot {
        snapshot: Snapshot,
    },
    ProjectsUpdated {
        provider: ProviderId,
        projects: Vec<ProjectSummary>,
        capabilities: Vec<ProviderCapability>,
    },
    ProjectSyncCompleted {
        command_id: CommandId,
        project_id: ProjectId,
        provider: ProviderId,
        conversations_synced: u32,
        full_history_fallback: bool,
    },
    ConversationPage {
        conversation_id: ConversationId,
        items: Vec<TimelineItem>,
        next_before: Option<TimelinePageCursor>,
    },
    ProviderChanged {
        capability: ProviderCapability,
    },
    ConversationUpserted {
        conversation: Conversation,
    },
    TimelineItemUpserted {
        item: TimelineItem,
    },
    ConversationRemoved {
        conversation_id: ConversationId,
    },
    AttachmentData {
        metadata: AttachmentMetadata,
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    HostStatus {
        host_id: HostId,
        online: bool,
        message: Option<String>,
    },
    SendTrace {
        command_id: CommandId,
        client_message_id: String,
        conversation_id: ConversationId,
        stage: SendTraceStage,
        elapsed_ms: u64,
    },
    CommandAccepted {
        command_id: CommandId,
    },
    CommandRejected {
        command_id: Option<CommandId>,
        code: String,
        message: String,
    },
    ProtocolError {
        supported_version: u16,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelayFrame {
    RegisterHost {
        host_id: HostId,
        access_token: String,
    },
    HostRegistered {
        host_id: HostId,
    },
    OpenClient {
        host_id: HostId,
        connection_id: Uuid,
    },
    ClientOpened {
        connection_id: Uuid,
    },
    Payload {
        connection_id: Uuid,
        #[serde(with = "serde_bytes")]
        payload: Vec<u8>,
    },
    Close {
        connection_id: Uuid,
        reason: String,
    },
    HostAvailability {
        host_id: HostId,
        online: bool,
    },
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub protocol_version: u16,
    pub message: T,
}

impl<T> Envelope<T> {
    pub fn new(message: T) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            message,
        }
    }
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("CBOR encode failed: {0}")]
    Encode(String),
    #[error("CBOR decode failed: {0}")]
    Decode(String),
    #[error("unsupported protocol version {received}; this build supports {supported}")]
    Version { received: u16, supported: u16 },
}

pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, ProtocolError> {
    encode_with_version(message, PROTOCOL_VERSION)
}

pub fn encode_relay(message: &RelayFrame) -> Result<Vec<u8>, ProtocolError> {
    encode_with_version(message, RELAY_PROTOCOL_VERSION)
}

fn encode_with_version<T: Serialize>(
    message: &T,
    protocol_version: u16,
) -> Result<Vec<u8>, ProtocolError> {
    let envelope = Envelope {
        protocol_version,
        message,
    };
    let mut bytes = Vec::new();
    ciborium::into_writer(&envelope, &mut bytes)
        .map_err(|error| ProtocolError::Encode(error.to_string()))?;
    Ok(bytes)
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ProtocolError> {
    decode_with_version(bytes, PROTOCOL_VERSION)
}

pub fn decode_relay(bytes: &[u8]) -> Result<RelayFrame, ProtocolError> {
    decode_with_version(bytes, RELAY_PROTOCOL_VERSION)
}

fn decode_with_version<T: DeserializeOwned>(
    bytes: &[u8],
    protocol_version: u16,
) -> Result<T, ProtocolError> {
    let envelope: Envelope<T> =
        ciborium::from_reader(bytes).map_err(|error| ProtocolError::Decode(error.to_string()))?;
    if envelope.protocol_version != protocol_version {
        return Err(ProtocolError::Version {
            received: envelope.protocol_version,
            supported: protocol_version,
        });
    }
    Ok(envelope.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_command_round_trips_as_versioned_cbor() {
        let command = ClientCommand::SendMessage {
            command_id: CommandId::new(),
            attempt: 0,
            conversation_id: ConversationId::new(),
            client_message_id: Some("client-message-1".to_owned()),
            text: "hello".to_owned(),
            attachments: Vec::new(),
        };
        let bytes = encode(&command).expect("encode command");
        let decoded: ClientCommand = decode(&bytes).expect("decode command");
        assert_eq!(decoded, command);
    }

    #[test]
    fn unsupported_version_is_explicit() {
        let envelope = Envelope {
            protocol_version: PROTOCOL_VERSION + 1,
            message: ClientCommand::GetSnapshot,
        };
        let mut bytes = Vec::new();
        ciborium::into_writer(&envelope, &mut bytes).expect("encode envelope");
        let error = decode::<ClientCommand>(&bytes).expect_err("version must be rejected");
        assert!(matches!(error, ProtocolError::Version { .. }));
    }

    #[test]
    fn relay_frames_keep_the_stable_outer_protocol_version() {
        let frame = RelayFrame::HostAvailability {
            host_id: HostId::new(),
            online: true,
        };
        let bytes = encode_relay(&frame).expect("encode relay frame");
        let envelope: Envelope<RelayFrame> =
            ciborium::from_reader(bytes.as_slice()).expect("decode relay envelope");
        assert_eq!(envelope.protocol_version, RELAY_PROTOCOL_VERSION);
        assert_eq!(decode_relay(&bytes).expect("decode relay frame"), frame);
        assert!(matches!(
            decode::<RelayFrame>(&bytes),
            Err(ProtocolError::Version {
                received: RELAY_PROTOCOL_VERSION,
                supported: PROTOCOL_VERSION,
            })
        ));
    }
}
