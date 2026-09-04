use std::collections::{HashMap, HashSet};

use agent_remote_protocol::{
    ApprovalId, AttachmentId, ClientAttachment, ClientCommand, CommandId, Conversation,
    ConversationId, ConversationState, DeviceId, HostId, PermissionRisk, ProjectId,
    ProviderCapability, ProviderId, ProviderState, SendTraceStage, ServerMessage, Snapshot,
    TimelineItem, TimelineItemId, TimelineItemKind, TimelinePageCursor, decode, encode,
};
use js_sys::{Array, ArrayBuffer, Math, Uint8Array};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{
    BinaryType, Blob, ClipboardEvent, CloseEvent, DataTransfer, DragEvent, Event, File, FileReader,
    HtmlInputElement, HtmlSelectElement, HtmlTextAreaElement, MessageEvent, Url, UrlSearchParams,
    WebSocket, window,
};
use yew::{Component, Context, Html, InputEvent, MouseEvent, TargetCast, classes, html};

use crate::{
    conversation_belongs_to_project, increment_send_attempt, markdown_to_safe_html,
    retryable_send_rejection, sort_conversations_newest_first,
};

const CREDENTIALS_KEY: &str = "agent_remote_credentials_v1";
const LAST_HOST_KEY: &str = "agent_remote_last_host_v2";
const CACHE_PREFIX: &str = "agent_remote_cache_v2_";
const WS_SUBPROTOCOL: &str = "agent-remote.cbor.v2";
const MAX_RECONNECT_ATTEMPTS: u8 = 6;
const CACHE_VERSION: u16 = 3;
const CACHE_WRITE_DELAY_MS: i32 = 250;

type TimelineIndex = HashMap<ConversationId, Vec<TimelineItem>>;
type MarkdownRenderCache = HashMap<TimelineItemId, (u64, Html)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct ProjectTreeScope {
    host_id: HostId,
    provider: ProviderId,
    project_id: ProjectId,
}

#[derive(Clone, Copy)]
struct ProjectTreeMetadata {
    conversation_count: usize,
    last_activity_at_ms: Option<i64>,
    status_label: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PendingSendState {
    Queued,
    AwaitingAck,
    WriteFailed,
    Rejected,
}

#[derive(Debug, Clone)]
struct PendingSend {
    command_id: CommandId,
    client_message_id: String,
    command: ClientCommand,
    state: PendingSendState,
    error: Option<String>,
    rejection_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredCredential {
    host_id: HostId,
    device_id: DeviceId,
    device_token: String,
    origin: String,
    relay: bool,
}

#[derive(Debug, Clone)]
struct ConnectionConfig {
    host_id: HostId,
    pair_token: Option<String>,
    credential: Option<StoredCredential>,
    origin: String,
    relay: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppCache {
    version: u16,
    host_id: HostId,
    snapshot: Snapshot,
    selected_conversation: Option<ConversationId>,
    selected_project: Option<ProjectId>,
    selected_provider: ProviderId,
    selected_model: Option<String>,
    selected_effort: Option<String>,
    selected_permission: Option<String>,
    #[serde(default)]
    draft_conversation: Option<ConversationId>,
    #[serde(default)]
    pending_command: Option<ClientCommand>,
    composer: String,
    sidebar_collapsed: bool,
    pinned_projects: Vec<ProjectId>,
    recent_projects: Vec<ProjectId>,
    #[serde(default)]
    expanded_projects: Vec<ProjectTreeScope>,
    #[serde(default)]
    pending_send_state: Option<PendingSendState>,
    #[serde(default)]
    pending_rejection_code: Option<String>,
}

#[derive(Debug, Clone)]
struct BrowserAttachment {
    id: AttachmentId,
    file_name: String,
    mime_type: String,
    byte_len: u64,
    bytes: Option<Vec<u8>>,
    error: Option<String>,
}

struct SocketCallbacks {
    _open: Closure<dyn FnMut(Event)>,
    _message: Closure<dyn FnMut(MessageEvent)>,
    _error: Closure<dyn FnMut(Event)>,
    _close: Closure<dyn FnMut(CloseEvent)>,
}

pub struct App {
    socket: Option<WebSocket>,
    _socket_callbacks: Option<SocketCallbacks>,
    connection: Option<ConnectionConfig>,
    credentials: Vec<StoredCredential>,
    snapshot: Option<Snapshot>,
    timeline_by_conversation: TimelineIndex,
    markdown_render_cache: MarkdownRenderCache,
    connected: bool,
    authenticated: bool,
    connection_generation: u64,
    reconnect_attempt: u8,
    reconnect_timer: Option<i32>,
    retry_enabled: bool,
    manually_disconnected: bool,
    _online_callback: Option<Closure<dyn FnMut(Event)>>,
    _flush_callback: Option<Closure<dyn FnMut(Event)>>,
    status: String,
    pair_link: String,
    selected_conversation: Option<ConversationId>,
    selected_project: Option<ProjectId>,
    selected_provider: ProviderId,
    selected_model: Option<String>,
    selected_effort: Option<String>,
    selected_permission: Option<String>,
    draft_conversation: Option<ConversationId>,
    pending_send: Option<PendingSend>,
    composer: String,
    pending_attachments: Vec<BrowserAttachment>,
    attachments: HashMap<AttachmentId, String>,
    fullscreen_image: Option<AttachmentId>,
    conversation_search: String,
    project_picker_open: bool,
    project_search: String,
    pinned_projects: Vec<ProjectId>,
    recent_projects: Vec<ProjectId>,
    expanded_projects: HashSet<ProjectTreeScope>,
    sidebar_collapsed: bool,
    sidebar_open: bool,
    history_before: HashMap<ConversationId, TimelinePageCursor>,
    history_exhausted: HashSet<ConversationId>,
    history_requested: HashSet<ConversationId>,
    editing_title: bool,
    title_draft: String,
    sync_in_flight: HashMap<CommandId, ProjectTreeScope>,
    refresh_in_flight: HashSet<ProviderId>,
    cache_persist_timer: Option<i32>,
    cache_persist_epoch: u64,
    send_started_at: HashMap<CommandId, f64>,
}

pub enum Msg {
    Opened(u64),
    Closed(u64, String),
    SocketError(u64),
    Reconnect(u64),
    RetryNow,
    StopRetrying,
    NetworkOnline,
    Disconnect,
    Server(u64, ServerMessage),
    DecodeError(u64, String),
    PairLinkChanged(String),
    OpenPairLink,
    ConnectStored(usize),
    ForgetCredentials,
    SelectConversation(ConversationId),
    SelectProject(ProjectId),
    ToggleProject(ProjectId),
    SelectProvider(ProviderId),
    SelectModel(Option<String>),
    SelectEffort(Option<String>),
    SelectPermission(Option<String>),
    NewConversation,
    ToggleProjectPicker,
    ProjectSearchChanged(String),
    ToggleProjectPin(ProjectId),
    ConversationSearchChanged(String),
    ToggleSidebar,
    OpenSidebar,
    CloseSidebar,
    ComposerChanged(String),
    FilesSelected(Vec<File>),
    AttachmentLoaded(AttachmentId, String, String, Vec<u8>),
    AttachmentFailed(AttachmentId, String),
    RemoveAttachment(AttachmentId),
    Send,
    DispatchPending(CommandId),
    RetrySend,
    DismissPendingSend,
    Steer,
    Interrupt,
    ResolveApproval(ApprovalId, String),
    SetSessionOption(String, String),
    EditTitle,
    TitleChanged(String),
    SaveTitle,
    LoadOlder,
    OpenImage(AttachmentId),
    CloseImage,
    CopyText(String),
    PersistCache(u64),
    FlushCache,
}

impl Component for App {
    type Message = Msg;
    type Properties = ();

    fn create(context: &Context<Self>) -> Self {
        let credentials = load_credentials();
        let fragment = fragment_connection();
        let stored = load_last_host()
            .and_then(|host_id| {
                credentials
                    .iter()
                    .find(|item| item.host_id == host_id)
                    .cloned()
            })
            .or_else(|| credentials.first().cloned());
        let connection = fragment.or_else(|| {
            stored.map(|credential| ConnectionConfig {
                host_id: credential.host_id,
                pair_token: None,
                origin: credential.origin.clone(),
                relay: credential.relay,
                credential: Some(credential),
            })
        });
        let cache = connection
            .as_ref()
            .and_then(|connection| load_cache(connection.host_id));
        let pending_send = cache.as_ref().and_then(|cache| {
            cache.pending_command.clone().and_then(|command| {
                pending_send_from_cached(
                    command,
                    cache.pending_send_state,
                    cache.pending_rejection_code.clone(),
                )
            })
        });
        let pending_attachments = pending_send
            .as_ref()
            .map_or_else(Vec::new, pending_browser_attachments);
        let cache_needs_tree_default = cache
            .as_ref()
            .is_none_or(|cache| cache.version < CACHE_VERSION);
        let (timeline_by_conversation, markdown_render_cache) = cache
            .as_ref()
            .map(|cache| index_timeline(&cache.snapshot.timeline))
            .unwrap_or_default();
        let mut app = Self {
            socket: None,
            _socket_callbacks: None,
            connection,
            credentials,
            snapshot: cache.as_ref().map(|cache| cache.snapshot.clone()),
            timeline_by_conversation,
            markdown_render_cache,
            connected: false,
            authenticated: false,
            connection_generation: 0,
            reconnect_attempt: 0,
            reconnect_timer: None,
            retry_enabled: true,
            manually_disconnected: false,
            _online_callback: None,
            _flush_callback: None,
            status: if cache.is_some() {
                "离线 · 正在恢复连接".to_owned()
            } else {
                "等待连接".to_owned()
            },
            pair_link: String::new(),
            selected_conversation: cache.as_ref().and_then(|cache| cache.selected_conversation),
            selected_project: cache.as_ref().and_then(|cache| cache.selected_project),
            selected_provider: cache
                .as_ref()
                .map_or(ProviderId::Codex, |cache| cache.selected_provider),
            selected_model: cache
                .as_ref()
                .and_then(|cache| cache.selected_model.clone()),
            selected_effort: cache
                .as_ref()
                .and_then(|cache| cache.selected_effort.clone()),
            selected_permission: cache
                .as_ref()
                .and_then(|cache| cache.selected_permission.clone()),
            draft_conversation: cache.as_ref().and_then(|cache| cache.draft_conversation),
            pending_send,
            composer: cache
                .as_ref()
                .map_or_else(String::new, |cache| cache.composer.clone()),
            pending_attachments,
            attachments: HashMap::new(),
            fullscreen_image: None,
            conversation_search: String::new(),
            project_picker_open: false,
            project_search: String::new(),
            pinned_projects: cache
                .as_ref()
                .map_or_else(Vec::new, |cache| cache.pinned_projects.clone()),
            recent_projects: cache
                .as_ref()
                .map_or_else(Vec::new, |cache| cache.recent_projects.clone()),
            expanded_projects: cache.as_ref().map_or_else(HashSet::new, |cache| {
                cache.expanded_projects.iter().copied().collect()
            }),
            sidebar_collapsed: cache.as_ref().is_some_and(|cache| cache.sidebar_collapsed),
            sidebar_open: false,
            history_before: HashMap::new(),
            history_exhausted: HashSet::new(),
            history_requested: HashSet::new(),
            editing_title: false,
            title_draft: String::new(),
            sync_in_flight: HashMap::new(),
            refresh_in_flight: HashSet::new(),
            cache_persist_timer: None,
            cache_persist_epoch: 0,
            send_started_at: HashMap::new(),
        };
        if cache_needs_tree_default {
            app.expand_selected_project();
        }
        if let Some(browser) = window() {
            let link = context.link().clone();
            let callback =
                Closure::wrap(
                    Box::new(move |_: Event| link.send_message(Msg::NetworkOnline))
                        as Box<dyn FnMut(_)>,
                );
            let _ = browser
                .add_event_listener_with_callback("online", callback.as_ref().unchecked_ref());
            app._online_callback = Some(callback);
        }
        if let Some(browser) = window() {
            let link = context.link().clone();
            let callback =
                Closure::wrap(Box::new(move |_: Event| link.send_message(Msg::FlushCache))
                    as Box<dyn FnMut(_)>);
            let _ = browser
                .add_event_listener_with_callback("pagehide", callback.as_ref().unchecked_ref());
            if let Some(document) = browser.document() {
                let _ = document.add_event_listener_with_callback(
                    "visibilitychange",
                    callback.as_ref().unchecked_ref(),
                );
            }
            app._flush_callback = Some(callback);
        }
        if app.connection.is_some() {
            app.connect(context);
        }
        app
    }

