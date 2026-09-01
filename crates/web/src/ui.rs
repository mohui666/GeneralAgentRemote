use std::collections::HashMap;

use agent_remote_protocol::{
    ApprovalId, AttachmentId, ClientCommand, CommandId, Conversation, ConversationId,
    ConversationState, DeviceId, HostId, ProjectId, ProviderCapability, ProviderId, ServerMessage,
    Snapshot, TimelineItem, TimelineItemKind, decode, encode,
};
use js_sys::{Array, ArrayBuffer, Uint8Array};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{
    BinaryType, Blob, CloseEvent, Event, HtmlInputElement, HtmlSelectElement, HtmlTextAreaElement,
    MessageEvent, Url, UrlSearchParams, WebSocket, window,
};
use yew::{Component, Context, Html, InputEvent, MouseEvent, TargetCast, classes, html};

const CREDENTIALS_KEY: &str = "agent_remote_credentials_v1";
const WS_SUBPROTOCOL: &str = "agent-remote.cbor.v1";

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
    connected: bool,
    authenticated: bool,
    status: String,
    pair_link: String,
    selected_conversation: Option<ConversationId>,
    selected_project: Option<ProjectId>,
    selected_provider: ProviderId,
    selected_model: Option<String>,
    selected_effort: Option<String>,
    selected_native_session: Option<String>,
    composer: String,
    attachments: HashMap<AttachmentId, String>,
    fullscreen_image: Option<AttachmentId>,
}

pub enum Msg {
    Opened,
    Closed(String),
    SocketError,
    Server(ServerMessage),
    DecodeError(String),
    PairLinkChanged(String),
    OpenPairLink,
    ConnectStored(usize),
    ForgetCredentials,
    SelectConversation(ConversationId),
    SelectProject(ProjectId),
    SelectProvider(ProviderId),
    SelectModel(Option<String>),
    SelectEffort(Option<String>),
    SelectNativeSession(Option<String>),
    CreateConversation,
    ComposerChanged(String),
    Send,
    Steer,
    Interrupt,
    ResolveApproval(ApprovalId, String),
    SetSessionOption(String, String),
    OpenImage(AttachmentId),
    CloseImage,
}

impl Component for App {
    type Message = Msg;
    type Properties = ();

    fn create(context: &Context<Self>) -> Self {
        let credentials = load_credentials();
        let connection = fragment_connection().or_else(|| {
            credentials
                .first()
                .cloned()
                .map(|credential| ConnectionConfig {
                    host_id: credential.host_id,
                    pair_token: None,
                    origin: credential.origin.clone(),
                    relay: credential.relay,
                    credential: Some(credential),
                })
        });
        let mut app = Self {
            socket: None,
            _socket_callbacks: None,
            connection,
            credentials,
            snapshot: None,
            connected: false,
            authenticated: false,
            status: "等待连接".to_owned(),
            pair_link: String::new(),
            selected_conversation: None,
            selected_project: None,
            selected_provider: ProviderId::Codex,
            selected_model: None,
            selected_effort: None,
            selected_native_session: None,
            composer: String::new(),
            attachments: HashMap::new(),
            fullscreen_image: None,
        };
        if app.connection.is_some() {
            app.connect(context);
        }
        app
    }

