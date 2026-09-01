use std::fmt;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;
use uuid::Uuid;

pub const PROTOCOL_VERSION: u16 = 1;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Codex,
    Grok,
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codex => formatter.write_str("Codex"),
            Self::Grok => formatter.write_str("Grok"),
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
    pub enabled_providers: Vec<ProviderId>,
    pub valid: bool,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: ConversationId,
    pub revision: u64,
    pub provider: ProviderId,
    pub project_id: ProjectId,
    pub native_session_id: String,
    pub title: String,
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
    pub supports_steer: bool,
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
    CreateConversation {
        command_id: CommandId,
        project_id: ProjectId,
        provider: ProviderId,
        native_session_id: Option<String>,
        model: Option<String>,
        effort: Option<String>,
    },
    SendMessage {
        command_id: CommandId,
        conversation_id: ConversationId,
        text: String,
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
    GetAttachment {
        attachment_id: AttachmentId,
    },
}

impl ClientCommand {
    pub fn command_id(&self) -> Option<CommandId> {
        match self {
            Self::CreateConversation { command_id, .. }
            | Self::SendMessage { command_id, .. }
            | Self::Steer { command_id, .. }
            | Self::Interrupt { command_id, .. }
            | Self::ResolveApproval { command_id, .. }
            | Self::SetSessionOption { command_id, .. } => Some(*command_id),
            Self::Pair { .. }
            | Self::Authenticate { .. }
            | Self::GetSnapshot
            | Self::GetAttachment { .. } => None,
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
    let envelope = Envelope::new(message);
    let mut bytes = Vec::new();
    ciborium::into_writer(&envelope, &mut bytes)
        .map_err(|error| ProtocolError::Encode(error.to_string()))?;
    Ok(bytes)
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ProtocolError> {
    let envelope: Envelope<T> =
        ciborium::from_reader(bytes).map_err(|error| ProtocolError::Decode(error.to_string()))?;
    if envelope.protocol_version != PROTOCOL_VERSION {
        return Err(ProtocolError::Version {
            received: envelope.protocol_version,
            supported: PROTOCOL_VERSION,
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
            conversation_id: ConversationId::new(),
            text: "hello".to_owned(),
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
}