    fn update(&mut self, context: &Context<Self>, message: Self::Message) -> bool {
        if let Msg::PersistCache(epoch) = &message {
            if *epoch == self.cache_persist_epoch {
                self.cache_persist_timer = None;
                self.persist_cache();
            }
            return false;
        }
        if matches!(&message, Msg::FlushCache) {
            self.cancel_cache_persist_timer();
            self.persist_cache();
            return false;
        }
        if let Msg::CopyText(text) = &message {
            if let Some(browser) = window() {
                let _ = browser.navigator().clipboard().write_text(text);
            }
            return false;
        }
        match message {
            Msg::Opened(generation) => {
                if generation != self.connection_generation {
                    return false;
                }
                self.connected = true;
                self.reconnect_attempt = 0;
                self.status = "正在认证…".to_owned();
                if let Some(connection) = &self.connection {
                    let command = if let Some(pair_token) = &connection.pair_token {
                        ClientCommand::Pair {
                            host_id: connection.host_id,
                            pair_token: pair_token.clone(),
                            device_name: browser_device_name(),
                        }
                    } else if let Some(credential) = &connection.credential {
                        ClientCommand::Authenticate {
                            host_id: credential.host_id,
                            device_id: credential.device_id,
                            device_token: credential.device_token.clone(),
                        }
                    } else {
                        self.status = "缺少配对凭证".to_owned();
                        return true;
                    };
                    if !self.send_command(command) {
                        self.close_socket("authentication write failed");
                        self.handle_disconnect(context, "认证请求未能写入 WebSocket".to_owned());
                    }
                }
            }
            Msg::Closed(generation, reason) => {
                if generation != self.connection_generation {
                    return false;
                }
                self.handle_disconnect(context, reason);
            }
            Msg::SocketError(generation) => {
                if generation == self.connection_generation {
                    self.status = "连接异常，等待 WebSocket 关闭后重试".to_owned();
                }
            }
            Msg::Reconnect(generation) => {
                if generation == self.connection_generation
                    && self.retry_enabled
                    && !self.manually_disconnected
                    && !self.connected
                {
                    self.reconnect_timer = None;
                    self.connect(context);
                }
            }
            Msg::RetryNow => {
                self.retry_enabled = true;
                self.manually_disconnected = false;
                self.reconnect_attempt = 0;
                self.cancel_reconnect_timer();
                self.connect(context);
            }
            Msg::StopRetrying => {
                self.retry_enabled = false;
                self.cancel_reconnect_timer();
                self.status = "离线 · 已停止自动重连".to_owned();
            }
            Msg::NetworkOnline => {
                if !self.connected
                    && self.retry_enabled
                    && !self.manually_disconnected
                    && self.connection.is_some()
                {
                    self.reconnect_attempt = 0;
                    self.cancel_reconnect_timer();
                    self.connect(context);
                }
            }
            Msg::Disconnect => {
                self.manually_disconnected = true;
                self.retry_enabled = false;
                self.cancel_reconnect_timer();
                self.close_socket("manual disconnect");
                self.connected = false;
                self.authenticated = false;
                self.status = "离线 · 已手动断开".to_owned();
            }
            Msg::DecodeError(generation, error) => {
                if generation != self.connection_generation {
                    return false;
                }
                self.status = format!("协议错误：{error}");
            }
            Msg::Server(generation, server_message) => {
                if generation != self.connection_generation {
                    return false;
                }
                let should_render = !matches!(&server_message, ServerMessage::SendTrace { .. });
                self.apply_server_message(context, server_message);
                if !should_render {
                    return false;
                }
            }
            Msg::PairLinkChanged(value) => self.pair_link = value,
            Msg::OpenPairLink => {
                if let Some(location) = window().map(|window| window.location()) {
                    let _ = location.set_href(self.pair_link.trim());
                }
            }
            Msg::ConnectStored(index) => {
                if let Some(credential) = self.credentials.get(index).cloned() {
                    self.connection = Some(ConnectionConfig {
                        host_id: credential.host_id,
                        pair_token: None,
                        origin: credential.origin.clone(),
                        relay: credential.relay,
                        credential: Some(credential),
                    });
                    self.restore_cache_for_connection();
                    self.retry_enabled = true;
                    self.manually_disconnected = false;
                    self.connect(context);
                }
            }
            Msg::ForgetCredentials => {
                if let Some(connection) = &self.connection {
                    remove_cache(connection.host_id);
                }
                self.credentials.clear();
                save_credentials(&self.credentials);
                self.manually_disconnected = true;
                self.retry_enabled = false;
                self.cancel_reconnect_timer();
                self.close_socket("credentials removed");
                self.connection = None;
                self.snapshot = None;
                self.timeline_by_conversation.clear();
                self.markdown_render_cache.clear();
                self.status = "本地设备凭证已删除".to_owned();
            }
            Msg::SelectConversation(id) => {
                self.select_conversation(id);
            }
            Msg::SelectProject(id) => {
                self.select_project(id);
            }
            Msg::ToggleProject(id) => {
                if let Some(scope) = self.project_scope(self.selected_provider, id)
                    && !self.expanded_projects.remove(&scope)
                {
                    self.expanded_projects.insert(scope);
                }
            }
            Msg::SelectProvider(provider) => {
                if provider == self.selected_provider {
                    return false;
                }
                if !self.provider_is_available(provider) {
                    self.status = "此 Agent 没有当前 Host 授权的可用项目".to_owned();
                    return true;
                }
                if self.pending_send.is_some() {
                    self.status = "请先等待发送确认，或取消重试后再切换 Provider".to_owned();
                    return true;
                }
                self.selected_provider = provider;
                self.selected_project = None;
                self.selected_conversation = None;
                self.draft_conversation = None;
                self.pending_attachments.clear();
                self.reset_dynamic_selection();
                self.ensure_project_for_provider();
                self.expand_selected_project();
                self.request_project_refresh(provider);
                self.sync_selected_project();
            }
            Msg::SelectModel(model) => {
                self.selected_model = model;
                self.selected_effort =
                    self.selected_capability()
                        .and_then(|capability| {
                            capability.models.iter().find(|model| {
                                Some(model.id.as_str()) == self.selected_model.as_deref()
                            })
                        })
                        .and_then(|model| model.default_effort.clone());
            }
            Msg::SelectEffort(effort) => self.selected_effort = effort,
            Msg::SelectPermission(permission) => {
                if permission
                    .as_deref()
                    .is_none_or(|permission| self.confirm_permission_change(permission))
                {
                    self.selected_permission = permission;
                }
            }
            Msg::NewConversation => {
                if self.pending_send.is_some() {
                    self.status = "请先等待发送确认，或取消重试后再新建对话".to_owned();
                } else if self.selected_project.is_some() {
                    self.selected_conversation = None;
                    self.draft_conversation = Some(ConversationId::new());
                    self.pending_attachments.clear();
                    self.sidebar_open = false;
                    self.editing_title = false;
                    self.expand_selected_project();
                }
            }
            Msg::ToggleProjectPicker => self.project_picker_open = !self.project_picker_open,
            Msg::ProjectSearchChanged(value) => self.project_search = value,
            Msg::ToggleProjectPin(id) => {
                if self.pinned_projects.contains(&id) {
                    self.pinned_projects.retain(|project| *project != id);
                } else {
                    self.pinned_projects.push(id);
                }
            }
            Msg::ConversationSearchChanged(value) => self.conversation_search = value,
            Msg::ToggleSidebar => self.sidebar_collapsed = !self.sidebar_collapsed,
            Msg::OpenSidebar => self.sidebar_open = true,
            Msg::CloseSidebar => self.sidebar_open = false,
            Msg::ComposerChanged(value) => {
                if self.pending_send.is_none() {
                    self.composer = value;
                }
            }
            Msg::FilesSelected(files) => {
                if self.pending_send.is_none() {
                    self.read_files(context, files);
                }
            }
            Msg::AttachmentLoaded(id, file_name, mime_type, bytes) => {
                if let Some(attachment) = self
                    .pending_attachments
                    .iter_mut()
                    .find(|attachment| attachment.id == id)
                {
                    attachment.file_name = file_name;
                    attachment.mime_type = mime_type;
                    attachment.bytes = Some(bytes);
                    attachment.error = None;
                }
            }
            Msg::AttachmentFailed(id, error) => {
                if let Some(attachment) = self
                    .pending_attachments
                    .iter_mut()
                    .find(|attachment| attachment.id == id)
                {
                    attachment.error = Some(error);
                }
            }
            Msg::RemoveAttachment(id) => {
                if self.pending_send.is_none() {
                    self.pending_attachments
                        .retain(|attachment| attachment.id != id);
                }
            }
            Msg::Send => {
                if !self.authenticated {
                    self.status = "连接尚未完成认证，消息已保留".to_owned();
                } else if self.pending_send.is_none()
                    && !self.composer.trim().is_empty()
                    && self
                        .pending_attachments
                        .iter()
                        .all(|attachment| attachment.bytes.is_some() && attachment.error.is_none())
                {
                    let text = self.composer.trim().to_owned();
                    let command_id = CommandId::new();
                    let client_message_id = Uuid::new_v4().to_string();
                    self.send_started_at.insert(command_id, monotonic_now_ms());
                    trace_send_stage(
                        "click",
                        command_id,
                        &client_message_id,
                        Some(self.connection_generation),
                        Some(0),
                    );
                    if let Some(conversation_id) = self.selected_conversation {
                        let attachments = self.take_client_attachments();
                        let command = ClientCommand::SendMessage {
                            command_id,
                            attempt: 0,
                            conversation_id,
                            client_message_id: Some(client_message_id.clone()),
                            text: text.clone(),
                            attachments,
                        };
                        self.queue_pending_send(context, command_id, client_message_id, command);
                    } else if let (Some(conversation_id), Some(project_id)) =
                        (self.draft_conversation, self.selected_project)
                    {
                        let attachments = self.take_client_attachments();
                        let command = ClientCommand::StartConversation {
                            command_id,
                            attempt: 0,
                            conversation_id,
                            project_id,
                            provider: self.selected_provider,
                            client_message_id: Some(client_message_id.clone()),
                            model: self.selected_model.clone(),
                            effort: self.selected_effort.clone(),
                            permission_mode: self.selected_permission.clone(),
                            text: text.clone(),
                            attachments,
                        };
                        self.queue_pending_send(context, command_id, client_message_id, command);
                    }
                }
            }
            Msg::DispatchPending(command_id) => {
                if self
                    .pending_send
                    .as_ref()
                    .is_some_and(|pending| pending.command_id == command_id)
                {
                    self.try_send_pending("initial");
                }
            }
            Msg::RetrySend => {
                if self.authenticated {
                    let mut retry_command_id = None;
                    if let Some(pending) = &mut self.pending_send {
                        if pending_can_retry(pending) {
                            increment_send_attempt(&mut pending.command);
                            pending.state = PendingSendState::Queued;
                            pending.error = None;
                            pending.rejection_code = None;
                            let command_id = pending.command_id;
                            retry_command_id = Some(command_id);
                            if command_is_send(&pending.command) {
                                let client_message_id = pending.client_message_id.clone();
                                self.send_started_at.insert(command_id, monotonic_now_ms());
                                trace_send_stage(
                                    "click",
                                    command_id,
                                    &client_message_id,
                                    Some(self.connection_generation),
                                    Some(0),
                                );
                            }
                        } else {
                            self.status =
                                "该拒绝结果不能安全重放；草稿仍保留，可修改后重新发送".to_owned();
                        }
                    }
                    if let Some(command_id) = retry_command_id {
                        self.status = "正在重试…".to_owned();
                        self.schedule_pending_dispatch(context, command_id);
                    }
                } else {
                    self.status = "连接尚未完成认证，消息仍保留待重试".to_owned();
                }
            }
            Msg::DismissPendingSend => {
                if let Some(pending) = self.pending_send.take() {
                    self.send_started_at.remove(&pending.command_id);
                    self.restore_pending_attachments(pending.command);
                }
                self.status = "已保留草稿，可修改后重新发送".to_owned();
            }
            Msg::Steer => {
                if let Some(conversation_id) = self.selected_conversation
                    && self.pending_send.is_none()
                    && !self.composer.trim().is_empty()
                {
                    let command_id = CommandId::new();
                    let client_message_id = format!("steer:{command_id}");
                    let command = ClientCommand::Steer {
                        command_id,
                        conversation_id,
                        text: self.composer.clone(),
                    };
                    self.queue_pending_send(context, command_id, client_message_id, command);
                }
            }
            Msg::Interrupt => {
                if let Some(conversation_id) = self.selected_conversation {
                    self.send_authenticated(ClientCommand::Interrupt {
                        command_id: CommandId::new(),
                        conversation_id,
                    });
                }
            }
            Msg::ResolveApproval(approval_id, option_id) => {
                self.send_authenticated(ClientCommand::ResolveApproval {
                    command_id: CommandId::new(),
                    approval_id,
                    option_id,
                });
            }
            Msg::SetSessionOption(option_id, value) => {
                if option_id == "permission_mode" && !self.confirm_permission_change(&value) {
                    return true;
                }
                if let Some(conversation_id) = self.selected_conversation {
                    self.send_authenticated(ClientCommand::SetSessionOption {
                        command_id: CommandId::new(),
                        conversation_id,
                        option_id,
                        value,
                    });
                }
            }
            Msg::EditTitle => {
                if let Some(conversation) = self.selected_conversation_ref() {
                    self.title_draft = conversation.title.clone();
                    self.editing_title = true;
                }
            }
            Msg::TitleChanged(value) => self.title_draft = value,
            Msg::SaveTitle => {
                if let Some(conversation_id) = self.selected_conversation
                    && !self.title_draft.trim().is_empty()
                    && self.send_authenticated(ClientCommand::RenameConversation {
                        command_id: CommandId::new(),
                        conversation_id,
                        title: self.title_draft.trim().to_owned(),
                    })
                {
                    self.editing_title = false;
                }
            }
            Msg::LoadOlder => {
                if let Some(conversation_id) = self.selected_conversation
                    && !self.history_exhausted.contains(&conversation_id)
                {
                    let before = self
                        .history_before
                        .get(&conversation_id)
                        .copied()
                        .or_else(|| {
                            self.snapshot.as_ref().and_then(|snapshot| {
                                snapshot
                                    .timeline
                                    .iter()
                                    .filter(|item| item.conversation_id == conversation_id)
                                    .min_by_key(|item| (item.created_at_ms, item.id))
                                    .map(|item| TimelinePageCursor {
                                        created_at_ms: item.created_at_ms,
                                        item_id: item.id,
                                    })
                            })
                        });
                    self.send_authenticated(ClientCommand::GetConversationPage {
                        conversation_id,
                        before,
                        limit: 100,
                    });
                }
            }
            Msg::OpenImage(id) => {
                self.fullscreen_image = Some(id);
                if !self.attachments.contains_key(&id) {
                    self.send_authenticated(ClientCommand::GetAttachment { attachment_id: id });
                }
            }
            Msg::CloseImage => self.fullscreen_image = None,
            Msg::CopyText(_) | Msg::PersistCache(_) | Msg::FlushCache => unreachable!(),
        }
        self.schedule_cache_persist(context);
        true
    }