    fn update(&mut self, context: &Context<Self>, message: Self::Message) -> bool {
        match message {
            Msg::Opened => {
                self.connected = true;
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
                    self.send(command);
                }
            }
            Msg::Closed(reason) => {
                self.connected = false;
                self.authenticated = false;
                self.status = if reason.is_empty() {
                    "Host 离线或连接已关闭".to_owned()
                } else {
                    format!("连接已关闭：{reason}")
                };
            }
            Msg::SocketError => {
                self.status = "无法连接 Host；请确认 Host 在线且地址可达".to_owned();
            }
            Msg::DecodeError(error) => self.status = format!("协议错误：{error}"),
            Msg::Server(server_message) => self.apply_server_message(server_message),
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
                    self.connect(context);
                }
            }
            Msg::ForgetCredentials => {
                self.credentials.clear();
                save_credentials(&self.credentials);
                self.connection = None;
                self.snapshot = None;
                self.socket = None;
                self._socket_callbacks = None;
                self.status = "本地设备凭证已删除".to_owned();
            }
            Msg::SelectConversation(id) => {
                self.selected_conversation = Some(id);
                self.request_images_for_selected();
            }
            Msg::SelectProject(id) => {
                self.selected_project = Some(id);
                self.reset_dynamic_selection();
            }
            Msg::SelectProvider(provider) => {
                self.selected_provider = provider;
                self.ensure_project_for_provider();
                self.reset_dynamic_selection();
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
            Msg::SelectNativeSession(session) => self.selected_native_session = session,
            Msg::CreateConversation => {
                if let Some(project_id) = self.selected_project {
                    self.send(ClientCommand::CreateConversation {
                        command_id: CommandId::new(),
                        project_id,
                        provider: self.selected_provider,
                        native_session_id: self.selected_native_session.clone(),
                        model: self.selected_model.clone(),
                        effort: self.selected_effort.clone(),
                    });
                }
            }
            Msg::ComposerChanged(value) => self.composer = value,
            Msg::Send => {
                if let Some(conversation_id) = self.selected_conversation
                    && !self.composer.trim().is_empty()
                {
                    let text = std::mem::take(&mut self.composer);
                    self.send(ClientCommand::SendMessage {
                        command_id: CommandId::new(),
                        conversation_id,
                        text,
                    });
                }
            }
            Msg::Steer => {
                if let Some(conversation_id) = self.selected_conversation
                    && !self.composer.trim().is_empty()
                {
                    let text = std::mem::take(&mut self.composer);
                    self.send(ClientCommand::Steer {
                        command_id: CommandId::new(),
                        conversation_id,
                        text,
                    });
                }
            }
            Msg::Interrupt => {
                if let Some(conversation_id) = self.selected_conversation {
                    self.send(ClientCommand::Interrupt {
                        command_id: CommandId::new(),
                        conversation_id,
                    });
                }
            }
            Msg::ResolveApproval(approval_id, option_id) => {
                self.send(ClientCommand::ResolveApproval {
                    command_id: CommandId::new(),
                    approval_id,
                    option_id,
                });
            }
            Msg::SetSessionOption(option_id, value) => {
                if let Some(conversation_id) = self.selected_conversation {
                    self.send(ClientCommand::SetSessionOption {
                        command_id: CommandId::new(),
                        conversation_id,
                        option_id,
                        value,
                    });
                }
            }
            Msg::OpenImage(id) => {
                self.fullscreen_image = Some(id);
                if !self.attachments.contains_key(&id) {
                    self.send(ClientCommand::GetAttachment { attachment_id: id });
                }
            }
            Msg::CloseImage => self.fullscreen_image = None,
        }
        true
    }

    fn view(&self, context: &Context<Self>) -> Html {
        let link = context.link();
        let Some(snapshot) = &self.snapshot else {
            return self.view_connection(link);
        };
        let selected = self.selected_conversation.and_then(|id| {
            snapshot
                .conversations
                .iter()
                .find(|conversation| conversation.id == id)
        });
        html! {
            <main class="app-shell">
                <aside class="sidebar">
                    <div class="brand">
                        <span class="brand-mark">{"AR"}</span>
                        <div><strong>{"Agent Remote"}</strong><small>{&snapshot.host_name}</small></div>
                    </div>
                    {self.view_new_conversation(link, snapshot)}
                    <nav class="conversation-list" aria-label="会话列表">
                        {for snapshot.conversations.iter().map(|conversation| self.view_conversation_row(link, conversation, snapshot))}
                    </nav>
                    <div class="sidebar-footer">
                        <span class={if self.connected {"status-dot online"} else {"status-dot"}}></span>
                        <span>{if self.connected {"Host 在线"} else {"Host 离线"}}</span>
                    </div>
                </aside>
                <section class="chat-pane">
                    {if let Some(conversation) = selected {
                        self.view_chat(link, conversation, snapshot)
                    } else {
                        html! { <div class="empty-state"><h2>{"选择或新建会话"}</h2><p>{"消息、过程、审批和图片会显示在这里。"}</p></div> }
                    }}
                </section>
                {self.view_fullscreen_image(link)}
            </main>
        }
    }
}

impl App {
    fn connect(&mut self, context: &Context<Self>) {
        let Some(connection) = self.connection.clone() else {
            return;
        };
        match open_socket(context, &connection) {
            Ok((socket, callbacks)) => {
                self.socket = Some(socket);
                self._socket_callbacks = Some(callbacks);
                self.status = "正在连接…".to_owned();
            }
            Err(error) => self.status = error,
        }
    }

