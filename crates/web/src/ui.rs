use std::collections::{HashMap, HashSet};

use agent_remote_protocol::{
    ApprovalId, AttachmentId, ClientAttachment, ClientCommand, CommandId, Conversation,
    ConversationId, ConversationState, DeviceId, HostId, PermissionRisk, ProjectId,
    ProviderCapability, ProviderId, ProviderState, SendTraceStage, ServerMessage, SessionOption,
    Snapshot, TimelineItem, TimelineItemId, TimelineItemKind, TimelinePageCursor, decode, encode,
};
use js_sys::{Array, ArrayBuffer, Math, Uint8Array};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{
    BinaryType, Blob, ClipboardEvent, CloseEvent, DataTransfer, DragEvent, Event, File, FileReader,
    HtmlElement, HtmlInputElement, HtmlSelectElement, HtmlTextAreaElement, MessageEvent, Url,
    UrlSearchParams, WebSocket, window,
};
use yew::{Component, Context, Html, InputEvent, MouseEvent, NodeRef, TargetCast, classes, html};

use crate::{
    ConversationSortMode, DraftScope, conversation_belongs_to_project, draft_scope,
    increment_send_attempt, is_collapsible_activity, markdown_to_safe_html,
    retryable_send_rejection, sort_conversations, timeline_item_matches_query,
};

