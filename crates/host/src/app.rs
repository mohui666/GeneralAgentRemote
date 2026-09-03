use std::{
    collections::{HashMap, HashSet},
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
    storage::{IssuedDevice, Project, Storage, StoredCommand, now_ms},
};

#[derive(Debug, Clone)]
struct PendingApproval {
    provider: ProviderId,
    conversation_id: ConversationId,
    provider_request_id: String,
}

type CommandExecutionKey = (DeviceId, agent_remote_protocol::CommandId);

struct CommandExecutionEntry {
    lock: Arc<tokio::sync::Mutex<()>>,
    users: usize,
}

struct CommandExecutionTicket<'a> {
    key: CommandExecutionKey,
    lock: Arc<tokio::sync::Mutex<()>>,
    entries: &'a Mutex<HashMap<CommandExecutionKey, CommandExecutionEntry>>,
}

impl Drop for CommandExecutionTicket<'_> {
    fn drop(&mut self) {
        let mut entries = self.entries.lock().expect("command lock mutex poisoned");
        let remove = {
            let entry = entries.get_mut(&self.key).expect("command lock entry");
            entry.users -= 1;
            entry.users == 0
        };
        if remove {
            entries.remove(&self.key);
        }
    }
}

struct ConversationMutationEntry {
    lock: Arc<tokio::sync::Mutex<()>>,
    users: usize,
}

struct ConversationMutationTicket<'a> {
    conversation_id: ConversationId,
    lock: Arc<tokio::sync::Mutex<()>>,
    entries: &'a Mutex<HashMap<ConversationId, ConversationMutationEntry>>,
}