    fn view(&self, context: &Context<Self>) -> Html {
        let link = context.link();
        let Some(snapshot) = &self.snapshot else {
            return self.view_connection(link);
        };
        let selected = self.selected_conversation.and_then(|id| {
            snapshot.conversations.iter().find(|conversation| {
                conversation.id == id
                    && Some(conversation.project_id) == self.selected_project
                    && conversation.provider == self.selected_provider
            })
        });
        let project = self
            .selected_project
            .and_then(|id| snapshot.projects.iter().find(|project| project.id == id));
        let providers = available_providers(snapshot);
        html! {
            <main class={classes!("app-shell", self.sidebar_collapsed.then_some("sidebar-collapsed"))}>
                {if self.sidebar_open { html! {<button class="drawer-scrim" aria-label="关闭侧边栏" onclick={link.callback(|_| Msg::CloseSidebar)}></button>} } else {html! {}}}
                <aside class={classes!("sidebar", self.sidebar_open.then_some("open"))}>
                    <div class="brand">
                        <span class="brand-mark">{"AR"}</span>
                        <div class="collapsible-copy"><strong>{"Agent Remote"}</strong><small>{&snapshot.host_name}</small></div>
                    </div>
                    <div class="connection-strip">
                        <span class={if self.connected {"status-dot online"} else {"status-dot"}}></span>
                        <span class="collapsible-copy">{&self.status}</span>
                        {if !self.connected && self.retry_enabled {html! {<button title="停止重连" onclick={link.callback(|_| Msg::StopRetrying)}>{"×"}</button>}} else if !self.connected {html! {<button title="立即重试" onclick={link.callback(|_| Msg::RetryNow)}>{"↻"}</button>}} else {html! {}}}
                    </div>
                    {self.view_agent_selector(link, &providers)}
                    <div class="project-control">
                        <button class="project-trigger" onclick={link.callback(|_| Msg::ToggleProjectPicker)} title="选择项目">
                            <span class="project-icon">{"▣"}</span>
                            <span class="collapsible-copy"><strong>{project.map_or("选择项目", |project| project.display_name.as_str())}</strong><small>{project.map_or("", |project| project.short_path.as_str())}</small></span>
                            <span class="collapsible-copy">{"⌄"}</span>
                        </button>
                        {if self.project_picker_open { self.view_project_picker(link, snapshot) } else {html! {}}}
                    </div>
                    <button class="new-button" onclick={link.callback(|_| Msg::NewConversation)} disabled={self.selected_project.is_none()}><span>{"＋"}</span><b class="collapsible-copy">{"新建对话"}</b></button>
                    <label class="conversation-search collapsible-copy"><span>{"⌕"}</span><input placeholder="搜索项目或对话" value={self.conversation_search.clone()} oninput={link.callback(|event: InputEvent| Msg::ConversationSearchChanged(event.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                    <nav class="conversation-list" aria-label="项目与会话">
                        {self.view_project_tree(link, snapshot)}
                    </nav>
                    <div class="sidebar-footer">
                        <button onclick={link.callback(|_| Msg::Disconnect)} title="断开连接">{"⏻"}<span class="collapsible-copy">{"断开"}</span></button>
                        <button onclick={link.callback(|_| Msg::ToggleSidebar)} title="收起侧边栏">{if self.sidebar_collapsed {"›"} else {"‹"}}</button>
                    </div>
                </aside>
                <section class="chat-pane">
                    {if let Some(conversation) = selected {
                        self.view_chat(link, conversation, snapshot)
                    } else if self.draft_conversation.is_some() && self.selected_project.is_some() {
                        self.view_draft_chat(link, snapshot)
                    } else {
                        html! { <div class="empty-state"><button class="mobile-menu" onclick={link.callback(|_| Msg::OpenSidebar)}>{"☰"}</button><h2>{"选择或新建对话"}</h2><p>{"这里只显示当前 Agent 与项目的远程对话。"}</p></div> }
                    }}
                </section>
                {self.view_fullscreen_image(link)}
            </main>
        }
    }

    fn destroy(&mut self, _context: &Context<Self>) {
        self.cancel_cache_persist_timer();
        self.persist_cache();
        for url in self.attachments.values() {
            let _ = Url::revoke_object_url(url);
        }
    }
}

impl App {
    fn view_agent_selector(&self, link: &yew::html::Scope<Self>, providers: &[ProviderId]) -> Html {
        if providers.len() <= 1 {
            let label = providers
                .first()
                .map_or("无可用 Agent", |provider| provider_label(*provider));
            return html! {
                <div class="sidebar-agent" aria-label="当前 Agent">
                    <span class="collapsible-copy">{"Agent"}</span>
                    <strong class="agent-current">{label}</strong>
                </div>
            };
        }
        html! {
            <label class="sidebar-agent">
                <span class="collapsible-copy">{"Agent"}</span>
                <select aria-label="选择 Agent" onchange={link.callback(|event: Event| {
                    let value = event.target_unchecked_into::<HtmlSelectElement>().value();
                    Msg::SelectProvider(if value == "grok" { ProviderId::Grok } else { ProviderId::Codex })
                })}>
                    {for providers.iter().map(|provider| {
                        let value = match provider {
                            ProviderId::Codex => "codex",
                            ProviderId::Grok => "grok",
                        };
                        html! {<option value={value} selected={self.selected_provider == *provider}>{provider_label(*provider)}</option>}
                    })}
                </select>
            </label>
        }
    }

    fn connect(&mut self, context: &Context<Self>) {
        let Some(connection) = self.connection.clone() else {
            return;
        };
        self.cancel_reconnect_timer();
        self.close_socket("superseded");
        self.sync_in_flight.clear();
        self.refresh_in_flight.clear();
        self.connection_generation += 1;
        let generation = self.connection_generation;
        save_last_host(connection.host_id);
        match open_socket(context, &connection, generation) {
            Ok((socket, callbacks)) => {
                self.socket = Some(socket);
                self._socket_callbacks = Some(callbacks);
                self.connected = false;
                self.authenticated = false;
                self.status = "连接中…".to_owned();
            }
            Err(error) => self.handle_disconnect(context, error),
        }
    }

    fn handle_disconnect(&mut self, context: &Context<Self>, reason: String) {
        self.connected = false;
        self.authenticated = false;
        self.socket = None;
        self._socket_callbacks = None;
        self.sync_in_flight.clear();
        self.refresh_in_flight.clear();
        if let Some(pending) = &mut self.pending_send
            && pending.state == PendingSendState::AwaitingAck
        {
            pending.state = PendingSendState::Queued;
            pending.error = Some("连接中断，认证恢复后将重试".to_owned());
            pending.rejection_code = None;
        }
        if self.manually_disconnected || !self.retry_enabled {
            self.status = "离线 · 已停止自动重连".to_owned();
            return;
        }
        if self
            .connection
            .as_ref()
            .and_then(|connection| connection.credential.as_ref())
            .is_none()
        {
            self.status = format!("连接失败：{reason}");
            return;
        }
        if self.reconnect_attempt >= MAX_RECONNECT_ATTEMPTS {
            self.status = format!("连接失败 · 已达到重试上限：{reason}");
            return;
        }
        let base_seconds = 1_u32 << self.reconnect_attempt.min(5);
        let jitter_ms = (Math::random() * 800.0) as i32;
        let delay_ms = (base_seconds.min(30) * 1_000) as i32 + jitter_ms;
        self.reconnect_attempt += 1;
        self.status = format!(
            "重连中 · 第 {} 次，约 {:.1} 秒后重试",
            self.reconnect_attempt,
            f64::from(delay_ms) / 1_000.0
        );
        if let Some(browser) = window() {
            let generation = self.connection_generation;
            let link = context.link().clone();
            let callback =
                Closure::once_into_js(move || link.send_message(Msg::Reconnect(generation)));
            if let Ok(handle) = browser.set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.unchecked_ref(),
                delay_ms,
            ) {
                self.reconnect_timer = Some(handle);
            }
        }
    }

    fn cancel_reconnect_timer(&mut self) {
        if let Some(handle) = self.reconnect_timer.take()
            && let Some(browser) = window()
        {
            browser.clear_timeout_with_handle(handle);
        }
    }

    fn close_socket(&mut self, reason: &str) {
        if let Some(socket) = self.socket.take() {
            socket.set_onopen(None);
            socket.set_onmessage(None);
            socket.set_onerror(None);
            socket.set_onclose(None);
            let _ = socket.close_with_code_and_reason(1000, reason);
        }
        self._socket_callbacks = None;
    }

    fn write_command(&self, command: &ClientCommand) -> Result<(), String> {
        let socket = self.socket.as_ref().ok_or("WebSocket 尚未建立")?;
        if socket.ready_state() != WebSocket::OPEN {
            return Err("WebSocket 尚未打开".to_owned());
        }
        let bytes = encode(command).map_err(|error| format!("命令编码失败：{error}"))?;
        socket
            .send_with_u8_array(&bytes)
            .map_err(|_| "WebSocket 写入失败".to_owned())
    }

    fn send_command(&mut self, command: ClientCommand) -> bool {
        match self.write_command(&command) {
            Ok(()) => true,
            Err(error) => {
                self.status = error;
                false
            }
        }
    }

    fn send_authenticated(&mut self, command: ClientCommand) -> bool {
        if !self.authenticated || !self.connected {
            self.status = "连接尚未完成认证，请稍后重试".to_owned();
            return false;
        }
        self.send_command(command)
    }

    fn queue_pending_send(
        &mut self,
        context: &Context<Self>,
        command_id: CommandId,
        client_message_id: String,
        command: ClientCommand,
    ) {
        let trace_send = command_is_send(&command);
        let trace_client_message_id = client_message_id.clone();
        self.pending_send = Some(PendingSend {
            command_id,
            client_message_id,
            command,
            state: PendingSendState::Queued,
            error: None,
            rejection_code: None,
        });
        if trace_send {
            trace_send_stage(
                "local_pending",
                command_id,
                &trace_client_message_id,
                Some(self.connection_generation),
                self.send_elapsed_ms(command_id),
            );
        }
        self.status = "正在发送…".to_owned();
        self.schedule_pending_dispatch(context, command_id);
    }

    fn schedule_pending_dispatch(&self, context: &Context<Self>, command_id: CommandId) {
        let link = context.link().clone();
        let callback = Closure::once_into_js(move || {
            let fallback_link = link.clone();
            let dispatch =
                Closure::once_into_js(move || link.send_message(Msg::DispatchPending(command_id)));
            if window()
                .and_then(|browser| {
                    browser
                        .request_animation_frame(dispatch.unchecked_ref())
                        .ok()
                })
                .is_none()
            {
                fallback_link.send_message(Msg::DispatchPending(command_id));
            }
        });
        if window()
            .and_then(|browser| {
                browser
                    .request_animation_frame(callback.unchecked_ref())
                    .ok()
            })
            .is_none()
        {
            context
                .link()
                .send_message(Msg::DispatchPending(command_id));
        }
    }

    fn try_send_pending(&mut self, reason: &str) {
        let Some(pending) = &self.pending_send else {
            return;
        };
        if !self.authenticated || !self.connected {
            self.status = "连接尚未完成认证，消息仍保留待重试".to_owned();
            return;
        }
        let command = pending.command.clone();
        let command_id = pending.command_id;
        let client_message_id = pending.client_message_id.clone();
        self.send_started_at
            .entry(command_id)
            .or_insert_with(monotonic_now_ms);
        let trace_send = command_is_send(&command);
        match self.write_command(&command) {
            Ok(()) => {
                if trace_send {
                    trace_send_stage(
                        "websocket_write",
                        command_id,
                        &client_message_id,
                        Some(self.connection_generation),
                        self.send_elapsed_ms(command_id),
                    );
                }
                if let Some(pending) = &mut self.pending_send {
                    pending.state = PendingSendState::AwaitingAck;
                    pending.error = None;
                    pending.rejection_code = None;
                }
                self.status = if reason == "initial" {
                    "已排队 · 等待 Host 确认".to_owned()
                } else {
                    "正在重试 · 等待 Host 确认".to_owned()
                };
            }
            Err(error) => {
                if let Some(pending) = &mut self.pending_send {
                    pending.state = PendingSendState::WriteFailed;
                    pending.error = Some(error.clone());
                    pending.rejection_code = None;
                }
                self.status = format!("发送失败，草稿已保留：{error}");
            }
        }
    }

    fn send_elapsed_ms(&self, command_id: CommandId) -> Option<u64> {
        self.send_started_at
            .get(&command_id)
            .map(|started_at| (monotonic_now_ms() - started_at).max(0.0).round() as u64)
    }

    fn apply_server_message(&mut self, context: &Context<Self>, message: ServerMessage) {
        match message {
            ServerMessage::Paired {
                host_id,
                device_id,
                device_token,
            } => {
                if self
                    .connection
                    .as_ref()
                    .is_none_or(|connection| connection.host_id != host_id)
                {
                    self.authenticated = false;
                    self.status = "认证响应的 Host 与当前连接不匹配".to_owned();
                    return;
                }
                self.authenticated = true;
                self.status = "配对成功".to_owned();
                if let Some(connection) = &self.connection {
                    let credential = StoredCredential {
                        host_id,
                        device_id,
                        device_token,
                        origin: connection.origin.clone(),
                        relay: connection.relay,
                    };
                    self.credentials.retain(|item| item.host_id != host_id);
                    self.credentials.push(credential.clone());
                    save_credentials(&self.credentials);
                    if let Some(connection) = &mut self.connection {
                        connection.credential = Some(credential);
                        connection.pair_token = None;
                    }
                    clear_fragment();
                }
                if !self.send_command(ClientCommand::GetSnapshot) {
                    self.close_socket("snapshot write failed");
                    self.handle_disconnect(context, "初始状态请求未能写入 WebSocket".to_owned());
                }
            }
            ServerMessage::Authenticated { host_id, .. } => {
                if self
                    .connection
                    .as_ref()
                    .is_none_or(|connection| connection.host_id != host_id)
                {
                    self.authenticated = false;
                    self.status = "认证响应的 Host 与当前连接不匹配".to_owned();
                    return;
                }
                self.authenticated = true;
                self.status = "同步中…".to_owned();
                if !self.send_command(ClientCommand::GetSnapshot) {
                    self.close_socket("snapshot write failed");
                    self.handle_disconnect(context, "初始状态请求未能写入 WebSocket".to_owned());
                }
            }
            ServerMessage::Snapshot { snapshot } => {
                if self
                    .connection
                    .as_ref()
                    .is_none_or(|connection| connection.host_id != snapshot.host_id)
                {
                    self.status = "忽略了其他 Host 的状态快照".to_owned();
                    return;
                }
                self.snapshot = Some(snapshot);
                self.reindex_timeline();
                self.sync_in_flight.clear();
                self.refresh_in_flight.clear();
                self.history_before.clear();
                self.history_exhausted.clear();
                self.history_requested.clear();
                let previous_project = self.selected_project;
                self.ensure_provider_available();
                self.ensure_project_for_provider();
                self.reset_dynamic_selection();
                self.selected_conversation = self.selected_conversation.filter(|selected| {
                    self.snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot.conversations.iter().any(|conversation| {
                            conversation.id == *selected
                                && Some(conversation.project_id) == self.selected_project
                                && conversation.provider == self.selected_provider
                        })
                    })
                });
                if previous_project != self.selected_project {
                    self.expand_selected_project();
                }
                self.status = "同步中…".to_owned();
                self.request_project_refresh(self.selected_provider);
                if self
                    .pending_send
                    .as_ref()
                    .is_some_and(|pending| pending.state != PendingSendState::Rejected)
                {
                    self.try_send_pending("reconnect");
                }
                self.request_selected_conversation_page();
                self.request_images_for_selected();
            }
            ServerMessage::ProjectsUpdated {
                provider,
                projects,
                capabilities,
            } => {
                self.refresh_in_flight.remove(&provider);
                if let Some(snapshot) = &mut self.snapshot {
                    let incoming_ids = projects
                        .iter()
                        .map(|project| project.id)
                        .collect::<HashSet<_>>();
                    for project in &mut snapshot.projects {
                        if project.enabled_providers.contains(&provider)
                            && !incoming_ids.contains(&project.id)
                        {
                            project
                                .enabled_providers
                                .retain(|candidate| *candidate != provider);
                        }
                    }
                    for project in projects {
                        upsert_project(&mut snapshot.projects, project);
                    }
                    snapshot
                        .provider_capabilities
                        .retain(|capability| capability.provider != provider);
                    for capability in capabilities {
                        upsert_capability(&mut snapshot.provider_capabilities, capability);
                    }
                }
                if provider == self.selected_provider {
                    let previous_project = self.selected_project;
                    self.ensure_provider_available();
                    self.ensure_project_for_provider();
                    if previous_project != self.selected_project {
                        self.expand_selected_project();
                    }
                    self.reset_dynamic_selection();
                }
            }
            ServerMessage::ProjectSyncCompleted {
                command_id,
                project_id,
                provider,
                conversations_synced,
                full_history_fallback,
            } => {
                self.sync_in_flight.remove(&command_id);
                if Some(project_id) == self.selected_project && provider == self.selected_provider {
                    self.status = if full_history_fallback {
                        format!("已连接 · 已同步 {conversations_synced} 个对话（全量去重）")
                    } else {
                        format!("已连接 · 已同步 {conversations_synced} 个对话")
                    };
                }
            }
            ServerMessage::ConversationPage {
                conversation_id,
                items,
                next_before,
            } => {
                for item in items {
                    let changed = self.snapshot.as_mut().is_some_and(|snapshot| {
                        upsert_timeline_item(&mut snapshot.timeline, item.clone())
                    });
                    if changed {
                        upsert_timeline_item(
                            self.timeline_by_conversation
                                .entry(item.conversation_id)
                                .or_default(),
                            item.clone(),
                        );
                        self.cache_markdown_item(&item);
                    }
                }
                match next_before {
                    Some(before) => {
                        self.history_before.insert(conversation_id, before);
                    }
                    None => {
                        self.history_exhausted.insert(conversation_id);
                    }
                }
            }
            ServerMessage::ProviderChanged { capability } => {
                if let Some(snapshot) = &mut self.snapshot {
                    upsert_capability(&mut snapshot.provider_capabilities, capability);
                }
            }
            ServerMessage::ConversationUpserted { conversation } => {
                let authorized_scope = self.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.projects.iter().any(|project| {
                        project.id == conversation.project_id
                            && project.valid
                            && project.enabled_providers.contains(&conversation.provider)
                    })
                });
                if authorized_scope && let Some(snapshot) = &mut self.snapshot {
                    let project_id = conversation.project_id;
                    let provider = conversation.provider;
                    if !upsert_conversation(&mut snapshot.conversations, conversation.clone()) {
                        return;
                    }
                    refresh_project_metrics(snapshot, project_id);
                    if self.draft_conversation == Some(conversation.id) {
                        self.selected_provider = provider;
                        self.selected_project = Some(project_id);
                        self.selected_conversation = Some(conversation.id);
                        self.draft_conversation = None;
                        self.expand_selected_project();
                    }
                }
            }
            ServerMessage::TimelineItemUpserted { item } => {
                let image_id = match &item.kind {
                    TimelineItemKind::Image { attachment_id, .. } => Some(*attachment_id),
                    _ => None,
                };
                let changed = self.snapshot.as_mut().is_some_and(|snapshot| {
                    upsert_timeline_item(&mut snapshot.timeline, item.clone())
                });
                if changed {
                    upsert_timeline_item(
                        self.timeline_by_conversation
                            .entry(item.conversation_id)
                            .or_default(),
                        item.clone(),
                    );
                    self.cache_markdown_item(&item);
                }
                if let Some(attachment_id) = image_id {
                    self.send_authenticated(ClientCommand::GetAttachment { attachment_id });
                }
            }
            ServerMessage::ConversationRemoved { conversation_id } => {
                if let Some(snapshot) = &mut self.snapshot {
                    let project_id = snapshot
                        .conversations
                        .iter()
                        .find(|conversation| conversation.id == conversation_id)
                        .map(|conversation| conversation.project_id);
                    snapshot
                        .conversations
                        .retain(|conversation| conversation.id != conversation_id);
                    snapshot
                        .timeline
                        .retain(|item| item.conversation_id != conversation_id);
                    if let Some(project_id) = project_id {
                        refresh_project_metrics(snapshot, project_id);
                    }
                }
                if self.selected_conversation == Some(conversation_id) {
                    self.selected_conversation = None;
                }
                if let Some(items) = self.timeline_by_conversation.remove(&conversation_id) {
                    for item in items {
                        self.markdown_render_cache.remove(&item.id);
                    }
                }
            }
            ServerMessage::AttachmentData { metadata, bytes } => {
                if let Some(url) = image_object_url(&metadata.mime_type, &bytes) {
                    self.attachments.insert(metadata.id, url);
                }
            }
            ServerMessage::HostStatus {
                host_id,
                online,
                message,
            } => {
                if self
                    .connection
                    .as_ref()
                    .is_none_or(|connection| connection.host_id != host_id)
                {
                    return;
                }
                self.connected = online;
                if !online {
                    self.status = message.unwrap_or_else(|| "离线".to_owned());
                }
            }
            ServerMessage::SendTrace {
                command_id,
                client_message_id,
                conversation_id,
                stage,
                elapsed_ms,
            } => {
                let click_elapsed_ms = self.send_elapsed_ms(command_id);
                trace_server_send_stage(
                    stage,
                    command_id,
                    &client_message_id,
                    conversation_id,
                    click_elapsed_ms,
                    elapsed_ms,
                    self.connection_generation,
                );
                if stage == SendTraceStage::FirstProviderEvent {
                    self.send_started_at.remove(&command_id);
                }
            }
            ServerMessage::CommandRejected {
                command_id,
                code,
                message,
            } => {
                if let Some(command_id) = command_id {
                    self.sync_in_flight.remove(&command_id);
                }
                let rejected_pending_id = match command_id {
                    Some(command_id)
                        if self
                            .pending_send
                            .as_ref()
                            .is_some_and(|pending| pending.command_id == command_id) =>
                    {
                        Some(command_id)
                    }
                    None => self.pending_send.as_ref().map(|pending| pending.command_id),
                    _ => None,
                };
                if let Some(command_id) = rejected_pending_id {
                    self.send_started_at.remove(&command_id);
                    if let Some(pending) = &mut self.pending_send {
                        pending.state = PendingSendState::Rejected;
                        pending.error = Some(format!("{code}: {message}"));
                        pending.rejection_code = Some(code.clone());
                    }
                }
                if code == "authentication_failed" {
                    self.authenticated = false;
                    self.retry_enabled = false;
                    self.status = format!("认证失效，请重新配对：{message}");
                } else {
                    self.status = format!("{code}: {message}");
                }
            }
            ServerMessage::ProtocolError { message, .. } => {
                self.retry_enabled = false;
                self.status = format!("协议不兼容：{message}");
            }
            ServerMessage::CommandAccepted { command_id } => {
                if self
                    .pending_send
                    .as_ref()
                    .is_some_and(|pending| pending.command_id == command_id)
                {
                    if let Some(pending) = &self.pending_send
                        && command_is_send(&pending.command)
                    {
                        trace_send_stage(
                            "command_accepted",
                            command_id,
                            &pending.client_message_id,
                            Some(self.connection_generation),
                            self.send_elapsed_ms(command_id),
                        );
                    }
                    self.pending_send = None;
                    self.composer.clear();
                    self.pending_attachments.clear();
                    self.status = "已发送".to_owned();
                }
            }
        }
    }

