use std::{
    collections::HashMap,
    fs,
    path::{Component, Path},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use agent_remote_protocol::{
    ApprovalId, AttachmentId, ClientCommand, Conversation, ConversationId, ConversationState,
    DeviceId, HostId, ProjectId, ProviderCapability, ProviderId, ServerMessage, Snapshot,
    TimelineItem, TimelineItemId, TimelineItemKind,
};
use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::broadcast;

use crate::{
    attachments::AttachmentStore,
    providers::{
        CreateSession, InterruptSession, ProviderEvent, ProviderEventKind, ProviderRegistry,
        ResolveApproval, ResumeSession, SendMessage, SetSessionOption, SteerMessage,
    },
    storage::{IssuedDevice, Project, Storage, now_ms},
};

#[derive(Debug, Clone)]
struct PendingApproval {
    provider: ProviderId,
    conversation_id: ConversationId,
    provider_request_id: String,
}

pub struct AppService {
    storage: Arc<Storage>,
    attachments: AttachmentStore,
    providers: ProviderRegistry,
    host_id: HostId,
    host_name: String,
    updates: broadcast::Sender<ServerMessage>,
    timeline_cache: Mutex<HashMap<TimelineItemId, TimelineItem>>,
    provider_item_ids: Mutex<HashMap<(ConversationId, String), TimelineItemId>>,
    approvals: Mutex<HashMap<ApprovalId, PendingApproval>>,
    provider_approval_ids: Mutex<HashMap<(ConversationId, String), ApprovalId>>,
    event_pumps_started: AtomicBool,
}

impl AppService {
    pub fn new(
        storage: Arc<Storage>,
        attachments: AttachmentStore,
        providers: ProviderRegistry,
        host_name: String,
    ) -> Result<Arc<Self>> {
        let host_id = storage.host_id()?;
        let timeline_cache = storage
            .list_timeline()?
            .into_iter()
            .map(|item| (item.id, item))
            .collect();
        let (updates, _) = broadcast::channel(256);
        Ok(Arc::new(Self {
            storage,
            attachments,
            providers,
            host_id,
            host_name,
            updates,
            timeline_cache: Mutex::new(timeline_cache),
            provider_item_ids: Mutex::new(HashMap::new()),
            approvals: Mutex::new(HashMap::new()),
            provider_approval_ids: Mutex::new(HashMap::new()),
            event_pumps_started: AtomicBool::new(false),
        }))
    }

    pub fn host_id(&self) -> HostId {
        self.host_id
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerMessage> {
        self.updates.subscribe()
    }

    pub fn start_provider_event_pumps(self: &Arc<Self>) {
        if self.event_pumps_started.swap(true, Ordering::AcqRel) {
            return;
        }
        for provider in self.providers.all() {
            let mut events = provider.subscribe();
            let service = Arc::clone(self);
            tokio::spawn(async move {
                loop {
                    match events.recv().await {
                        Ok(event) => service.apply_provider_event(event).await,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }
    }

    pub async fn snapshot(&self) -> Result<Snapshot> {
        let projects = self.storage.list_projects()?;
        let mut provider_capabilities = Vec::new();
        for project in &projects {
            for provider_id in &project.enabled_providers {
                let provider = match self.providers.get(*provider_id) {
                    Ok(provider) => provider,
                    Err(error) => {
                        provider_capabilities.push(ProviderCapability {
                            provider: *provider_id,
                            project_id: project.id,
                            health: agent_remote_protocol::ProviderHealth {
                                provider: *provider_id,
                                state: agent_remote_protocol::ProviderState::Offline,
                                version: None,
                                detail: Some(error.to_string()),
                            },
                            models: Vec::new(),
                            supports_session_list: false,
                            supports_steer: false,
                            sessions: Vec::new(),
                            limitation: Some("Provider adapter is unavailable".to_owned()),
                        });
                        continue;
                    }
                };
                let health = provider.health().await;
                let models_result = provider.list_models(project).await;
                let sessions_result = provider.list_sessions(project).await;
                let limitation = models_result
                    .as_ref()
                    .err()
                    .map(ToString::to_string)
                    .or_else(|| sessions_result.as_ref().err().map(ToString::to_string));
                let capabilities = provider.capabilities();
                provider_capabilities.push(ProviderCapability {
                    provider: *provider_id,
                    project_id: project.id,
                    health,
                    models: models_result.unwrap_or_default(),
                    supports_session_list: capabilities.supports_session_list,
                    supports_steer: capabilities.supports_steer,
                    sessions: sessions_result.unwrap_or_default(),
                    limitation,
                });
            }
        }
        Ok(Snapshot {
            host_id: self.host_id,
            host_name: self.host_name.clone(),
            projects: projects.iter().map(Project::summary).collect(),
            provider_capabilities,
            conversations: self.storage.list_conversations()?,
            timeline: self.storage.list_timeline()?,
        })
    }

    pub fn exchange_pairing_token(&self, token: &str, device_name: &str) -> Result<IssuedDevice> {
        self.storage.exchange_pairing_token(token, device_name)
    }

    pub fn authenticate_device(&self, device_id: DeviceId, token: &str) -> Result<bool> {
        self.storage.authenticate_device(device_id, token)
    }

    pub fn attachment_data(&self, id: AttachmentId) -> Result<ServerMessage> {
        let attachment = self.storage.attachment(id)?;
        let bytes = self.attachments.read(&attachment)?;
        Ok(ServerMessage::AttachmentData {
            metadata: attachment.metadata,
            bytes,
        })
    }

    pub async fn execute_command(
        &self,
        device_id: DeviceId,
        command: ClientCommand,
    ) -> Result<ServerMessage> {
        if let Some(command_id) = command.command_id()
            && !self.storage.begin_command(device_id, command_id)?
        {
            return Ok(ServerMessage::CommandAccepted { command_id });
        }
        let command_id = command.command_id();
        match command {
            ClientCommand::GetSnapshot => Ok(ServerMessage::Snapshot {
                snapshot: self.snapshot().await?,
            }),
            ClientCommand::CreateConversation {
                command_id,
                project_id,
                provider,
                native_session_id,
                model,
                effort,
            } => {
                self.create_conversation(project_id, provider, native_session_id, model, effort)
                    .await?;
                Ok(ServerMessage::CommandAccepted { command_id })
            }
            ClientCommand::SendMessage {
                command_id,
                conversation_id,
                text,
            } => {
                self.send_message(conversation_id, text).await?;
                Ok(ServerMessage::CommandAccepted { command_id })
            }
            ClientCommand::Steer {
                command_id,
                conversation_id,
                text,
            } => {
                self.steer(conversation_id, text).await?;
                Ok(ServerMessage::CommandAccepted { command_id })
            }
            ClientCommand::Interrupt {
                command_id,
                conversation_id,
            } => {
                self.interrupt(conversation_id).await?;
                Ok(ServerMessage::CommandAccepted { command_id })
            }
            ClientCommand::ResolveApproval {
                command_id,
                approval_id,
                option_id,
            } => {
                self.resolve_approval(approval_id, option_id).await?;
                Ok(ServerMessage::CommandAccepted { command_id })
            }
            ClientCommand::SetSessionOption {
                command_id,
                conversation_id,
                option_id,
                value,
            } => {
                self.set_session_option(conversation_id, option_id, value)
                    .await?;
                Ok(ServerMessage::CommandAccepted { command_id })
            }
            ClientCommand::GetAttachment { attachment_id } => self.attachment_data(attachment_id),
            ClientCommand::Pair { .. } | ClientCommand::Authenticate { .. } => {
                bail!("authentication commands are only valid as the first WebSocket message")
            }
        }
        .with_context(|| {
            command_id
                .map(|id| format!("execute command {id}"))
                .unwrap_or_else(|| "execute command".to_owned())
        })
    }

    async fn create_conversation(
        &self,
        project_id: ProjectId,
        provider_id: ProviderId,
        native_session_id: Option<String>,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<Conversation> {
        let project = self.authorized_project(project_id, provider_id)?;
        let provider = self.providers.get(provider_id)?;
        validate_model_selection(
            &provider.list_models(&project).await?,
            model.as_deref(),
            effort.as_deref(),
        )?;
        let conversation_id = ConversationId::new();
        let native = if let Some(native_session_id) = native_session_id {
            if !provider.capabilities().supports_resume {
                bail!("{provider_id} does not support resuming sessions in this version");
            }
            provider
                .resume_session(ResumeSession {
                    conversation_id,
                    project: project.clone(),
                    native_session_id,
                    model: model.clone(),
                    effort: effort.clone(),
                })
                .await?
        } else {
            provider
                .create_session(CreateSession {
                    conversation_id,
                    project: project.clone(),
                    model: model.clone(),
                    effort: effort.clone(),
                })
                .await?
        };
        let conversation = Conversation {
            id: conversation_id,
            revision: 1,
            provider: provider_id,
            project_id,
            native_session_id: native.native_session_id,
            title: native.title,
            selected_model: native.selected_model.or(model),
            selected_effort: native.selected_effort.or(effort),
            state: ConversationState::Idle,
            session_options: native.session_options,
            updated_at_ms: now_ms(),
        };
        self.storage.upsert_conversation(&conversation)?;
        self.emit(ServerMessage::ConversationUpserted {
            conversation: conversation.clone(),
        });
        Ok(conversation)
    }

    async fn send_message(&self, conversation_id: ConversationId, text: String) -> Result<()> {
        if text.trim().is_empty() {
            bail!("message text cannot be empty");
        }
        let mut conversation = self.storage.conversation(conversation_id)?;
        let project = self.authorized_project(conversation.project_id, conversation.provider)?;
        let provider = self.providers.get(conversation.provider)?;
        let user_item = TimelineItem {
            id: TimelineItemId::new(),
            conversation_id,
            revision: 1,
            created_at_ms: now_ms(),
            kind: TimelineItemKind::UserMessage { text: text.clone() },
        };
        self.save_and_emit_item(user_item)?;
        conversation.state = ConversationState::Running;
        conversation.revision += 1;
        conversation.updated_at_ms = now_ms();
        self.save_and_emit_conversation(&conversation)?;
        if let Err(error) = provider
            .send_message(SendMessage {
                conversation_id,
                native_session_id: conversation.native_session_id.clone(),
                text,
                model: conversation.selected_model.clone(),
                effort: conversation.selected_effort.clone(),
            })
            .await
        {
            self.record_failure(&project, conversation, "provider_error", error.to_string())?;
            return Err(error);
        }
        Ok(())
    }

    async fn steer(&self, conversation_id: ConversationId, text: String) -> Result<()> {
        let conversation = self.storage.conversation(conversation_id)?;
        let provider = self.providers.get(conversation.provider)?;
        if !provider.capabilities().supports_steer {
            bail!(
                "{} does not support steering the running turn",
                conversation.provider
            );
        }
        provider
            .steer(SteerMessage {
                conversation_id,
                native_session_id: conversation.native_session_id,
                text,
            })
            .await?;
        Ok(())
    }

    async fn interrupt(&self, conversation_id: ConversationId) -> Result<()> {
        let conversation = self.storage.conversation(conversation_id)?;
        let provider = self.providers.get(conversation.provider)?;
        provider
            .interrupt(InterruptSession {
                conversation_id,
                native_session_id: conversation.native_session_id,
            })
            .await?;
        Ok(())
    }

    async fn resolve_approval(&self, approval_id: ApprovalId, option_id: String) -> Result<()> {
        let pending = self
            .approvals
            .lock()
            .expect("approval mutex poisoned")
            .get(&approval_id)
            .cloned()
            .ok_or_else(|| anyhow!("approval {approval_id} is no longer pending"))?;
        let provider = self.providers.get(pending.provider)?;
        provider
            .resolve_approval(ResolveApproval {
                conversation_id: pending.conversation_id,
                provider_request_id: pending.provider_request_id,
                option_id: option_id.clone(),
            })
            .await?;
        self.approvals
            .lock()
            .expect("approval mutex poisoned")
            .remove(&approval_id);
        let item_id = self
            .provider_item_ids
            .lock()
            .expect("item mutex poisoned")
            .get(&(pending.conversation_id, format!("approval:{approval_id}")))
            .copied();
        if let Some(item_id) = item_id {
            let mut cache = self.timeline_cache.lock().expect("timeline mutex poisoned");
            if let Some(item) = cache.get_mut(&item_id) {
                item.revision += 1;
                if let TimelineItemKind::Approval {
                    resolved_option, ..
                } = &mut item.kind
                {
                    *resolved_option = Some(option_id);
                }
                let item = item.clone();
                drop(cache);
                self.storage.upsert_timeline_item(&item)?;
                self.emit(ServerMessage::TimelineItemUpserted { item });
            }
        }
        let mut conversation = self.storage.conversation(pending.conversation_id)?;
        conversation.state = ConversationState::Running;
        conversation.revision += 1;
        conversation.updated_at_ms = now_ms();
        self.save_and_emit_conversation(&conversation)?;
        Ok(())
    }

    async fn set_session_option(
        &self,
        conversation_id: ConversationId,
        option_id: String,
        value: String,
    ) -> Result<()> {
        let mut conversation = self.storage.conversation(conversation_id)?;
        if conversation.state == ConversationState::Running {
            bail!("session options cannot be changed while this provider is running");
        }
        let option = conversation
            .session_options
            .iter()
            .find(|option| option.id == option_id)
            .ok_or_else(|| anyhow!("session option {option_id} is not available"))?;
        if !option
            .values
            .iter()
            .any(|candidate| candidate.value == value)
        {
            bail!("value {value} is not supported for session option {option_id}");
        }
        let provider = self.providers.get(conversation.provider)?;
        provider
            .set_session_option(SetSessionOption {
                conversation_id,
                native_session_id: conversation.native_session_id.clone(),
                option_id: option_id.clone(),
                value: value.clone(),
            })
            .await?;
        if let Some(option) = conversation
            .session_options
            .iter_mut()
            .find(|option| option.id == option_id)
        {
            option.current_value = value.clone();
        }
        if option_id == "model" {
            conversation.selected_model = Some(value.clone());
        }
        if option_id == "thought_level" || option_id == "reasoning_effort" {
            conversation.selected_effort = Some(value);
        }
        conversation.revision += 1;
        conversation.updated_at_ms = now_ms();
        self.save_and_emit_conversation(&conversation)?;
        Ok(())
    }

    async fn apply_provider_event(&self, event: ProviderEvent) {
        if let Err(error) = self.apply_provider_event_inner(event).await {
            tracing::warn!(error = %error, "provider event was rejected");
        }
    }

    async fn apply_provider_event_inner(&self, event: ProviderEvent) -> Result<()> {
        let mut conversation = self.storage.conversation(event.conversation_id)?;
        if conversation.provider != event.provider || conversation.project_id != event.project_id {
            bail!("provider event did not match the conversation authority boundary");
        }
        let project = self.storage.project(event.project_id)?;
        match event.kind {
            ProviderEventKind::AgentTextDelta {
                provider_item_id,
                phase,
                delta,
            } => {
                let item_id = self.item_id(event.conversation_id, provider_item_id);
                let mut cache = self.timeline_cache.lock().expect("timeline mutex poisoned");
                let item = cache.entry(item_id).or_insert_with(|| TimelineItem {
                    id: item_id,
                    conversation_id: event.conversation_id,
                    revision: 0,
                    created_at_ms: now_ms(),
                    kind: TimelineItemKind::AgentMessage {
                        phase,
                        text: String::new(),
                    },
                });
                match &mut item.kind {
                    TimelineItemKind::AgentMessage {
                        phase: item_phase,
                        text,
                    } if *item_phase == phase => text.push_str(&delta),
                    _ => bail!("provider reused an item id for a different event kind"),
                }
                item.revision += 1;
                let item = item.clone();
                drop(cache);
                self.storage.upsert_timeline_item(&item)?;
                self.emit(ServerMessage::TimelineItemUpserted { item });
            }
            ProviderEventKind::Plan {
                provider_item_id,
                steps,
            } => {
                self.upsert_provider_item(
                    event.conversation_id,
                    provider_item_id,
                    TimelineItemKind::Plan { steps },
                )?;
            }
            ProviderEventKind::ToolCall {
                provider_item_id,
                name,
                status,
                input_summary,
                output_summary,
            } => {
                self.upsert_provider_item(
                    event.conversation_id,
                    provider_item_id,
                    TimelineItemKind::ToolCall {
                        name,
                        status,
                        input_summary: input_summary
                            .map(|value| redact_project_path(&value, &project)),
                        output_summary: output_summary
                            .map(|value| redact_project_path(&value, &project)),
                    },
                )?;
            }
            ProviderEventKind::Command {
                provider_item_id,
                command,
                relative_cwd,
                status,
                exit_code,
                output,
            } => {
                self.upsert_provider_item(
                    event.conversation_id,
                    provider_item_id,
                    TimelineItemKind::Command {
                        command: redact_project_path(&command, &project),
                        relative_cwd: relative_cwd
                            .and_then(|path| safe_relative_display(&project, &path)),
                        status,
                        exit_code,
                        output: output
                            .map(|value| truncate_output(redact_project_path(&value, &project))),
                    },
                )?;
            }
            ProviderEventKind::FileChange {
                provider_item_id,
                relative_path,
                change_kind,
                status,
            } => {
                let relative_path = safe_relative_display(&project, &relative_path)
                    .ok_or_else(|| anyhow!("provider file change was outside the project"))?;
                self.upsert_provider_item(
                    event.conversation_id,
                    provider_item_id,
                    TimelineItemKind::FileChange {
                        relative_path,
                        change_kind,
                        status,
                    },
                )?;
            }
            ProviderEventKind::Approval {
                provider_request_id,
                prompt,
                options,
            } => {
                let approval_id = {
                    let mut ids = self
                        .provider_approval_ids
                        .lock()
                        .expect("approval id mutex poisoned");
                    *ids.entry((event.conversation_id, provider_request_id.clone()))
                        .or_default()
                };
                self.approvals
                    .lock()
                    .expect("approval mutex poisoned")
                    .insert(
                        approval_id,
                        PendingApproval {
                            provider: event.provider,
                            conversation_id: event.conversation_id,
                            provider_request_id,
                        },
                    );
                self.upsert_provider_item(
                    event.conversation_id,
                    format!("approval:{approval_id}"),
                    TimelineItemKind::Approval {
                        approval_id,
                        prompt: redact_project_path(&prompt, &project),
                        options,
                        resolved_option: None,
                    },
                )?;
                conversation.state = ConversationState::NeedsApproval;
                conversation.revision += 1;
                conversation.updated_at_ms = now_ms();
                self.save_and_emit_conversation(&conversation)?;
            }
            ProviderEventKind::ImagePath {
                path,
                mut controlled_temp_roots,
                alt,
            } => {
                controlled_temp_roots.push(project.canonical_path.clone());
                match self.attachments.import_file(
                    event.conversation_id,
                    &path,
                    &controlled_temp_roots,
                ) {
                    Ok(attachment) => {
                        self.storage.save_attachment(&attachment)?;
                        self.save_and_emit_item(TimelineItem {
                            id: TimelineItemId::new(),
                            conversation_id: event.conversation_id,
                            revision: 1,
                            created_at_ms: now_ms(),
                            kind: TimelineItemKind::Image {
                                attachment_id: attachment.metadata.id,
                                alt,
                            },
                        })?;
                    }
                    Err(error) => self.record_attachment_error(event.conversation_id, error)?,
                }
            }
            ProviderEventKind::ImageBytes {
                bytes,
                mime_type,
                alt,
            } => {
                match self
                    .attachments
                    .import_bytes(event.conversation_id, &bytes, Some(&mime_type))
                {
                    Ok(attachment) => {
                        self.storage.save_attachment(&attachment)?;
                        self.save_and_emit_item(TimelineItem {
                            id: TimelineItemId::new(),
                            conversation_id: event.conversation_id,
                            revision: 1,
                            created_at_ms: now_ms(),
                            kind: TimelineItemKind::Image {
                                attachment_id: attachment.metadata.id,
                                alt,
                            },
                        })?;
                    }
                    Err(error) => self.record_attachment_error(event.conversation_id, error)?,
                }
            }
            ProviderEventKind::Completed => {
                conversation.state = ConversationState::Completed;
                conversation.revision += 1;
                conversation.updated_at_ms = now_ms();
                self.save_and_emit_conversation(&conversation)?;
            }
            ProviderEventKind::Interrupted => {
                conversation.state = ConversationState::Interrupted;
                conversation.revision += 1;
                conversation.updated_at_ms = now_ms();
                self.save_and_emit_conversation(&conversation)?;
            }
            ProviderEventKind::Failed { code, message } => {
                self.record_failure(&project, conversation, &code, message)?;
            }
            ProviderEventKind::Crashed { message } => {
                self.record_failure(&project, conversation, "provider_crashed", message)?;
            }
        }
        Ok(())
    }

    fn authorized_project(&self, project_id: ProjectId, provider: ProviderId) -> Result<Project> {
        let project = self.storage.project(project_id)?;
        if !project.canonical_path.is_dir() {
            bail!("project path no longer exists");
        }
        if !project.enabled_providers.contains(&provider) {
            bail!("provider {provider} is not enabled for this project");
        }
        Ok(project)
    }

    fn item_id(&self, conversation_id: ConversationId, provider_item_id: String) -> TimelineItemId {
        let mut ids = self
            .provider_item_ids
            .lock()
            .expect("provider item mutex poisoned");
        *ids.entry((conversation_id, provider_item_id)).or_default()
    }

    fn upsert_provider_item(
        &self,
        conversation_id: ConversationId,
        provider_item_id: String,
        kind: TimelineItemKind,
    ) -> Result<()> {
        let item_id = self.item_id(conversation_id, provider_item_id);
        let mut cache = self.timeline_cache.lock().expect("timeline mutex poisoned");
        let revision = cache.get(&item_id).map_or(1, |item| item.revision + 1);
        let created_at_ms = cache
            .get(&item_id)
            .map_or_else(now_ms, |item| item.created_at_ms);
        let item = TimelineItem {
            id: item_id,
            conversation_id,
            revision,
            created_at_ms,
            kind,
        };
        cache.insert(item_id, item.clone());
        drop(cache);
        self.storage.upsert_timeline_item(&item)?;
        self.emit(ServerMessage::TimelineItemUpserted { item });
        Ok(())
    }

    fn save_and_emit_item(&self, item: TimelineItem) -> Result<()> {
        self.storage.upsert_timeline_item(&item)?;
        self.timeline_cache
            .lock()
            .expect("timeline mutex poisoned")
            .insert(item.id, item.clone());
        self.emit(ServerMessage::TimelineItemUpserted { item });
        Ok(())
    }

    fn save_and_emit_conversation(&self, conversation: &Conversation) -> Result<()> {
        self.storage.upsert_conversation(conversation)?;
        self.emit(ServerMessage::ConversationUpserted {
            conversation: conversation.clone(),
        });
        Ok(())
    }

    fn record_attachment_error(
        &self,
        conversation_id: ConversationId,
        error: anyhow::Error,
    ) -> Result<()> {
        self.save_and_emit_item(TimelineItem {
            id: TimelineItemId::new(),
            conversation_id,
            revision: 1,
            created_at_ms: now_ms(),
            kind: TimelineItemKind::Error {
                code: "attachment_error".to_owned(),
                message: error.to_string(),
            },
        })
    }

    fn record_failure(
        &self,
        project: &Project,
        mut conversation: Conversation,
        code: &str,
        message: String,
    ) -> Result<()> {
        conversation.state = ConversationState::Failed;
        conversation.revision += 1;
        conversation.updated_at_ms = now_ms();
        self.save_and_emit_conversation(&conversation)?;
        self.save_and_emit_item(TimelineItem {
            id: TimelineItemId::new(),
            conversation_id: conversation.id,
            revision: 1,
            created_at_ms: now_ms(),
            kind: TimelineItemKind::Error {
                code: code.to_owned(),
                message: redact_project_path(&message, project),
            },
        })
    }

    fn emit(&self, message: ServerMessage) {
        let _ = self.updates.send(message);
    }
}

fn validate_model_selection(
    models: &[agent_remote_protocol::ModelOption],
    selected_model: Option<&str>,
    selected_effort: Option<&str>,
) -> Result<()> {
    match selected_model {
        Some(selected_model) => {
            let model = models
                .iter()
                .find(|model| model.id == selected_model)
                .ok_or_else(|| {
                    anyhow!("model {selected_model} was not reported by the provider")
                })?;
            if let Some(selected_effort) = selected_effort
                && !model
                    .effort_options
                    .iter()
                    .any(|effort| effort.id == selected_effort)
            {
                bail!("effort {selected_effort} is not supported by model {selected_model}");
            }
        }
        None if selected_effort.is_some() => bail!("effort cannot be selected without a model"),
        None => {}
    }
    Ok(())
}

fn safe_relative_display(project: &Project, raw: &str) -> Option<String> {
    let path = Path::new(raw);
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return None;
    }
    if path.is_absolute() {
        return path
            .strip_prefix(&project.canonical_path)
            .ok()
            .map(|relative| relative.to_string_lossy().replace('\\', "/"));
    }
    Some(path.to_string_lossy().replace('\\', "/"))
}

fn redact_project_path(value: &str, project: &Project) -> String {
    let canonical = project.canonical_path.to_string_lossy();
    value.replace(canonical.as_ref(), ".")
}

fn truncate_output(mut output: String) -> String {
    const LIMIT: usize = 64 * 1024;
    if output.len() > LIMIT {
        output.truncate(LIMIT);
        output.push_str("\n[output truncated at 64 KiB]");
    }
    output
}

pub fn default_data_root() -> Result<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| anyhow!("cannot determine a data directory"))?;
    let root = base.join("AgentRemoteMessenger");
    fs::create_dir_all(&root)?;
    Ok(root)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use agent_remote_protocol::{
        AgentMessagePhase, ApprovalOption, EffortOption, ModelOption, ProviderHealth,
        ProviderState, SessionSummary,
    };
    use async_trait::async_trait;
    use tokio::sync::broadcast;

    use super::*;
    use crate::{
        attachments::DEFAULT_MAX_IMAGE_BYTES,
        providers::{
            AgentProvider, CommandAck, NativeSession, ProviderCapabilities, SetSessionOption,
        },
    };

    struct MockProvider {
        events: broadcast::Sender<ProviderEvent>,
        models: Vec<ModelOption>,
        projects: Mutex<HashMap<ConversationId, ProjectId>>,
        messages: Mutex<Vec<(ConversationId, String)>>,
        approvals: Mutex<Vec<String>>,
        interruptions: Mutex<Vec<ConversationId>>,
    }

    impl MockProvider {
        fn new() -> Arc<Self> {
            let (events, _) = broadcast::channel(32);
            Arc::new(Self {
                events,
                models: vec![ModelOption {
                    id: "dynamic-model".to_owned(),
                    display_name: "Dynamic Model".to_owned(),
                    effort_options: vec![
                        EffortOption {
                            id: "low".to_owned(),
                            display_name: "Low".to_owned(),
                        },
                        EffortOption {
                            id: "high".to_owned(),
                            display_name: "High".to_owned(),
                        },
                    ],
                    default_effort: Some("high".to_owned()),
                }],
                projects: Mutex::new(HashMap::new()),
                messages: Mutex::new(Vec::new()),
                approvals: Mutex::new(Vec::new()),
                interruptions: Mutex::new(Vec::new()),
            })
        }

        fn event(&self, conversation_id: ConversationId, kind: ProviderEventKind) {
            let project_id = self.projects.lock().expect("projects mutex")[&conversation_id];
            let _ = self.events.send(ProviderEvent {
                provider: ProviderId::Codex,
                project_id,
                conversation_id,
                kind,
            });
        }
    }

    #[async_trait]
    impl AgentProvider for MockProvider {
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
            self.events.subscribe()
        }

        async fn health(&self) -> ProviderHealth {
            ProviderHealth {
                provider: ProviderId::Codex,
                state: ProviderState::Ready,
                version: Some("mock".to_owned()),
                detail: None,
            }
        }

        async fn list_models(&self, _project: &Project) -> Result<Vec<ModelOption>> {
            Ok(self.models.clone())
        }

        async fn list_sessions(&self, _project: &Project) -> Result<Vec<SessionSummary>> {
            Ok(Vec::new())
        }

        async fn create_session(
            &self,
            request: CreateSession,
        ) -> Result<crate::providers::NativeSession> {
            self.projects
                .lock()
                .expect("projects mutex")
                .insert(request.conversation_id, request.project.id);
            Ok(NativeSession {
                native_session_id: format!("native-{}", request.conversation_id),
                title: request.project.display_name,
                selected_model: request.model,
                selected_effort: request.effort,
                session_options: Vec::new(),
            })
        }

        async fn resume_session(&self, request: ResumeSession) -> Result<NativeSession> {
            self.projects
                .lock()
                .expect("projects mutex")
                .insert(request.conversation_id, request.project.id);
            Ok(NativeSession {
                native_session_id: request.native_session_id,
                title: "resumed".to_owned(),
                selected_model: request.model,
                selected_effort: request.effort,
                session_options: Vec::new(),
            })
        }

        async fn send_message(&self, request: SendMessage) -> Result<CommandAck> {
            self.messages
                .lock()
                .expect("messages mutex")
                .push((request.conversation_id, request.text));
            Ok(CommandAck)
        }

        async fn steer(&self, _request: SteerMessage) -> Result<CommandAck> {
            Ok(CommandAck)
        }

        async fn interrupt(&self, request: InterruptSession) -> Result<CommandAck> {
            self.interruptions
                .lock()
                .expect("interruptions mutex")
                .push(request.conversation_id);
            Ok(CommandAck)
        }

        async fn resolve_approval(&self, request: ResolveApproval) -> Result<CommandAck> {
            self.approvals
                .lock()
                .expect("approvals mutex")
                .push(request.option_id);
            Ok(CommandAck)
        }

        async fn set_session_option(&self, _request: SetSessionOption) -> Result<CommandAck> {
            Ok(CommandAck)
        }
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        service: Arc<AppService>,
        provider: Arc<MockProvider>,
        project_a: Project,
        project_b: Project,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().expect("temp dir");
        let root_a = temp.path().join("project-a");
        let root_b = temp.path().join("project-b");
        fs::create_dir(&root_a).expect("project A");
        fs::create_dir(&root_b).expect("project B");
        let storage = Arc::new(Storage::open(temp.path().join("state.db")).expect("storage"));
        let project_a = storage
            .add_project(&root_a, Some("A"), &[ProviderId::Codex])
            .expect("add project A");
        let project_b = storage
            .add_project(&root_b, Some("B"), &[ProviderId::Codex])
            .expect("add project B");
        let provider = MockProvider::new();
        let provider_trait: Arc<dyn AgentProvider> = provider.clone();
        let service = AppService::new(
            storage,
            AttachmentStore::new(temp.path().join("attachments"), DEFAULT_MAX_IMAGE_BYTES)
                .expect("attachments"),
            ProviderRegistry::new([provider_trait]),
            "test-host".to_owned(),
        )
        .expect("app service");
        Fixture {
            _temp: temp,
            service,
            provider,
            project_a,
            project_b,
        }
    }

    async fn create(service: &AppService, project: ProjectId) -> Conversation {
        service
            .create_conversation(
                project,
                ProviderId::Codex,
                None,
                Some("dynamic-model".to_owned()),
                Some("high".to_owned()),
            )
            .await
            .expect("create conversation")
    }

    #[tokio::test]
    async fn provider_project_and_conversation_are_not_crossed() {
        let fixture = fixture();
        let conversation_a = create(&fixture.service, fixture.project_a.id).await;
        let conversation_b = create(&fixture.service, fixture.project_b.id).await;
        fixture
            .service
            .send_message(conversation_a.id, "message A".to_owned())
            .await
            .expect("send A");
        fixture
            .service
            .send_message(conversation_b.id, "message B".to_owned())
            .await
            .expect("send B");
        let projects = fixture.provider.projects.lock().expect("projects mutex");
        assert_eq!(projects[&conversation_a.id], fixture.project_a.id);
        assert_eq!(projects[&conversation_b.id], fixture.project_b.id);
        let messages = fixture.provider.messages.lock().expect("messages mutex");
        assert_eq!(
            messages.as_slice(),
            &[
                (conversation_a.id, "message A".to_owned()),
                (conversation_b.id, "message B".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn provider_reported_model_and_effort_are_enforced() {
        let fixture = fixture();
        assert!(
            fixture
                .service
                .create_conversation(
                    fixture.project_a.id,
                    ProviderId::Codex,
                    None,
                    Some("invented".to_owned()),
                    Some("ultra".to_owned()),
                )
                .await
                .is_err()
        );
        assert!(
            fixture
                .service
                .create_conversation(
                    fixture.project_a.id,
                    ProviderId::Codex,
                    None,
                    Some("dynamic-model".to_owned()),
                    Some("invented".to_owned()),
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn streaming_deltas_upsert_one_timeline_item() {
        let fixture = fixture();
        let conversation = create(&fixture.service, fixture.project_a.id).await;
        for delta in ["hello ", "world"] {
            fixture
                .service
                .apply_provider_event_inner(ProviderEvent {
                    provider: ProviderId::Codex,
                    project_id: fixture.project_a.id,
                    conversation_id: conversation.id,
                    kind: ProviderEventKind::AgentTextDelta {
                        provider_item_id: "agent-1".to_owned(),
                        phase: AgentMessagePhase::Final,
                        delta: delta.to_owned(),
                    },
                })
                .await
                .expect("apply delta");
        }
        let items = fixture.service.storage.list_timeline().expect("timeline");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].revision, 2);
        assert!(
            matches!(&items[0].kind, TimelineItemKind::AgentMessage { text, .. } if text == "hello world")
        );
    }

    #[tokio::test]
    async fn approval_and_interrupt_reach_the_selected_provider_session() {
        let fixture = fixture();
        let conversation = create(&fixture.service, fixture.project_a.id).await;
        fixture
            .service
            .apply_provider_event_inner(ProviderEvent {
                provider: ProviderId::Codex,
                project_id: fixture.project_a.id,
                conversation_id: conversation.id,
                kind: ProviderEventKind::Approval {
                    provider_request_id: "rpc-7".to_owned(),
                    prompt: "Allow command?".to_owned(),
                    options: vec![ApprovalOption {
                        id: "accept".to_owned(),
                        label: "Allow".to_owned(),
                    }],
                },
            })
            .await
            .expect("approval event");
        let item = fixture
            .service
            .storage
            .list_timeline()
            .expect("timeline")
            .pop()
            .expect("approval item");
        let approval_id = match item.kind {
            TimelineItemKind::Approval { approval_id, .. } => approval_id,
            _ => panic!("expected approval"),
        };
        fixture
            .service
            .resolve_approval(approval_id, "accept".to_owned())
            .await
            .expect("resolve approval");
        fixture
            .service
            .interrupt(conversation.id)
            .await
            .expect("interrupt");
        assert_eq!(
            fixture
                .provider
                .approvals
                .lock()
                .expect("approval mutex")
                .as_slice(),
            &["accept".to_owned()]
        );
        assert_eq!(
            fixture
                .provider
                .interruptions
                .lock()
                .expect("interruptions mutex")
                .as_slice(),
            &[conversation.id]
        );
    }

    #[tokio::test]
    async fn provider_events_with_wrong_project_are_rejected() {
        let fixture = fixture();
        let conversation = create(&fixture.service, fixture.project_a.id).await;
        let result = fixture
            .service
            .apply_provider_event_inner(ProviderEvent {
                provider: ProviderId::Codex,
                project_id: fixture.project_b.id,
                conversation_id: conversation.id,
                kind: ProviderEventKind::Completed,
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn live_event_pump_completes_the_conversation() {
        let fixture = fixture();
        fixture.service.start_provider_event_pumps();
        let conversation = create(&fixture.service, fixture.project_a.id).await;
        fixture
            .provider
            .event(conversation.id, ProviderEventKind::Completed);
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert_eq!(
            fixture
                .service
                .storage
                .conversation(conversation.id)
                .expect("conversation")
                .state,
            ConversationState::Completed
        );
    }
}