impl Drop for ConversationMutationTicket<'_> {
    fn drop(&mut self) {
        let mut entries = self
            .entries
            .lock()
            .expect("conversation mutation mutex poisoned");
        let remove = {
            let entry = entries
                .get_mut(&self.conversation_id)
                .expect("conversation mutation entry");
            entry.users -= 1;
            entry.users == 0
        };
        if remove {
            entries.remove(&self.conversation_id);
        }
    }
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
    command_execution_locks: Mutex<HashMap<CommandExecutionKey, CommandExecutionEntry>>,
    conversation_mutation_locks: Mutex<HashMap<ConversationId, ConversationMutationEntry>>,
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
            command_execution_locks: Mutex::new(HashMap::new()),
            conversation_mutation_locks: Mutex::new(HashMap::new()),
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
                let mut rejected_histories = HashSet::new();
                let mut stream_lag_epoch = 0_u64;
                let mut acknowledged_lag_epochs = HashMap::new();
                loop {
                    match events.recv().await {
                        Ok(event) => {
                            let conversation_id = event.conversation_id;
                            let stream_lagged = acknowledged_lag_epochs
                                .get(&conversation_id)
                                .copied()
                                .unwrap_or_default()
                                < stream_lag_epoch;
                            if let ProviderEventKind::HistoryBarrier { barrier } = &event.kind {
                                let rejected = rejected_histories.remove(&conversation_id);
                                if stream_lagged {
                                    acknowledged_lag_epochs
                                        .insert(conversation_id, stream_lag_epoch);
                                }
                                if rejected || stream_lagged {
                                    barrier.mark_lagged();
                                }
                                barrier.complete();
                                continue;
                            }
                            if (rejected_histories.contains(&conversation_id) || stream_lagged)
                                && matches!(&event.kind, ProviderEventKind::HistoryWatermark { .. })
                            {
                                continue;
                            }
                            if !service.apply_provider_event(event).await {
                                rejected_histories.insert(conversation_id);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => stream_lag_epoch += 1,
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
        let Some(command_id) = command.command_id() else {
            return self.execute_command_inner(command).await;
        };
        let ticket = self.command_execution_ticket(device_id, command_id);
        let _guard = ticket.lock.lock().await;
        match self.storage.command_state(device_id, command_id)? {
            StoredCommand::Complete(result) => return Ok(*result),
            StoredCommand::Pending => {
                let result = ServerMessage::CommandRejected {
                    command_id: Some(command_id),
                    code: "command_outcome_unknown".to_owned(),
                    message: "The Host stopped before recording this command's outcome; retry with a new command ID"
                        .to_owned(),
                };
                self.storage
                    .finish_command(device_id, command_id, &result)?;
                return Ok(result);
            }
            StoredCommand::Missing => {}
        }
        let command_project = self.project_for_command(&command);
        let mutation_ticket = ordered_conversation_mutation(&command)
            .map(|conversation_id| self.conversation_mutation_ticket(conversation_id));
        let _mutation_guard = match mutation_ticket.as_ref() {
            Some(ticket) => Some(ticket.lock.lock().await),
            None => None,
        };
        self.storage.begin_command(device_id, command_id)?;
        let result = match self.execute_command_inner(command).await {
            Ok(result) => result,
            Err(error) => ServerMessage::CommandRejected {
                command_id: Some(command_id),
                code: "command_failed".to_owned(),
                message: redact_remote_error(&format!("{error:#}"), command_project.as_ref()),
            },
        };
        self.storage
            .finish_command(device_id, command_id, &result)?;
        Ok(result)
    }

    fn command_execution_ticket(
        &self,
        device_id: DeviceId,
        command_id: agent_remote_protocol::CommandId,
    ) -> CommandExecutionTicket<'_> {
        let key = (device_id, command_id);
        let mut entries = self
            .command_execution_locks
            .lock()
            .expect("command lock mutex poisoned");
        let entry = entries.entry(key).or_insert_with(|| CommandExecutionEntry {
            lock: Arc::new(tokio::sync::Mutex::new(())),
            users: 0,
        });
        entry.users += 1;
        CommandExecutionTicket {
            key,
            lock: Arc::clone(&entry.lock),
            entries: &self.command_execution_locks,
        }
    }

    fn conversation_mutation_ticket(
        &self,
        conversation_id: ConversationId,
    ) -> ConversationMutationTicket<'_> {
        let mut entries = self
            .conversation_mutation_locks
            .lock()
            .expect("conversation mutation mutex poisoned");
        let entry = entries
            .entry(conversation_id)
            .or_insert_with(|| ConversationMutationEntry {
                lock: Arc::new(tokio::sync::Mutex::new(())),
                users: 0,
            });
        entry.users += 1;
        ConversationMutationTicket {
            conversation_id,
            lock: Arc::clone(&entry.lock),
            entries: &self.conversation_mutation_locks,
        }
    }

    fn project_for_command(&self, command: &ClientCommand) -> Option<Project> {
        let project_id = match command {
            ClientCommand::SyncProject { project_id, .. }
            | ClientCommand::CreateConversation { project_id, .. }
            | ClientCommand::StartConversation { project_id, .. } => *project_id,
            ClientCommand::SendMessage {
                conversation_id, ..
            }
            | ClientCommand::Steer {
                conversation_id, ..
            }
            | ClientCommand::Interrupt {
                conversation_id, ..
            }
            | ClientCommand::SetSessionOption {
                conversation_id, ..
            }
            | ClientCommand::RenameConversation {
                conversation_id, ..
            } => self.storage.conversation(*conversation_id).ok()?.project_id,
            ClientCommand::ResolveApproval { approval_id, .. } => {
                let conversation_id = self
                    .approvals
                    .lock()
                    .expect("approval mutex poisoned")
                    .get(approval_id)?
                    .conversation_id;
                self.storage.conversation(conversation_id).ok()?.project_id
            }
            ClientCommand::Pair { .. }
            | ClientCommand::Authenticate { .. }
            | ClientCommand::GetSnapshot
            | ClientCommand::RefreshProjects { .. }
            | ClientCommand::GetConversationPage { .. }
            | ClientCommand::GetAttachment { .. } => return None,
        };
        self.storage.project(project_id).ok()
    }

    async fn execute_command_inner(&self, command: ClientCommand) -> Result<ServerMessage> {
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
                let mut history_items = Vec::new();
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
                    history_items.extend(page.items);
                    match page.next_cursor {
                        Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
                        _ => break,
                    }
                }
                provider
                    .flush_history_events(project_id, conversation.id)
                    .await?;
                for item in history_items {
                    self.upsert_history_item(conversation.id, item)?;
                }
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
        mut history: ProviderHistoryItem,
    ) -> Result<()> {
        let conversation = self.storage.conversation(conversation_id)?;
        let project = self.storage.project(conversation.project_id)?;
        history.kind = sanitize_history_kind(&project, history.kind)?;
        let item_id = if history
            .provider_item_id
            .starts_with(crate::providers::codex::CANONICAL_ITEM_PREFIX)
        {
            match self.storage.reconcile_provider_item_alias(
                conversation_id,
                &history.provider_item_id,
                &history.kind,
                crate::providers::codex::CANONICAL_ITEM_PREFIX,
            )? {
                Some(item_id) => item_id,
                None => self
                    .storage
                    .provider_item_id(conversation_id, &history.provider_item_id)?,
            }
        } else {
            self.storage
                .provider_item_id(conversation_id, &history.provider_item_id)?
        };
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
        self.save_and_emit_item_locked(&mut cache, item)
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
            self.record_failure(
                &project,
                conversation,
                None,
                "provider_error",
                error.to_string(),
            )?;
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
            if let Some(mut item) = cache.get(&item_id).cloned() {
                item.revision += 1;
                if let TimelineItemKind::Approval {
                    resolved_option, ..
                } = &mut item.kind
                {
                    *resolved_option = Some(option_id);
                }
                self.save_and_emit_item_locked(&mut cache, item)?;
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

    async fn apply_provider_event(self: &Arc<Self>, event: ProviderEvent) -> bool {
        match self.apply_provider_event_inner(event).await {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(error = %error, "provider event was rejected");
                false
            }
        }
    }

    async fn apply_provider_event_inner(self: &Arc<Self>, event: ProviderEvent) -> Result<()> {
        let mut conversation = self.storage.conversation(event.conversation_id)?;
        if conversation.provider != event.provider || conversation.project_id != event.project_id {
            bail!("provider event did not match the conversation authority boundary");
        }
        let project = self.storage.project(event.project_id)?;
        match event.kind {
            ProviderEventKind::HistoryBarrier { barrier } => barrier.complete(),
            ProviderEventKind::HistoryWatermark {
                remote_updated_at_ms,
            } => self.storage.mark_remote_history_synced(
                conversation.provider,
                conversation.project_id,
                &conversation.native_session_id,
                remote_updated_at_ms,
            )?,
            ProviderEventKind::HistoryItem {
                provider_item_id,
                kind,
            } => {
                self.upsert_provider_item(
                    event.conversation_id,
                    provider_item_id,
                    sanitize_history_kind(&project, kind)?,
                )?;
            }
            ProviderEventKind::ProviderItemAlias {
                provider_item_id,
                alias_provider_item_id,
            } => {
                if let Some(item_id) = self.storage.alias_provider_item_id(
                    event.conversation_id,
                    &provider_item_id,
                    &alias_provider_item_id,
                )? {
                    let mut ids = self
                        .provider_item_ids
                        .lock()
                        .expect("provider item mutex poisoned");
                    ids.insert((event.conversation_id, provider_item_id), item_id);
                    ids.insert((event.conversation_id, alias_provider_item_id), item_id);
                }
            }
            ProviderEventKind::UserMessageDelta {
                provider_item_id,
                delta,
            } => {
                let item_id = self.item_id(event.conversation_id, provider_item_id)?;
                let mut cache = self.timeline_cache.lock().expect("timeline mutex poisoned");
                let mut item = cache
                    .get(&item_id)
                    .cloned()
                    .unwrap_or_else(|| TimelineItem {
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
                self.save_and_emit_item_locked(&mut cache, item)?;
            }
            ProviderEventKind::AgentTextDelta {
                provider_item_id,
                phase,
                delta,
            } => {
                let item_id = self.item_id(event.conversation_id, provider_item_id)?;
                let mut cache = self.timeline_cache.lock().expect("timeline mutex poisoned");
                let mut item = cache
                    .get(&item_id)
                    .cloned()
                    .unwrap_or_else(|| TimelineItem {
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
                self.save_and_emit_item_locked(&mut cache, item)?;
            }
            ProviderEventKind::AgentTextSnapshot {
                provider_item_id,
                phase,
                text,
            } => {
                self.upsert_provider_item(
                    event.conversation_id,
                    provider_item_id,
                    TimelineItemKind::AgentMessage { phase, text },
                )?;
            }
            ProviderEventKind::Plan {
                provider_item_id,
                steps,
            } => {
                self.upsert_provider_item(
                    event.conversation_id,
                    provider_item_id,
                    TimelineItemKind::Plan {
                        steps: redact_plan_steps(steps, &project),
                    },
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
                    Err(error) => {
                        self.record_attachment_error(&project, event.conversation_id, error)?
                    }
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
                    Err(error) => {
                        self.record_attachment_error(&project, event.conversation_id, error)?
                    }
                }
            }
            ProviderEventKind::Completed => {
                conversation.state = ConversationState::Completed;
                conversation.revision += 1;
                conversation.updated_at_ms = now_ms();
                self.save_and_emit_conversation(&conversation)?;
                self.spawn_provider_title_refresh(&conversation, project);
            }
            ProviderEventKind::Interrupted => {
                conversation.state = ConversationState::Interrupted;
                conversation.revision += 1;
                conversation.updated_at_ms = now_ms();
                self.save_and_emit_conversation(&conversation)?;
            }
            ProviderEventKind::Failed {
                provider_item_id,
                code,
                message,
            } => {
                self.record_failure(&project, conversation, provider_item_id, &code, message)?;
            }
            ProviderEventKind::Crashed { message } => {
                self.record_failure(&project, conversation, None, "provider_crashed", message)?;
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

    fn spawn_provider_title_refresh(
        self: &Arc<Self>,
        conversation: &Conversation,
        project: Project,
    ) {
        let service = Arc::clone(self);
        let expected = conversation.clone();
        tokio::spawn(async move {
            if let Err(error) = service.refresh_provider_title(&expected, project).await {
                tracing::warn!(
                    conversation_id = %expected.id,
                    error = %error,
                    "provider title refresh failed after the completed turn"
                );
            }
        });
    }

    async fn refresh_provider_title(
        &self,
        expected: &Conversation,
        project: Project,
    ) -> Result<()> {
        let conversation = self.storage.conversation(expected.id)?;
        if conversation.title_source == ConversationTitleSource::User
            || conversation.provider != expected.provider
            || conversation.project_id != project.id
            || conversation.native_session_id != expected.native_session_id
            || conversation.title_updated_at_ms != expected.title_updated_at_ms
            || conversation.title != expected.title
        {
            return Ok(());
        }
        let provider = self.providers.get(expected.provider)?;
        if !provider.capabilities().supports_session_list {
            return Ok(());
        }
        let Some(session) = provider
            .list_sessions(&project)
            .await?
            .into_iter()
            .find(|session| session.native_session_id == expected.native_session_id)
        else {
            return Ok(());
        };
        let title = provider_title(&session.title);
        if title == "新对话" {
            return Ok(());
        }
        if let Some(conversation) = self
            .storage
            .update_provider_title_if_current(expected, &title)?
        {
            self.emit(ServerMessage::ConversationUpserted { conversation });
        }
        Ok(())
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
        self.save_and_emit_item_locked(&mut cache, item)
    }

    fn save_and_emit_item(&self, item: TimelineItem) -> Result<()> {
        let mut cache = self.timeline_cache.lock().expect("timeline mutex poisoned");
        self.save_and_emit_item_locked(&mut cache, item)
    }

    fn save_and_emit_item_locked(
        &self,
        cache: &mut HashMap<TimelineItemId, TimelineItem>,
        item: TimelineItem,
    ) -> Result<()> {
        if cache
            .get(&item.id)
            .is_some_and(|existing| existing.revision >= item.revision)
        {
            return Ok(());
        }
        if self.storage.upsert_timeline_item(&item)? {
            cache.insert(item.id, item.clone());
            self.emit(ServerMessage::TimelineItemUpserted { item });
        }
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
        project: &Project,
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
                message: redact_remote_error(&format!("{error:#}"), Some(project)),
            },
        })
    }

    fn record_failure(
        &self,
        project: &Project,
        mut conversation: Conversation,
        provider_item_id: Option<String>,
        code: &str,
        message: String,
    ) -> Result<()> {
        if conversation.state != ConversationState::Failed {
            conversation.state = ConversationState::Failed;
            conversation.revision += 1;
            conversation.updated_at_ms = now_ms();
            self.save_and_emit_conversation(&conversation)?;
        }
        let kind = TimelineItemKind::Error {
            code: code.to_owned(),
            message: redact_remote_error(&message, Some(project)),
        };
        if let Some(provider_item_id) = provider_item_id {
            self.upsert_provider_item(conversation.id, provider_item_id, kind)
        } else {
            self.save_and_emit_item(TimelineItem {
                id: TimelineItemId::new(),
                conversation_id: conversation.id,
                revision: 1,
                created_at_ms: now_ms(),
                kind,
            })
        }
    }

    fn emit(&self, message: ServerMessage) {
        let _ = self.updates.send(message);
    }
}

fn ordered_conversation_mutation(command: &ClientCommand) -> Option<ConversationId> {
    match command {
        ClientCommand::StartConversation {
            conversation_id, ..
        }
        | ClientCommand::SendMessage {
            conversation_id, ..
        }
        | ClientCommand::Steer {
            conversation_id, ..
        }
        | ClientCommand::SetSessionOption {
            conversation_id, ..
        }
        | ClientCommand::RenameConversation {
            conversation_id, ..
        } => Some(*conversation_id),
        ClientCommand::Pair { .. }
        | ClientCommand::Authenticate { .. }
        | ClientCommand::GetSnapshot
        | ClientCommand::RefreshProjects { .. }
        | ClientCommand::SyncProject { .. }
        | ClientCommand::CreateConversation { .. }
        | ClientCommand::Interrupt { .. }
        | ClientCommand::ResolveApproval { .. }
        | ClientCommand::GetConversationPage { .. }
        | ClientCommand::GetAttachment { .. } => None,
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

fn sanitize_history_kind(project: &Project, kind: TimelineItemKind) -> Result<TimelineItemKind> {
    Ok(match kind {
        TimelineItemKind::Plan { steps } => TimelineItemKind::Plan {
            steps: redact_plan_steps(steps, project),
        },
        TimelineItemKind::Progress {
            kind,
            label,
            status,
            detail,
        } => TimelineItemKind::Progress {
            kind,
            label: redact_project_path(&label, project),
            status,
            detail: detail.map(|value| redact_project_path(&value, project)),
        },
        TimelineItemKind::ToolCall {
            name,
            status,
            input_summary,
            output_summary,
        } => TimelineItemKind::ToolCall {
            name,
            status,
            input_summary: input_summary.map(|value| redact_project_path(&value, project)),
            output_summary: output_summary.map(|value| redact_project_path(&value, project)),
        },
        TimelineItemKind::Command {
            command,
            relative_cwd,
            status,
            exit_code,
            output,
        } => TimelineItemKind::Command {
            command: redact_project_path(&command, project),
            relative_cwd: relative_cwd.and_then(|path| safe_relative_display(project, &path)),
            status,
            exit_code,
            output: output.map(|value| truncate_output(redact_project_path(&value, project))),
        },
        TimelineItemKind::FileChange {
            relative_path,
            change_kind,
            status,
        } => TimelineItemKind::FileChange {
            relative_path: safe_relative_display(project, &relative_path)
                .ok_or_else(|| anyhow!("provider history file change was outside the project"))?,
            change_kind,
            status,
        },
        TimelineItemKind::Approval {
            approval_id,
            prompt,
            options,
            resolved_option,
        } => TimelineItemKind::Approval {
            approval_id,
            prompt: redact_project_path(&prompt, project),
            options,
            resolved_option,
        },
        TimelineItemKind::Error { code, message } => TimelineItemKind::Error {
            code,
            message: redact_remote_error(&message, Some(project)),
        },
        kind => kind,
    })
}

fn redact_plan_steps(
    steps: Vec<agent_remote_protocol::PlanStep>,
    project: &Project,
) -> Vec<agent_remote_protocol::PlanStep> {
    steps
        .into_iter()
        .map(|mut step| {
            step.text = redact_project_path(&step.text, project);
            step
        })
        .collect()
}

fn redact_project_path(value: &str, project: &Project) -> String {
    let canonical = project.canonical_path.to_string_lossy();
    value.replace(canonical.as_ref(), ".")
}

fn redact_remote_error(value: &str, project: Option<&Project>) -> String {
    let value = project.map_or_else(
        || value.to_owned(),
        |project| redact_project_path(value, project),
    );
    value
        .split_inclusive(char::is_whitespace)
        .map(redact_absolute_path_fragment)
        .collect()
}

fn redact_absolute_path_fragment(fragment: &str) -> String {
    let content_len = fragment.trim_end_matches(char::is_whitespace).len();
    let (content, whitespace) = fragment.split_at(content_len);
    let Some(start) = absolute_path_start(content) else {
        return fragment.to_owned();
    };
    let path_end = content
        .trim_end_matches(|character| {
            matches!(character, '"' | '\'' | ')' | ']' | '}' | ',' | ';' | ':')
        })
        .len()
        .max(start);
    format!(
        "{}<host-path>{}{}",
        &content[..start],
        &content[path_end..],
        whitespace
    )
}

fn absolute_path_start(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    for index in 0..bytes.len() {
        let boundary =
            index == 0 || matches!(bytes[index - 1], b'=' | b'(' | b'[' | b'{' | b'"' | b'\'');
        if !boundary {
            continue;
        }
        if bytes[index] == b'/'
            || (bytes[index] == b'\\' && bytes.get(index + 1).is_some_and(|next| *next == b'\\'))
            || (bytes[index].is_ascii_alphabetic()
                && bytes.get(index + 1).is_some_and(|next| *next == b':')
                && bytes
                    .get(index + 2)
                    .is_some_and(|next| matches!(*next, b'/' | b'\\')))
        {
            return Some(index);
        }
    }
    None
}

fn truncate_output(mut output: String) -> String {
    const LIMIT: usize = 64 * 1024;
    if output.len() > LIMIT {
        let mut end = LIMIT;
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        output.truncate(end);
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
        AgentMessagePhase, ApprovalOption, CommandId, EffortOption, ItemStatus, ModelOption,
        PlanStep, ProviderHealth, ProviderState, SessionSummary,
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
        history_read_event: Mutex<Option<ProviderEventKind>>,
        renames: Mutex<Vec<(String, String)>>,
        session_list_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
        session_list_error: Mutex<Option<String>>,
        session_list_started: tokio::sync::Notify,
        session_list_calls: std::sync::atomic::AtomicUsize,
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
                history_read_event: Mutex::new(None),
                renames: Mutex::new(Vec::new()),
                session_list_gate: Mutex::new(None),
                session_list_error: Mutex::new(None),
                session_list_started: tokio::sync::Notify::new(),
                session_list_calls: std::sync::atomic::AtomicUsize::new(0),
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
            self.session_list_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let gate = self
                .session_list_gate
                .lock()
                .expect("session list gate mutex")
                .clone();
            if let Some(gate) = gate {
                self.session_list_started.notify_one();
                gate.acquire().await.expect("session list gate").forget();
            }
            if let Some(error) = self
                .session_list_error
                .lock()
                .expect("session list error mutex")
                .clone()
            {
                bail!(error);
            }
            Ok(self.sessions.lock().expect("sessions mutex").clone())
        }

        async fn read_session_history(
            &self,
            request: ReadSessionHistory,
        ) -> Result<ProviderHistoryPage> {
            if let Some(kind) = self
                .history_read_event
                .lock()
                .expect("history read event mutex")
                .take()
            {
                self.events
                    .send(ProviderEvent {
                        provider: ProviderId::Codex,
                        project_id: request.project.id,
                        conversation_id: request.conversation_id,
                        kind,
                    })
                    .map_err(|_| anyhow!("mock history event has no active event pump"))?;
            }
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

        async fn flush_history_events(
            &self,
            project_id: ProjectId,
            conversation_id: ConversationId,
        ) -> Result<()> {
            let barrier = Arc::new(crate::providers::ProviderHistoryBarrier::default());
            self.events
                .send(ProviderEvent {
                    provider: ProviderId::Codex,
                    project_id,
                    conversation_id,
                    kind: ProviderEventKind::HistoryBarrier {
                        barrier: Arc::clone(&barrier),
                    },
                })
                .map_err(|_| anyhow!("mock history barrier has no active event pump"))?;
            barrier.wait().await
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
    async fn successful_duplicate_command_replays_the_original_response() {
        let fixture = fixture();
        let pairing = fixture
            .service
            .storage
            .create_pairing_token()
            .expect("pairing token");
        let device = fixture
            .service
            .storage
            .exchange_pairing_token(&pairing.token, "phone")
            .expect("paired device");
        let command_id = CommandId::new();
        let command = ClientCommand::SyncProject {
            command_id,
            project_id: fixture.project_a.id,
            provider: ProviderId::Codex,
        };

        let first = fixture
            .service
            .execute_command(device.id, command.clone())
            .await
            .expect("first command");
        let duplicate = fixture
            .service
            .execute_command(device.id, command)
            .await
            .expect("duplicate command");

        assert!(matches!(
            first,
            ServerMessage::ProjectSyncCompleted {
                command_id: completed_id,
                ..
            } if completed_id == command_id
        ));
        assert_eq!(duplicate, first);
    }

    #[tokio::test]
    async fn different_command_ids_execute_while_a_duplicate_waits() {
        let fixture = fixture();
        let pairing = fixture
            .service
            .storage
            .create_pairing_token()
            .expect("pairing token");
        let device = fixture
            .service
            .storage
            .exchange_pairing_token(&pairing.token, "phone")
            .expect("paired device");
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *fixture
            .provider
            .session_list_gate
            .lock()
            .expect("session list gate mutex") = Some(Arc::clone(&gate));

        let sync_id = CommandId::new();
        let sync = ClientCommand::SyncProject {
            command_id: sync_id,
            project_id: fixture.project_a.id,
            provider: ProviderId::Codex,
        };
        let service = Arc::clone(&fixture.service);
        let first_command = sync.clone();
        let first =
            tokio::spawn(async move { service.execute_command(device.id, first_command).await });
        fixture.provider.session_list_started.notified().await;

        let service = Arc::clone(&fixture.service);
        let duplicate = tokio::spawn(async move { service.execute_command(device.id, sync).await });
        tokio::task::yield_now().await;

        let unrelated_id = CommandId::new();
        let unrelated = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            fixture.service.execute_command(
                device.id,
                ClientCommand::CreateConversation {
                    command_id: unrelated_id,
                    project_id: fixture.project_a.id,
                    provider: ProviderId::Codex,
                    native_session_id: None,
                    model: Some("dynamic-model".to_owned()),
                    effort: Some("high".to_owned()),
                },
            ),
        )
        .await
        .expect("unrelated command was not blocked")
        .expect("unrelated command");
        assert_eq!(
            unrelated,
            ServerMessage::CommandAccepted {
                command_id: unrelated_id
            }
        );

        gate.add_permits(1);
        let first = first.await.expect("first task").expect("first response");
        let duplicate = duplicate
            .await
            .expect("duplicate task")
            .expect("duplicate response");
        assert!(matches!(
            first,
            ServerMessage::ProjectSyncCompleted {
                command_id: completed_id,
                ..
            } if completed_id == sync_id
        ));
        assert_eq!(duplicate, first);
        assert_eq!(
            fixture
                .provider
                .session_list_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert!(
            fixture
                .service
                .command_execution_locks
                .lock()
                .expect("command lock mutex")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn concurrent_failed_duplicate_replays_the_original_rejection() {
        let fixture = fixture();
        let pairing = fixture
            .service
            .storage
            .create_pairing_token()
            .expect("pairing token");
        let device = fixture
            .service
            .storage
            .exchange_pairing_token(&pairing.token, "phone")
            .expect("paired device");
        let command_id = CommandId::new();
        let command = ClientCommand::SyncProject {
            command_id,
            project_id: ProjectId::new(),
            provider: ProviderId::Codex,
        };

        let (first, duplicate) = tokio::join!(
            fixture.service.execute_command(device.id, command.clone()),
            fixture.service.execute_command(device.id, command),
        );
        let first = first.expect("first response");
        let duplicate = duplicate.expect("duplicate response");

        assert!(matches!(
            &first,
            ServerMessage::CommandRejected {
                command_id: Some(rejected_id),
                code,
                ..
            } if *rejected_id == command_id && code == "command_failed"
        ));
        assert_eq!(duplicate, first);
    }

    #[tokio::test]
    async fn command_rejection_redacts_provider_host_path_without_hiding_error() {
        let fixture = fixture();
        let hidden_path = fixture.project_b.canonical_path.join("provider.log");
        *fixture
            .provider
            .session_list_error
            .lock()
            .expect("session list error mutex") = Some(format!(
            "provider failed while reading {}",
            hidden_path.display()
        ));
        let pairing = fixture
            .service
            .storage
            .create_pairing_token()
            .expect("pairing token");
        let device = fixture
            .service
            .storage
            .exchange_pairing_token(&pairing.token, "phone")
            .expect("paired device");
        let command_id = CommandId::new();

        let response = fixture
            .service
            .execute_command(
                device.id,
                ClientCommand::SyncProject {
                    command_id,
                    project_id: fixture.project_a.id,
                    provider: ProviderId::Codex,
                },
            )
            .await
            .expect("command response");
        let ServerMessage::CommandRejected { message, .. } = response else {
            panic!("expected command rejection");
        };
        assert!(message.contains("provider failed while reading"));
        assert!(message.contains("<host-path>"));
        assert!(!message.contains(hidden_path.to_string_lossy().as_ref()));
    }

    #[tokio::test]
    async fn pending_command_is_finalized_as_unknown_without_reexecution() {
        let fixture = fixture();
        let pairing = fixture
            .service
            .storage
            .create_pairing_token()
            .expect("pairing token");
        let device = fixture
            .service
            .storage
            .exchange_pairing_token(&pairing.token, "phone")
            .expect("paired device");
        let command_id = CommandId::new();
        fixture
            .service
            .storage
            .begin_command(device.id, command_id)
            .expect("record pending command");
        let command = ClientCommand::SyncProject {
            command_id,
            project_id: fixture.project_a.id,
            provider: ProviderId::Codex,
        };

        let first = fixture
            .service
            .execute_command(device.id, command.clone())
            .await
            .expect("pending response");
        let duplicate = fixture
            .service
            .execute_command(device.id, command)
            .await
            .expect("duplicate response");

        assert!(matches!(
            &first,
            ServerMessage::CommandRejected {
                command_id: Some(rejected_id),
                code,
                ..
            } if *rejected_id == command_id && code == "command_outcome_unknown"
        ));
        assert_eq!(duplicate, first);
        assert_eq!(
            fixture
                .service
                .storage
                .command_state(device.id, command_id)
                .expect("stored result"),
            StoredCommand::Complete(Box::new(first))
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
    async fn stale_timeline_revision_cannot_replace_memory_or_reopened_storage() {
        let fixture = fixture();
        let conversation = create(&fixture.service, fixture.project_a.id).await;
        let item_id = TimelineItemId::new();
        let newer = TimelineItem {
            id: item_id,
            conversation_id: conversation.id,
            revision: 2,
            created_at_ms: 20,
            kind: TimelineItemKind::AgentMessage {
                phase: AgentMessagePhase::Final,
                text: "newer".to_owned(),
            },
        };
        let older = TimelineItem {
            revision: 1,
            created_at_ms: 10,
            kind: TimelineItemKind::AgentMessage {
                phase: AgentMessagePhase::Final,
                text: "older".to_owned(),
            },
            ..newer.clone()
        };
        let mut updates = fixture.service.subscribe();

        fixture
            .service
            .save_and_emit_item(newer.clone())
            .expect("save newer revision");
        assert!(matches!(
            updates.try_recv().expect("newer update"),
            ServerMessage::TimelineItemUpserted { item } if item == newer
        ));
        assert!(
            !fixture
                .service
                .storage
                .upsert_timeline_item(&older)
                .expect("reject stale database write")
        );
        fixture
            .service
            .save_and_emit_item(older)
            .expect("ignore stale cache write");
        assert!(matches!(
            updates.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        assert_eq!(
            fixture
                .service
                .timeline_cache
                .lock()
                .expect("timeline mutex")[&item_id],
            newer
        );

        let reopened = Storage::open(fixture._temp.path().join("state.db")).expect("reopen");
        let stored = reopened
            .list_timeline()
            .expect("timeline after reopen")
            .into_iter()
            .find(|item| item.id == item_id)
            .expect("stored item");
        assert_eq!(stored, newer);
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
    async fn delayed_provider_title_refresh_does_not_block_or_override_manual_title() {
        let fixture = fixture();
        fixture.service.start_provider_event_pumps();
        let conversation = create(&fixture.service, fixture.project_a.id).await;
        fixture
            .service
            .send_message(
                conversation.id,
                "first message".to_owned(),
                Vec::new(),
                None,
            )
            .await
            .expect("first message");
        fixture
            .provider
            .sessions
            .lock()
            .expect("sessions mutex")
            .push(SessionSummary {
                native_session_id: conversation.native_session_id.clone(),
                title: "Provider title".to_owned(),
                updated_at_ms: 50,
            });
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *fixture
            .provider
            .session_list_gate
            .lock()
            .expect("session list gate mutex") = Some(Arc::clone(&gate));

        fixture
            .provider
            .event(conversation.id, ProviderEventKind::Completed);
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            fixture.provider.session_list_started.notified(),
        )
        .await
        .expect("title refresh did not start");
        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            fixture
                .provider
                .flush_history_events(fixture.project_a.id, conversation.id),
        )
        .await
        .expect("title refresh blocked the provider event pump")
        .expect("history barrier");
        fixture
            .service
            .rename_conversation(conversation.id, "Manual title".to_owned())
            .await
            .expect("manual rename");
        let mut updates = fixture.service.subscribe();
        gate.add_permits(1);

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), updates.recv())
                .await
                .is_err(),
            "stale provider title refresh emitted an update"
        );
        let conversation = fixture
            .service
            .storage
            .conversation(conversation.id)
            .expect("conversation");
        assert_eq!(conversation.title, "Manual title");
        assert_eq!(conversation.title_source, ConversationTitleSource::User);
        assert_eq!(conversation.state, ConversationState::Completed);
    }

    #[tokio::test]
    async fn delayed_provider_title_refresh_preserves_a_new_running_turn() {
        let fixture = fixture();
        fixture.service.start_provider_event_pumps();
        let conversation = create(&fixture.service, fixture.project_a.id).await;
        fixture
            .service
            .send_message(
                conversation.id,
                "first message".to_owned(),
                Vec::new(),
                None,
            )
            .await
            .expect("first message");
        fixture
            .provider
            .sessions
            .lock()
            .expect("sessions mutex")
            .push(SessionSummary {
                native_session_id: conversation.native_session_id.clone(),
                title: "Provider title".to_owned(),
                updated_at_ms: 50,
            });
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *fixture
            .provider
            .session_list_gate
            .lock()
            .expect("session list gate mutex") = Some(Arc::clone(&gate));

        fixture
            .provider
            .event(conversation.id, ProviderEventKind::Completed);
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            fixture.provider.session_list_started.notified(),
        )
        .await
        .expect("title refresh did not start");
        fixture
            .service
            .send_message(
                conversation.id,
                "second message".to_owned(),
                Vec::new(),
                None,
            )
            .await
            .expect("next turn");
        let mut updates = fixture.service.subscribe();
        gate.add_permits(1);
        let conversation_id = conversation.id;

        let refreshed = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if let ServerMessage::ConversationUpserted {
                    conversation: updated,
                } = updates.recv().await.expect("conversation update")
                    && updated.id == conversation_id
                    && updated.title == "Provider title"
                {
                    break updated;
                }
            }
        })
        .await
        .expect("provider title was not refreshed");
        assert_eq!(refreshed.state, ConversationState::Running);
        assert_eq!(refreshed.title_source, ConversationTitleSource::Provider);
        let stored = fixture
            .service
            .storage
            .conversation(conversation.id)
            .expect("conversation");
        assert_eq!(stored.state, ConversationState::Running);
        assert_eq!(stored.title, "Provider title");
    }

    #[tokio::test]
    async fn history_barriers_recover_the_event_pump_after_a_rejected_event() {
        let fixture = fixture();
        fixture.service.start_provider_event_pumps();
        let conversation = create(&fixture.service, fixture.project_a.id).await;
        let _ = fixture.provider.events.send(ProviderEvent {
            provider: ProviderId::Codex,
            project_id: fixture.project_b.id,
            conversation_id: conversation.id,
            kind: ProviderEventKind::Completed,
        });

        fixture.provider.event(
            conversation.id,
            ProviderEventKind::HistoryWatermark {
                remote_updated_at_ms: 120,
            },
        );

        assert!(
            fixture
                .provider
                .flush_history_events(fixture.project_a.id, conversation.id)
                .await
                .is_err()
        );

        assert!(
            fixture
                .service
                .storage
                .remote_history_is_stale(
                    ProviderId::Codex,
                    fixture.project_a.id,
                    &conversation.native_session_id,
                    120,
                )
                .expect("history remains stale")
        );
        assert_eq!(
            fixture
                .service
                .storage
                .conversation(conversation.id)
                .expect("conversation")
                .state,
            ConversationState::Idle
        );

        fixture.provider.event(
            conversation.id,
            ProviderEventKind::HistoryWatermark {
                remote_updated_at_ms: 120,
            },
        );
        fixture
            .provider
            .flush_history_events(fixture.project_a.id, conversation.id)
            .await
            .expect("healthy history barrier");

        assert!(
            !fixture
                .service
                .storage
                .remote_history_is_stale(
                    ProviderId::Codex,
                    fixture.project_a.id,
                    &conversation.native_session_id,
                    120,
                )
                .expect("history watermark advances after recovery")
        );
    }

    #[tokio::test]
    async fn history_lag_is_scoped_to_the_rejected_conversation() {
        let fixture = fixture();
        fixture.service.start_provider_event_pumps();
        let conversation_a = create(&fixture.service, fixture.project_a.id).await;
        let conversation_b = create(&fixture.service, fixture.project_b.id).await;
        let _ = fixture.provider.events.send(ProviderEvent {
            provider: ProviderId::Codex,
            project_id: fixture.project_b.id,
            conversation_id: conversation_a.id,
            kind: ProviderEventKind::Completed,
        });

        fixture
            .provider
            .flush_history_events(fixture.project_b.id, conversation_b.id)
            .await
            .expect("conversation B remains healthy");
        fixture.provider.event(
            conversation_a.id,
            ProviderEventKind::HistoryWatermark {
                remote_updated_at_ms: 120,
            },
        );
        assert!(
            fixture
                .provider
                .flush_history_events(fixture.project_a.id, conversation_a.id)
                .await
                .is_err()
        );

        assert!(
            fixture
                .service
                .storage
                .remote_history_is_stale(
                    ProviderId::Codex,
                    fixture.project_a.id,
                    &conversation_a.native_session_id,
                    120,
                )
                .expect("conversation A history remains stale")
        );
    }

    #[tokio::test]
    async fn canonical_history_id_reuses_a_matching_legacy_provider_item() {
        let fixture = fixture();
        let conversation = create(&fixture.service, fixture.project_a.id).await;
        let legacy_kind = TimelineItemKind::AgentMessage {
            phase: AgentMessagePhase::Final,
            text: "do".to_owned(),
        };
        fixture
            .service
            .upsert_provider_item(conversation.id, "msg_live_1".to_owned(), legacy_kind)
            .expect("legacy live item");
        let legacy_item_id = fixture
            .service
            .storage
            .list_timeline()
            .expect("legacy timeline")
            .into_iter()
            .next()
            .expect("legacy item")
            .id;

        fixture
            .service
            .upsert_history_item(
                conversation.id,
                ProviderHistoryItem {
                    provider_item_id: "codex:v1:turn-1:agent:0".to_owned(),
                    created_at_ms: 10,
                    kind: TimelineItemKind::AgentMessage {
                        phase: AgentMessagePhase::Final,
                        text: "done".to_owned(),
                    },
                },
            )
            .expect("canonical history item");

        let timeline = fixture
            .service
            .storage
            .list_timeline()
            .expect("reconciled timeline");
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].id, legacy_item_id);
        assert!(matches!(
            &timeline[0].kind,
            TimelineItemKind::AgentMessage { text, .. } if text == "done"
        ));
        assert_eq!(
            fixture
                .service
                .storage
                .provider_item_id(conversation.id, "codex:v1:turn-1:agent:0")
                .expect("canonical alias"),
            legacy_item_id
        );
    }

    #[tokio::test]
    async fn canonical_subitem_aliases_reuse_reasoning_and_file_timeline_items() {
        let fixture = fixture();
        let conversation = create(&fixture.service, fixture.project_a.id).await;
        let provisional_reasoning = "codex:live:turn-1:reasoning:reason-live:summary:0".to_owned();
        let canonical_reasoning = "codex:v1:turn-1:reasoning:0:summary:0".to_owned();
        let provisional_file = "codex:live:turn-1:file:file-live:change:0".to_owned();
        let canonical_file = "codex:v1:turn-1:file:0:change:0".to_owned();
        fixture
            .service
            .upsert_provider_item(
                conversation.id,
                provisional_reasoning.clone(),
                TimelineItemKind::AgentMessage {
                    phase: AgentMessagePhase::ReasoningSummary,
                    text: "check".to_owned(),
                },
            )
            .expect("provisional reasoning");
        fixture
            .service
            .upsert_provider_item(
                conversation.id,
                provisional_file.clone(),
                TimelineItemKind::FileChange {
                    relative_path: "src/lib.rs".to_owned(),
                    change_kind: "update".to_owned(),
                    status: ItemStatus::Running,
                },
            )
            .expect("provisional file");

        for (provider_item_id, alias_provider_item_id) in [
            (canonical_reasoning.clone(), provisional_reasoning),
            (canonical_file.clone(), provisional_file),
        ] {
            fixture
                .service
                .apply_provider_event_inner(ProviderEvent {
                    provider: ProviderId::Codex,
                    project_id: fixture.project_a.id,
                    conversation_id: conversation.id,
                    kind: ProviderEventKind::ProviderItemAlias {
                        provider_item_id,
                        alias_provider_item_id,
                    },
                })
                .await
                .expect("apply canonical alias");
        }
        fixture
            .service
            .apply_provider_event_inner(ProviderEvent {
                provider: ProviderId::Codex,
                project_id: fixture.project_a.id,
                conversation_id: conversation.id,
                kind: ProviderEventKind::AgentTextSnapshot {
                    provider_item_id: canonical_reasoning,
                    phase: AgentMessagePhase::ReasoningSummary,
                    text: "check complete".to_owned(),
                },
            })
            .await
            .expect("final reasoning");
        fixture
            .service
            .apply_provider_event_inner(ProviderEvent {
                provider: ProviderId::Codex,
                project_id: fixture.project_a.id,
                conversation_id: conversation.id,
                kind: ProviderEventKind::FileChange {
                    provider_item_id: canonical_file,
                    relative_path: "src/lib.rs".to_owned(),
                    change_kind: "update".to_owned(),
                    status: ItemStatus::Completed,
                },
            })
            .await
            .expect("final file");

        let timeline = fixture
            .service
            .storage
            .list_timeline()
            .expect("canonical timeline");
        assert_eq!(timeline.len(), 2);
        assert!(timeline.iter().any(|item| matches!(
            &item.kind,
            TimelineItemKind::AgentMessage { text, .. } if text == "check complete"
        )));
        assert!(timeline.iter().any(|item| matches!(
            &item.kind,
            TimelineItemKind::FileChange {
                status: ItemStatus::Completed,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn history_then_live_text_snapshot_does_not_append_duplicate_text() {
        let fixture = fixture();
        let conversation = create(&fixture.service, fixture.project_a.id).await;
        let provider_item_id = "codex:v1:turn-1:agent:0".to_owned();
        fixture
            .service
            .upsert_history_item(
                conversation.id,
                ProviderHistoryItem {
                    provider_item_id: provider_item_id.clone(),
                    created_at_ms: 10,
                    kind: TimelineItemKind::AgentMessage {
                        phase: AgentMessagePhase::Final,
                        text: "hello".to_owned(),
                    },
                },
            )
            .expect("history snapshot");
        fixture
            .service
            .apply_provider_event_inner(ProviderEvent {
                provider: ProviderId::Codex,
                project_id: fixture.project_a.id,
                conversation_id: conversation.id,
                kind: ProviderEventKind::AgentTextSnapshot {
                    provider_item_id,
                    phase: AgentMessagePhase::Final,
                    text: "hello".to_owned(),
                },
            })
            .await
            .expect("live snapshot");

        let timeline = fixture.service.storage.list_timeline().expect("timeline");
        assert_eq!(timeline.len(), 1);
        assert!(matches!(
            &timeline[0].kind,
            TimelineItemKind::AgentMessage { text, .. } if text == "hello"
        ));
    }

    #[tokio::test]
    async fn sync_drains_queued_live_items_before_writing_thread_history() {
        let fixture = fixture();
        fixture.service.start_provider_event_pumps();
        fixture
            .provider
            .sessions
            .lock()
            .expect("sessions mutex")
            .push(SessionSummary {
                native_session_id: "remote-barrier".to_owned(),
                title: "Barrier".to_owned(),
                updated_at_ms: 50,
            });
        fixture
            .provider
            .history
            .lock()
            .expect("history mutex")
            .insert(
                "remote-barrier".to_owned(),
                vec![ProviderHistoryItem {
                    provider_item_id: "codex:v1:turn-1:agent:0".to_owned(),
                    created_at_ms: 25,
                    kind: TimelineItemKind::AgentMessage {
                        phase: AgentMessagePhase::Final,
                        text: "hello".to_owned(),
                    },
                }],
            );
        *fixture
            .provider
            .history_read_event
            .lock()
            .expect("history read event mutex") = Some(ProviderEventKind::AgentTextSnapshot {
            provider_item_id: "codex:live:turn-1:agent:msg-live".to_owned(),
            phase: AgentMessagePhase::Final,
            text: "hel".to_owned(),
        });

        fixture
            .service
            .sync_project(CommandId::new(), fixture.project_a.id, ProviderId::Codex)
            .await
            .expect("sync with queued live event");
        let conversation = fixture
            .service
            .storage
            .conversation_by_native_session(
                ProviderId::Codex,
                fixture.project_a.id,
                "remote-barrier",
            )
            .expect("conversation lookup")
            .expect("synced conversation");
        let timeline = fixture
            .service
            .storage
            .list_timeline()
            .expect("reconciled timeline");
        assert_eq!(timeline.len(), 1);
        assert!(matches!(
            &timeline[0].kind,
            TimelineItemKind::AgentMessage { text, .. } if text == "hello"
        ));
        assert_eq!(
            fixture
                .service
                .storage
                .provider_item_id(conversation.id, "codex:v1:turn-1:agent:0")
                .expect("canonical item"),
            fixture
                .service
                .storage
                .provider_item_id(conversation.id, "codex:live:turn-1:agent:msg-live")
                .expect("live item")
        );
    }

    #[tokio::test]
    async fn history_items_use_live_path_redaction_truncation_and_path_rules() {
        let fixture = fixture();
        let conversation = create(&fixture.service, fixture.project_a.id).await;
        let project_path = fixture.project_a.canonical_path.to_string_lossy();
        let command_output = format!("{project_path}\n{}", "你".repeat(24 * 1024));
        fixture
            .service
            .apply_provider_event_inner(ProviderEvent {
                provider: ProviderId::Codex,
                project_id: fixture.project_a.id,
                conversation_id: conversation.id,
                kind: ProviderEventKind::HistoryItem {
                    provider_item_id: "history-command".to_owned(),
                    kind: TimelineItemKind::Command {
                        command: format!("cat {project_path}/src/lib.rs"),
                        relative_cwd: Some(format!("{project_path}/src")),
                        status: ItemStatus::Completed,
                        exit_code: Some(0),
                        output: Some(command_output),
                    },
                },
            })
            .await
            .expect("sanitized history command");
        fixture
            .service
            .upsert_history_item(
                conversation.id,
                ProviderHistoryItem {
                    provider_item_id: "history-file".to_owned(),
                    created_at_ms: 10,
                    kind: TimelineItemKind::FileChange {
                        relative_path: format!("{project_path}/src/lib.rs"),
                        change_kind: "update".to_owned(),
                        status: ItemStatus::Completed,
                    },
                },
            )
            .expect("sanitized history file");
        fixture
            .service
            .apply_provider_event_inner(ProviderEvent {
                provider: ProviderId::Codex,
                project_id: fixture.project_a.id,
                conversation_id: conversation.id,
                kind: ProviderEventKind::Plan {
                    provider_item_id: "live-plan".to_owned(),
                    steps: vec![PlanStep {
                        text: format!("inspect {project_path}/src/live.rs"),
                        status: ItemStatus::Running,
                    }],
                },
            })
            .await
            .expect("sanitized live plan");
        fixture
            .service
            .upsert_history_item(
                conversation.id,
                ProviderHistoryItem {
                    provider_item_id: "history-plan".to_owned(),
                    created_at_ms: 11,
                    kind: TimelineItemKind::Plan {
                        steps: vec![PlanStep {
                            text: format!("test {project_path}/src/history.rs"),
                            status: ItemStatus::Completed,
                        }],
                    },
                },
            )
            .expect("sanitized history plan");

        let timeline = fixture
            .service
            .storage
            .list_timeline()
            .expect("sanitized timeline");
        assert_eq!(timeline.len(), 4);
        assert!(timeline.iter().any(|item| matches!(
            &item.kind,
            TimelineItemKind::Command {
                command,
                relative_cwd: Some(relative_cwd),
                output: Some(output),
                ..
            } if command == "cat ./src/lib.rs"
                && relative_cwd == "src"
                && !output.contains(project_path.as_ref())
                && output.ends_with("[output truncated at 64 KiB]")
        )));
        assert!(timeline.iter().any(|item| matches!(
            &item.kind,
            TimelineItemKind::FileChange { relative_path, .. }
                if relative_path == "src/lib.rs"
        )));
        assert!(timeline.iter().any(|item| matches!(
            &item.kind,
            TimelineItemKind::Plan { steps }
                if steps[0].text == "inspect ./src/live.rs"
        )));
        assert!(timeline.iter().any(|item| matches!(
            &item.kind,
            TimelineItemKind::Plan { steps }
                if steps[0].text == "test ./src/history.rs"
        )));

        let outside = fixture.project_b.canonical_path.join("outside.rs");
        let result = fixture
            .service
            .apply_provider_event_inner(ProviderEvent {
                provider: ProviderId::Codex,
                project_id: fixture.project_a.id,
                conversation_id: conversation.id,
                kind: ProviderEventKind::HistoryItem {
                    provider_item_id: "history-outside-file".to_owned(),
                    kind: TimelineItemKind::FileChange {
                        relative_path: outside.to_string_lossy().into_owned(),
                        change_kind: "update".to_owned(),
                        status: ItemStatus::Completed,
                    },
                },
            })
            .await;
        assert!(result.is_err());
        assert_eq!(
            fixture
                .service
                .storage
                .list_timeline()
                .expect("outside path was not stored")
                .len(),
            4
        );
    }

    #[tokio::test]
    async fn attachment_error_redacts_host_path_without_hiding_cause() {
        let fixture = fixture();
        let conversation = create(&fixture.service, fixture.project_a.id).await;
        let hidden_path = fixture.project_b.canonical_path.join("missing image.png");

        fixture
            .service
            .apply_provider_event_inner(ProviderEvent {
                provider: ProviderId::Codex,
                project_id: fixture.project_a.id,
                conversation_id: conversation.id,
                kind: ProviderEventKind::ImagePath {
                    provider_item_id: Some("missing-image".to_owned()),
                    path: hidden_path.clone(),
                    controlled_temp_roots: Vec::new(),
                    alt: "missing".to_owned(),
                },
            })
            .await
            .expect("record attachment error");

        let error = fixture
            .service
            .storage
            .list_timeline()
            .expect("timeline")
            .into_iter()
            .find_map(|item| match item.kind {
                TimelineItemKind::Error { code, message } if code == "attachment_error" => {
                    Some(message)
                }
                _ => None,
            })
            .expect("attachment error");
        assert!(error.contains("image path does not exist"));
        assert!(error.contains("<host-path>"));
        assert!(!error.contains(hidden_path.to_string_lossy().as_ref()));
    }

    #[tokio::test]
    async fn repeated_stable_failure_updates_one_error_item() {
        let fixture = fixture();
        let conversation = create(&fixture.service, fixture.project_a.id).await;
        for (code, message) in [
            ("codex_error", "early failure"),
            ("codex_turn_failed", "terminal failure"),
        ] {
            fixture
                .service
                .apply_provider_event_inner(ProviderEvent {
                    provider: ProviderId::Codex,
                    project_id: fixture.project_a.id,
                    conversation_id: conversation.id,
                    kind: ProviderEventKind::Failed {
                        provider_item_id: Some("codex:v1:turn-1:failure".to_owned()),
                        code: code.to_owned(),
                        message: message.to_owned(),
                    },
                })
                .await
                .expect("apply failure");
        }

        let timeline = fixture
            .service
            .storage
            .list_timeline()
            .expect("failure timeline");
        assert_eq!(timeline.len(), 1);
        assert!(matches!(
            &timeline[0].kind,
            TimelineItemKind::Error { code, message }
                if code == "codex_turn_failed" && message == "terminal failure"
        ));
        assert_eq!(
            fixture
                .service
                .storage
                .conversation(conversation.id)
                .expect("failed conversation")
                .state,
            ConversationState::Failed
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
        fixture.service.start_provider_event_pumps();
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