    fn reset_dynamic_selection(&mut self) {
        let capability = self.selected_capability().cloned();
        self.selected_model = self
            .selected_model
            .take()
            .filter(|selected| {
                capability.as_ref().is_some_and(|capability| {
                    capability.models.iter().any(|model| &model.id == selected)
                })
            })
            .or_else(|| {
                capability
                    .as_ref()
                    .and_then(|capability| capability.models.first())
                    .map(|model| model.id.clone())
            });
        let valid_efforts = capability.as_ref().and_then(|capability| {
            capability
                .models
                .iter()
                .find(|model| Some(model.id.as_str()) == self.selected_model.as_deref())
        });
        self.selected_effort = self
            .selected_effort
            .take()
            .filter(|selected| {
                valid_efforts.is_some_and(|model| {
                    model
                        .effort_options
                        .iter()
                        .any(|effort| &effort.id == selected)
                })
            })
            .or_else(|| {
                capability.as_ref().and_then(|capability| {
                    capability.models.first().and_then(|model| {
                        model.default_effort.clone().or_else(|| {
                            model.effort_options.first().map(|effort| effort.id.clone())
                        })
                    })
                })
            });
        self.selected_permission = self
            .selected_permission
            .take()
            .filter(|selected| {
                capability.as_ref().is_some_and(|capability| {
                    capability
                        .permission_modes
                        .iter()
                        .any(|mode| &mode.id == selected)
                })
            })
            .or_else(|| {
                capability
                    .as_ref()
                    .and_then(|capability| capability.default_permission_mode.clone())
            });
    }

    fn ensure_project_for_provider(&mut self) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let selected_is_available = self.selected_project.is_some_and(|project_id| {
            snapshot.projects.iter().any(|project| {
                project.id == project_id
                    && project.valid
                    && project.enabled_providers.contains(&self.selected_provider)
            })
        });
        if !selected_is_available {
            self.selected_project = self
                .recent_projects
                .iter()
                .copied()
                .find(|project_id| {
                    snapshot.projects.iter().any(|project| {
                        project.id == *project_id
                            && project.valid
                            && project.enabled_providers.contains(&self.selected_provider)
                    })
                })
                .or_else(|| {
                    snapshot
                        .projects
                        .iter()
                        .find(|project| {
                            project.valid
                                && project.enabled_providers.contains(&self.selected_provider)
                        })
                        .map(|project| project.id)
                });
        }
    }

    fn provider_is_available(&self, provider: ProviderId) -> bool {
        self.snapshot
            .as_ref()
            .is_some_and(|snapshot| available_providers(snapshot).contains(&provider))
    }

