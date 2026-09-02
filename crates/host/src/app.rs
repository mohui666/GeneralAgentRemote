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
    ApprovalId, AttachmentCapability, AttachmentId, ClientAttachment, ClientCommand, Conversation,
    ConversationId, ConversationState, ConversationTitleSource, DeviceId, HostId, ProjectId,
    ProjectSummary, ProviderCapability, ProviderId, ServerMessage, Snapshot, TimelineItem,
    TimelineItemId, TimelineItemKind,
};
use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::broadcast;

use crate::{
    attachments::AttachmentStore,
    providers::{
        CreateSession, InterruptSession, PromptAttachment, ProviderEvent, ProviderEventKind,
        ProviderHistoryItem, ProviderRegistry, ReadSessionHistory, RenameSession, ResolveApproval,
        ResumeSession, SendMessage, SetSessionOption, SteerMessage,
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
    conversation_start_lock: tokio::sync::Mutex<()>,
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
            conversation_start_lock: tokio::sync::Mutex::new(()),
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
                let mut history_lagged = false;
                loop {
                    match events.recv().await {
                        Ok(event) => {
                            if let ProviderEventKind::HistoryBarrier { barrier } = &event.kind {
                                if history_lagged {
                                    barrier.mark_lagged();
                                }
                                barrier.complete();
                                history_lagged = false;
                                continue;
                            }
                            if !service.apply_provider_event(event).await {
                                history_lagged = true;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => history_lagged = true,
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
                            supports_history: false,
                            supports_incremental_sync: false,
                            supports_rename: false,
                            supports_steer: false,
                            permission_modes: Vec::new(),
                            default_permission_mode: None,
                            attachments: AttachmentCapability::default(),
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
                    supports_history: capabilities.supports_history,
                    supports_incremental_sync: capabilities.supports_incremental_sync,
                    supports_rename: capabilities.supports_rename,
                    supports_steer: capabilities.supports_steer,
                    permission_modes: provider.permission_modes(),
                    default_permission_mode: provider.default_permission_mode(),
                    attachments: provider.attachment_capability(),
                    sessions: sessions_result.unwrap_or_default(),
                    limitation,
                });
            }
        }
        let conversations = self.storage.list_conversations()?;
        let mut timeline = Vec::new();
        for conversation in &conversations {
            let (recent, _) = self
                .storage
                .list_timeline_page(conversation.id, None, 100)?;
            timeline.extend(recent);
        }
        timeline.sort_by_key(|item| (item.created_at_ms, item.id));
        Ok(Snapshot {
            host_id: self.host_id,
            host_name: self.host_name.clone(),
            projects: project_summaries(&projects, &conversations),
            provider_capabilities,
            conversations,
            timeline,
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
            ClientCommand::RefreshProjects { provider } => self.projects_updated(provider).await,
            ClientCommand::SyncProject {
                command_id,
                project_id,
                provider,
            } => self.sync_project(command_id, project_id, provider).await,
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
            ClientCommand::StartConversation {
                command_id,
                conversation_id,
                project_id,
                provider,
                model,
                effort,
                permission_mode,
                text,
                attachments,
            } => {
                self.start_conversation(
                    conversation_id,
                    project_id,
                    provider,
                    model,
                    effort,
                    permission_mode,
                    text,
                    attachments,
                    format!("start:{command_id}"),
                )
                .await?;
                Ok(ServerMessage::CommandAccepted { command_id })
            }
            ClientCommand::SendMessage {
                command_id,
                conversation_id,
                client_message_id,
                text,
                attachments,
            } => {
                self.send_message(
                    conversation_id,
                    text,
                    attachments,
                    client_message_id.or_else(|| Some(format!("command:{command_id}"))),
                )
                .await?;
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
            ClientCommand::RenameConversation {
                command_id,
                conversation_id,
                title,
            } => {
                self.rename_conversation(conversation_id, title).await?;
                Ok(ServerMessage::CommandAccepted { command_id })
            }
            ClientCommand::GetConversationPage {
                conversation_id,
                before,
                limit,
            } => {
                let conversation = self.storage.conversation(conversation_id)?;
                self.authorized_project(conversation.project_id, conversation.provider)?;
                let (items, next_before) =
                    self.storage
                        .list_timeline_page(conversation_id, before, limit)?;
                Ok(ServerMessage::ConversationPage {
                    conversation_id,
                    items,
                    next_before,
                })
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

    async fn projects_updated(&self, provider_id: ProviderId) -> Result<ServerMessage> {
        let projects = self
            .storage
            .list_projects()?
            .into_iter()
            .filter(|project| project.enabled_providers.contains(&provider_id))
            .collect::<Vec<_>>();
        let conversations = self.storage.list_conversations()?;
        let mut capabilities = Vec::with_capacity(projects.len());
        for project in &projects {
            capabilities.push(self.provider_capability(project, provider_id).await);
        }
        Ok(ServerMessage::ProjectsUpdated {
            provider: provider_id,
            projects: project_summaries(&projects, &conversations),
            capabilities,
        })
    }

    async fn provider_capability(
        &self,
        project: &Project,
        provider_id: ProviderId,
    ) -> ProviderCapability {
        let provider = match self.providers.get(provider_id) {
            Ok(provider) => provider,
            Err(error) => {
                return ProviderCapability {
                    provider: provider_id,
                    project_id: project.id,
                    health: agent_remote_protocol::ProviderHealth {
                        provider: provider_id,
                        state: agent_remote_protocol::ProviderState::Offline,
                        version: None,
                        detail: Some(error.to_string()),
                    },
                    models: Vec::new(),
                    supports_session_list: false,
                    supports_history: false,
                    supports_incremental_sync: false,
                    supports_rename: false,
                    supports_steer: false,
                    permission_modes: Vec::new(),
                    default_permission_mode: None,
                    attachments: AttachmentCapability::default(),
                    sessions: Vec::new(),
                    limitation: Some("Provider adapter is unavailable".to_owned()),
                };
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
        ProviderCapability {
            provider: provider_id,
            project_id: project.id,
            health,
            models: models_result.unwrap_or_default(),
            supports_session_list: capabilities.supports_session_list,
            supports_history: capabilities.supports_history,
            supports_incremental_sync: capabilities.supports_incremental_sync,
            supports_rename: capabilities.supports_rename,
            supports_steer: capabilities.supports_steer,
            permission_modes: provider.permission_modes(),
            default_permission_mode: provider.default_permission_mode(),
            attachments: provider.attachment_capability(),
            sessions: sessions_result.unwrap_or_default(),
            limitation,
        }
    }

    async fn sync_project(
        &self,
        command_id: agent_remote_protocol::CommandId,
        project_id: ProjectId,
        provider_id: ProviderId,
    ) -> Result<ServerMessage> {
        let project = self.authorized_project(project_id, provider_id)?;
        let provider = self.providers.get(provider_id)?;
        let capabilities = provider.capabilities();
        if !capabilities.supports_session_list {
            bail!("{provider_id} does not support remote conversation listing");
        }
        let sessions = provider.list_sessions(&project).await?;
        let mut full_history_fallback = false;
        for session in &sessions {
            let mut conversation = match self.storage.conversation_by_native_session(
                provider_id,
                project_id,
                &session.native_session_id,
            )? {
                Some(conversation) => conversation,
                None => Conversation {
                    id: ConversationId::new(),
                    revision: 1,
                    provider: provider_id,
                    project_id,
                    native_session_id: session.native_session_id.clone(),
                    title: provider_title(&session.title),
                    title_source: ConversationTitleSource::Provider,
                    title_updated_at_ms: now_ms(),
                    selected_model: None,
                    selected_effort: None,
                    state: ConversationState::Idle,
                    session_options: Vec::new(),
                    updated_at_ms: session.updated_at_ms,
                },
            };
            if conversation.title_source != ConversationTitleSource::User
                && !session.title.trim().is_empty()
                && conversation.title != session.title
            {
                conversation.title = provider_title(&session.title);
                conversation.title_source = ConversationTitleSource::Provider;
                conversation.title_updated_at_ms = now_ms();
                conversation.revision += 1;
            }
            conversation.updated_at_ms = conversation.updated_at_ms.max(session.updated_at_ms);
            self.storage.upsert_conversation(&conversation)?;
            self.emit(ServerMessage::ConversationUpserted {
                conversation: conversation.clone(),
            });

            if capabilities.supports_history
                && self.storage.remote_history_is_stale(
                    provider_id,
                    project_id,
                    &session.native_session_id,
                    session.updated_at_ms,
                )?
            {
                let mut cursor = None;
                loop {
                    let page = provider
                        .read_session_history(ReadSessionHistory {
                            conversation_id: conversation.id,
                            project: project.clone(),
                            native_session_id: session.native_session_id.clone(),
                            cursor: cursor.clone(),
                            limit: 200,
                        })
                        .await?;
                    full_history_fallback |= page.full_read_fallback;
                    for item in page.items {
                        self.upsert_history_item(conversation.id, item)?;
                    }
                    match page.next_cursor {
                        Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
                        _ => break,
                    }
                }
                provider
                    .flush_history_events(project_id, conversation.id)
                    .await?;
                self.storage.mark_remote_history_synced(
                    provider_id,
                    project_id,
                    &session.native_session_id,
                    session.updated_at_ms,
                )?;
            }
        }
        Ok(ServerMessage::ProjectSyncCompleted {
            command_id,
            project_id,
            provider: provider_id,
            conversations_synced: sessions.len() as u32,
            full_history_fallback,
        })
    }

    fn upsert_history_item(
        &self,
        conversation_id: ConversationId,
        history: ProviderHistoryItem,
    ) -> Result<()> {
        let item_id = self
            .storage
            .provider_item_id(conversation_id, &history.provider_item_id)?;
        let mut cache = self.timeline_cache.lock().expect("timeline mutex poisoned");
        if cache.get(&item_id).is_some_and(|existing| {
            existing.created_at_ms == history.created_at_ms && existing.kind == history.kind
        }) {
            return Ok(());
        }
        let revision = cache.get(&item_id).map_or(1, |item| item.revision + 1);
        let item = TimelineItem {
            id: item_id,
            conversation_id,
            revision,
            created_at_ms: history.created_at_ms,
            kind: history.kind,
        };
        cache.insert(item_id, item.clone());
        drop(cache);
        self.storage.upsert_timeline_item(&item)?;
        self.emit(ServerMessage::TimelineItemUpserted { item });
        Ok(())
    }

    async fn rename_conversation(
        &self,
        conversation_id: ConversationId,
        title: String,
    ) -> Result<()> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 80 {
            bail!("conversation title must contain 1 to 80 characters");
        }
        let mut conversation = self.storage.conversation(conversation_id)?;
        let project = self.authorized_project(conversation.project_id, conversation.provider)?;
        let provider = self.providers.get(conversation.provider)?;
        conversation.title = title.to_owned();
        conversation.title_source = ConversationTitleSource::User;
        conversation.title_updated_at_ms = now_ms();
        conversation.revision += 1;
        self.save_and_emit_conversation(&conversation)?;
        if provider.capabilities().supports_rename {
            provider
                .rename_session(RenameSession {
                    conversation_id,
                    project,
                    native_session_id: conversation.native_session_id,
                    title: title.to_owned(),
                })
                .await?;
        }
        Ok(())
    }

    async fn create_conversation(
        &self,
        project_id: ProjectId,
        provider_id: ProviderId,
        native_session_id: Option<String>,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<Conversation> {
        self.create_conversation_with_id(
            ConversationId::new(),
            project_id,
            provider_id,
            native_session_id,
            model,
            effort,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_conversation_with_id(
        &self,
        conversation_id: ConversationId,
        project_id: ProjectId,
        provider_id: ProviderId,
        native_session_id: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        permission_mode: Option<String>,
    ) -> Result<Conversation> {
        let project = self.authorized_project(project_id, provider_id)?;
        let provider = self.providers.get(provider_id)?;
        validate_model_selection(
            &provider.list_models(&project).await?,
            model.as_deref(),
            effort.as_deref(),
        )?;
        let resuming = native_session_id.is_some();
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
        let timestamp = now_ms();
        let (title, title_source) = if resuming && !native.title.trim().is_empty() {
            (native.title, ConversationTitleSource::Provider)
        } else {
            ("新对话".to_owned(), ConversationTitleSource::Fallback)
        };
        let default_permission_mode = provider.default_permission_mode();
        let session_options = permission_session_options(
            native.session_options,
            &provider.permission_modes(),
            permission_mode
                .as_deref()
                .or(default_permission_mode.as_deref()),
        )?;
        let conversation = Conversation {
            id: conversation_id,
            revision: 1,
            provider: provider_id,
            project_id,
            native_session_id: native.native_session_id,
            title,
            title_source,
            title_updated_at_ms: timestamp,
            selected_model: native.selected_model.or(model),
            selected_effort: native.selected_effort.or(effort),
            state: ConversationState::Idle,
            session_options,
            updated_at_ms: timestamp,
        };
        self.storage.upsert_conversation(&conversation)?;
        self.emit(ServerMessage::ConversationUpserted {
            conversation: conversation.clone(),
        });
        Ok(conversation)
    }

    #[allow(clippy::too_many_arguments)]
    async fn start_conversation(
        &self,
        conversation_id: ConversationId,
        project_id: ProjectId,
        provider_id: ProviderId,
        model: Option<String>,
        effort: Option<String>,
        permission_mode: Option<String>,
        text: String,
        attachments: Vec<ClientAttachment>,
        client_message_id: String,
    ) -> Result<()> {
        let _guard = self.conversation_start_lock.lock().await;
        let existing = self
            .storage
            .list_conversations()?
            .into_iter()
            .find(|conversation| conversation.id == conversation_id);
        if let Some(existing) = existing {
            if existing.project_id != project_id || existing.provider != provider_id {
                bail!("conversation id is already bound to a different project or provider");
            }
        } else {
            self.create_conversation_with_id(
                conversation_id,
                project_id,
                provider_id,
                None,
                model,
                effort,
                permission_mode,
            )
            .await?;
        }
        self.send_message(conversation_id, text, attachments, Some(client_message_id))
            .await
    }

    async fn send_message(
        &self,
        conversation_id: ConversationId,
        text: String,
        attachments: Vec<ClientAttachment>,
        client_message_id: Option<String>,
    ) -> Result<()> {
        if text.trim().is_empty() {
            bail!("message text cannot be empty");
        }
        let mut conversation = self.storage.conversation(conversation_id)?;
        let project = self.authorized_project(conversation.project_id, conversation.provider)?;
        let provider = self.providers.get(conversation.provider)?;
        let item_id = match client_message_id.as_deref() {
            Some(client_message_id) => self
                .storage
                .provider_item_id(conversation_id, &format!("client:{client_message_id}"))?,
            None => TimelineItemId::new(),
        };
        if let Some(existing) = self
            .timeline_cache
            .lock()
            .expect("timeline mutex poisoned")
            .get(&item_id)
        {
            if existing.kind == (TimelineItemKind::UserMessage { text: text.clone() }) {
                return Ok(());
            }
            bail!("client message id was reused with different content");
        }
        let prompt_attachments = self.import_prompt_attachments(
            &conversation,
            attachments,
            &provider.attachment_capability(),
        )?;
        if conversation.title_source == ConversationTitleSource::Fallback {
            conversation.title = provisional_title(&text);
            conversation.title_source = ConversationTitleSource::Generated;
            conversation.title_updated_at_ms = now_ms();
        }
        let user_item = TimelineItem {
            id: item_id,
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
        let permission_mode = conversation
            .session_options
            .iter()
            .find(|option| option.id == "permission_mode")
            .map(|option| option.current_value.clone());
        if let Err(error) = provider
            .send_message(SendMessage {
                conversation_id,
                project: project.clone(),
                native_session_id: conversation.native_session_id.clone(),
                client_message_id,
                text,
                attachments: prompt_attachments,
                model: conversation.selected_model.clone(),
                effort: conversation.selected_effort.clone(),
                permission_mode,
            })
            .await
        {
            self.record_failure(&project, conversation, "provider_error", error.to_string())?;
            return Err(error);
        }
        Ok(())
    }

    fn import_prompt_attachments(
        &self,
        conversation: &Conversation,
        attachments: Vec<ClientAttachment>,
        capability: &AttachmentCapability,
    ) -> Result<Vec<PromptAttachment>> {
        if attachments.is_empty() {
            return Ok(Vec::new());
        }
        if !capability.supported() {
            bail!(
                "{} does not accept prompt attachments",
                conversation.provider
            );
        }
        if attachments.len() > usize::from(capability.max_count) {
            bail!("at most {} attachments are allowed", capability.max_count);
        }
        if attachments
            .iter()
            .map(|attachment| attachment.bytes.len() as u64)
            .sum::<u64>()
            > capability.max_total_bytes
        {
            bail!(
                "attachments exceed the provider total size limit of {} bytes",
                capability.max_total_bytes
            );
        }
        let mut imported = Vec::with_capacity(attachments.len());
        for attachment in attachments {
            if attachment.bytes.len() as u64 > capability.max_bytes {
                bail!(
                    "attachment {} exceeds the provider size limit",
                    attachment.file_name
                );
            }
            if !capability
                .allowed_mime_types
                .iter()
                .any(|mime| mime == &attachment.mime_type)
            {
                bail!("attachment type {} is not supported", attachment.mime_type);
            }
            let stored = self.attachments.import_bytes(
                conversation.id,
                &attachment.bytes,
                Some(&attachment.mime_type),
            )?;
            if !capability
                .allowed_mime_types
                .iter()
                .any(|mime| mime == &stored.metadata.mime_type)
            {
                bail!("decoded attachment type is not supported by the provider");
            }
            self.storage.save_attachment(&stored)?;
            self.save_and_emit_item(TimelineItem {
                id: TimelineItemId::new(),
                conversation_id: conversation.id,
                revision: 1,
                created_at_ms: now_ms(),
                kind: TimelineItemKind::Image {
                    attachment_id: stored.metadata.id,
                    alt: attachment.file_name.chars().take(128).collect(),
                },
            })?;
            imported.push(PromptAttachment {
                id: stored.metadata.id,
                path: stored.managed_path,
                mime_type: stored.metadata.mime_type,
            });
        }
        Ok(imported)
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

    async fn apply_provider_event(&self, event: ProviderEvent) -> bool {
        match self.apply_provider_event_inner(event).await {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(error = %error, "provider event was rejected");
                false
            }
        }
    }

    async fn apply_provider_event_inner(&self, event: ProviderEvent) -> Result<()> {
        let mut conversation = self.storage.conversation(event.conversation_id)?;
        if conversation.provider != event.provider || conversation.project_id != event.project_id {
            bail!("provider event did not match the conversation authority boundary");
        }
        let project = self.storage.project(event.project_id)?;
        match event.kind {
            ProviderEventKind::HistoryBarrier { barrier } => barrier.complete(),
            ProviderEventKind::HistoryItem {
                provider_item_id,
                kind,
            } => {
                self.upsert_provider_item(event.conversation_id, provider_item_id, kind)?;
            }
            ProviderEventKind::UserMessageDelta {
                provider_item_id,
                delta,
            } => {
                let item_id = self.item_id(event.conversation_id, provider_item_id)?;
                let mut cache = self.timeline_cache.lock().expect("timeline mutex poisoned");
                let item = cache.entry(item_id).or_insert_with(|| TimelineItem {
                    id: item_id,
                    conversation_id: event.conversation_id,
                    revision: 0,
                    created_at_ms: now_ms(),
                    kind: TimelineItemKind::UserMessage {
                        text: String::new(),
                    },
                });
                match &mut item.kind {
                    TimelineItemKind::UserMessage { text } => text.push_str(&delta),
                    _ => bail!("provider reused an item id for a different event kind"),
                }
                item.revision += 1;
                let item = item.clone();
                drop(cache);
                self.storage.upsert_timeline_item(&item)?;
                self.emit(ServerMessage::TimelineItemUpserted { item });
            }
            ProviderEventKind::AgentTextDelta {
                provider_item_id,
                phase,
                delta,
            } => {
                let item_id = self.item_id(event.conversation_id, provider_item_id)?;
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
                provider_item_id,
                path,
                mut controlled_temp_roots,
                alt,
            } => {
                if let Some(provider_item_id) = provider_item_id.as_deref()
                    && self.provider_item_is_image(event.conversation_id, provider_item_id)?
                {
                    return Ok(());
                }
                controlled_temp_roots.push(project.canonical_path.clone());
                match self.attachments.import_file(
                    event.conversation_id,
                    &path,
                    &controlled_temp_roots,
                ) {
                    Ok(attachment) => {
                        self.storage.save_attachment(&attachment)?;
                        self.save_and_emit_image(
                            event.conversation_id,
                            provider_item_id,
                            attachment.metadata.id,
                            alt,
                        )?;
                    }
                    Err(error) => self.record_attachment_error(event.conversation_id, error)?,
                }
            }
            ProviderEventKind::ImageBytes {
                provider_item_id,
                bytes,
                mime_type,
                alt,
            } => {
                if let Some(provider_item_id) = provider_item_id.as_deref()
                    && self.provider_item_is_image(event.conversation_id, provider_item_id)?
                {
                    return Ok(());
                }
                match self
                    .attachments
                    .import_bytes(event.conversation_id, &bytes, Some(&mime_type))
                {
                    Ok(attachment) => {
                        self.storage.save_attachment(&attachment)?;
                        self.save_and_emit_image(
                            event.conversation_id,
                            provider_item_id,
                            attachment.metadata.id,
                            alt,
                        )?;
                    }
                    Err(error) => self.record_attachment_error(event.conversation_id, error)?,
                }
            }
            ProviderEventKind::Completed => {
                conversation.state = ConversationState::Completed;
                conversation.revision += 1;
                conversation.updated_at_ms = now_ms();
                self.save_and_emit_conversation(&conversation)?;
                if let Err(error) = self
                    .refresh_provider_title(&mut conversation, &project)
                    .await
                {
                    tracing::warn!(
                        conversation_id = %conversation.id,
                        error = %error,
                        "provider title refresh failed after the completed turn"
                    );
                }
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

    async fn refresh_provider_title(
        &self,
        conversation: &mut Conversation,
        project: &Project,
    ) -> Result<()> {
        if conversation.title_source == ConversationTitleSource::User {
            return Ok(());
        }
        let provider = self.providers.get(conversation.provider)?;
        if !provider.capabilities().supports_session_list {
            return Ok(());
        }
        let Some(session) = provider
            .list_sessions(project)
            .await?
            .into_iter()
            .find(|session| session.native_session_id == conversation.native_session_id)
        else {
            return Ok(());
        };
        let title = provider_title(&session.title);
        if title == "新对话" || title == conversation.title {
            return Ok(());
        }
        conversation.title = title;
        conversation.title_source = ConversationTitleSource::Provider;
        conversation.title_updated_at_ms = now_ms();
        conversation.revision += 1;
        self.save_and_emit_conversation(conversation)
    }

    fn item_id(
        &self,
        conversation_id: ConversationId,
        provider_item_id: String,
    ) -> Result<TimelineItemId> {
        let mut ids = self
            .provider_item_ids
            .lock()
            .expect("provider item mutex poisoned");
        if let Some(id) = ids.get(&(conversation_id, provider_item_id.clone())) {
            return Ok(*id);
        }
        let id = self
            .storage
            .provider_item_id(conversation_id, &provider_item_id)?;
        ids.insert((conversation_id, provider_item_id), id);
        Ok(id)
    }

    fn upsert_provider_item(
        &self,
        conversation_id: ConversationId,
        provider_item_id: String,
        kind: TimelineItemKind,
    ) -> Result<()> {
        let item_id = self.item_id(conversation_id, provider_item_id)?;
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

    fn save_and_emit_image(
        &self,
        conversation_id: ConversationId,
        provider_item_id: Option<String>,
        attachment_id: AttachmentId,
        alt: String,
    ) -> Result<()> {
        let kind = TimelineItemKind::Image { attachment_id, alt };
        match provider_item_id {
            Some(provider_item_id) => {
                self.upsert_provider_item(conversation_id, provider_item_id, kind)
            }
            None => self.save_and_emit_item(TimelineItem {
                id: TimelineItemId::new(),
                conversation_id,
                revision: 1,
                created_at_ms: now_ms(),
                kind,
            }),
        }
    }

    fn provider_item_is_image(
        &self,
        conversation_id: ConversationId,
        provider_item_id: &str,
    ) -> Result<bool> {
        let item_id = self.item_id(conversation_id, provider_item_id.to_owned())?;
        Ok(self
            .timeline_cache
            .lock()
            .expect("timeline mutex poisoned")
            .get(&item_id)
            .is_some_and(|item| matches!(item.kind, TimelineItemKind::Image { .. })))
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

fn project_summaries(projects: &[Project], conversations: &[Conversation]) -> Vec<ProjectSummary> {
    projects
        .iter()
        .map(|project| {
            let mut summary = project.summary();
            let project_conversations = conversations
                .iter()
                .filter(|conversation| conversation.project_id == project.id)
                .collect::<Vec<_>>();
            summary.conversation_count = project_conversations.len() as u32;
            summary.last_activity_at_ms = project_conversations
                .iter()
                .map(|conversation| conversation.updated_at_ms)
                .max();
            summary
        })
        .collect()
}

fn permission_session_options(
    mut session_options: Vec<agent_remote_protocol::SessionOption>,
    modes: &[agent_remote_protocol::PermissionModeOption],
    selected: Option<&str>,
) -> Result<Vec<agent_remote_protocol::SessionOption>> {
    let Some(selected) = selected else {
        return Ok(session_options);
    };
    if !modes.iter().any(|mode| mode.id == selected) {
        bail!("permission mode {selected} was not reported by the provider");
    }
    if let Some(option) = session_options
        .iter_mut()
        .find(|option| option.id == "permission_mode")
    {
        option.current_value = selected.to_owned();
        return Ok(session_options);
    }
    session_options.push(agent_remote_protocol::SessionOption {
        id: "permission_mode".to_owned(),
        display_name: "Permissions".to_owned(),
        category: Some("permission".to_owned()),
        current_value: selected.to_owned(),
        values: modes
            .iter()
            .map(|mode| agent_remote_protocol::SessionOptionValue {
                value: mode.id.clone(),
                display_name: mode.display_name.clone(),
            })
            .collect(),
    });
    Ok(session_options)
}

fn provisional_title(message: &str) -> String {
    let clean = message
        .lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with("```")
                && !line.starts_with("# Files mentioned")
                && !line.starts_with("C:\\")
                && !line.starts_with('/')
        })
        .unwrap_or("新对话")
        .trim_matches(|character: char| {
            character.is_ascii_punctuation() || character.is_whitespace()
        });
    let title = if clean
        .chars()
        .any(|character| ('\u{4e00}'..='\u{9fff}').contains(&character))
    {
        clean.chars().take(20).collect()
    } else {
        clean
            .split_whitespace()
            .take(8)
            .collect::<Vec<_>>()
            .join(" ")
    };
    if title.is_empty() {
        "新对话".to_owned()
    } else {
        title
    }
}

fn provider_title(title: &str) -> String {
    let title = title.trim();
    if title.is_empty() {
        "新对话".to_owned()
    } else {
        title.chars().take(80).collect()
    }
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
        AgentMessagePhase, ApprovalOption, CommandId, EffortOption, ModelOption, ProviderHealth,
        ProviderState, SessionSummary,
    };
    use async_trait::async_trait;
    use tokio::sync::broadcast;

    use super::*;
    use crate::{
        attachments::DEFAULT_MAX_IMAGE_BYTES,
        providers::{
            AgentProvider, CommandAck, NativeSession, ProviderCapabilities, ProviderHistoryPage,
            ReadSessionHistory, RenameSession, SetSessionOption,
        },
    };

    struct MockProvider {
        events: broadcast::Sender<ProviderEvent>,
        models: Vec<ModelOption>,
        projects: Mutex<HashMap<ConversationId, ProjectId>>,
        messages: Mutex<Vec<(ConversationId, String)>>,
        approvals: Mutex<Vec<String>>,
        interruptions: Mutex<Vec<ConversationId>>,
        sessions: Mutex<Vec<SessionSummary>>,
        history: Mutex<HashMap<String, Vec<ProviderHistoryItem>>>,
        renames: Mutex<Vec<(String, String)>>,
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
                sessions: Mutex::new(Vec::new()),
                history: Mutex::new(HashMap::new()),
                renames: Mutex::new(Vec::new()),
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
                supports_history: true,
                supports_rename: true,
                supports_steer: true,
                ..ProviderCapabilities::default()
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
            Ok(self.sessions.lock().expect("sessions mutex").clone())
        }

        async fn read_session_history(
            &self,
            request: ReadSessionHistory,
        ) -> Result<ProviderHistoryPage> {
            Ok(ProviderHistoryPage {
                items: self
                    .history
                    .lock()
                    .expect("history mutex")
                    .get(&request.native_session_id)
                    .cloned()
                    .unwrap_or_default(),
                next_cursor: None,
                full_read_fallback: true,
            })
        }

        async fn rename_session(&self, request: RenameSession) -> Result<CommandAck> {
            self.renames
                .lock()
                .expect("renames mutex")
                .push((request.native_session_id, request.title));
            Ok(CommandAck)
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
            .send_message(conversation_a.id, "message A".to_owned(), Vec::new(), None)
            .await
            .expect("send A");
        fixture
            .service
            .send_message(conversation_b.id, "message B".to_owned(), Vec::new(), None)
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

    #[tokio::test]
    async fn first_send_creates_once_and_client_message_id_is_idempotent() {
        let fixture = fixture();
        let conversation_id = ConversationId::new();
        fixture
            .service
            .start_conversation(
                conversation_id,
                fixture.project_a.id,
                ProviderId::Codex,
                Some("dynamic-model".to_owned()),
                Some("high".to_owned()),
                None,
                "实现一个增量同步器".to_owned(),
                Vec::new(),
                "message-1".to_owned(),
            )
            .await
            .expect("start conversation");
        fixture
            .service
            .send_message(
                conversation_id,
                "实现一个增量同步器".to_owned(),
                Vec::new(),
                Some("message-1".to_owned()),
            )
            .await
            .expect("repeat message");

        let conversation = fixture
            .service
            .storage
            .conversation(conversation_id)
            .expect("conversation");
        assert_eq!(conversation.title, "实现一个增量同步器");
        assert_eq!(
            conversation.title_source,
            ConversationTitleSource::Generated
        );
        assert_eq!(
            fixture
                .provider
                .messages
                .lock()
                .expect("messages mutex")
                .as_slice(),
            &[(conversation_id, "实现一个增量同步器".to_owned())]
        );
        assert_eq!(
            fixture
                .service
                .storage
                .list_timeline()
                .expect("timeline")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn remote_history_is_deduplicated_and_manual_title_is_locked() {
        let fixture = fixture();
        fixture
            .provider
            .sessions
            .lock()
            .expect("sessions mutex")
            .push(SessionSummary {
                native_session_id: "remote-1".to_owned(),
                title: "Provider title".to_owned(),
                updated_at_ms: 50,
            });
        fixture
            .provider
            .history
            .lock()
            .expect("history mutex")
            .insert(
                "remote-1".to_owned(),
                vec![ProviderHistoryItem {
                    provider_item_id: "remote-user-1".to_owned(),
                    created_at_ms: 25,
                    kind: TimelineItemKind::UserMessage {
                        text: "from provider".to_owned(),
                    },
                }],
            );

        fixture
            .service
            .sync_project(CommandId::new(), fixture.project_a.id, ProviderId::Codex)
            .await
            .expect("first sync");
        let conversation = fixture
            .service
            .storage
            .conversation_by_native_session(ProviderId::Codex, fixture.project_a.id, "remote-1")
            .expect("conversation lookup")
            .expect("conversation");
        fixture
            .service
            .rename_conversation(conversation.id, "我的固定标题".to_owned())
            .await
            .expect("rename");
        fixture.provider.sessions.lock().expect("sessions mutex")[0].title =
            "Changed provider title".to_owned();
        fixture.provider.sessions.lock().expect("sessions mutex")[0].updated_at_ms = 75;
        fixture
            .service
            .sync_project(CommandId::new(), fixture.project_a.id, ProviderId::Codex)
            .await
            .expect("second sync");

        let conversation = fixture
            .service
            .storage
            .conversation(conversation.id)
            .expect("conversation");
        assert_eq!(conversation.title, "我的固定标题");
        assert_eq!(conversation.title_source, ConversationTitleSource::User);
        assert_eq!(
            fixture
                .service
                .storage
                .list_timeline()
                .expect("timeline")
                .len(),
            1
        );
        assert_eq!(
            fixture
                .provider
                .renames
                .lock()
                .expect("renames mutex")
                .as_slice(),
            &[("remote-1".to_owned(), "我的固定标题".to_owned())]
        );
    }
}