const CREDENTIALS_KEY: &str = "agent_remote_credentials_v1";
const LAST_HOST_KEY: &str = "agent_remote_last_host_v2";
const CACHE_PREFIX: &str = "agent_remote_cache_v2_";
const WS_SUBPROTOCOL: &str = "agent-remote.cbor.v5";
const MAX_RECONNECT_ATTEMPTS: u8 = 6;
const CACHE_VERSION: u16 = 4;
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
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredDraft {
    scope: DraftScope,
    value: String,
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
    #[serde(default)]
    composer: String,
    #[serde(default)]
    drafts: Vec<StoredDraft>,
    #[serde(default)]
    conversation_sort: ConversationSortMode,
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

struct TimelineAnchor {
    conversation_id: ConversationId,
    item_id: String,
    offset: f64,
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
    scoped_drafts: HashMap<DraftScope, String>,
    pending_attachments: Vec<BrowserAttachment>,
    attachments: HashMap<AttachmentId, String>,
    fullscreen_image: Option<AttachmentId>,
    conversation_search: String,
    project_picker_open: bool,
    project_search: String,
    pinned_projects: Vec<ProjectId>,
    recent_projects: Vec<ProjectId>,
    expanded_projects: HashSet<ProjectTreeScope>,
    conversation_sort: ConversationSortMode,
    sidebar_collapsed: bool,
    sidebar_open: bool,
    history_before: HashMap<ConversationId, TimelinePageCursor>,
    history_exhausted: HashSet<ConversationId>,
    history_requested: HashSet<ConversationId>,
    history_loading: HashSet<ConversationId>,
    editing_title: bool,
    title_draft: String,
    session_settings_open: bool,
    pending_approvals: HashSet<ApprovalId>,
    approval_commands: HashMap<CommandId, ApprovalId>,
    timeline_ref: NodeRef,
    follow_timeline_tail: bool,
    timeline_anchor: Option<TimelineAnchor>,
    unread_timeline_items: HashSet<TimelineItemId>,
    timeline_search_open: bool,
    timeline_search: String,
    timeline_search_ref: NodeRef,
    focus_timeline_search: bool,
    selected_search_result: Option<TimelineItemId>,
    timeline_jump_target: Option<TimelineItemId>,
    copy_feedback: Option<String>,
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
    ForgetCredential(usize),
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
    SelectConversationSort(ConversationSortMode),
    ToggleSidebar,
    OpenSidebar,
    CloseSidebar,
    ToggleSessionSettings,
    CloseSessionSettings,
    TimelineScrolled,
    JumpToLatest,
    OpenTimelineSearch,
    CloseTimelineSearch,
    TimelineSearchChanged(String),
    NavigateTimelineSearch(bool),
    JumpToItem(TimelineItemId),
    CopyFinished(bool),
    DismissFeedback,
    CloseOverlays,
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
        let cache_needs_tree_default = cache.as_ref().is_none_or(|cache| cache.version < 3);
        let (timeline_by_conversation, markdown_render_cache) = cache
            .as_ref()
            .map(|cache| index_timeline(&cache.snapshot.timeline))
            .unwrap_or_default();
        let selected_conversation = cache.as_ref().and_then(|cache| cache.selected_conversation);
        let selected_project = cache.as_ref().and_then(|cache| cache.selected_project);
        let selected_provider = cache
            .as_ref()
            .map_or(ProviderId::Codex, |cache| cache.selected_provider);
        let draft_conversation = cache.as_ref().and_then(|cache| cache.draft_conversation);
        let current_draft_scope = connection.as_ref().and_then(|connection| {
            draft_scope(
                connection.host_id,
                selected_provider,
                selected_project,
                selected_conversation.filter(|_| draft_conversation.is_none()),
            )
        });
        let mut scoped_drafts = cache.as_ref().map_or_else(HashMap::new, |cache| {
            cache
                .drafts
                .iter()
                .map(|draft| (draft.scope, draft.value.clone()))
                .collect()
        });
        if let (Some(scope), Some(cache)) = (current_draft_scope, cache.as_ref())
            && scoped_drafts.is_empty()
            && !cache.composer.is_empty()
        {
            scoped_drafts.insert(scope, cache.composer.clone());
        }
        let composer = current_draft_scope
            .and_then(|scope| scoped_drafts.get(&scope).cloned())
            .or_else(|| cache.as_ref().map(|cache| cache.composer.clone()))
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
            selected_conversation,
            selected_project,
            selected_provider,
            selected_model: cache
                .as_ref()
                .and_then(|cache| cache.selected_model.clone()),
            selected_effort: cache
                .as_ref()
                .and_then(|cache| cache.selected_effort.clone()),
            selected_permission: cache
                .as_ref()
                .and_then(|cache| cache.selected_permission.clone()),
            draft_conversation,
            pending_send,
            composer,
            scoped_drafts,
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
            conversation_sort: cache
                .as_ref()
                .map_or(ConversationSortMode::Recent, |cache| {
                    cache.conversation_sort
                }),
            sidebar_collapsed: cache.as_ref().is_some_and(|cache| cache.sidebar_collapsed),
            sidebar_open: false,
            history_before: HashMap::new(),
            history_exhausted: HashSet::new(),
            history_requested: HashSet::new(),
            history_loading: HashSet::new(),
            editing_title: false,
            title_draft: String::new(),
            session_settings_open: false,
            pending_approvals: HashSet::new(),
            approval_commands: HashMap::new(),
            timeline_ref: NodeRef::default(),
            follow_timeline_tail: true,
            timeline_anchor: None,
            unread_timeline_items: HashSet::new(),
            timeline_search_open: false,
            timeline_search: String::new(),
            timeline_search_ref: NodeRef::default(),
            focus_timeline_search: false,
            selected_search_result: None,
            timeline_jump_target: None,
            copy_feedback: None,
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
            let Some(browser) = window().filter(|browser| browser.is_secure_context()) else {
                self.copy_feedback =
                    Some("当前连接不支持剪贴板，请选中文字复制，或使用 HTTPS 连接".to_owned());
                return true;
            };
            let promise = browser.navigator().clipboard().write_text(text);
            self.copy_feedback = Some("正在复制…".to_owned());
            context.link().send_future(async move {
                Msg::CopyFinished(js_sys::futures::JsFuture::from(promise).await.is_ok())
            });
            return true;
        }
        let previous_scope = self.current_draft_scope();
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
                    let Ok(url) = Url::new(self.pair_link.trim()) else {
                        self.status = "请输入完整的配对链接".to_owned();
                        return true;
                    };
                    if url.protocol() != "http:" && url.protocol() != "https:" {
                        self.status = "配对链接需要以 http:// 或 https:// 开头".to_owned();
                        return true;
                    }
                    let same_page = location.href().ok().is_some_and(|current| {
                        current.split('#').next() == url.href().split('#').next()
                    });
                    self.cancel_cache_persist_timer();
                    self.persist_cache();
                    if location.set_href(&url.href()).is_err()
                        || (same_page && location.reload().is_err())
                    {
                        self.status = "无法打开配对链接，请检查浏览器设置".to_owned();
                    }
                }
            }
            Msg::ConnectStored(index) => {
                if let Some(credential) = self.credentials.get(index).cloned() {
                    self.cancel_cache_persist_timer();
                    self.persist_cache();
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
            Msg::ForgetCredential(index) => self.forget_credential(index),
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
                self.remember_current_draft();
                self.selected_provider = provider;
                self.selected_project = None;
                self.selected_conversation = None;
                self.draft_conversation = None;
                self.pending_attachments.clear();
                self.session_settings_open = false;
                self.reset_dynamic_selection();
                self.ensure_project_for_provider();
                self.expand_selected_project();
                self.restore_current_draft();
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
                        .and_then(|model| {
                            model.default_effort.clone().or_else(|| {
                                model.effort_options.first().map(|effort| effort.id.clone())
                            })
                        });
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
                    self.remember_current_draft();
                    self.selected_conversation = None;
                    self.draft_conversation = Some(ConversationId::new());
                    self.pending_attachments.clear();
                    self.sidebar_open = false;
                    self.editing_title = false;
                    self.session_settings_open = false;
                    self.follow_timeline_tail = true;
                    self.expand_selected_project();
                    self.restore_current_draft();
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
            Msg::SelectConversationSort(mode) => self.conversation_sort = mode,
            Msg::ToggleSidebar => self.sidebar_collapsed = !self.sidebar_collapsed,
            Msg::OpenSidebar => self.sidebar_open = true,
            Msg::CloseSidebar => self.sidebar_open = false,
            Msg::ToggleSessionSettings => self.session_settings_open = !self.session_settings_open,
            Msg::CloseSessionSettings => self.session_settings_open = false,
            Msg::TimelineScrolled => {
                let was_following = self.follow_timeline_tail;
                self.update_timeline_follow_state();
                let near_top = self
                    .timeline_ref
                    .cast::<HtmlElement>()
                    .is_some_and(|element| element.scroll_top() <= 48);
                if near_top && !self.follow_timeline_tail && !self.timeline_search_open {
                    self.load_older();
                } else if was_following == self.follow_timeline_tail {
                    return false;
                }
            }
            Msg::JumpToLatest => {
                self.follow_timeline_tail = true;
                self.timeline_anchor = None;
                self.timeline_jump_target = None;
                self.unread_timeline_items.clear();
            }
            Msg::OpenTimelineSearch => {
                self.timeline_search_open = self.selected_conversation.is_some();
                self.focus_timeline_search = self.timeline_search_open;
            }
            Msg::CloseTimelineSearch => self.close_timeline_search(),
            Msg::TimelineSearchChanged(query) => {
                self.timeline_search = query;
                self.selected_search_result = self.timeline_search_results().first().copied();
                self.queue_timeline_jump(self.selected_search_result);
            }
            Msg::NavigateTimelineSearch(previous) => {
                let results = self.timeline_search_results();
                if !results.is_empty() {
                    let current = self
                        .selected_search_result
                        .and_then(|selected| results.iter().position(|id| *id == selected));
                    let next = match current {
                        Some(index) if previous => (index + results.len() - 1) % results.len(),
                        Some(index) => (index + 1) % results.len(),
                        None if previous => results.len() - 1,
                        None => 0,
                    };
                    self.selected_search_result = Some(results[next]);
                    self.queue_timeline_jump(self.selected_search_result);
                }
            }
            Msg::JumpToItem(item_id) => {
                self.queue_timeline_jump(Some(item_id));
            }
            Msg::DismissFeedback => self.copy_feedback = None,
            Msg::CloseOverlays => {
                self.session_settings_open = false;
                self.project_picker_open = false;
                self.sidebar_open = false;
                self.fullscreen_image = None;
                self.editing_title = false;
                self.close_timeline_search();
                if let Some(document) = window().and_then(|browser| browser.document())
                    && let Ok(Some(menu)) = document.query_selector(".host-switcher[open]")
                {
                    let _ = menu.remove_attribute("open");
                }
            }
            Msg::CopyFinished(success) => {
                self.copy_feedback = Some(
                    if success {
                        "已复制"
                    } else {
                        "复制失败，请允许剪贴板访问或选中文字复制"
                    }
                    .to_owned(),
                );
            }
            Msg::ComposerChanged(value) => {
                self.copy_feedback = None;
                if self.pending_send.is_none() {
                    self.composer = value;
                    self.remember_current_draft();
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
                let provider_ready = self.selected_conversation.is_some()
                    || self
                        .selected_capability()
                        .is_some_and(|capability| capability.health.state == ProviderState::Ready);
                if !self.authenticated {
                    self.status = "连接尚未完成认证，消息已保留".to_owned();
                } else if !provider_ready {
                    self.status = "当前 Provider 尚未就绪，消息已保留".to_owned();
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
                if !self.pending_approvals.contains(&approval_id) {
                    let command_id = CommandId::new();
                    if self.send_authenticated(ClientCommand::ResolveApproval {
                        command_id,
                        approval_id,
                        option_id,
                    }) {
                        self.pending_approvals.insert(approval_id);
                        self.approval_commands.insert(command_id, approval_id);
                    }
                }
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
            Msg::LoadOlder => self.load_older(),
            Msg::OpenImage(id) => {
                self.fullscreen_image = Some(id);
                if !self.attachments.contains_key(&id) {
                    self.send_authenticated(ClientCommand::GetAttachment { attachment_id: id });
                }
            }
            Msg::CloseImage => self.fullscreen_image = None,
            Msg::CopyText(_) | Msg::PersistCache(_) | Msg::FlushCache => unreachable!(),
        }
        if previous_scope != self.current_draft_scope() {
            self.close_timeline_search();
            self.unread_timeline_items.clear();
            self.timeline_jump_target = None;
            self.timeline_anchor = None;
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
            <main class={classes!("app-shell", self.sidebar_collapsed.then_some("sidebar-collapsed"))} onkeydown={link.batch_callback({ let has_conversation = self.selected_conversation.is_some(); move |event: web_sys::KeyboardEvent| {
                if has_conversation && (event.ctrl_key() || event.meta_key()) && event.key().eq_ignore_ascii_case("f") {
                    event.prevent_default();
                    Some(Msg::OpenTimelineSearch)
                } else {
                    (event.key() == "Escape").then_some(Msg::CloseOverlays)
                }
            }})}>
                {if self.sidebar_open { html! {<button class="drawer-scrim" aria-label="关闭侧边栏" onclick={link.callback(|_| Msg::CloseSidebar)}></button>} } else {html! {}}}
                <aside class={classes!("sidebar", self.sidebar_open.then_some("open"))}>
                    <details key={snapshot.host_id.to_string()} class="host-switcher">
                        <summary class="brand" title="连接与主机"><span class="brand-mark">{icon("connection")}</span><span class="collapsible-copy"><strong>{"Agent Remote"}</strong><small>{&snapshot.host_name}</small></span><span class="collapsible-copy">{icon("chevron-down")}</span></summary>
                        <div class="host-menu">
                            <h3>{"主机"}</h3>
                            {for self.credentials.iter().enumerate().map(|(index, credential)| html! {
                                <button class={classes!("host-row", (credential.host_id == snapshot.host_id).then_some("active"))} onclick={link.callback(move |_| Msg::ConnectStored(index))}>
                                    <span>{credential.display_name.as_deref().unwrap_or("已保存的主机")}</span>
                                    {if credential.host_id == snapshot.host_id {icon("check")} else {icon("chevron-right")}}
                                </button>
                            })}
                            <label class="field"><span>{"连接其他主机"}</span><input aria-label="新主机配对链接" placeholder="粘贴配对链接" value={self.pair_link.clone()} oninput={link.callback(|event: InputEvent| Msg::PairLinkChanged(event.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                            <button class="primary wide" disabled={self.pair_link.trim().is_empty()} onclick={link.callback(|_| Msg::OpenPairLink)}>{"连接"}</button>
                        </div>
                    </details>
                    <div class="connection-strip">
                        <span class={if self.connected {"status-dot online"} else {"status-dot"}}></span>
                        <span class="collapsible-copy">{&self.status}</span>
                        {if !self.connected && self.retry_enabled {html! {<button title="停止重连" onclick={link.callback(|_| Msg::StopRetrying)}>{"×"}</button>}} else if !self.connected {html! {<button title="立即重试" onclick={link.callback(|_| Msg::RetryNow)}>{"↻"}</button>}} else {html! {}}}
                    </div>
                    {self.view_agent_selector(link, &providers)}
                    <div class="project-control">
                        <button class="project-trigger" onclick={link.callback(|_| Msg::ToggleProjectPicker)} title="选择项目">
                            <span class="project-icon">{icon("folder")}</span>
                            <span class="collapsible-copy"><strong>{project.map_or("选择项目", |project| project.display_name.as_str())}</strong><small>{project.map_or("", |project| project.short_path.as_str())}</small></span>
                            <span class="collapsible-copy">{icon("chevron-down")}</span>
                        </button>
                        {if self.project_picker_open { self.view_project_picker(link, snapshot) } else {html! {}}}
                    </div>
                    <button class="new-button" title="新建对话" onclick={link.callback(|_| Msg::NewConversation)} disabled={self.selected_project.is_none()}>{icon("plus")}<span class="collapsible-copy">{"新建对话"}</span></button>
                    <label class="conversation-search collapsible-copy">{icon("search")}<input aria-label="搜索项目或对话" placeholder="搜索项目或对话" value={self.conversation_search.clone()} oninput={link.callback(|event: InputEvent| Msg::ConversationSearchChanged(event.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
                    <div class="sidebar-sort collapsible-copy" aria-label="会话排序">
                        <button class={classes!((self.conversation_sort == ConversationSortMode::Recent).then_some("active"))} aria-pressed={(self.conversation_sort == ConversationSortMode::Recent).to_string()} onclick={link.callback(|_| Msg::SelectConversationSort(ConversationSortMode::Recent))}>{"最新"}</button>
                        <button class={classes!((self.conversation_sort == ConversationSortMode::Active).then_some("active"))} aria-pressed={(self.conversation_sort == ConversationSortMode::Active).to_string()} onclick={link.callback(|_| Msg::SelectConversationSort(ConversationSortMode::Active))}>{"进行中"}</button>
                    </div>
                    <nav class="conversation-list" aria-label="项目与会话">
                        {self.view_project_tree(link, snapshot)}
                    </nav>
                    <div class="sidebar-footer">
                        <button onclick={link.callback(|_| Msg::Disconnect)} title="断开连接">{icon("disconnect")}<span class="collapsible-copy">{"断开连接"}</span></button>
                        <button onclick={link.callback(|_| Msg::ToggleSidebar)} title={if self.sidebar_collapsed {"展开侧边栏"} else {"收起侧边栏"}}>{icon("panel")}</button>
                    </div>
                </aside>
                <section class="chat-pane">
                    {if let Some(conversation) = selected {
                        self.view_chat(link, conversation, snapshot)
                    } else if self.draft_conversation.is_some() && self.selected_project.is_some() {
                        self.view_draft_chat(link, snapshot)
                    } else if let Some(project) = project {
                        self.view_project_home(link, project, snapshot)
                    } else {
                        html! { <div class="empty-state"><button class="mobile-menu" aria-label="打开侧边栏" onclick={link.callback(|_| Msg::OpenSidebar)}>{icon("panel")}</button><h2>{"选择或新建对话"}</h2><p>{"这里只显示当前 Agent 与项目的远程对话。"}</p></div> }
                    }}
                </section>
                {self.view_fullscreen_image(link)}
                {self.view_session_settings(link)}
                {self.copy_feedback.as_ref().map(|feedback| html! {<div class="copy-feedback" role="status" onclick={link.callback(|_| Msg::DismissFeedback)}>{feedback}<button aria-label="关闭提示">{icon("close")}</button></div>}).unwrap_or_default()}
            </main>
        }
    }

    fn rendered(&mut self, _context: &Context<Self>, _first_render: bool) {
        self.restore_timeline_anchor();
        self.apply_timeline_jump();
        self.scroll_timeline_to_tail();
        if self.focus_timeline_search {
            self.focus_timeline_search = false;
            if let Some(input) = self.timeline_search_ref.cast::<HtmlInputElement>() {
                let _ = input.focus();
                input.select();
            }
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
                    let provider = ProviderId::ALL
                        .into_iter()
                        .find(|provider| provider.wire_name() == value)
                        .unwrap_or(ProviderId::Codex);
                    Msg::SelectProvider(provider)
                })}>
                    {for providers.iter().map(|provider| {
                        let value = provider.wire_name();
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
        self.history_loading.clear();
        self.history_requested.clear();
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
            self.retry_enabled = false;
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
                        display_name: None,
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
                if !self.send_command(ClientCommand::GetSnapshot {
                    metadata_only: false,
                }) {
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
                if !self.send_command(ClientCommand::GetSnapshot {
                    metadata_only: false,
                }) {
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
                if let Some(credential) = self
                    .credentials
                    .iter_mut()
                    .find(|credential| credential.host_id == snapshot.host_id)
                {
                    credential.display_name = Some(snapshot.host_name.clone());
                    save_credentials(&self.credentials);
                }
                self.remember_current_draft();
                self.snapshot = Some(snapshot);
                self.reindex_timeline();
                self.sync_in_flight.clear();
                self.refresh_in_flight.clear();
                self.history_before.clear();
                self.history_exhausted.clear();
                self.history_requested.clear();
                self.history_loading.clear();
                self.pending_approvals.clear();
                self.approval_commands.clear();
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
                self.restore_current_draft();
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
                if provider == self.selected_provider {
                    self.remember_current_draft();
                }
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
                    let previous_provider = self.selected_provider;
                    let previous_project = self.selected_project;
                    let previous_conversation = self.selected_conversation;
                    self.ensure_provider_available();
                    self.ensure_project_for_provider();
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
                    if previous_provider != self.selected_provider
                        || previous_project != self.selected_project
                        || previous_conversation != self.selected_conversation
                    {
                        self.draft_conversation = None;
                        self.pending_attachments.clear();
                        self.session_settings_open = false;
                        self.restore_current_draft();
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
                error,
            } => {
                self.history_loading.remove(&conversation_id);
                if self.selected_conversation == Some(conversation_id) && !self.follow_timeline_tail
                {
                    self.timeline_anchor = self.capture_timeline_anchor(conversation_id);
                }
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
                if let Some(error) = error {
                    self.history_requested.remove(&conversation_id);
                    self.history_exhausted.remove(&conversation_id);
                    self.status = format!("历史加载失败：{error}");
                } else {
                    match next_before {
                        Some(before) => {
                            self.history_before.insert(conversation_id, before);
                        }
                        None => {
                            self.history_exhausted.insert(conversation_id);
                        }
                    }
                }
                self.request_images_for_selected();
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
                        if let Some(host_id) = self
                            .connection
                            .as_ref()
                            .map(|connection| connection.host_id)
                            && let Some(scope) =
                                draft_scope(host_id, provider, Some(project_id), None)
                        {
                            self.scoped_drafts.remove(&scope);
                        }
                        self.selected_provider = provider;
                        self.selected_project = Some(project_id);
                        self.selected_conversation = Some(conversation.id);
                        self.draft_conversation = None;
                        self.follow_timeline_tail = true;
                        self.remember_current_draft();
                        self.expand_selected_project();
                    }
                }
            }
            ServerMessage::TimelineItemUpserted { item } => {
                if let TimelineItemKind::Approval {
                    approval_id,
                    resolved_option: Some(_),
                    ..
                } = &item.kind
                {
                    self.pending_approvals.remove(approval_id);
                    self.approval_commands
                        .retain(|_, pending| pending != approval_id);
                }
                let image_id = match &item.kind {
                    TimelineItemKind::Image { attachment_id, .. } => Some(*attachment_id),
                    _ => None,
                };
                let reading_history = self.selected_conversation == Some(item.conversation_id)
                    && !self.follow_timeline_tail;
                if reading_history {
                    self.timeline_anchor = self.capture_timeline_anchor(item.conversation_id);
                }
                let changed = self.snapshot.as_mut().is_some_and(|snapshot| {
                    upsert_timeline_item(&mut snapshot.timeline, item.clone())
                });
                if changed {
                    if reading_history {
                        self.unread_timeline_items.insert(item.id);
                    }
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
                let removed_selected = self.selected_conversation == Some(conversation_id);
                if removed_selected {
                    self.remember_current_draft();
                }
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
                if removed_selected {
                    self.selected_conversation = None;
                    self.pending_attachments.clear();
                    self.session_settings_open = false;
                    self.follow_timeline_tail = true;
                    self.restore_current_draft();
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
                if command_id.is_none() {
                    self.history_requested
                        .retain(|id| !self.history_loading.contains(id));
                    self.history_loading.clear();
                }
                if let Some(command_id) = command_id {
                    self.sync_in_flight.remove(&command_id);
                    if let Some(approval_id) = self.approval_commands.remove(&command_id) {
                        self.pending_approvals.remove(&approval_id);
                    }
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
                    if let Some(host_id) = self
                        .connection
                        .as_ref()
                        .map(|connection| connection.host_id)
                        && let Some(index) = self
                            .credentials
                            .iter()
                            .position(|credential| credential.host_id == host_id)
                    {
                        self.forget_credential(index);
                    }
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
                    if let Some(pending) = self.pending_send.take() {
                        let clear_visible_draft = self.command_owns_current_draft(&pending.command);
                        if command_is_send(&pending.command) {
                            trace_send_stage(
                                "command_accepted",
                                command_id,
                                &pending.client_message_id,
                                Some(self.connection_generation),
                                self.send_elapsed_ms(command_id),
                            );
                        }
                        self.clear_draft_for_command(&pending.command);
                        if clear_visible_draft {
                            self.composer.clear();
                            self.pending_attachments.clear();
                        }
                    }
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
        self.selected_effort =
            self.selected_effort
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
                    valid_efforts.and_then(|model| {
                        model.default_effort.clone().or_else(|| {
                            model.effort_options.first().map(|effort| effort.id.clone())
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

    fn current_draft_scope(&self) -> Option<DraftScope> {
        draft_scope(
            self.connection.as_ref()?.host_id,
            self.selected_provider,
            self.selected_project,
            self.selected_conversation
                .filter(|_| self.draft_conversation.is_none()),
        )
    }

    fn remember_current_draft(&mut self) {
        let Some(scope) = self.current_draft_scope() else {
            return;
        };
        if self.composer.is_empty() {
            self.scoped_drafts.remove(&scope);
        } else {
            self.scoped_drafts.insert(scope, self.composer.clone());
        }
    }

    fn restore_current_draft(&mut self) {
        self.composer = self
            .current_draft_scope()
            .and_then(|scope| self.scoped_drafts.get(&scope).cloned())
            .unwrap_or_default();
    }

    fn clear_draft_for_command(&mut self, command: &ClientCommand) {
        let Some(host_id) = self
            .connection
            .as_ref()
            .map(|connection| connection.host_id)
        else {
            return;
        };
        match command {
            ClientCommand::StartConversation {
                conversation_id,
                project_id,
                provider,
                ..
            } => {
                if let Some(scope) = draft_scope(host_id, *provider, Some(*project_id), None) {
                    self.scoped_drafts.remove(&scope);
                }
                if let Some(scope) = draft_scope(
                    host_id,
                    *provider,
                    Some(*project_id),
                    Some(*conversation_id),
                ) {
                    self.scoped_drafts.remove(&scope);
                }
            }
            ClientCommand::SendMessage {
                conversation_id, ..
            }
            | ClientCommand::Steer {
                conversation_id, ..
            } => {
                if let Some(conversation) = self.snapshot.as_ref().and_then(|snapshot| {
                    snapshot
                        .conversations
                        .iter()
                        .find(|conversation| conversation.id == *conversation_id)
                }) && let Some(scope) = draft_scope(
                    host_id,
                    conversation.provider,
                    Some(conversation.project_id),
                    Some(*conversation_id),
                ) {
                    self.scoped_drafts.remove(&scope);
                }
            }
            _ => {}
        }
    }

    fn command_owns_current_draft(&self, command: &ClientCommand) -> bool {
        let Some(host_id) = self
            .connection
            .as_ref()
            .map(|connection| connection.host_id)
        else {
            return false;
        };
        let current = self.current_draft_scope();
        match command {
            ClientCommand::StartConversation {
                conversation_id,
                project_id,
                provider,
                ..
            } => {
                current == draft_scope(host_id, *provider, Some(*project_id), None)
                    || current
                        == draft_scope(
                            host_id,
                            *provider,
                            Some(*project_id),
                            Some(*conversation_id),
                        )
            }
            ClientCommand::SendMessage {
                conversation_id, ..
            }
            | ClientCommand::Steer {
                conversation_id, ..
            } => self
                .snapshot
                .as_ref()
                .and_then(|snapshot| {
                    snapshot
                        .conversations
                        .iter()
                        .find(|conversation| conversation.id == *conversation_id)
                })
                .is_some_and(|conversation| {
                    current
                        == draft_scope(
                            host_id,
                            conversation.provider,
                            Some(conversation.project_id),
                            Some(*conversation_id),
                        )
                }),
            _ => false,
        }
    }

    fn update_timeline_follow_state(&mut self) {
        let Some(element) = self.timeline_ref.cast::<HtmlElement>() else {
            return;
        };
        let remaining = element.scroll_height() - element.scroll_top() - element.client_height();
        self.follow_timeline_tail = remaining <= 72;
        if self.follow_timeline_tail {
            self.unread_timeline_items.clear();
        }
    }

    fn close_timeline_search(&mut self) {
        self.timeline_search_open = false;
        self.timeline_search.clear();
        self.selected_search_result = None;
        self.focus_timeline_search = false;
    }

    fn timeline_search_results(&self) -> Vec<TimelineItemId> {
        self.selected_conversation
            .and_then(|id| self.timeline_by_conversation.get(&id))
            .into_iter()
            .flatten()
            .filter(|item| timeline_item_matches_query(&item.kind, &self.timeline_search))
            .map(|item| item.id)
            .collect()
    }

    fn queue_timeline_jump(&mut self, item_id: Option<TimelineItemId>) {
        if let Some(item_id) = item_id {
            self.follow_timeline_tail = false;
            self.timeline_anchor = None;
            self.timeline_jump_target = Some(item_id);
        }
    }

    fn apply_timeline_jump(&mut self) {
        let Some(item_id) = self.timeline_jump_target.take() else {
            return;
        };
        if let Some(timeline) = self.timeline_ref.cast::<HtmlElement>()
            && let Ok(Some(item)) =
                timeline.query_selector(&format!("[data-timeline-id='{item_id}']"))
        {
            if let Ok(Some(group)) = item.closest(".activity-group") {
                let _ = group.set_attribute("open", "");
            }
            let top = item.get_bounding_client_rect().top()
                - timeline.get_bounding_client_rect().top()
                + f64::from(timeline.scroll_top());
            timeline.set_scroll_top((top - 20.0).max(0.0).round() as i32);
        }
    }

    fn load_older(&mut self) {
        let Some(conversation_id) = self.selected_conversation else {
            return;
        };
        if !self.authenticated
            || !self.connected
            || self.history_exhausted.contains(&conversation_id)
            || self.history_loading.contains(&conversation_id)
        {
            return;
        }
        let before = self
            .history_before
            .get(&conversation_id)
            .copied()
            .or_else(|| {
                self.timeline_by_conversation
                    .get(&conversation_id)?
                    .first()
                    .map(|item| TimelinePageCursor {
                        created_at_ms: item.created_at_ms,
                        item_id: item.id,
                    })
            });
        if self.send_authenticated(ClientCommand::GetConversationPage {
            conversation_id,
            before,
            limit: 100,
        }) {
            self.follow_timeline_tail = false;
            self.history_loading.insert(conversation_id);
        }
    }

    fn capture_timeline_anchor(&self, conversation_id: ConversationId) -> Option<TimelineAnchor> {
        let element = self.timeline_ref.cast::<HtmlElement>()?;
        let top = element.get_bounding_client_rect().top();
        let mut child = element.first_element_child();
        while let Some(node) = child {
            let rect = node.get_bounding_client_rect();
            if rect.bottom() > top
                && let Some(item_id) = node.get_attribute("data-timeline-id")
            {
                return Some(TimelineAnchor {
                    conversation_id,
                    item_id,
                    offset: rect.top() - top,
                });
            }
            child = node.next_element_sibling();
        }
        None
    }

    fn restore_timeline_anchor(&mut self) {
        let Some(anchor) = self.timeline_anchor.take() else {
            return;
        };
        if self.selected_conversation != Some(anchor.conversation_id) || self.follow_timeline_tail {
            return;
        }
        if let Some(element) = self.timeline_ref.cast::<HtmlElement>()
            && let Ok(Some(node)) =
                element.query_selector(&format!("[data-timeline-id='{}']", anchor.item_id))
        {
            let node = node
                .closest(".activity-group:not([open])")
                .ok()
                .flatten()
                .unwrap_or(node);
            let delta = node.get_bounding_client_rect().top()
                - element.get_bounding_client_rect().top()
                - anchor.offset;
            element.set_scroll_top(element.scroll_top() + delta.round() as i32);
        }
    }

    fn scroll_timeline_to_tail(&self) {
        if !self.follow_timeline_tail {
            return;
        }
        if let Some(element) = self.timeline_ref.cast::<HtmlElement>() {
            element.set_scroll_top(element.scroll_height());
        }
    }

    fn forget_credential(&mut self, index: usize) {
        let Some(credential) = self.credentials.get(index).cloned() else {
            return;
        };
        self.credentials.remove(index);
        save_credentials(&self.credentials);
        remove_cache(credential.host_id);
        if self
            .connection
            .as_ref()
            .is_some_and(|connection| connection.host_id == credential.host_id)
        {
            self.manually_disconnected = true;
            self.retry_enabled = false;
            self.cancel_reconnect_timer();
            self.close_socket("credentials removed");
            self.connection = None;
            self.snapshot = None;
            self.timeline_by_conversation.clear();
            self.markdown_render_cache.clear();
            self.scoped_drafts.clear();
            self.pending_send = None;
            self.pending_attachments.clear();
            self.pending_approvals.clear();
            self.approval_commands.clear();
        }
        self.status = "本地设备凭证已删除".to_owned();
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
        self.remember_current_draft();
        let changed = self.selected_project != Some(project_id);
        self.selected_project = Some(project_id);
        self.selected_conversation = None;
        self.draft_conversation = None;
        self.pending_attachments.clear();
        self.project_picker_open = false;
        self.sidebar_open = false;
        self.session_settings_open = false;
        self.recent_projects
            .retain(|project| *project != project_id);
        self.recent_projects.insert(0, project_id);
        self.recent_projects.truncate(8);
        self.expand_selected_project();
        if changed {
            self.reset_dynamic_selection();
            self.sync_selected_project();
        }
        self.restore_current_draft();
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
        self.remember_current_draft();
        let scope_changed =
            self.selected_provider != provider || self.selected_project != Some(project_id);
        self.selected_provider = provider;
        self.selected_project = Some(project_id);
        self.selected_conversation = Some(conversation_id);
        self.draft_conversation = None;
        self.pending_attachments.clear();
        self.sidebar_open = false;
        self.editing_title = false;
        self.session_settings_open = false;
        self.follow_timeline_tail = true;
        self.recent_projects
            .retain(|project| *project != project_id);
        self.recent_projects.insert(0, project_id);
        self.recent_projects.truncate(8);
        self.expand_selected_project();
        if scope_changed {
            self.reset_dynamic_selection();
        }
        self.restore_current_draft();
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
        let mut drafts = self.scoped_drafts.clone();
        if let Some(scope) = self.current_draft_scope() {
            if self.composer.is_empty() {
                drafts.remove(&scope);
            } else {
                drafts.insert(scope, self.composer.clone());
            }
        }
        let mut drafts = drafts
            .into_iter()
            .map(|(scope, value)| StoredDraft { scope, value })
            .collect::<Vec<_>>();
        drafts.sort_by_key(|draft| {
            (
                draft.scope.host_id.to_string(),
                draft.scope.provider.to_string(),
                draft.scope.project_id.to_string(),
                draft.scope.conversation_id.map(|id| id.to_string()),
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
            drafts,
            conversation_sort: self.conversation_sort,
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
        self.history_loading.clear();
        self.timeline_anchor = None;
        self.copy_feedback = None;
        self.conversation_search.clear();
        self.project_picker_open = false;
        self.project_search.clear();
        self.sidebar_open = false;
        self.editing_title = false;
        self.title_draft.clear();
        self.session_settings_open = false;
        self.pending_approvals.clear();
        self.approval_commands.clear();
        self.follow_timeline_tail = true;
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
            self.scoped_drafts.clear();
            self.sidebar_collapsed = false;
            self.pinned_projects.clear();
            self.recent_projects.clear();
            self.expanded_projects.clear();
            self.conversation_sort = ConversationSortMode::Recent;
            return;
        };
        let cache_needs_tree_default = cache.version < 3;
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
        self.scoped_drafts = cache
            .drafts
            .into_iter()
            .map(|draft| (draft.scope, draft.value))
            .collect();
        if self.scoped_drafts.is_empty()
            && !cache.composer.is_empty()
            && let Some(scope) = self.current_draft_scope()
        {
            self.scoped_drafts.insert(scope, cache.composer.clone());
        }
        self.composer = self
            .current_draft_scope()
            .and_then(|scope| self.scoped_drafts.get(&scope).cloned())
            .unwrap_or(cache.composer);
        self.conversation_sort = cache.conversation_sort;
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
        } else {
            self.history_loading.insert(conversation_id);
        }
    }

    fn view_connection(&self, link: &yew::html::Scope<Self>) -> Html {
        html! {
            <main class="connection-page">
                <section class="connection-card">
                    <div class="connection-logo">{icon("connection")}</div>
                    <p class="eyebrow">{"Agent Remote"}</p>
                    <h1>{"连接工作电脑"}</h1>
                    <p class="lead">{"粘贴电脑上的配对链接，在这里继续对话。"}</p>
                    <div class="connection-status"><span class="status-dot"></span><span>{&self.status}</span></div>
                    {if self.credentials.is_empty() { html! {} } else { html! {
                        <div class="saved-hosts">
                            <h2>{"已保存的主机"}</h2>
                            {for self.credentials.iter().enumerate().map(|(index, credential)| {
                                let connect = link.callback(move |_| Msg::ConnectStored(index));
                                let forget = link.callback(move |_| Msg::ForgetCredential(index));
                                html! {
                                    <article class="saved-host-card">
                                        <div><strong>{credential.display_name.as_deref().unwrap_or("已保存的主机")}</strong><small>{&credential.origin}</small><small>{if credential.relay {"公网 Relay"} else {"直接连接"}}</small></div>
                                        <div class="host-actions"><button onclick={connect}>{"连接"}</button><button class="danger" onclick={forget}>{"删除"}</button></div>
                                    </article>
                                }
                            })}
                        </div>
                    }}}
                    <label class="field">
                        <span>{"配对链接"}</span>
                        <input
                            aria-label="配对链接"
                            placeholder="https://…"
                            value={self.pair_link.clone()}
                            oninput={link.callback(|event: InputEvent| Msg::PairLinkChanged(event.target_unchecked_into::<HtmlInputElement>().value()))}
                        />
                    </label>
                    <button class="primary wide" onclick={link.callback(|_| Msg::OpenPairLink)} disabled={self.pair_link.trim().is_empty()}>{"连接"}{icon("arrow-right")}</button><p class="connection-footer">{icon("lock")}{"项目、文件与 Agent 均在你的电脑上运行。"}</p>
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
                sort_conversations(&mut conversations, self.conversation_sort);
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
                    >{icon(if expanded {"chevron-down"} else {"chevron-right"})}</button>
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
                <label>{icon("search")}<input aria-label="搜索项目" autofocus=true placeholder="搜索项目" value={self.project_search.clone()} oninput={link.callback(|event: InputEvent| Msg::ProjectSearchChanged(event.target_unchecked_into::<HtmlInputElement>().value()))}/></label>
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

    fn view_project_home(
        &self,
        link: &yew::html::Scope<Self>,
        project: &agent_remote_protocol::ProjectSummary,
        snapshot: &Snapshot,
    ) -> Html {
        let query = self.conversation_search.trim().to_lowercase();
        let mut conversations = snapshot
            .conversations
            .iter()
            .filter(|conversation| {
                conversation_belongs_to_project(conversation, project.id, self.selected_provider)
                    && (query.is_empty() || conversation.title.to_lowercase().contains(&query))
            })
            .collect::<Vec<_>>();
        sort_conversations(&mut conversations, self.conversation_sort);
        html! {
            <>
                <header class="chat-header">
                    <div class="chat-header-inner">
                        <button class="mobile-menu" aria-label="打开侧边栏" onclick={link.callback(|_| Msg::OpenSidebar)}>{icon("panel")}</button>
                        <div class="chat-title"><p class="eyebrow">{provider_label(self.selected_provider)}</p><h1>{&project.display_name}</h1><small>{&project.short_path}</small></div>
                        <button class="primary" onclick={link.callback(|_| Msg::NewConversation)}>{"新建对话"}</button>
                    </div>
                </header>
                <div class="timeline project-home">
                    <div class="project-home-heading">
                        <div><h2>{"对话"}</h2><p>{format!("{} 个对话", conversations.len())}</p></div>
                        <div class="sidebar-sort" aria-label="会话排序">
                            <button class={classes!((self.conversation_sort == ConversationSortMode::Recent).then_some("active"))} aria-pressed={(self.conversation_sort == ConversationSortMode::Recent).to_string()} onclick={link.callback(|_| Msg::SelectConversationSort(ConversationSortMode::Recent))}>{"按时间"}</button>
                            <button class={classes!((self.conversation_sort == ConversationSortMode::Active).then_some("active"))} aria-pressed={(self.conversation_sort == ConversationSortMode::Active).to_string()} onclick={link.callback(|_| Msg::SelectConversationSort(ConversationSortMode::Active))}>{"进行中优先"}</button>
                        </div>
                    </div>
                    {if conversations.is_empty() {
                        html! {<div class="empty-state"><h2>{if query.is_empty() {"这个项目还没有会话"} else {"没有匹配的聊天记录"}}</h2><p>{if query.is_empty() {"从一条消息开始。"} else {"试试其他项目名或对话标题。"}}</p></div>}
                    } else {
                        html! {<div class="project-home-list"><div class="project-home-list-header"><span>{"对话"}</span><span>{"状态"}</span><span>{"更新时间"}</span></div>{for conversations.into_iter().map(|conversation| self.view_conversation_row(link, conversation))}</div>}
                    }}
                </div>
            </>
        }
    }

    fn view_draft_chat(&self, link: &yew::html::Scope<Self>, snapshot: &Snapshot) -> Html {
        let project_name = self
            .selected_project
            .and_then(|id| snapshot.projects.iter().find(|project| project.id == id))
            .map_or("未知项目", |project| project.display_name.as_str());
        let capability = self.selected_capability();
        html! {
            <>
                <header class="chat-header">
                    <div class="chat-header-inner">
                        <button class="mobile-menu" aria-label="打开侧边栏" onclick={link.callback(|_| Msg::OpenSidebar)}>{icon("panel")}</button>
                        <div class="chat-title"><p class="eyebrow">{format!("{} · {}", self.selected_provider, project_name)}</p><h1>{"新对话"}</h1><small>{"首次发送时才会创建远程会话"}</small></div>
                    </div>
                </header>
                <div class="timeline empty-draft"><div class="draft-intro">{icon("message")}<h2>{"开始一段对话"}</h2><p>{"描述任务，或添加需要参考的文件。"}</p>{capability.and_then(|capability| capability.limitation.as_ref()).map(|limitation| html! {<p class="provider-limitation">{limitation}</p>}).unwrap_or_default()}</div></div>
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
                <span class={classes!("provider-badge", provider_class(conversation.provider))}>{icon("message")}</span>
                <span class="conversation-copy"><strong>{&conversation.title}</strong><small>{state_label(conversation.state)}</small></span>
                <span class="conversation-trailing"><span class={classes!("state-pill", state_class(conversation.state))}>{state_label(conversation.state)}</span><time class="conversation-updated">{format_relative_activity(Some(conversation.updated_at_ms))}</time>{icon("chevron-right")}</span>
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
        let items = self
            .timeline_by_conversation
            .get(&conversation.id)
            .map_or(&[][..], Vec::as_slice);
        let unresolved_approvals = items
            .iter()
            .filter(|item| {
                running
                    && matches!(
                        &item.kind,
                        TimelineItemKind::Approval {
                            resolved_option: None,
                            ..
                        }
                    )
            })
            .collect::<Vec<_>>();
        html! {
            <>
                <header class="chat-header">
                    <div class="chat-header-inner">
                        <button class="mobile-menu" aria-label="打开侧边栏" onclick={link.callback(|_| Msg::OpenSidebar)}>{icon("panel")}</button>
                        <div class="chat-title"><p class="eyebrow">{format!("{} · {}", conversation.provider, project_name)}</p>
                            {if self.editing_title {html! {<div class="title-editor"><input value={self.title_draft.clone()} maxlength="80" oninput={link.callback(|event: InputEvent| Msg::TitleChanged(event.target_unchecked_into::<HtmlInputElement>().value()))}/><button onclick={link.callback(|_| Msg::SaveTitle)}>{"保存"}</button></div>}} else {html! {<button class="title-button" onclick={link.callback(|_| Msg::EditTitle)}><h1>{&conversation.title}</h1><span>{icon("edit")}</span></button>}}}
                        </div>
                        <div class="header-controls">
                            <button class="header-action" aria-label="搜索当前对话" title="搜索当前对话 (Ctrl/Cmd+F)" aria-expanded={self.timeline_search_open.to_string()} onclick={link.callback(|_| Msg::OpenTimelineSearch)}>{icon("search")}</button>
                            <span class={classes!("state-pill", state_class(conversation.state))}>{state_label(conversation.state)}</span>
                        </div>
                    </div>
                    {self.view_timeline_search(link, conversation.id)}
                    {unresolved_approvals.first().map(|item| {
                        let item_id = item.id;
                        html! {<button class="pending-approval-banner" onclick={link.callback(move |_| Msg::JumpToItem(item_id))}>{icon("lock")}<span>{format!("{} 项审批待处理", unresolved_approvals.len())}</span><span>{"查看并处理"}</span>{icon("arrow-down")}</button>}
                    }).unwrap_or_default()}
                </header>
                <div class="timeline-area">
                <div class="timeline" ref={self.timeline_ref.clone()} onscroll={link.callback(|_| Msg::TimelineScrolled)} onclick={link.batch_callback(|event: MouseEvent| {
                    let target = event.target()?.dyn_into::<web_sys::Element>().ok()?;
                    let button = target.closest(".code-copy").ok()??;
                    let code = button.parent_element()?.query_selector("pre > code").ok()??;
                    Some(Msg::CopyText(code.text_content().unwrap_or_default()))
                })}>
                    {if self.history_exhausted.contains(&conversation.id) {html! {}} else {html! {<button class="load-older" disabled={!self.authenticated || self.history_loading.contains(&conversation.id)} onclick={link.callback(|_| Msg::LoadOlder)}>{if self.history_loading.contains(&conversation.id) {"正在加载…"} else {"查看更早的消息"}}</button>}}}
                    {self.view_timeline(link, items)}
                </div>
                {if !self.follow_timeline_tail {html! {<button class="jump-latest" onclick={link.callback(|_| Msg::JumpToLatest)}>{icon("arrow-down")}{if self.unread_timeline_items.is_empty() {"回到最新".to_owned()} else {format!("{} 条新动态 · 回到最新", self.unread_timeline_items.len())}}</button>}} else {html! {}}}
                </div>
                {self.view_composer(link, running, capability)}
            </>
        }
    }

    fn view_timeline_search(
        &self,
        link: &yew::html::Scope<Self>,
        conversation_id: ConversationId,
    ) -> Html {
        if !self.timeline_search_open {
            return html! {};
        }
        let results = self.timeline_search_results();
        let position = self
            .selected_search_result
            .and_then(|selected| results.iter().position(|id| *id == selected));
        let status = if self.timeline_search.trim().is_empty() {
            "输入关键词".to_owned()
        } else if results.is_empty() {
            "没有匹配内容".to_owned()
        } else {
            format!(
                "{} / {} 条",
                position.map_or(0, |index| index + 1),
                results.len()
            )
        };
        html! {
            <div class="timeline-search-panel" role="search" aria-label="搜索当前对话已加载内容">
                <div class="timeline-search-controls">
                    {icon("search")}
                    <input ref={self.timeline_search_ref.clone()} aria-label="搜索消息和活动" placeholder="搜索消息和活动" value={self.timeline_search.clone()} oninput={link.callback(|event: InputEvent| Msg::TimelineSearchChanged(event.target_unchecked_into::<HtmlInputElement>().value()))} onkeydown={link.batch_callback(|event: web_sys::KeyboardEvent| {
                        if event.is_composing() { return None; }
                        match event.key().as_str() {
                            "Enter" => { event.prevent_default(); Some(Msg::NavigateTimelineSearch(event.shift_key())) },
                            "Escape" => { event.stop_propagation(); Some(Msg::CloseTimelineSearch) },
                            _ => None,
                        }
                    })}/>
                    <span class="search-result-count" role="status">{status}</span>
                    <button aria-label="上一个匹配结果" title="上一个 (Shift+Enter)" disabled={results.is_empty()} onclick={link.callback(|_| Msg::NavigateTimelineSearch(true))}>{icon("arrow-up")}</button>
                    <button aria-label="下一个匹配结果" title="下一个 (Enter)" disabled={results.is_empty()} onclick={link.callback(|_| Msg::NavigateTimelineSearch(false))}>{icon("arrow-down")}</button>
                    <button aria-label="关闭对话搜索" title="关闭 (Esc)" onclick={link.callback(|_| Msg::CloseTimelineSearch)}>{icon("close")}</button>
                </div>
                <div class="timeline-search-scope"><span>{"仅搜索当前对话已加载的消息与活动"}</span>{if !self.history_exhausted.contains(&conversation_id) {html! {<button disabled={!self.authenticated || self.history_loading.contains(&conversation_id)} onclick={link.callback(|_| Msg::LoadOlder)}>{if self.history_loading.contains(&conversation_id) {"正在加载…"} else {"加载更早内容"}}</button>}} else {html! {}}}</div>
            </div>
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
        let model_label = conversation
            .and_then(|conversation| session_value_label(&conversation.session_options, "model"))
            .or_else(|| {
                models
                    .iter()
                    .find(|model| Some(model.id.as_str()) == selected_model)
                    .map(|model| model.display_name.as_str())
            })
            .unwrap_or("会话设置");
        let effort_label = conversation
            .and_then(|conversation| {
                session_value_label(&conversation.session_options, "thought_level")
            })
            .or_else(|| {
                models
                    .iter()
                    .find(|model| Some(model.id.as_str()) == selected_model)
                    .and_then(|model| {
                        model
                            .effort_options
                            .iter()
                            .find(|effort| Some(effort.id.as_str()) == selected_effort)
                    })
                    .map(|effort| effort.display_name.as_str())
            });
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
        let provider_ready = conversation.is_some()
            || capability.is_some_and(|capability| capability.health.state == ProviderState::Ready);
        let has_session_settings = conversation.map_or_else(
            || {
                capability.is_some_and(|capability| {
                    !capability.models.is_empty() || !capability.permission_modes.is_empty()
                })
            },
            |conversation| !conversation.session_options.is_empty(),
        );
        let can_send = self.connected
            && self.authenticated
            && provider_ready
            && !running
            && self.pending_send.is_none()
            && !self.composer.trim().is_empty()
            && self
                .pending_attachments
                .iter()
                .all(|attachment| attachment.bytes.is_some() && attachment.error.is_none());
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
                <div class="composer-inner">
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
                <div class="composer-surface">
                    <textarea
                        aria-label="消息"
                        placeholder={if running {"补充下一步指令…"} else {"写下你的消息…"}}
                        onkeydown={link.batch_callback(move |event: web_sys::KeyboardEvent| {
                            if can_send && event.key() == "Enter" && (event.ctrl_key() || event.meta_key()) && !event.is_composing() {
                                event.prevent_default(); Some(Msg::Send)
                            } else { None }
                        })}
                        value={self.composer.clone()}
                        disabled={self.pending_send.is_some()}
                        oninput={link.callback(|event: InputEvent| Msg::ComposerChanged(event.target_unchecked_into::<HtmlTextAreaElement>().value()))}
                    />
                    <div class="composer-bar">
                        <div class="composer-left">
                            <label class={classes!("attachment-action", (!attachment_enabled).then_some("disabled"))} title={attachment_capability.map_or("当前 Provider 不支持附件".to_owned(), |capability| format!("最多 {} 个，每个 {} MiB，总计 {} MiB", capability.max_count, capability.max_bytes / 1024 / 1024, capability.max_total_bytes / 1024 / 1024))}>{icon("plus")}<span class="action-label">{"附件"}</span><input type="file" multiple=true accept={accepts} disabled={!attachment_enabled} onchange={files_on_change}/></label>
                            {if let Some(option) = permission_option {html! {<select class="permission-select" disabled={running || !self.authenticated} aria-label="权限" title="权限" onchange={{let id=option.id.clone(); link.callback(move |event: Event| Msg::SetSessionOption(id.clone(), event.target_unchecked_into::<HtmlSelectElement>().value()))}}>{for option.values.iter().map(|value| html! {<option value={value.value.clone()} selected={value.value == option.current_value}>{&value.display_name}</option>})}</select>}} else if let Some(capability) = capability {html! {<select class="permission-select" aria-label="权限" title="权限" disabled={conversation.is_some() || capability.permission_modes.is_empty()} onchange={link.callback(|event: Event| Msg::SelectPermission(nonempty(event.target_unchecked_into::<HtmlSelectElement>().value())))}><option value="" selected={selected_permission.is_none()}>{"按次审批"}</option>{for capability.permission_modes.iter().map(|mode| html! {<option value={mode.id.clone()} selected={Some(mode.id.as_str()) == selected_permission}>{&mode.display_name}</option>})}</select>}} else {html! {}}}
                        </div>
                        <div class="composer-right">
                            {if has_session_settings {html! {<button class="model-trigger session-settings-trigger" onclick={link.callback(|_| Msg::ToggleSessionSettings)} aria-label="模型与会话设置" aria-expanded={self.session_settings_open.to_string()} title="模型与会话设置"><span>{model_label}{effort_label.map(|effort| format!(" · {effort}")).unwrap_or_default()}</span>{icon("chevron-down")}</button>}} else {html! {}}}
                            {if running {html! {<button class="stop" onclick={link.callback(|_| Msg::Interrupt)}>{"停止"}</button>}} else {html! {}}}
                            {if running && capability.is_some_and(|capability| capability.supports_steer) {html! {<button class="secondary" onclick={link.callback(|_| Msg::Steer)} disabled={self.composer.trim().is_empty()}>{"追加"}</button>}} else {html! {}}}
                            <button class="send-button" onclick={link.callback(|_| Msg::Send)} aria-label="发送消息" title="发送消息 · Ctrl / ⌘ + Enter" disabled={!can_send}>{if self.pending_send.is_some() {html! {"…"}} else {icon("arrow-up")}}</button>
                        </div>
                    </div>
                </div>
                <p class="composer-hint">{if !self.authenticated {"离线 · 草稿保存在此设备，连接后可发送"} else {"Ctrl / ⌘ + Enter 发送"}}</p>
                </div>
            </footer>
        }
    }

    fn view_timeline(&self, link: &yew::html::Scope<Self>, items: &[TimelineItem]) -> Html {
        let mut rendered = Vec::new();
        let mut index = 0;
        while index < items.len() {
            if is_collapsible_activity(&items[index].kind) {
                let start = index;
                while index < items.len() && is_collapsible_activity(&items[index].kind) {
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
            <details key={key.clone()} data-timeline-id={key} class="activity-group">
                <summary><span>{icon("chevron-right")}</span><strong>{summary}</strong><small>{format!("{} 项", items.len())}</small></summary>
                <div class="activity-details">{for items.iter().map(|item| self.view_timeline_item(link, item))}</div>
            </details>
        }
    }

    fn view_timeline_item(&self, link: &yew::html::Scope<Self>, item: &TimelineItem) -> Html {
        let is_match = self.timeline_search_open
            && timeline_item_matches_query(&item.kind, &self.timeline_search);
        let is_current = is_match && self.selected_search_result == Some(item.id);
        match &item.kind {
            TimelineItemKind::UserMessage { text } => {
                let copy_text = text.clone();
                html! { <article key={item.id.to_string()} data-timeline-id={item.id.to_string()} data-search-match={is_match.to_string()} data-search-current={is_current.to_string()} class="bubble user"><div class="message-heading"><span>{"你"}</span><time class="message-time">{format_message_time(item.created_at_ms)}</time></div><button class="message-copy" aria-label="复制用户消息原文" title="复制原文" onclick={link.callback(move |_| Msg::CopyText(copy_text.clone()))}>{icon("copy")}</button><div class="markdown-body">{self.cached_markdown(item, text)}</div></article> }
            }
            TimelineItemKind::AgentMessage { phase, text } => {
                let copy_text = text.clone();
                html! { <article key={item.id.to_string()} data-timeline-id={item.id.to_string()} data-search-match={is_match.to_string()} data-search-current={is_current.to_string()} class={classes!("bubble", "agent", format!("phase-{phase:?}").to_lowercase())}><button class="message-copy" aria-label="复制 Agent 消息原文" title="复制原文" onclick={link.callback(move |_| Msg::CopyText(copy_text.clone()))}>{icon("copy")}</button><div class="message-heading"><span>{provider_label(self.selected_provider)}</span><span class="message-phase">{if *phase == agent_remote_protocol::AgentMessagePhase::Commentary {"进展"} else {""}}</span><time class="message-time">{format_message_time(item.created_at_ms)}</time></div><div class="markdown-body">{self.cached_markdown(item, text)}</div></article> }
            }
            TimelineItemKind::Progress {
                kind,
                status,
                label,
                detail,
            } => {
                html! { <article key={item.id.to_string()} data-timeline-id={item.id.to_string()} data-search-match={is_match.to_string()} data-search-current={is_current.to_string()} class="process-card"><div class="process-title"><span>{format!("{kind:?}")}</span><b>{label}</b><em>{format!("{status:?}")}</em></div>{detail.as_ref().map(|value| html! {<pre>{value}</pre>}).unwrap_or_default()}</article> }
            }
            TimelineItemKind::Plan { steps } => {
                html! { <article key={item.id.to_string()} data-timeline-id={item.id.to_string()} data-search-match={is_match.to_string()} data-search-current={is_current.to_string()} class="process-card plan"><strong>{"计划"}</strong><ol>{for steps.iter().map(|step| html! {<li class={state_class_for_item(step.status)}>{&step.text}</li>})}</ol></article> }
            }
            TimelineItemKind::ToolCall {
                name,
                status,
                input_summary,
                output_summary,
            } => {
                html! { <article key={item.id.to_string()} data-timeline-id={item.id.to_string()} data-search-match={is_match.to_string()} data-search-current={is_current.to_string()} class="process-card"><div class="process-title"><span>{"工具"}</span><b>{name}</b><em>{format!("{status:?}")}</em></div>{summary_pair(input_summary, output_summary)}</article> }
            }
            TimelineItemKind::Command {
                command,
                relative_cwd,
                status,
                exit_code,
                output,
            } => {
                let copy_text = output.as_ref().map_or_else(
                    || command.clone(),
                    |output| format!("{command}\n\n{output}"),
                );
                html! { <article key={item.id.to_string()} data-timeline-id={item.id.to_string()} data-search-match={is_match.to_string()} data-search-current={is_current.to_string()} class="process-card command"><div class="process-title"><span>{"命令"}</span><b>{relative_cwd.as_deref().unwrap_or("项目根目录")}</b><em>{format!("{status:?}{}", exit_code.map(|code| format!(" · {code}")).unwrap_or_default())}</em><button class="command-copy" aria-label="复制命令和输出" onclick={link.callback(move |_| Msg::CopyText(copy_text.clone()))}>{"复制"}</button></div><code>{command}</code>{output.as_ref().map(|value| html! {<pre>{value}</pre>}).unwrap_or_default()}</article> }
            }
            TimelineItemKind::FileChange {
                relative_path,
                change_kind,
                status,
            } => {
                html! { <article key={item.id.to_string()} data-timeline-id={item.id.to_string()} data-search-match={is_match.to_string()} data-search-current={is_current.to_string()} class="process-card"><div class="process-title"><span>{"文件"}</span><b>{relative_path}</b><em>{format!("{} · {status:?}", change_kind)}</em></div></article> }
            }
            TimelineItemKind::Approval {
                approval_id,
                prompt,
                options,
                resolved_option,
            } => {
                let id = *approval_id;
                let pending = self.pending_approvals.contains(&id);
                let active = self
                    .selected_conversation_ref()
                    .is_some_and(|conversation| {
                        matches!(
                            conversation.state,
                            ConversationState::Running | ConversationState::NeedsApproval
                        )
                    });
                html! { <article key={item.id.to_string()} data-timeline-id={item.id.to_string()} data-search-match={is_match.to_string()} data-search-current={is_current.to_string()} class={classes!("approval-card", pending.then_some("pending"))} aria-busy={pending.to_string()}><span class="item-label">{"需要权限"}</span><p>{prompt}</p><div class={classes!("approval-actions", pending.then_some("pending"))}>{if let Some(resolved) = resolved_option { html! {<b>{format!("已选择：{}", options.iter().find(|option| &option.id == resolved).map_or(resolved.as_str(), |option| option.label.as_str()))}</b>} } else if !active {html! {<span class="approval-expired">{"该回合已结束，审批已失效"}</span>}} else { html! {{for options.iter().map(|option| { let value=option.id.clone(); html! {<button disabled={pending || !self.authenticated} onclick={link.callback(move |_| Msg::ResolveApproval(id, value.clone()))}>{if pending {"正在提交…"} else {option.label.as_str()}}</button>} })}} }}</div></article> }
            }
            TimelineItemKind::Image { attachment_id, alt } => {
                let id = *attachment_id;
                let onclick = link.callback(move |_| Msg::OpenImage(id));
                html! { <article key={item.id.to_string()} data-timeline-id={item.id.to_string()} data-search-match={is_match.to_string()} data-search-current={is_current.to_string()} class="image-card" {onclick}>{self.attachments.get(attachment_id).map(|url| html! {<img src={url.clone()} alt={alt.clone()} />}).unwrap_or_else(|| html! {<div class="image-loading">{"正在读取图片…"}</div>})}<span>{alt}</span></article> }
            }
            TimelineItemKind::Error { code, message } => {
                html! { <article key={item.id.to_string()} data-timeline-id={item.id.to_string()} data-search-match={is_match.to_string()} data-search-current={is_current.to_string()} class="error-card"><b>{code}</b><p>{message}</p></article> }
            }
        }
    }

    fn view_session_settings(&self, link: &yew::html::Scope<Self>) -> Html {
        if !self.session_settings_open {
            return html! {};
        }
        let conversation = self.selected_conversation_ref();
        let existing_conversation = conversation.is_some();
        let selected_model = conversation
            .and_then(|conversation| conversation.selected_model.as_deref())
            .or(self.selected_model.as_deref());
        let selected_effort = conversation
            .and_then(|conversation| conversation.selected_effort.as_deref())
            .or(self.selected_effort.as_deref());
        let selected_permission = conversation
            .and_then(|conversation| {
                conversation
                    .session_options
                    .iter()
                    .find(|option| option.id == "permission_mode")
            })
            .map(|option| option.current_value.as_str())
            .or(self.selected_permission.as_deref());
        let options = conversation
            .map(|conversation| conversation.session_options.clone())
            .unwrap_or_else(|| {
                new_conversation_session_options(
                    self.selected_capability(),
                    selected_model,
                    selected_effort,
                    selected_permission,
                )
            });
        if options.is_empty() {
            return html! {};
        }
        let running = conversation.is_some_and(|conversation| {
            matches!(
                conversation.state,
                ConversationState::Running | ConversationState::NeedsApproval
            )
        });
        html! {
            <div class="session-settings-modal" role="presentation" onclick={link.callback(|_| Msg::CloseSessionSettings)}>
                <section class="session-settings-panel" role="dialog" aria-modal="true" aria-label="会话设置" onclick={yew::Callback::from(|event: MouseEvent| event.stop_propagation())}>
                    <header><h2>{if existing_conversation {"当前对话设置"} else {"新对话设置"}}</h2><button aria-label="关闭会话设置" onclick={link.callback(|_| Msg::CloseSessionSettings)}>{icon("close")}</button></header>
                    <div class="session-settings-list">
                        {for options.iter().map(|option| {
                            let option_id = option.id.clone();
                            let onchange = if existing_conversation {
                                link.callback(move |event: Event| Msg::SetSessionOption(option_id.clone(), event.target_unchecked_into::<HtmlSelectElement>().value()))
                            } else {
                                link.callback(move |event: Event| {
                                    let value = nonempty(event.target_unchecked_into::<HtmlSelectElement>().value());
                                    match option_id.as_str() {
                                        "model" => Msg::SelectModel(value),
                                        "reasoning_effort" | "thought_level" => Msg::SelectEffort(value),
                                        "permission_mode" => Msg::SelectPermission(value),
                                        _ => Msg::CloseSessionSettings,
                                    }
                                })
                            };
                            let description = if option.id == "permission_mode" {
                                self.selected_capability().and_then(|capability| {
                                    capability.permission_modes.iter().find(|mode| mode.id == option.current_value)
                                }).map(|mode| mode.description.as_str())
                            } else {
                                None
                            };
                            html! {
                                <label class="session-settings-row">
                                    <span><strong>{&option.display_name}</strong>{description.map(|description| html! {<small>{description}</small>}).unwrap_or_default()}</span>
                                    <select disabled={running || (existing_conversation && !self.authenticated)} {onchange}>{for option.values.iter().map(|value| html! {<option value={value.value.clone()} selected={value.value == option.current_value}>{&value.display_name}</option>})}</select>
                                </label>
                            }
                        })}
                        {if running {html! {<p class="provider-limitation">{"Agent 运行时不能修改这些设置。"}</p>}} else {html! {}}}
                    </div>
                </section>
            </div>
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

fn session_value_label<'a>(options: &'a [SessionOption], category: &str) -> Option<&'a str> {
    let option = options.iter().find(|option| {
        option.category.as_deref() == Some(category)
            || option.id == category
            || (category == "thought_level" && option.id == "reasoning_effort")
    })?;
    option
        .values
        .iter()
        .find(|value| value.value == option.current_value)
        .map(|value| value.display_name.as_str())
}

fn format_message_time(timestamp_ms: i64) -> String {
    let date = js_sys::Date::new(&JsValue::from_f64(timestamp_ms as f64));
    format!("{:02}:{:02}", date.get_hours(), date.get_minutes())
}

fn icon(name: &str) -> Html {
    let path = match name {
        "connection" => "M8 7h8M8 17h8M5 4v6m0 4v6m14-16v6m0 4v6M2 7h6m8 0h6M2 17h6m8 0h6",
        "panel" => "M9 3v18M4 3h16a1 1 0 0 1 1 1v16a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1Z",
        "folder" => {
            "M3 7V5a1 1 0 0 1 1-1h5l2 3h9a1 1 0 0 1 1 1v11a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7Z"
        }
        "plus" => "M12 5v14M5 12h14",
        "search" => "m21 21-5-5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0",
        "message" => {
            "M21 11.5a8.5 8.5 0 0 1-8.5 8.5H4l-2 2V11.5A8.5 8.5 0 0 1 10.5 3h2A8.5 8.5 0 0 1 21 11.5Z"
        }
        "arrow-up" => "M12 19V5m-6 6 6-6 6 6",
        "arrow-down" => "M12 5v14m-6-6 6 6 6-6",
        "arrow-right" => "M5 12h14m-6-6 6 6-6 6",
        "chevron-down" => "m6 9 6 6 6-6",
        "chevron-right" => "m9 6 6 6-6 6",
        "copy" => "M8 8h12v12H8zM16 8V4H4v12h4",
        "edit" => "m16 3 5 5-12 12H4v-5L16 3Zm-3 3 5 5",
        "disconnect" => "M12 2v10M6 5a9 9 0 1 0 12 0",
        "lock" => "M7 10V7a5 5 0 0 1 10 0v3M5 10h14v11H5zM12 14v3",
        "check" => "m5 12 4 4L19 6",
        "close" => "m6 6 12 12M6 18 18 6",
        _ => unreachable!("unknown UI icon"),
    };
    html! {<svg class="ui-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={path}/></svg>}
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

fn new_conversation_session_options(
    capability: Option<&ProviderCapability>,
    selected_model: Option<&str>,
    selected_effort: Option<&str>,
    selected_permission: Option<&str>,
) -> Vec<SessionOption> {
    let Some(capability) = capability else {
        return Vec::new();
    };
    let model = capability
        .models
        .iter()
        .find(|model| Some(model.id.as_str()) == selected_model)
        .or_else(|| capability.models.first());
    let mut options = Vec::new();
    if !capability.models.is_empty() {
        options.push(SessionOption {
            id: "model".to_owned(),
            display_name: "模型".to_owned(),
            category: None,
            current_value: selected_model
                .or_else(|| model.map(|model| model.id.as_str()))
                .unwrap_or_default()
                .to_owned(),
            values: capability
                .models
                .iter()
                .map(|model| agent_remote_protocol::SessionOptionValue {
                    value: model.id.clone(),
                    display_name: model.display_name.clone(),
                })
                .collect(),
        });
    }
    if let Some(model) = model
        && !model.effort_options.is_empty()
    {
        options.push(SessionOption {
            id: "reasoning_effort".to_owned(),
            display_name: "推理强度".to_owned(),
            category: None,
            current_value: selected_effort
                .or(model.default_effort.as_deref())
                .or_else(|| {
                    model
                        .effort_options
                        .first()
                        .map(|effort| effort.id.as_str())
                })
                .unwrap_or_default()
                .to_owned(),
            values: model
                .effort_options
                .iter()
                .map(|effort| agent_remote_protocol::SessionOptionValue {
                    value: effort.id.clone(),
                    display_name: effort.display_name.clone(),
                })
                .collect(),
        });
    }
    if !capability.permission_modes.is_empty() {
        options.push(SessionOption {
            id: "permission_mode".to_owned(),
            display_name: "权限".to_owned(),
            category: None,
            current_value: selected_permission
                .or(capability.default_permission_mode.as_deref())
                .or_else(|| {
                    capability
                        .permission_modes
                        .first()
                        .map(|mode| mode.id.as_str())
                })
                .unwrap_or_default()
                .to_owned(),
            values: capability
                .permission_modes
                .iter()
                .map(|mode| agent_remote_protocol::SessionOptionValue {
                    value: mode.id.clone(),
                    display_name: mode.display_name.clone(),
                })
                .collect(),
        });
    }
    options
}

fn provider_class(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Codex => "codex",
        ProviderId::Grok => "grok",
        ProviderId::ClaudeCode => "claude-code",
        ProviderId::GeminiCli => "gemini-cli",
        ProviderId::CopilotCli => "copilot-cli",
        ProviderId::OpenCode => "open-code",
        ProviderId::Cursor => "cursor",
        ProviderId::Cline => "cline",
        ProviderId::Goose => "goose",
        ProviderId::Junie => "junie",
        ProviderId::QwenCode => "qwen-code",
        ProviderId::KimiCli => "kimi-cli",
        ProviderId::KiroCli => "kiro-cli",
        ProviderId::MistralVibe => "mistral-vibe",
        ProviderId::QoderCli => "qoder-cli",
        ProviderId::Auggie => "auggie",
        ProviderId::FactoryDroid => "factory-droid",
        ProviderId::Devin => "devin",
        ProviderId::CodeBuddy => "codebuddy",
        ProviderId::GlmAgent => "glm-agent",
        ProviderId::KiloCode => "kilo-code",
        ProviderId::Amp => "amp",
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
    ProviderId::ALL
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
        ProviderId::ClaudeCode => "Claude Code",
        ProviderId::GeminiCli => "Gemini CLI",
        ProviderId::CopilotCli => "GitHub Copilot",
        ProviderId::OpenCode => "OpenCode",
        ProviderId::Cursor => "Cursor Agent",
        ProviderId::Cline => "Cline",
        ProviderId::Goose => "Goose",
        ProviderId::Junie => "JetBrains Junie",
        ProviderId::QwenCode => "Qwen Code",
        ProviderId::KimiCli => "Kimi CLI",
        ProviderId::KiroCli => "Kiro CLI",
        ProviderId::MistralVibe => "Mistral Vibe",
        ProviderId::QoderCli => "Qoder CLI",
        ProviderId::Auggie => "Augment Auggie",
        ProviderId::FactoryDroid => "Factory Droid",
        ProviderId::Devin => "Devin",
        ProviderId::CodeBuddy => "Tencent CodeBuddy",
        ProviderId::GlmAgent => "GLM Agent",
        ProviderId::KiloCode => "Kilo Code",
        ProviderId::Amp => "Amp",
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