    fn ensure_provider_available(&mut self) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let providers = available_providers(snapshot);
        if !providers.contains(&self.selected_provider)
            && let Some(provider) = providers.first()
        {
            self.selected_provider = *provider;
            self.selected_project = None;
            self.selected_conversation = None;
            self.draft_conversation = None;
        }
    }

    fn reindex_timeline(&mut self) {
        let (timeline_by_conversation, markdown_render_cache) = self
            .snapshot
            .as_ref()
            .map(|snapshot| index_timeline(&snapshot.timeline))
            .unwrap_or_default();
        self.timeline_by_conversation = timeline_by_conversation;
        self.markdown_render_cache = markdown_render_cache;
    }

    fn cache_markdown_item(&mut self, item: &TimelineItem) {
        let markdown = match &item.kind {
            TimelineItemKind::UserMessage { text }
            | TimelineItemKind::AgentMessage { text, .. } => text,
            _ => {
                self.markdown_render_cache.remove(&item.id);
                return;
            }
        };
        self.markdown_render_cache
            .insert(item.id, (item.revision, markdown_html(markdown)));
    }

    fn project_scope(
        &self,
        provider: ProviderId,
        project_id: ProjectId,
    ) -> Option<ProjectTreeScope> {
        Some(ProjectTreeScope {
            host_id: self.connection.as_ref()?.host_id,
            provider,
            project_id,
        })
    }

    fn expand_selected_project(&mut self) {
        if let Some(project_id) = self.selected_project
            && let Some(scope) = self.project_scope(self.selected_provider, project_id)
        {
            self.expanded_projects.insert(scope);
        }
    }

    fn project_is_expanded(&self, provider: ProviderId, project_id: ProjectId) -> bool {
        self.project_scope(provider, project_id)
            .is_some_and(|scope| self.expanded_projects.contains(&scope))
    }

    fn select_project(&mut self, project_id: ProjectId) {
        if self.pending_send.is_some() {
            self.status = "请先等待发送确认，或取消重试后再切换项目".to_owned();
            return;
        }
        let available = self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.projects.iter().any(|project| {
                project.id == project_id
                    && project.valid
                    && project.enabled_providers.contains(&self.selected_provider)
            })
        });
        if !available {
            self.status = "项目不属于当前 Host/Provider，已忽略选择".to_owned();
            return;
        }
        let changed = self.selected_project != Some(project_id);
        self.selected_project = Some(project_id);
        self.selected_conversation = None;
        self.draft_conversation = None;
        self.pending_attachments.clear();
        self.project_picker_open = false;
        self.sidebar_open = false;
        self.recent_projects
            .retain(|project| *project != project_id);
        self.recent_projects.insert(0, project_id);
        self.recent_projects.truncate(8);
        self.expand_selected_project();
        if changed {
            self.reset_dynamic_selection();
            self.sync_selected_project();
        }
    }

    fn select_conversation(&mut self, conversation_id: ConversationId) {
        if self.pending_send.is_some() {
            self.status = "请先等待发送确认，或取消重试后再切换对话".to_owned();
            return;
        }
        let conversation_scope = self.snapshot.as_ref().and_then(|snapshot| {
            let conversation = snapshot
                .conversations
                .iter()
                .find(|conversation| conversation.id == conversation_id)?;
            snapshot
                .projects
                .iter()
                .any(|project| {
                    project.id == conversation.project_id
                        && project.valid
                        && project.enabled_providers.contains(&conversation.provider)
                })
                .then_some((conversation.provider, conversation.project_id))
        });
        let Some((provider, project_id)) = conversation_scope else {
            self.status = "对话不属于已授权的 Host/Provider/Project，已忽略选择".to_owned();
            return;
        };
        let scope_changed =
            self.selected_provider != provider || self.selected_project != Some(project_id);
        self.selected_provider = provider;
        self.selected_project = Some(project_id);
        self.selected_conversation = Some(conversation_id);
        self.draft_conversation = None;
        self.pending_attachments.clear();
        self.sidebar_open = false;
        self.editing_title = false;
        self.recent_projects
            .retain(|project| *project != project_id);
        self.recent_projects.insert(0, project_id);
        self.recent_projects.truncate(8);
        self.expand_selected_project();
        if scope_changed {
            self.reset_dynamic_selection();
        }
        self.request_images_for_selected();
        self.request_selected_conversation_page();
    }

    fn selected_capability(&self) -> Option<&ProviderCapability> {
        let project_id = self.selected_project?;
        self.snapshot
            .as_ref()?
            .provider_capabilities
            .iter()
            .find(|capability| {
                capability.project_id == project_id && capability.provider == self.selected_provider
            })
    }

    fn selected_conversation_ref(&self) -> Option<&Conversation> {
        let id = self.selected_conversation?;
        self.snapshot
            .as_ref()?
            .conversations
            .iter()
            .find(|conversation| {
                conversation.id == id
                    && Some(conversation.project_id) == self.selected_project
                    && conversation.provider == self.selected_provider
            })
    }

    fn confirm_permission_change(&self, permission: &str) -> bool {
        let elevated = self.selected_capability().is_some_and(|capability| {
            capability
                .permission_modes
                .iter()
                .any(|mode| mode.id == permission && mode.risk == PermissionRisk::Elevated)
        });
        !elevated
            || window()
                .and_then(|window| {
                    window
                        .confirm_with_message(
                            "此权限模式由 Provider 标记为高风险，可能允许无需逐次确认的写入或命令。确定切换吗？",
                        )
                        .ok()
                })
                .unwrap_or(false)
    }

    fn sync_selected_project(&mut self) {
        if !self.authenticated || !self.connected {
            return;
        }
        let Some(project_id) = self.selected_project else {
            return;
        };
        let Some(scope) = self.project_scope(self.selected_provider, project_id) else {
            return;
        };
        if self
            .sync_in_flight
            .values()
            .any(|pending| *pending == scope)
        {
            return;
        }
        let command_id = CommandId::new();
        self.status = "同步中…".to_owned();
        if self.send_authenticated(ClientCommand::SyncProject {
            command_id,
            project_id,
            provider: self.selected_provider,
        }) {
            self.sync_in_flight.insert(command_id, scope);
        }
    }

    fn request_project_refresh(&mut self, provider: ProviderId) {
        if !self.authenticated || !self.connected || !self.refresh_in_flight.insert(provider) {
            return;
        }
        if !self.send_authenticated(ClientCommand::RefreshProjects { provider }) {
            self.refresh_in_flight.remove(&provider);
        }
    }

    fn take_client_attachments(&mut self) -> Vec<ClientAttachment> {
        self.pending_attachments
            .iter_mut()
            .filter_map(|attachment| {
                Some(ClientAttachment {
                    id: attachment.id,
                    file_name: attachment.file_name.clone(),
                    mime_type: attachment.mime_type.clone(),
                    bytes: attachment.bytes.take()?,
                })
            })
            .collect()
    }

    fn restore_pending_attachments(&mut self, command: ClientCommand) {
        let attachments = match command {
            ClientCommand::StartConversation { attachments, .. }
            | ClientCommand::SendMessage { attachments, .. } => attachments,
            _ => return,
        };
        for attachment in attachments {
            if let Some(pending) = self
                .pending_attachments
                .iter_mut()
                .find(|pending| pending.id == attachment.id)
            {
                pending.bytes = Some(attachment.bytes);
                pending.error = None;
            }
        }
    }

    fn read_files(&mut self, context: &Context<Self>, files: Vec<File>) {
        let Some(capability) = self.selected_capability().cloned() else {
            return;
        };
        let attachment_capability = capability.attachments;
        let available = usize::from(attachment_capability.max_count)
            .saturating_sub(self.pending_attachments.len());
        let mut selected_bytes = self
            .pending_attachments
            .iter()
            .filter(|attachment| attachment.error.is_none())
            .map(|attachment| attachment.byte_len)
            .sum::<u64>();
        for file in files.into_iter().take(available) {
            let id = AttachmentId::new();
            let file_name = file.name();
            let mime_type = file.type_();
            let error = if !attachment_capability.supported() {
                Some("当前 Provider 不支持输入附件".to_owned())
            } else if file.size() as u64 > attachment_capability.max_bytes {
                Some(format!(
                    "文件超过 {} MiB 限制",
                    attachment_capability.max_bytes / 1024 / 1024
                ))
            } else if selected_bytes + file.size() as u64 > attachment_capability.max_total_bytes {
                Some(format!(
                    "附件总大小超过 {} MiB 限制",
                    attachment_capability.max_total_bytes / 1024 / 1024
                ))
            } else if !attachment_capability
                .allowed_mime_types
                .iter()
                .any(|allowed| allowed == &mime_type)
            {
                Some(format!("不支持 {mime_type}"))
            } else {
                None
            };
            self.pending_attachments.push(BrowserAttachment {
                id,
                file_name: file_name.clone(),
                mime_type: mime_type.clone(),
                byte_len: file.size() as u64,
                bytes: None,
                error: error.clone(),
            });
            if error.is_some() {
                continue;
            }
            selected_bytes += file.size() as u64;
            let Ok(reader) = FileReader::new() else {
                context
                    .link()
                    .send_message(Msg::AttachmentFailed(id, "浏览器无法读取文件".to_owned()));
                continue;
            };
            let callback_reader = reader.clone();
            let link = context.link().clone();
            let onload = Closure::wrap(Box::new(move |_: Event| {
                let result = callback_reader
                    .result()
                    .ok()
                    .and_then(|value| value.dyn_into::<ArrayBuffer>().ok())
                    .map(|buffer| Uint8Array::new(&buffer).to_vec());
                match result {
                    Some(bytes) => link.send_message(Msg::AttachmentLoaded(
                        id,
                        file_name.clone(),
                        mime_type.clone(),
                        bytes,
                    )),
                    None => link
                        .send_message(Msg::AttachmentFailed(id, "浏览器读取附件失败".to_owned())),
                }
            }) as Box<dyn FnMut(_)>);
            reader.set_onloadend(Some(onload.as_ref().unchecked_ref()));
            if reader.read_as_array_buffer(&file).is_err() {
                context
                    .link()
                    .send_message(Msg::AttachmentFailed(id, "浏览器读取附件失败".to_owned()));
            }
            onload.forget();
        }
    }

    fn persist_cache(&self) {
        let (Some(connection), Some(snapshot)) = (&self.connection, &self.snapshot) else {
            return;
        };
        let mut snapshot = snapshot.clone();
        if snapshot.timeline.len() > 1_000 {
            snapshot
                .timeline
                .drain(..snapshot.timeline.len().saturating_sub(1_000));
        }
        let mut expanded_projects = self.expanded_projects.iter().copied().collect::<Vec<_>>();
        expanded_projects.sort_by_key(|scope| {
            (
                scope.host_id.to_string(),
                scope.provider.to_string(),
                scope.project_id.to_string(),
            )
        });
        save_cache(&AppCache {
            version: CACHE_VERSION,
            host_id: connection.host_id,
            snapshot,
            selected_conversation: self.selected_conversation,
            selected_project: self.selected_project,
            selected_provider: self.selected_provider,
            selected_model: self.selected_model.clone(),
            selected_effort: self.selected_effort.clone(),
            selected_permission: self.selected_permission.clone(),
            draft_conversation: self.draft_conversation,
            pending_command: self
                .pending_send
                .as_ref()
                .map(|pending| pending.command.clone()),
            composer: self.composer.clone(),
            sidebar_collapsed: self.sidebar_collapsed,
            pinned_projects: self.pinned_projects.clone(),
            recent_projects: self.recent_projects.clone(),
            expanded_projects,
            pending_send_state: self.pending_send.as_ref().map(|pending| pending.state),
            pending_rejection_code: self
                .pending_send
                .as_ref()
                .and_then(|pending| pending.rejection_code.clone()),
        });
    }

    fn schedule_cache_persist(&mut self, context: &Context<Self>) {
        self.cancel_cache_persist_timer();
        self.cache_persist_epoch = self.cache_persist_epoch.wrapping_add(1);
        let epoch = self.cache_persist_epoch;
        let Some(browser) = window() else {
            self.persist_cache();
            return;
        };
        let link = context.link().clone();
        let callback = Closure::once_into_js(move || link.send_message(Msg::PersistCache(epoch)));
        match browser.set_timeout_with_callback_and_timeout_and_arguments_0(
            callback.unchecked_ref(),
            CACHE_WRITE_DELAY_MS,
        ) {
            Ok(handle) => self.cache_persist_timer = Some(handle),
            Err(_) => self.persist_cache(),
        }
    }

    fn cancel_cache_persist_timer(&mut self) {
        if let Some(handle) = self.cache_persist_timer.take()
            && let Some(browser) = window()
        {
            browser.clear_timeout_with_handle(handle);
        }
    }

    fn restore_cache_for_connection(&mut self) {
        let Some(host_id) = self
            .connection
            .as_ref()
            .map(|connection| connection.host_id)
        else {
            return;
        };
        for url in self.attachments.values() {
            let _ = Url::revoke_object_url(url);
        }
        self.attachments.clear();
        self.timeline_by_conversation.clear();
        self.markdown_render_cache.clear();
        self.pending_attachments.clear();
        self.fullscreen_image = None;
        self.history_before.clear();
        self.history_exhausted.clear();
        self.history_requested.clear();
        self.conversation_search.clear();
        self.project_picker_open = false;
        self.project_search.clear();
        self.sidebar_open = false;
        self.editing_title = false;
        self.title_draft.clear();
        let Some(cache) = load_cache(host_id) else {
            self.snapshot = None;
            self.selected_conversation = None;
            self.selected_project = None;
            self.selected_provider = ProviderId::Codex;
            self.selected_model = None;
            self.selected_effort = None;
            self.selected_permission = None;
            self.draft_conversation = None;
            self.pending_send = None;
            self.composer.clear();
            self.sidebar_collapsed = false;
            self.pinned_projects.clear();
            self.recent_projects.clear();
            self.expanded_projects.clear();
            return;
        };
        let cache_needs_tree_default = cache.version < CACHE_VERSION;
        self.snapshot = Some(cache.snapshot);
        self.reindex_timeline();
        self.selected_conversation = cache.selected_conversation;
        self.selected_project = cache.selected_project;
        self.selected_provider = cache.selected_provider;
        self.selected_model = cache.selected_model;
        self.selected_effort = cache.selected_effort;
        self.selected_permission = cache.selected_permission;
        self.draft_conversation = cache.draft_conversation;
        self.pending_send = cache.pending_command.and_then(|command| {
            pending_send_from_cached(
                command,
                cache.pending_send_state,
                cache.pending_rejection_code,
            )
        });
        self.pending_attachments = self
            .pending_send
            .as_ref()
            .map_or_else(Vec::new, pending_browser_attachments);
        self.composer = cache.composer;
        self.sidebar_collapsed = cache.sidebar_collapsed;
        self.pinned_projects = cache.pinned_projects;
        self.recent_projects = cache.recent_projects;
        self.expanded_projects = cache.expanded_projects.into_iter().collect();
        if cache_needs_tree_default {
            self.expand_selected_project();
        }
    }

    fn request_images_for_selected(&mut self) {
        if !self.authenticated || !self.connected {
            return;
        }
        let Some(conversation_id) = self.selected_conversation else {
            return;
        };
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let attachment_ids = snapshot
            .timeline
            .iter()
            .filter_map(|item| {
                if item.conversation_id == conversation_id
                    && let TimelineItemKind::Image { attachment_id, .. } = item.kind
                    && !self.attachments.contains_key(&attachment_id)
                {
                    Some(attachment_id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for attachment_id in attachment_ids {
            self.send_authenticated(ClientCommand::GetAttachment { attachment_id });
        }
    }

    fn request_selected_conversation_page(&mut self) {
        if !self.authenticated || !self.connected {
            return;
        }
        let Some(conversation_id) = self.selected_conversation else {
            return;
        };
        if !self.history_requested.insert(conversation_id) {
            return;
        }
        if !self.send_authenticated(ClientCommand::GetConversationPage {
            conversation_id,
            before: None,
            limit: 100,
        }) {
            self.history_requested.remove(&conversation_id);
        }
    }

    fn view_connection(&self, link: &yew::html::Scope<Self>) -> Html {
        html! {
            <main class="connection-page">
                <section class="connection-card">
                    <div class="connection-logo">{"AR"}</div>
                    <p class="eyebrow">{"AGENT REMOTE MESSENGER"}</p>
                    <h1>{"连接你的编程 Agent"}</h1>
                    <p class="lead">{"在电脑上运行 Host，然后用 Host 输出的配对链接打开本页。项目和 Agent 始终留在电脑上。"}</p>
                    <div class="connection-status"><span class="status-dot"></span><span>{&self.status}</span></div>
                    {if self.credentials.is_empty() { html! {} } else { html! {
                        <div class="saved-hosts">
                            <h2>{"已配对 Host"}</h2>
                            {for self.credentials.iter().enumerate().map(|(index, credential)| {
                                let onclick = link.callback(move |_| Msg::ConnectStored(index));
                                html! { <button class="host-row" {onclick}><span>{credential.host_id.to_string()}</span><b>{"连接"}</b></button> }
                            })}
                        </div>
                    }}}
                    <label class="field">
                        <span>{"配对链接"}</span>
                        <input
                            placeholder="https://host/#host=…&pair=…"
                            value={self.pair_link.clone()}
                            oninput={link.callback(|event: InputEvent| Msg::PairLinkChanged(event.target_unchecked_into::<HtmlInputElement>().value()))}
                        />
                    </label>
                    <button class="primary wide" onclick={link.callback(|_| Msg::OpenPairLink)} disabled={self.pair_link.trim().is_empty()}>{"打开并配对"}</button>
                    {if self.credentials.is_empty() { html! {} } else { html! {
                        <button class="text-button" onclick={link.callback(|_| Msg::ForgetCredentials)}>{"删除此浏览器保存的设备凭证"}</button>
                    }}}
                </section>
            </main>
        }
    }

    fn view_project_tree(&self, link: &yew::html::Scope<Self>, snapshot: &Snapshot) -> Html {
        let query = self.conversation_search.trim().to_lowercase();
        let mut projects = snapshot
            .projects
            .iter()
            .filter(|project| project.enabled_providers.contains(&self.selected_provider))
            .collect::<Vec<_>>();
        projects.sort_by(|left, right| {
            right
                .last_activity_at_ms
                .cmp(&left.last_activity_at_ms)
                .then_with(|| left.display_name.cmp(&right.display_name))
        });
        let rows = projects
            .into_iter()
            .filter_map(|project| {
                let project_matches = !query.is_empty()
                    && (project.display_name.to_lowercase().contains(&query)
                        || project.short_path.to_lowercase().contains(&query));
                let mut conversations = snapshot
                    .conversations
                    .iter()
                    .filter(|conversation| {
                        conversation_belongs_to_project(
                            conversation,
                            project.id,
                            self.selected_provider,
                        )
                    })
                    .collect::<Vec<_>>();
                let conversation_count = conversations.len();
                let last_activity_at_ms = conversations
                    .iter()
                    .map(|conversation| conversation.updated_at_ms)
                    .max()
                    .max(project.last_activity_at_ms);
                let capability = snapshot.provider_capabilities.iter().find(|capability| {
                    capability.project_id == project.id
                        && capability.provider == self.selected_provider
                });
                let status_label = project_provider_status(project.valid, capability);
                conversations.retain(|conversation| {
                    query.is_empty()
                        || project_matches
                        || conversation.title.to_lowercase().contains(&query)
                });
                sort_conversations_newest_first(&mut conversations);
                if !query.is_empty() && !project_matches && conversations.is_empty() {
                    return None;
                }
                let expanded = self.project_is_expanded(self.selected_provider, project.id)
                    || (!query.is_empty() && (project_matches || !conversations.is_empty()));
                Some(self.view_project_tree_node(
                    link,
                    project,
                    conversations,
                    ProjectTreeMetadata {
                        conversation_count,
                        last_activity_at_ms,
                        status_label,
                    },
                    expanded,
                ))
            })
            .collect::<Vec<_>>();
        if rows.is_empty() {
            html! { <p class="tree-empty">{"没有匹配的项目或对话"}</p> }
        } else {
            html! { <>{for rows}</> }
        }
    }

    fn view_project_tree_node(
        &self,
        link: &yew::html::Scope<Self>,
        project: &agent_remote_protocol::ProjectSummary,
        conversations: Vec<&Conversation>,
        metadata: ProjectTreeMetadata,
        expanded: bool,
    ) -> Html {
        let project_id = project.id;
        let key = format!(
            "{}:{}:{}",
            self.connection
                .as_ref()
                .map_or_else(String::new, |connection| connection.host_id.to_string()),
            self.selected_provider,
            project.id
        );
        html! {
            <section key={key} class={classes!("project-tree-node", (Some(project.id) == self.selected_project).then_some("active"))}>
                <div class="project-tree-row">
                    <button
                        class="project-expand"
                        aria-label={if expanded {"折叠项目"} else {"展开项目"}}
                        aria-expanded={expanded.to_string()}
                        onclick={link.callback(move |_| Msg::ToggleProject(project_id))}
                    >{if expanded {"⌄"} else {"›"}}</button>
                    <button class="project-tree-main" disabled={!project.valid} onclick={link.callback(move |_| Msg::SelectProject(project_id))}>
                        <strong>{&project.display_name}</strong>
                        <small class="project-tree-meta">
                            <span>{metadata.status_label}</span>
                            <span>{format_relative_activity(metadata.last_activity_at_ms)}</span>
                            <span>{format!("{} 个对话", metadata.conversation_count)}</span>
                        </small>
                    </button>
                </div>
                {if expanded {html! {
                    <div class="project-conversations">
                        {if conversations.is_empty() {
                            html! {<p class="project-empty">{"暂无对话"}</p>}
                        } else {
                            html! {{for conversations.into_iter().map(|conversation| self.view_conversation_row(link, conversation))}}
                        }}
                    </div>
                }} else {html! {}}}
            </section>
        }
    }

    fn view_project_picker(&self, link: &yew::html::Scope<Self>, snapshot: &Snapshot) -> Html {
        let query = self.project_search.trim().to_lowercase();
        let mut projects = snapshot
            .projects
            .iter()
            .filter(|project| {
                project.valid
                    && project.enabled_providers.contains(&self.selected_provider)
                    && (query.is_empty()
                        || project.display_name.to_lowercase().contains(&query)
                        || project.short_path.to_lowercase().contains(&query))
            })
            .collect::<Vec<_>>();
        projects.sort_by(|left, right| {
            right
                .last_activity_at_ms
                .cmp(&left.last_activity_at_ms)
                .then_with(|| left.display_name.cmp(&right.display_name))
        });
        let pinned = projects
            .iter()
            .copied()
            .filter(|project| self.pinned_projects.contains(&project.id))
            .collect::<Vec<_>>();
        let recent = self
            .recent_projects
            .iter()
            .filter_map(|id| projects.iter().copied().find(|project| project.id == *id))
            .filter(|project| !self.pinned_projects.contains(&project.id))
            .collect::<Vec<_>>();
        html! {
            <div class="project-popover">
                <label><span>{"⌕"}</span><input autofocus=true placeholder="搜索项目" value={self.project_search.clone()} oninput={link.callback(|event: InputEvent| Msg::ProjectSearchChanged(event.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                {if pinned.is_empty() {html! {}} else {html! {<section><h3>{"固定"}</h3>{for pinned.into_iter().map(|project| self.view_project_row(link, project))}</section>}}}
                {if recent.is_empty() {html! {}} else {html! {<section><h3>{"最近"}</h3>{for recent.into_iter().map(|project| self.view_project_row(link, project))}</section>}}}
                <section><h3>{"全部项目"}</h3>{for projects.into_iter().map(|project| self.view_project_row(link, project))}</section>
            </div>
        }
    }

    fn view_project_row(
        &self,
        link: &yew::html::Scope<Self>,
        project: &agent_remote_protocol::ProjectSummary,
    ) -> Html {
        let id = project.id;
        let pinned = self.pinned_projects.contains(&id);
        html! {
            <div key={id.to_string()} class={classes!("project-row", (Some(id) == self.selected_project).then_some("active"))}>
                <button class="project-main" onclick={link.callback(move |_| Msg::SelectProject(id))}><strong>{&project.display_name}</strong><small>{format!("{} · {} 个对话", project.short_path, project.conversation_count)}</small></button>
                <button class="pin-button" title={if pinned {"取消固定"} else {"固定项目"}} onclick={link.callback(move |_| Msg::ToggleProjectPin(id))}>{if pinned {"★"} else {"☆"}}</button>
            </div>
        }
    }

    fn view_draft_chat(&self, link: &yew::html::Scope<Self>, snapshot: &Snapshot) -> Html {
        let project_name = self
            .selected_project
            .and_then(|id| snapshot.projects.iter().find(|project| project.id == id))
            .map_or("未知项目", |project| project.display_name.as_str());
        html! {
            <>
                <header class="chat-header">
                    <button class="mobile-menu" onclick={link.callback(|_| Msg::OpenSidebar)}>{"☰"}</button>
                    <div><p class="eyebrow">{format!("{} · {}", self.selected_provider, project_name)}</p><h1>{"新对话"}</h1><small>{"首次发送时才会创建远程会话"}</small></div>
                </header>
                <div class="timeline empty-draft"><p>{"写下第一条消息。当前项目的 Provider 默认设置会自动继承。"}</p></div>
                {self.view_composer(link, false, None)}
            </>
        }
    }

    fn view_conversation_row(
        &self,
        link: &yew::html::Scope<Self>,
        conversation: &Conversation,
    ) -> Html {
        let id = conversation.id;
        let onclick = link.callback(move |_| Msg::SelectConversation(id));
        html! {
            <button key={id.to_string()} class={classes!("conversation-row", (Some(id) == self.selected_conversation).then_some("active"))} {onclick}>
                <span class={classes!("provider-badge", provider_class(conversation.provider))}>{provider_short(conversation.provider)}</span>
                <span class="conversation-copy"><strong>{&conversation.title}</strong><small>{state_label(conversation.state)}</small></span>
            </button>
        }
    }

    fn view_chat(
        &self,
        link: &yew::html::Scope<Self>,
        conversation: &Conversation,
        snapshot: &Snapshot,
    ) -> Html {
        let project_name = snapshot
            .projects
            .iter()
            .find(|project| project.id == conversation.project_id)
            .map_or("未知项目", |project| project.display_name.as_str());
        let capability = snapshot.provider_capabilities.iter().find(|capability| {
            capability.project_id == conversation.project_id
                && capability.provider == conversation.provider
        });
        let running = conversation.state == ConversationState::Running
            || conversation.state == ConversationState::NeedsApproval;
        html! {
            <>
                <header class="chat-header">
                    <button class="mobile-menu" onclick={link.callback(|_| Msg::OpenSidebar)}>{"☰"}</button>
                    <div class="chat-title"><p class="eyebrow">{format!("{} · {}", conversation.provider, project_name)}</p>
                        {if self.editing_title {html! {<div class="title-editor"><input value={self.title_draft.clone()} maxlength="80" oninput={link.callback(|event: InputEvent| Msg::TitleChanged(event.target_unchecked_into::<HtmlInputElement>().value()))}/><button onclick={link.callback(|_| Msg::SaveTitle)}>{"保存"}</button></div>}} else {html! {<button class="title-button" onclick={link.callback(|_| Msg::EditTitle)}><h1>{&conversation.title}</h1><span>{"✎"}</span></button>}}}
                    </div>
                    <div class="header-controls">
                        <span class={classes!("state-pill", state_class(conversation.state))}>{state_label(conversation.state)}</span>
                    </div>
                </header>
                <div class="timeline">
                    {if self.history_exhausted.contains(&conversation.id) {html! {}} else {html! {<button class="load-older" onclick={link.callback(|_| Msg::LoadOlder)}>{"加载更早消息"}</button>}}}
                    {self.view_timeline(link, self.timeline_by_conversation.get(&conversation.id).map_or(&[][..], Vec::as_slice))}
                </div>
                {self.view_composer(link, running, capability)}
            </>
        }
    }

    fn view_composer(
        &self,
        link: &yew::html::Scope<Self>,
        running: bool,
        capability_override: Option<&ProviderCapability>,
    ) -> Html {
        let capability = capability_override.or_else(|| self.selected_capability());
        let conversation = self.selected_conversation_ref();
        let selected_model = conversation
            .and_then(|conversation| conversation.selected_model.as_deref())
            .or(self.selected_model.as_deref());
        let selected_effort = conversation
            .and_then(|conversation| conversation.selected_effort.as_deref())
            .or(self.selected_effort.as_deref());
        let models = capability.map_or(&[][..], |capability| capability.models.as_slice());
        let efforts = models
            .iter()
            .find(|model| Some(model.id.as_str()) == selected_model)
            .map_or(&[][..], |model| model.effort_options.as_slice());
        let permission_option = conversation.and_then(|conversation| {
            conversation
                .session_options
                .iter()
                .find(|option| option.id == "permission_mode")
        });
        let selected_permission = permission_option
            .map(|option| option.current_value.as_str())
            .or(self.selected_permission.as_deref());
        let attachment_capability = capability.map(|capability| &capability.attachments);
        let accepts = attachment_capability
            .map(|capability| capability.allowed_mime_types.join(","))
            .unwrap_or_default();
        let attachment_enabled = attachment_capability.is_some_and(|capability| {
            self.pending_send.is_none()
                && capability.supported()
                && self.pending_attachments.len() < usize::from(capability.max_count)
                && self
                    .pending_attachments
                    .iter()
                    .filter(|attachment| attachment.error.is_none())
                    .map(|attachment| attachment.byte_len)
                    .sum::<u64>()
                    < capability.max_total_bytes
        });
        let files_on_change = link.callback(|event: Event| {
            Msg::FilesSelected(files_from_input(
                &event.target_unchecked_into::<HtmlInputElement>(),
            ))
        });
        let ondrop = link.callback(|event: DragEvent| {
            event.prevent_default();
            Msg::FilesSelected(files_from_data_transfer(event.data_transfer()))
        });
        let onpaste = link.callback(|event: Event| {
            let event: ClipboardEvent = event.unchecked_into();
            Msg::FilesSelected(files_from_data_transfer(event.clipboard_data()))
        });
        html! {
            <footer class="composer" {ondrop} ondragover={yew::Callback::from(|event: DragEvent| event.prevent_default())} {onpaste}>
                {if let Some(pending) = &self.pending_send {
                    let retryable = pending_can_retry(pending);
                    let failed = matches!(pending.state, PendingSendState::WriteFailed | PendingSendState::Rejected);
                    html! {
                        <div class={classes!("pending-send-status", failed.then_some("failed"))}>
                            <span>{match pending.state {
                                PendingSendState::Queued => "已排队，等待连接",
                                PendingSendState::AwaitingAck => "正在发送，等待 Host 确认",
                                PendingSendState::WriteFailed => "WebSocket 写入失败，草稿与附件已保留",
                                PendingSendState::Rejected => "Host 拒绝了发送，草稿与附件已保留",
                            }}</span>
                            {pending.error.as_ref().map(|error| html! {<small>{error}</small>}).unwrap_or_default()}
                            {if failed {html! {
                                <div>
                                    {if retryable {html! {<button onclick={link.callback(|_| Msg::RetrySend)} disabled={!self.authenticated || !self.connected}>{"重试"}</button>}} else {html! {}}}
                                    <button onclick={link.callback(|_| Msg::DismissPendingSend)}>{"修改草稿"}</button>
                                </div>
                            }} else {html! {}}}
                        </div>
                    }
                } else {html! {}}}
                {if self.pending_attachments.is_empty() {html! {}} else {html! {<div class="attachment-chips">{for self.pending_attachments.iter().map(|attachment| {
                    let id = attachment.id;
                    let attachment_status = if attachment.bytes.is_some() {"已就绪"} else if self.pending_send.is_some() {"已保留"} else {"读取中…"};
                    html! {<span key={id.to_string()} class={classes!(attachment.error.is_some().then_some("failed"))}><b>{&attachment.file_name}</b><small>{attachment.error.as_deref().unwrap_or(attachment_status)}</small><button disabled={self.pending_send.is_some()} onclick={link.callback(move |_| Msg::RemoveAttachment(id))}>{"×"}</button></span>}
                })}</div>}}}
                <textarea
                    placeholder={if running {"输入追加指令（Provider 支持时可用）"} else {"发送消息…"}}
                    value={self.composer.clone()}
                    disabled={self.pending_send.is_some()}
                    oninput={link.callback(|event: InputEvent| Msg::ComposerChanged(event.target_unchecked_into::<HtmlTextAreaElement>().value()))}
                />
                <div class="composer-bar">
                    <div class="composer-left">
                        <label class={classes!("attachment-action", (!attachment_enabled).then_some("disabled"))} title={attachment_capability.map_or("当前 Provider 不支持附件".to_owned(), |capability| format!("最多 {} 个，每个 {} MiB，总计 {} MiB", capability.max_count, capability.max_bytes / 1024 / 1024, capability.max_total_bytes / 1024 / 1024))}><span>{"＋"}</span><span class="action-label">{"附件"}</span><input type="file" multiple=true accept={accepts} disabled={!attachment_enabled} onchange={files_on_change}/></label>
                        {if let Some(option) = permission_option {html! {<select class="permission-select" disabled={running} title="权限" onchange={{let id=option.id.clone(); link.callback(move |event: Event| Msg::SetSessionOption(id.clone(), event.target_unchecked_into::<HtmlSelectElement>().value()))}}>{for option.values.iter().map(|value| html! {<option value={value.value.clone()} selected={value.value == option.current_value}>{&value.display_name}</option>})}</select>}} else if let Some(capability) = capability {html! {<select class="permission-select" title="权限" disabled={capability.permission_modes.is_empty()} onchange={link.callback(|event: Event| Msg::SelectPermission(nonempty(event.target_unchecked_into::<HtmlSelectElement>().value())))}><option value="" selected={selected_permission.is_none()}>{"Provider 按次审批"}</option>{for capability.permission_modes.iter().map(|mode| html! {<option value={mode.id.clone()} selected={Some(mode.id.as_str()) == selected_permission}>{&mode.display_name}</option>})}</select>}} else {html! {}}}
                    </div>
                    <div class="composer-right">
                        <div class="model-effort" title="模型 · effort">
                            <select disabled={running || models.is_empty()} onchange={if conversation.is_some() {{link.callback(|event: Event| Msg::SetSessionOption("model".to_owned(), event.target_unchecked_into::<HtmlSelectElement>().value()))}} else {{link.callback(|event: Event| Msg::SelectModel(nonempty(event.target_unchecked_into::<HtmlSelectElement>().value())))}}}>{for models.iter().map(|model| html! {<option value={model.id.clone()} selected={Some(model.id.as_str()) == selected_model}>{&model.display_name}</option>})}</select>
                            {if efforts.is_empty() {html! {}} else {html! {<><span>{"·"}</span><select disabled={running} onchange={if conversation.is_some() {{link.callback(|event: Event| Msg::SetSessionOption("reasoning_effort".to_owned(), event.target_unchecked_into::<HtmlSelectElement>().value()))}} else {{link.callback(|event: Event| Msg::SelectEffort(nonempty(event.target_unchecked_into::<HtmlSelectElement>().value())))}}}>{for efforts.iter().map(|effort| html! {<option value={effort.id.clone()} selected={Some(effort.id.as_str()) == selected_effort}>{&effort.display_name}</option>})}</select></>}}}
                        </div>
                        {if running {html! {<button class="stop" onclick={link.callback(|_| Msg::Interrupt)}>{"停止"}</button>}} else {html! {}}}
                        {if running && capability.is_some_and(|capability| capability.supports_steer) {html! {<button class="secondary" onclick={link.callback(|_| Msg::Steer)} disabled={self.composer.trim().is_empty()}>{"追加"}</button>}} else {html! {}}}
                        <button class="send-button" onclick={link.callback(|_| Msg::Send)} disabled={!self.connected || !self.authenticated || running || self.pending_send.is_some() || self.composer.trim().is_empty() || self.pending_attachments.iter().any(|attachment| attachment.bytes.is_none() || attachment.error.is_some())}>{if self.pending_send.is_some() {"…"} else {"↑"}}</button>
                    </div>
                </div>
            </footer>
        }
    }

    fn view_timeline(&self, link: &yew::html::Scope<Self>, items: &[TimelineItem]) -> Html {
        let mut rendered = Vec::new();
        let mut index = 0;
        while index < items.len() {
            if is_activity(&items[index].kind) {
                let start = index;
                while index < items.len() && is_activity(&items[index].kind) {
                    index += 1;
                }
                rendered.push(self.view_activity_group(link, &items[start..index]));
            } else {
                rendered.push(self.view_timeline_item(link, &items[index]));
                index += 1;
            }
        }
        html! {{for rendered}}
    }

    fn view_activity_group(&self, link: &yew::html::Scope<Self>, items: &[TimelineItem]) -> Html {
        let summary = activity_summary(items);
        let key = items
            .first()
            .map(|item| item.id.to_string())
            .unwrap_or_default();
        html! {
            <details key={key} class="activity-group">
                <summary><span>{"⌁"}</span><strong>{summary}</strong><small>{format!("{} 项", items.len())}</small></summary>
                <div class="activity-details">{for items.iter().map(|item| self.view_timeline_item(link, item))}</div>
            </details>
        }
    }

    fn view_timeline_item(&self, link: &yew::html::Scope<Self>, item: &TimelineItem) -> Html {
        match &item.kind {
            TimelineItemKind::UserMessage { text } => {
                let copy_text = text.clone();
                html! { <article key={item.id.to_string()} class="bubble user"><button class="message-copy" aria-label="复制用户消息原文" title="复制原文" onclick={link.callback(move |_| Msg::CopyText(copy_text.clone()))}>{"复制"}</button><div class="markdown-body">{self.cached_markdown(item, text)}</div></article> }
            }
            TimelineItemKind::AgentMessage { phase, text } => {
                let copy_text = text.clone();
                html! { <article key={item.id.to_string()} class={classes!("bubble", "agent", format!("phase-{phase:?}").to_lowercase())}><button class="message-copy" aria-label="复制 Agent 消息原文" title="复制原文" onclick={link.callback(move |_| Msg::CopyText(copy_text.clone()))}>{"复制"}</button><span class="item-label">{format!("{phase:?}")}</span><div class="markdown-body">{self.cached_markdown(item, text)}</div></article> }
            }
            TimelineItemKind::Progress {
                kind,
                status,
                label,
                detail,
            } => {
                html! { <article key={item.id.to_string()} class="process-card"><div class="process-title"><span>{format!("{kind:?}")}</span><b>{label}</b><em>{format!("{status:?}")}</em></div>{detail.as_ref().map(|value| html! {<pre>{value}</pre>}).unwrap_or_default()}</article> }
            }
            TimelineItemKind::Plan { steps } => {
                html! { <article key={item.id.to_string()} class="process-card plan"><strong>{"计划"}</strong><ol>{for steps.iter().map(|step| html! {<li class={state_class_for_item(step.status)}>{&step.text}</li>})}</ol></article> }
            }
            TimelineItemKind::ToolCall {
                name,
                status,
                input_summary,
                output_summary,
            } => {
                html! { <article key={item.id.to_string()} class="process-card"><div class="process-title"><span>{"工具"}</span><b>{name}</b><em>{format!("{status:?}")}</em></div>{summary_pair(input_summary, output_summary)}</article> }
            }
            TimelineItemKind::Command {
                command,
                relative_cwd,
                status,
                exit_code,
                output,
            } => {
                html! { <article key={item.id.to_string()} class="process-card command"><div class="process-title"><span>{"命令"}</span><b>{relative_cwd.as_deref().unwrap_or("项目根目录")}</b><em>{format!("{status:?}{}", exit_code.map(|code| format!(" · {code}")).unwrap_or_default())}</em></div><code>{command}</code>{output.as_ref().map(|value| html! {<pre>{value}</pre>}).unwrap_or_default()}</article> }
            }
            TimelineItemKind::FileChange {
                relative_path,
                change_kind,
                status,
            } => {
                html! { <article key={item.id.to_string()} class="process-card"><div class="process-title"><span>{"文件"}</span><b>{relative_path}</b><em>{format!("{} · {status:?}", change_kind)}</em></div></article> }
            }
            TimelineItemKind::Approval {
                approval_id,
                prompt,
                options,
                resolved_option,
            } => {
                let id = *approval_id;
                html! { <article key={item.id.to_string()} class="approval-card"><span class="item-label">{"需要权限"}</span><p>{prompt}</p><div class="approval-actions">{if let Some(resolved) = resolved_option { html! {<b>{format!("已选择：{resolved}")}</b>} } else { html! {{for options.iter().map(|option| { let value=option.id.clone(); html! {<button onclick={link.callback(move |_| Msg::ResolveApproval(id, value.clone()))}>{&option.label}</button>} })}} }}</div></article> }
            }
            TimelineItemKind::Image { attachment_id, alt } => {
                let id = *attachment_id;
                let onclick = link.callback(move |_| Msg::OpenImage(id));
                html! { <article key={item.id.to_string()} class="image-card" {onclick}>{self.attachments.get(attachment_id).map(|url| html! {<img src={url.clone()} alt={alt.clone()} />}).unwrap_or_else(|| html! {<div class="image-loading">{"正在读取图片…"}</div>})}<span>{alt}</span></article> }
            }
            TimelineItemKind::Error { code, message } => {
                html! { <article key={item.id.to_string()} class="error-card"><b>{code}</b><p>{message}</p></article> }
            }
        }
    }

    fn view_fullscreen_image(&self, link: &yew::html::Scope<Self>) -> Html {
        let Some(id) = self.fullscreen_image else {
            return html! {};
        };
        html! { <div class="lightbox" onclick={link.callback(|_: MouseEvent| Msg::CloseImage)}>{self.attachments.get(&id).map(|url| html! {<img src={url.clone()} alt="Agent output" />}).unwrap_or_default()}<button>{"关闭"}</button></div> }
    }

    fn cached_markdown(&self, item: &TimelineItem, fallback: &str) -> Html {
        self.markdown_render_cache
            .get(&item.id)
            .filter(|(revision, _)| *revision == item.revision)
            .map_or_else(|| markdown_html(fallback), |(_, rendered)| rendered.clone())
    }
}

fn open_socket(
    context: &Context<App>,
    config: &ConnectionConfig,
    generation: u64,
) -> Result<(WebSocket, SocketCallbacks), String> {
    let base = config.origin.trim_end_matches('/');
    let websocket_origin = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        return Err("配对来源必须是 http:// 或 https://".to_owned());
    };
    let endpoint = if config.relay {
        format!("{websocket_origin}/client/{}", config.host_id)
    } else {
        format!("{websocket_origin}/ws")
    };
    let socket = WebSocket::new_with_str(&endpoint, WS_SUBPROTOCOL)
        .map_err(|_| format!("无法打开 {endpoint}"))?;
    socket.set_binary_type(BinaryType::Arraybuffer);

    let link = context.link().clone();
    let open = Closure::wrap(
        Box::new(move |_: Event| link.send_message(Msg::Opened(generation))) as Box<dyn FnMut(_)>,
    );
    socket.set_onopen(Some(open.as_ref().unchecked_ref()));

    let link = context.link().clone();
    let message = Closure::wrap(Box::new(move |event: MessageEvent| {
        if let Ok(buffer) = event.data().dyn_into::<ArrayBuffer>() {
            let bytes = Uint8Array::new(&buffer).to_vec();
            match decode::<ServerMessage>(&bytes) {
                Ok(message) => link.send_message(Msg::Server(generation, message)),
                Err(error) => {
                    link.send_message(Msg::DecodeError(generation, error.to_string()));
                }
            }
        }
    }) as Box<dyn FnMut(_)>);
    socket.set_onmessage(Some(message.as_ref().unchecked_ref()));

    let link = context.link().clone();
    let error =
        Closure::wrap(
            Box::new(move |_: Event| link.send_message(Msg::SocketError(generation)))
                as Box<dyn FnMut(_)>,
        );
    socket.set_onerror(Some(error.as_ref().unchecked_ref()));

    let link = context.link().clone();
    let close = Closure::wrap(Box::new(move |event: CloseEvent| {
        link.send_message(Msg::Closed(generation, event.reason()))
    }) as Box<dyn FnMut(_)>);
    socket.set_onclose(Some(close.as_ref().unchecked_ref()));

    Ok((
        socket,
        SocketCallbacks {
            _open: open,
            _message: message,
            _error: error,
            _close: close,
        },
    ))
}

fn fragment_connection() -> Option<ConnectionConfig> {
    let browser = window()?;
    let location = browser.location();
    let hash = location.hash().ok()?;
    if hash.len() <= 1 {
        return None;
    }
    let params = UrlSearchParams::new_with_str(&hash[1..]).ok()?;
    let host_id = HostId(Uuid::parse_str(&params.get("host")?).ok()?);
    let pair_token = params.get("pair");
    let relay = params.get("relay").is_some_and(|value| value == "1");
    Some(ConnectionConfig {
        host_id,
        pair_token,
        credential: None,
        origin: location.origin().ok()?,
        relay,
    })
}

fn clear_fragment() {
    if let Some(browser) = window() {
        let location = browser.location();
        let clean = format!(
            "{}{}",
            location.pathname().unwrap_or_default(),
            location.search().unwrap_or_default()
        );
        let _ = browser
            .history()
            .and_then(|history| history.replace_state_with_url(&JsValue::NULL, "", Some(&clean)));
    }
}

fn load_credentials() -> Vec<StoredCredential> {
    window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(CREDENTIALS_KEY).ok().flatten())
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn save_credentials(credentials: &[StoredCredential]) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten())
        && let Ok(json) = serde_json::to_string(credentials)
    {
        let _ = storage.set_item(CREDENTIALS_KEY, &json);
    }
}

fn load_last_host() -> Option<HostId> {
    window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(LAST_HOST_KEY).ok().flatten())
        .and_then(|value| Uuid::parse_str(&value).ok())
        .map(HostId)
}

fn save_last_host(host_id: HostId) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten()) {
        let _ = storage.set_item(LAST_HOST_KEY, &host_id.to_string());
    }
}