    fn send(&self, command: ClientCommand) {
        if let Some(socket) = &self.socket
            && let Ok(bytes) = encode(&command)
        {
            let _ = socket.send_with_u8_array(&bytes);
        }
    }

    fn apply_server_message(&mut self, message: ServerMessage) {
        match message {
            ServerMessage::Paired {
                host_id,
                device_id,
                device_token,
            } => {
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
                self.send(ClientCommand::GetSnapshot);
            }
            ServerMessage::Authenticated { .. } => {
                self.authenticated = true;
                self.status = "已连接".to_owned();
                self.send(ClientCommand::GetSnapshot);
            }
            ServerMessage::Snapshot { snapshot } => {
                self.selected_conversation = self.selected_conversation.or_else(|| {
                    snapshot
                        .conversations
                        .first()
                        .map(|conversation| conversation.id)
                });
                self.snapshot = Some(snapshot);
                self.ensure_project_for_provider();
                self.reset_dynamic_selection();
                self.request_images_for_selected();
            }
            ServerMessage::ProviderChanged { capability } => {
                if let Some(snapshot) = &mut self.snapshot {
                    upsert_capability(&mut snapshot.provider_capabilities, capability);
                }
            }
            ServerMessage::ConversationUpserted { conversation } => {
                if let Some(snapshot) = &mut self.snapshot {
                    upsert_conversation(&mut snapshot.conversations, conversation.clone());
                    self.selected_conversation = Some(conversation.id);
                }
            }
            ServerMessage::TimelineItemUpserted { item } => {
                let image_id = match &item.kind {
                    TimelineItemKind::Image { attachment_id, .. } => Some(*attachment_id),
                    _ => None,
                };
                if let Some(snapshot) = &mut self.snapshot {
                    upsert_timeline_item(&mut snapshot.timeline, item);
                }
                if let Some(attachment_id) = image_id {
                    self.send(ClientCommand::GetAttachment { attachment_id });
                }
            }
            ServerMessage::ConversationRemoved { conversation_id } => {
                if let Some(snapshot) = &mut self.snapshot {
                    snapshot
                        .conversations
                        .retain(|conversation| conversation.id != conversation_id);
                }
            }
            ServerMessage::AttachmentData { metadata, bytes } => {
                if let Some(url) = image_object_url(&metadata.mime_type, &bytes) {
                    self.attachments.insert(metadata.id, url);
                }
            }
            ServerMessage::HostStatus {
                online, message, ..
            } => {
                self.connected = online;
                if !online {
                    self.status = message.unwrap_or_else(|| "Host 离线".to_owned());
                }
            }
            ServerMessage::CommandRejected { message, .. }
            | ServerMessage::ProtocolError { message, .. } => self.status = message,
            ServerMessage::CommandAccepted { .. } => {}
        }
    }

    fn reset_dynamic_selection(&mut self) {
        let capability = self.selected_capability().cloned();
        self.selected_model = capability
            .as_ref()
            .and_then(|capability| capability.models.first())
            .map(|model| model.id.clone());
        self.selected_effort = capability.as_ref().and_then(|capability| {
            capability.models.first().and_then(|model| {
                model
                    .default_effort
                    .clone()
                    .or_else(|| model.effort_options.first().map(|effort| effort.id.clone()))
            })
        });
        self.selected_native_session = None;
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
            self.selected_project = snapshot
                .projects
                .iter()
                .find(|project| {
                    project.valid && project.enabled_providers.contains(&self.selected_provider)
                })
                .map(|project| project.id);
        }
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

