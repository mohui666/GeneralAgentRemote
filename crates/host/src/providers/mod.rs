//! Typed boundary between the Host state authority and each provider protocol.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use agent_remote_protocol::{
    ApprovalOption, ConversationId, EffortOption, ItemStatus, ModelOption, PlanStep, ProjectId,
    ProviderHealth, ProviderId, SessionOption, SessionSummary,
};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::storage::Project;

pub mod codex;
pub mod grok;

#[derive(Debug, Clone)]
pub struct CreateSession {
    pub conversation_id: ConversationId,
    pub project: Project,
    pub model: Option<String>,
    pub effort: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResumeSession {
    pub conversation_id: ConversationId,
    pub project: Project,
    pub native_session_id: String,
    pub model: Option<String>,
    pub effort: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SendMessage {
    pub conversation_id: ConversationId,
    pub native_session_id: String,
    pub text: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SteerMessage {
    pub conversation_id: ConversationId,
    pub native_session_id: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct InterruptSession {
    pub conversation_id: ConversationId,
    pub native_session_id: String,
}

#[derive(Debug, Clone)]
pub struct ResolveApproval {
    pub conversation_id: ConversationId,
    pub provider_request_id: String,
    pub option_id: String,
}

#[derive(Debug, Clone)]
pub struct SetSessionOption {
    pub conversation_id: ConversationId,
    pub native_session_id: String,
    pub option_id: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeSession {
    pub native_session_id: String,
    pub title: String,
    pub selected_model: Option<String>,
    pub selected_effort: Option<String>,
    pub session_options: Vec<SessionOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub supports_session_list: bool,
    pub supports_resume: bool,
    pub supports_steer: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandAck;

#[derive(Debug, Clone)]
pub struct ProviderEvent {
    pub provider: ProviderId,
    pub project_id: ProjectId,
    pub conversation_id: ConversationId,
    pub kind: ProviderEventKind,
}

#[derive(Debug, Clone)]
pub enum ProviderEventKind {
    AgentTextDelta {
        provider_item_id: String,
        phase: agent_remote_protocol::AgentMessagePhase,
        delta: String,
    },
    Plan {
        provider_item_id: String,
        steps: Vec<PlanStep>,
    },
    ToolCall {
        provider_item_id: String,
        name: String,
        status: ItemStatus,
        input_summary: Option<String>,
        output_summary: Option<String>,
    },
    Command {
        provider_item_id: String,
        command: String,
        relative_cwd: Option<String>,
        status: ItemStatus,
        exit_code: Option<i32>,
        output: Option<String>,
    },
    FileChange {
        provider_item_id: String,
        relative_path: String,
        change_kind: String,
        status: ItemStatus,
    },
    Approval {
        provider_request_id: String,
        prompt: String,
        options: Vec<ApprovalOption>,
    },
    ImagePath {
        path: PathBuf,
        controlled_temp_roots: Vec<PathBuf>,
        alt: String,
    },
    ImageBytes {
        bytes: Vec<u8>,
        mime_type: String,
        alt: String,
    },
    Completed,
    Failed {
        code: String,
        message: String,
    },
    Interrupted,
    Crashed {
        message: String,
    },
}

#[async_trait]
pub trait AgentProvider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn capabilities(&self) -> ProviderCapabilities;
    fn subscribe(&self) -> broadcast::Receiver<ProviderEvent>;
    async fn health(&self) -> ProviderHealth;
    async fn list_models(&self, project: &Project) -> Result<Vec<ModelOption>>;
    async fn list_sessions(&self, project: &Project) -> Result<Vec<SessionSummary>>;
    async fn create_session(&self, request: CreateSession) -> Result<NativeSession>;
    async fn resume_session(&self, request: ResumeSession) -> Result<NativeSession>;
    async fn send_message(&self, request: SendMessage) -> Result<CommandAck>;
    async fn steer(&self, request: SteerMessage) -> Result<CommandAck>;
    async fn interrupt(&self, request: InterruptSession) -> Result<CommandAck>;
    async fn resolve_approval(&self, request: ResolveApproval) -> Result<CommandAck>;
    async fn set_session_option(&self, request: SetSessionOption) -> Result<CommandAck>;
}

#[derive(Clone, Default)]
pub struct ProviderRegistry {
    providers: HashMap<ProviderId, Arc<dyn AgentProvider>>,
}

impl ProviderRegistry {
    pub fn new(providers: impl IntoIterator<Item = Arc<dyn AgentProvider>>) -> Self {
        Self {
            providers: providers
                .into_iter()
                .map(|provider| (provider.id(), provider))
                .collect(),
        }
    }

    pub fn get(&self, id: ProviderId) -> Result<Arc<dyn AgentProvider>> {
        self.providers
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow!("provider {id} is not configured"))
    }

    pub fn all(&self) -> impl Iterator<Item = Arc<dyn AgentProvider>> + '_ {
        self.providers.values().cloned()
    }
}

pub fn effort_options(values: impl IntoIterator<Item = String>) -> Vec<EffortOption> {
    values
        .into_iter()
        .map(|value| EffortOption {
            display_name: value.clone(),
            id: value,
        })
        .collect()
}