fn cache_key(host_id: HostId) -> String {
    format!("{CACHE_PREFIX}{host_id}")
}

fn load_cache(host_id: HostId) -> Option<AppCache> {
    window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(&cache_key(host_id)).ok().flatten())
        .and_then(|json| serde_json::from_str::<AppCache>(&json).ok())
        .filter(|cache| {
            (2..=CACHE_VERSION).contains(&cache.version)
                && cache.host_id == host_id
                && cache.snapshot.host_id == host_id
        })
}

fn save_cache(cache: &AppCache) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten())
        && let Ok(json) = serde_json::to_string(cache)
    {
        let _ = storage.set_item(&cache_key(cache.host_id), &json);
    }
}

fn remove_cache(host_id: HostId) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten()) {
        let _ = storage.remove_item(&cache_key(host_id));
    }
}

fn files_from_input(input: &HtmlInputElement) -> Vec<File> {
    input.files().map(files_from_list).unwrap_or_default()
}

fn files_from_data_transfer(transfer: Option<DataTransfer>) -> Vec<File> {
    transfer
        .and_then(|transfer| transfer.files())
        .map(files_from_list)
        .unwrap_or_default()
}

fn files_from_list(files: web_sys::FileList) -> Vec<File> {
    (0..files.length())
        .filter_map(|index| files.get(index))
        .collect()
}