    fn request_images_for_selected(&self) {
        let Some(conversation_id) = self.selected_conversation else {
            return;
        };
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        for attachment_id in snapshot.timeline.iter().filter_map(|item| {
            if item.conversation_id == conversation_id
                && let TimelineItemKind::Image { attachment_id, .. } = item.kind
                && !self.attachments.contains_key(&attachment_id)
            {
                Some(attachment_id)
            } else {
                None
            }
        }) {
            self.send(ClientCommand::GetAttachment { attachment_id });
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

    fn view_new_conversation(&self, link: &yew::html::Scope<Self>, snapshot: &Snapshot) -> Html {
        let capability = self.selected_capability();
        let models = capability.map_or(&[][..], |capability| capability.models.as_slice());
        let efforts = models
            .iter()
            .find(|model| Some(model.id.as_str()) == self.selected_model.as_deref())
            .map_or(&[][..], |model| model.effort_options.as_slice());
        let sessions = capability.map_or(&[][..], |capability| capability.sessions.as_slice());
        html! {
            <section class="new-conversation">
                <h2>{"新建会话"}</h2>
                <label class="compact-field"><span>{"Agent"}</span>
                    <select onchange={link.callback(|event: Event| {
                        let value = event.target_unchecked_into::<HtmlSelectElement>().value();
                        Msg::SelectProvider(if value == "grok" { ProviderId::Grok } else { ProviderId::Codex })
                    })}>
                        <option value="codex" selected={self.selected_provider == ProviderId::Codex}>{"OpenAI Codex"}</option>
                        <option value="grok" selected={self.selected_provider == ProviderId::Grok}>{"Grok Build"}</option>
                    </select>
                </label>
                <label class="compact-field"><span>{"项目"}</span>
                    <select onchange={link.callback(|event: Event| {
                        Msg::SelectProject(ProjectId(Uuid::parse_str(&event.target_unchecked_into::<HtmlSelectElement>().value()).expect("project id from option")))
                    })}>
                        {for snapshot.projects.iter().filter(|project| project.valid && project.enabled_providers.contains(&self.selected_provider)).map(|project| html! {
                            <option value={project.id.to_string()} selected={Some(project.id) == self.selected_project}>{&project.display_name}</option>
                        })}
                    </select>
                </label>
                {if models.is_empty() { html! {
                    <p class="capability-note">{capability.and_then(|item| item.limitation.clone()).unwrap_or_else(|| "当前 Provider 未暴露预创建模型选择；会话建立后显示正式配置项。".to_owned())}</p>
                }} else { html! {
                    <>
                        <label class="compact-field"><span>{"模型"}</span>
                            <select onchange={link.callback(|event: Event| Msg::SelectModel(nonempty(event.target_unchecked_into::<HtmlSelectElement>().value())))}>
                                {for models.iter().map(|model| html! { <option value={model.id.clone()} selected={Some(model.id.as_str()) == self.selected_model.as_deref()}>{&model.display_name}</option> })}
                            </select>
                        </label>
                        <label class="compact-field"><span>{"Effort"}</span>
                            <select disabled={efforts.is_empty()} onchange={link.callback(|event: Event| Msg::SelectEffort(nonempty(event.target_unchecked_into::<HtmlSelectElement>().value())))}>
                                {for efforts.iter().map(|effort| html! { <option value={effort.id.clone()} selected={Some(effort.id.as_str()) == self.selected_effort.as_deref()}>{&effort.display_name}</option> })}
                            </select>
                        </label>
                    </>
                }}}
                {if sessions.is_empty() { html! {} } else { html! {
                    <label class="compact-field"><span>{"会话"}</span>
                        <select onchange={link.callback(|event: Event| Msg::SelectNativeSession(nonempty(event.target_unchecked_into::<HtmlSelectElement>().value())))}>
                            <option value="">{"新会话"}</option>
                            {for sessions.iter().map(|session| html! { <option value={session.native_session_id.clone()}>{&session.title}</option> })}
                        </select>
                    </label>
                }}}
                <button class="primary wide" onclick={link.callback(|_| Msg::CreateConversation)} disabled={self.selected_project.is_none()}>{"创建会话"}</button>
            </section>
        }
    }

    fn view_conversation_row(
        &self,
        link: &yew::html::Scope<Self>,
        conversation: &Conversation,
        snapshot: &Snapshot,
    ) -> Html {
        let id = conversation.id;
        let onclick = link.callback(move |_| Msg::SelectConversation(id));
        let project_name = snapshot
            .projects
            .iter()
            .find(|project| project.id == conversation.project_id)
            .map_or("未知项目", |project| project.display_name.as_str());
        html! {
            <button class={classes!("conversation-row", (Some(id) == self.selected_conversation).then_some("active"))} {onclick}>
                <span class={classes!("provider-badge", provider_class(conversation.provider))}>{provider_short(conversation.provider)}</span>
                <span class="conversation-copy"><strong>{&conversation.title}</strong><small>{format!("{} · {}", project_name, state_label(conversation.state))}</small></span>
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
                    <div><p class="eyebrow">{format!("{} · {}", conversation.provider, project_name)}</p><h1>{&conversation.title}</h1></div>
                    <div class="header-controls">
                        {if conversation.session_options.is_empty() { html! {
                            <><span class="header-chip">{conversation.selected_model.as_deref().unwrap_or("Provider 默认模型")}</span><span class="header-chip">{conversation.selected_effort.as_deref().unwrap_or("默认 effort")}</span></>
                        }} else { html! {
                            {for conversation.session_options.iter().map(|option| {
                                let option_id = option.id.clone();
                                html! {
                                    <label class="header-select"><span>{&option.display_name}</span><select disabled={running} onchange={link.callback(move |event: Event| Msg::SetSessionOption(option_id.clone(), event.target_unchecked_into::<HtmlSelectElement>().value()))}>
                                        {for option.values.iter().map(|value| html! { <option value={value.value.clone()} selected={value.value == option.current_value}>{&value.display_name}</option> })}
                                    </select></label>
                                }
                            })}
                        }}}
                        <span class={classes!("state-pill", state_class(conversation.state))}>{state_label(conversation.state)}</span>
                    </div>
                </header>
                <div class="timeline">
                    {for snapshot.timeline.iter().filter(|item| item.conversation_id == conversation.id).map(|item| self.view_timeline_item(link, item))}
                </div>
                <footer class="composer">
                    <textarea
                        placeholder={if running {"输入追加指令（Provider 支持时可用）"} else {"发送文字消息…"}}
                        value={self.composer.clone()}
                        oninput={link.callback(|event: InputEvent| Msg::ComposerChanged(event.target_unchecked_into::<HtmlTextAreaElement>().value()))}
                    />
                    <div class="composer-actions">
                        {if running { html! { <button class="stop" onclick={link.callback(|_| Msg::Interrupt)}>{"停止"}</button> } } else { html! {} }}
                        {if running && capability.is_some_and(|capability| capability.supports_steer) {
                            html! { <button class="secondary" onclick={link.callback(|_| Msg::Steer)} disabled={self.composer.trim().is_empty()}>{"追加指令"}</button> }
                        } else { html! {} }}
                        <button class="primary" onclick={link.callback(|_| Msg::Send)} disabled={running || self.composer.trim().is_empty()}>{"发送"}</button>
                    </div>
                </footer>
            </>
        }
    }

    fn view_timeline_item(&self, link: &yew::html::Scope<Self>, item: &TimelineItem) -> Html {
        match &item.kind {
            TimelineItemKind::UserMessage { text } => {
                html! { <article class="bubble user"><p>{text}</p></article> }
            }
            TimelineItemKind::AgentMessage { phase, text } => {
                html! { <article class={classes!("bubble", "agent", format!("phase-{phase:?}").to_lowercase())}><span class="item-label">{format!("{phase:?}")}</span><p>{text}</p></article> }
            }
            TimelineItemKind::Progress {
                kind,
                status,
                label,
                detail,
            } => {
                html! { <article class="process-card"><div class="process-title"><span>{format!("{kind:?}")}</span><b>{label}</b><em>{format!("{status:?}")}</em></div>{detail.as_ref().map(|value| html! {<pre>{value}</pre>}).unwrap_or_default()}</article> }
            }
            TimelineItemKind::Plan { steps } => {
                html! { <article class="process-card plan"><strong>{"计划"}</strong><ol>{for steps.iter().map(|step| html! {<li class={state_class_for_item(step.status)}>{&step.text}</li>})}</ol></article> }
            }
            TimelineItemKind::ToolCall {
                name,
                status,
                input_summary,
                output_summary,
            } => {
                html! { <article class="process-card"><div class="process-title"><span>{"工具"}</span><b>{name}</b><em>{format!("{status:?}")}</em></div>{summary_pair(input_summary, output_summary)}</article> }
            }
            TimelineItemKind::Command {
                command,
                relative_cwd,
                status,
                exit_code,
                output,
            } => {
                html! { <article class="process-card command"><div class="process-title"><span>{"命令"}</span><b>{relative_cwd.as_deref().unwrap_or("项目根目录")}</b><em>{format!("{status:?}{}", exit_code.map(|code| format!(" · {code}")).unwrap_or_default())}</em></div><code>{command}</code>{output.as_ref().map(|value| html! {<pre>{value}</pre>}).unwrap_or_default()}</article> }
            }
            TimelineItemKind::FileChange {
                relative_path,
                change_kind,
                status,
            } => {
                html! { <article class="process-card"><div class="process-title"><span>{"文件"}</span><b>{relative_path}</b><em>{format!("{} · {status:?}", change_kind)}</em></div></article> }
            }
            TimelineItemKind::Approval {
                approval_id,
                prompt,
                options,
                resolved_option,
            } => {
                let id = *approval_id;
                html! { <article class="approval-card"><span class="item-label">{"需要权限"}</span><p>{prompt}</p><div class="approval-actions">{if let Some(resolved) = resolved_option { html! {<b>{format!("已选择：{resolved}")}</b>} } else { html! {{for options.iter().map(|option| { let value=option.id.clone(); html! {<button onclick={link.callback(move |_| Msg::ResolveApproval(id, value.clone()))}>{&option.label}</button>} })}} }}</div></article> }
            }
            TimelineItemKind::Image { attachment_id, alt } => {
                let id = *attachment_id;
                let onclick = link.callback(move |_| Msg::OpenImage(id));
                html! { <article class="image-card" {onclick}>{self.attachments.get(attachment_id).map(|url| html! {<img src={url.clone()} alt={alt.clone()} />}).unwrap_or_else(|| html! {<div class="image-loading">{"正在读取图片…"}</div>})}<span>{alt}</span></article> }
            }
            TimelineItemKind::Error { code, message } => {
                html! { <article class="error-card"><b>{code}</b><p>{message}</p></article> }
            }
        }
    }

    fn view_fullscreen_image(&self, link: &yew::html::Scope<Self>) -> Html {
        let Some(id) = self.fullscreen_image else {
            return html! {};
        };
        html! { <div class="lightbox" onclick={link.callback(|_: MouseEvent| Msg::CloseImage)}>{self.attachments.get(&id).map(|url| html! {<img src={url.clone()} alt="Agent output" />}).unwrap_or_default()}<button>{"关闭"}</button></div> }
    }
}

fn open_socket(
    context: &Context<App>,
    config: &ConnectionConfig,
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
        Box::new(move |_: Event| link.send_message(Msg::Opened)) as Box<dyn FnMut(_)>
    );
    socket.set_onopen(Some(open.as_ref().unchecked_ref()));

    let link = context.link().clone();
    let message = Closure::wrap(Box::new(move |event: MessageEvent| {
        if let Ok(buffer) = event.data().dyn_into::<ArrayBuffer>() {
            let bytes = Uint8Array::new(&buffer).to_vec();
            match decode::<ServerMessage>(&bytes) {
                Ok(message) => link.send_message(Msg::Server(message)),
                Err(error) => link.send_message(Msg::DecodeError(error.to_string())),
            }
        }
    }) as Box<dyn FnMut(_)>);
    socket.set_onmessage(Some(message.as_ref().unchecked_ref()));

    let link = context.link().clone();
    let error = Closure::wrap(
        Box::new(move |_: Event| link.send_message(Msg::SocketError)) as Box<dyn FnMut(_)>,
    );
    socket.set_onerror(Some(error.as_ref().unchecked_ref()));

    let link = context.link().clone();
    let close = Closure::wrap(Box::new(move |event: CloseEvent| {
        link.send_message(Msg::Closed(event.reason()))
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

fn upsert_conversation(conversations: &mut Vec<Conversation>, incoming: Conversation) {
    if let Some(existing) = conversations.iter_mut().find(|item| item.id == incoming.id) {
        if incoming.revision >= existing.revision {
            *existing = incoming;
        }
    } else {
        conversations.push(incoming);
    }
    conversations.sort_by_key(|item| std::cmp::Reverse(item.updated_at_ms));
}

fn upsert_timeline_item(items: &mut Vec<TimelineItem>, incoming: TimelineItem) {
    if let Some(existing) = items.iter_mut().find(|item| item.id == incoming.id) {
        if incoming.revision >= existing.revision {
            *existing = incoming;
        }
    } else {
        items.push(incoming);
    }
    items.sort_by_key(|item| (item.created_at_ms, item.id));
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

fn summary_pair(input: &Option<String>, output: &Option<String>) -> Html {
    html! { <>{input.as_ref().map(|value| html! {<pre>{value}</pre>}).unwrap_or_default()}{output.as_ref().map(|value| html! {<pre>{value}</pre>}).unwrap_or_default()}</> }
}