fn browser_device_name() -> String {
    "Browser device".to_owned()
}

fn image_object_url(mime_type: &str, bytes: &[u8]) -> Option<String> {
    let data = Uint8Array::from(bytes);
    let parts = Array::new();
    parts.push(&data);
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(mime_type);
    let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &options).ok()?;
    Url::create_object_url_with_blob(&blob).ok()
}

fn pending_send_from_cached(
    mut command: ClientCommand,
    state: Option<PendingSendState>,
    rejection_code: Option<String>,
) -> Option<PendingSend> {
    let command_id = command.command_id()?;
    let client_message_id = match &mut command {
        ClientCommand::StartConversation {
            client_message_id, ..
        }
        | ClientCommand::SendMessage {
            client_message_id, ..
        } => client_message_id
            .get_or_insert_with(|| Uuid::new_v4().to_string())
            .clone(),
        ClientCommand::Steer { .. } => format!("steer:{command_id}"),
        _ => return None,
    };
    Some(PendingSend {
        command_id,
        client_message_id,
        command,
        state: state.unwrap_or(PendingSendState::AwaitingAck),
        error: None,
        rejection_code,
    })
}

fn command_is_send(command: &ClientCommand) -> bool {
    matches!(
        command,
        ClientCommand::StartConversation { .. } | ClientCommand::SendMessage { .. }
    )
}

fn pending_can_retry(pending: &PendingSend) -> bool {
    match pending.state {
        PendingSendState::WriteFailed => true,
        PendingSendState::Rejected => pending
            .rejection_code
            .as_deref()
            .is_some_and(|code| retryable_send_rejection(&pending.command, code)),
        PendingSendState::Queued | PendingSendState::AwaitingAck => false,
    }
}

fn pending_browser_attachments(pending: &PendingSend) -> Vec<BrowserAttachment> {
    let attachments = match &pending.command {
        ClientCommand::StartConversation { attachments, .. }
        | ClientCommand::SendMessage { attachments, .. } => attachments,
        _ => return Vec::new(),
    };
    attachments
        .iter()
        .map(|attachment| BrowserAttachment {
            id: attachment.id,
            file_name: attachment.file_name.clone(),
            mime_type: attachment.mime_type.clone(),
            byte_len: attachment.bytes.len() as u64,
            bytes: Some(attachment.bytes.clone()),
            error: None,
        })
        .collect()
}

fn trace_send_stage(
    stage: &str,
    command_id: CommandId,
    client_message_id: &str,
    generation: Option<u64>,
    elapsed_ms: Option<u64>,
) {
    let payload = serde_json::json!({
        "event": "send_latency",
        "stage": stage,
        "commandId": command_id.to_string(),
        "clientMessageId": client_message_id,
        "generation": generation,
        "elapsedMs": elapsed_ms,
        "timestampMs": js_sys::Date::now() as u64,
    });
    web_sys::console::info_1(&JsValue::from_str(&payload.to_string()));
}

fn trace_server_send_stage(
    stage: SendTraceStage,
    command_id: CommandId,
    client_message_id: &str,
    conversation_id: ConversationId,
    click_elapsed_ms: Option<u64>,
    host_elapsed_ms: u64,
    generation: u64,
) {
    let stage = match stage {
        SendTraceStage::HostReceived => "host_received",
        SendTraceStage::ProviderReceived => "provider_received",
        SendTraceStage::FirstProviderEvent => "first_provider_event",
    };
    let payload = serde_json::json!({
        "event": "send_latency",
        "stage": stage,
        "commandId": command_id.to_string(),
        "clientMessageId": client_message_id,
        "conversationId": conversation_id.to_string(),
        "generation": generation,
        "elapsedMs": click_elapsed_ms,
        "hostElapsedMs": host_elapsed_ms,
        "timestampMs": js_sys::Date::now() as u64,
    });
    web_sys::console::info_1(&JsValue::from_str(&payload.to_string()));
}

fn monotonic_now_ms() -> f64 {
    window()
        .and_then(|browser| browser.performance())
        .map_or_else(js_sys::Date::now, |performance| performance.now())
}

fn format_relative_activity(timestamp_ms: Option<i64>) -> String {
    let Some(timestamp_ms) = timestamp_ms else {
        return "暂无活动".to_owned();
    };
    let age_ms = (js_sys::Date::now() as i64)
        .saturating_sub(timestamp_ms)
        .max(0);
    let minutes = age_ms / 60_000;
    if minutes < 1 {
        "刚刚".to_owned()
    } else if minutes < 60 {
        format!("{minutes} 分钟前")
    } else if minutes < 24 * 60 {
        format!("{} 小时前", minutes / 60)
    } else {
        format!("{} 天前", minutes / (24 * 60))
    }
}

fn project_provider_status(
    project_valid: bool,
    capability: Option<&ProviderCapability>,
) -> &'static str {
    if !project_valid {
        return "不可用";
    }
    let Some(capability) = capability else {
        return "待同步";
    };
    match capability.health.state {
        ProviderState::Ready if capability.limitation.is_none() => "可用",
        ProviderState::Ready => "受限",
        ProviderState::Starting => "同步中",
        ProviderState::NotInstalled => "未安装",
        ProviderState::NotAuthenticated => "未认证",
        ProviderState::Crashed => "已崩溃",
        ProviderState::ProtocolIncompatible => "不兼容",
        ProviderState::Offline => "离线",
    }
}

fn upsert_conversation(conversations: &mut Vec<Conversation>, incoming: Conversation) -> bool {
    if let Some(index) = conversations.iter().position(|item| item.id == incoming.id) {
        let existing = &conversations[index];
        if existing.project_id != incoming.project_id || existing.provider != incoming.provider {
            return false;
        }
        if incoming.revision < existing.revision {
            return true;
        }
        conversations.remove(index);
    }
    let index = conversations.partition_point(|item| item.updated_at_ms >= incoming.updated_at_ms);
    conversations.insert(index, incoming);
    true
}

fn upsert_project(
    projects: &mut Vec<agent_remote_protocol::ProjectSummary>,
    incoming: agent_remote_protocol::ProjectSummary,
) {
    if let Some(existing) = projects.iter_mut().find(|item| item.id == incoming.id) {
        *existing = incoming;
    } else {
        projects.push(incoming);
    }
    projects.sort_by(|left, right| {
        right
            .last_activity_at_ms
            .cmp(&left.last_activity_at_ms)
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
}

fn refresh_project_metrics(snapshot: &mut Snapshot, project_id: ProjectId) {
    let conversations = snapshot
        .conversations
        .iter()
        .filter(|conversation| conversation.project_id == project_id)
        .collect::<Vec<_>>();
    if let Some(project) = snapshot
        .projects
        .iter_mut()
        .find(|project| project.id == project_id)
    {
        project.conversation_count = conversations.len() as u32;
        project.last_activity_at_ms = conversations
            .iter()
            .map(|conversation| conversation.updated_at_ms)
            .max();
    }
}

fn upsert_timeline_item(items: &mut Vec<TimelineItem>, incoming: TimelineItem) -> bool {
    if let Some(existing) = items.iter_mut().find(|item| item.id == incoming.id) {
        if incoming.revision >= existing.revision {
            *existing = incoming;
            return true;
        }
        return false;
    }
    let key = (incoming.created_at_ms, incoming.id);
    let index = items.partition_point(|item| (item.created_at_ms, item.id) <= key);
    items.insert(index, incoming);
    true
}

fn upsert_capability(items: &mut Vec<ProviderCapability>, incoming: ProviderCapability) {
    if let Some(existing) = items
        .iter_mut()
        .find(|item| item.provider == incoming.provider && item.project_id == incoming.project_id)
    {
        *existing = incoming;
    } else {
        items.push(incoming);
    }
}

fn nonempty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}
fn provider_short(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Codex => "C",
        ProviderId::Grok => "G",
    }
}
fn provider_class(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Codex => "codex",
        ProviderId::Grok => "grok",
    }
}
fn state_label(state: ConversationState) -> &'static str {
    match state {
        ConversationState::Idle => "空闲",
        ConversationState::Running => "运行中",
        ConversationState::NeedsApproval => "等待审批",
        ConversationState::Completed => "已完成",
        ConversationState::Failed => "失败",
        ConversationState::Interrupted => "已中止",
        ConversationState::Offline => "离线",
    }
}
fn state_class(state: ConversationState) -> &'static str {
    match state {
        ConversationState::Running => "running",
        ConversationState::NeedsApproval => "approval",
        ConversationState::Completed => "completed",
        ConversationState::Failed => "failed",
        ConversationState::Interrupted => "interrupted",
        ConversationState::Offline => "offline",
        ConversationState::Idle => "idle",
    }
}
fn state_class_for_item(status: agent_remote_protocol::ItemStatus) -> &'static str {
    match status {
        agent_remote_protocol::ItemStatus::Completed => "done",
        agent_remote_protocol::ItemStatus::Running => "running",
        agent_remote_protocol::ItemStatus::Failed => "failed",
        _ => "pending",
    }
}

fn is_activity(kind: &TimelineItemKind) -> bool {
    matches!(
        kind,
        TimelineItemKind::Progress { .. }
            | TimelineItemKind::ToolCall { .. }
            | TimelineItemKind::Command { .. }
            | TimelineItemKind::FileChange { .. }
            | TimelineItemKind::Approval { .. }
            | TimelineItemKind::Error { .. }
    )
}

fn activity_summary(items: &[TimelineItem]) -> String {
    let mut files = 0;
    let mut commands = 0;
    let mut tests = 0;
    let mut tools = 0;
    let mut approvals = 0;
    let mut errors = 0;
    for item in items {
        match &item.kind {
            TimelineItemKind::FileChange { .. } => files += 1,
            TimelineItemKind::Command { .. } => commands += 1,
            TimelineItemKind::Progress { kind, .. } => match kind {
                agent_remote_protocol::ProgressKind::File => files += 1,
                agent_remote_protocol::ProgressKind::Command => commands += 1,
                agent_remote_protocol::ProgressKind::Test => tests += 1,
                _ => tools += 1,
            },
            TimelineItemKind::ToolCall { .. } => tools += 1,
            TimelineItemKind::Approval { .. } => approvals += 1,
            TimelineItemKind::Error { .. } => errors += 1,
            _ => {}
        }
    }
    let mut parts = Vec::new();
    for (count, label) in [
        (files, "文件"),
        (commands, "命令"),
        (tests, "测试"),
        (tools, "工具"),
        (approvals, "审批"),
        (errors, "错误"),
    ] {
        if count > 0 {
            parts.push(format!("{label} {count}"));
        }
    }
    format!("活动 · {}", parts.join(" · "))
}

fn summary_pair(input: &Option<String>, output: &Option<String>) -> Html {
    let input = visible_tool_summary(input.as_deref());
    let output =
        visible_tool_summary(output.as_deref()).filter(|value| Some(value) != input.as_ref());
    html! { <>{input.map(|value| html! {<pre>{value}</pre>}).unwrap_or_default()}{output.map(|value| html! {<pre>{value}</pre>}).unwrap_or_default()}</> }
}

fn markdown_html(markdown: &str) -> Html {
    Html::from_html_unchecked(markdown_to_safe_html(markdown).into())
}

fn available_providers(snapshot: &Snapshot) -> Vec<ProviderId> {
    [ProviderId::Codex, ProviderId::Grok]
        .into_iter()
        .filter(|provider| {
            snapshot
                .projects
                .iter()
                .any(|project| project.valid && project.enabled_providers.contains(provider))
        })
        .collect()
}

fn provider_label(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Codex => "Codex",
        ProviderId::Grok => "Grok",
    }
}

fn index_timeline(timeline: &[TimelineItem]) -> (TimelineIndex, MarkdownRenderCache) {
    let mut indexed = HashMap::<ConversationId, Vec<TimelineItem>>::new();
    let mut markdown_cache = HashMap::new();
    for item in timeline {
        indexed
            .entry(item.conversation_id)
            .or_default()
            .push(item.clone());
        let markdown = match &item.kind {
            TimelineItemKind::UserMessage { text }
            | TimelineItemKind::AgentMessage { text, .. } => Some(text.as_str()),
            _ => None,
        };
        if let Some(markdown) = markdown {
            markdown_cache.insert(item.id, (item.revision, markdown_html(markdown)));
        }
    }
    for items in indexed.values_mut() {
        items.sort_by_key(|item| (item.created_at_ms, item.id));
    }
    (indexed, markdown_cache)
}

fn visible_tool_summary(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() || matches!(value, "[]" | "{}" | "null" | "\"\"") {
        return None;
    }
    if (value.starts_with('{') || value.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(value).is_ok()
    {
        Some("Provider 返回了结构化详情（已隐藏）".to_owned())
    } else {
        Some(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(command: ClientCommand, state: PendingSendState, code: Option<&str>) -> PendingSend {
        let command_id = command.command_id().expect("test command id");
        PendingSend {
            command_id,
            client_message_id: format!("test:{command_id}"),
            command,
            state,
            error: None,
            rejection_code: code.map(str::to_owned),
        }
    }

    #[test]
    fn only_safe_pending_commands_offer_same_id_retry() {
        let conversation_id = ConversationId::new();
        let steer = ClientCommand::Steer {
            command_id: CommandId::new(),
            conversation_id,
            text: "continue".to_owned(),
        };
        assert!(pending_can_retry(&pending(
            steer.clone(),
            PendingSendState::WriteFailed,
            None,
        )));
        assert!(!pending_can_retry(&pending(
            steer,
            PendingSendState::Rejected,
            Some("command_failed"),
        )));
        let send = ClientCommand::SendMessage {
            command_id: CommandId::new(),
            attempt: 0,
            conversation_id,
            client_message_id: Some("stable-message".to_owned()),
            text: "hello".to_owned(),
            attachments: Vec::new(),
        };
        assert!(pending_can_retry(&pending(
            send,
            PendingSendState::Rejected,
            Some("command_failed"),
        )));
    }
}
