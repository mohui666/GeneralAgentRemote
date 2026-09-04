use std::{collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use agent_remote_protocol::encode;
use anyhow::Result;
use axum::{
    Router,
    extract::{
        ConnectInfo, State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use tokio::{net::TcpListener, sync::mpsc};
use tower_http::services::{ServeDir, ServeFile};

use crate::{
    app::AppService,
    transport::session::{
        ApplicationSession, AuthRateLimiter, AuthenticatedSession, CommandSchedule, CommandScope,
    },
};

pub const WS_SUBPROTOCOL: &str = "agent-remote.cbor.v3";
const MAX_WS_MESSAGE_BYTES: usize = 12 * 1024 * 1024;
const COMMAND_QUEUE_CAPACITY: usize = 32;
const RESPONSE_QUEUE_CAPACITY: usize = 64;
const SCOPED_WORKER_IDLE: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct DirectState {
    service: Arc<AppService>,
    rate_limiter: Arc<AuthRateLimiter>,
}

pub fn router(service: Arc<AppService>, web_root: PathBuf) -> Router {
    let static_files = ServeDir::new(&web_root)
        .append_index_html_on_directories(true)
        .fallback(ServeFile::new(web_root.join("index.html")));
    Router::new()
        .route("/health", get(health))
        .route("/ws", get(upgrade))
        .fallback_service(static_files)
        .with_state(DirectState {
            service,
            rate_limiter: Arc::new(AuthRateLimiter::default()),
        })
}

pub async fn serve(
    listener: TcpListener,
    service: Arc<AppService>,
    web_root: PathBuf,
) -> Result<()> {
    service.start_provider_event_pumps();
    axum::serve(
        listener,
        router(service, web_root).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn upgrade(
    State(state): State<DirectState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    websocket: WebSocketUpgrade,
) -> Response {
    websocket
        .max_message_size(MAX_WS_MESSAGE_BYTES)
        .protocols([WS_SUBPROTOCOL])
        .on_upgrade(move |socket| run_socket(socket, state, peer))
        .into_response()
}

async fn run_socket(socket: WebSocket, state: DirectState, peer: SocketAddr) {
    let (mut sender, mut receiver) = socket.split();
    let mut session = ApplicationSession::new(
        Arc::clone(&state.service),
        Arc::clone(&state.rate_limiter),
        peer.ip().to_string(),
    );
    let mut updates = state.service.subscribe();
    let (response_tx, mut response_rx) = mpsc::channel(RESPONSE_QUEUE_CAPACITY);
    let mut scoped_txs: HashMap<CommandScope, mpsc::WeakSender<Vec<u8>>> = HashMap::new();
    loop {
        tokio::select! {
            incoming = receiver.next() => {
                let Some(incoming) = incoming else { break };
                match incoming {
                    Ok(Message::Binary(bytes)) => {
                        if let Some(authenticated) = session.authenticated() {
                            let payload = bytes.to_vec();
                            match AuthenticatedSession::schedule(&payload) {
                                CommandSchedule::Concurrent => {
                                    let response_tx = response_tx.clone();
                                    tokio::spawn(async move {
                                        let response = authenticated.process(&payload).await;
                                        let _ = response_tx.send(response).await;
                                    });
                                }
                                CommandSchedule::Scoped(scope) => {
                                    scoped_txs.retain(|_, sender| sender.strong_count() > 0);
                                    let scoped_tx = scoped_txs
                                        .get(&scope)
                                        .and_then(mpsc::WeakSender::upgrade)
                                        .unwrap_or_else(|| {
                                            let sender = spawn_scoped_worker(
                                                authenticated.clone(),
                                                response_tx.clone(),
                                            );
                                            scoped_txs.insert(scope, sender.downgrade());
                                            sender
                                        });
                                    if scoped_tx.try_send(payload).is_err() {
                                        let _ = sender.send(Message::Close(Some(CloseFrame {
                                            code: 1013,
                                            reason: "command scope queue is full; reconnect and retry unacknowledged commands".into(),
                                        }))).await;
                                        break;
                                    }
                                }
                            }
                        } else {
                            let response = session.process(&bytes).await;
                            if sender.send(Message::Binary(response.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(Message::Close(_)) | Err(_) => break,
                    Ok(Message::Ping(payload)) => {
                        if sender.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Ok(Message::Pong(_)) => {}
                    Ok(Message::Text(_)) => {
                        let _ = sender.send(Message::Close(Some(CloseFrame {
                            code: 1003,
                            reason: "binary CBOR frames are required".into(),
                        }))).await;
                        break;
                    }
                }
            }
            response = response_rx.recv() => {
                let Some(response) = response else { break };
                if sender.send(Message::Binary(response.into())).await.is_err() {
                    break;
                }
            }
            update = updates.recv(), if session.is_authenticated() => {
                match update {
                    Ok(update) => {
                        let bytes = encode(&update).expect("server update is serializable");
                        if sender.send(Message::Binary(bytes.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let _ = sender.send(Message::Close(Some(CloseFrame {
                            code: 1013,
                            reason: "client fell behind; reconnect and request a snapshot".into(),
                        }))).await;
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

fn spawn_scoped_worker(
    session: AuthenticatedSession,
    response_tx: mpsc::Sender<Vec<u8>>,
) -> mpsc::Sender<Vec<u8>> {
    let (command_tx, mut command_rx) = mpsc::channel::<Vec<u8>>(COMMAND_QUEUE_CAPACITY);
    let keepalive = command_tx.clone();
    tokio::spawn(async move {
        let _keepalive = keepalive;
        loop {
            match tokio::time::timeout(SCOPED_WORKER_IDLE, command_rx.recv()).await {
                Ok(Some(payload)) => {
                    let response = session.process(&payload).await;
                    if response_tx.send(response).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    command_rx.close();
                    while let Some(payload) = command_rx.recv().await {
                        let response = session.process(&payload).await;
                        if response_tx.send(response).await.is_err() {
                            return;
                        }
                    }
                    break;
                }
            }
        }
    });
    command_tx
}

pub async fn bind(address: SocketAddr) -> Result<TcpListener> {
    Ok(TcpListener::bind(address).await?)
}

pub fn public_plaintext_rejected(address: SocketAddr, dev_insecure: bool) -> Result<()> {
    if !address.ip().is_loopback() && !dev_insecure {
        anyhow::bail!(
            "plain HTTP/WebSocket on a non-loopback address requires explicit --dev-insecure; use a trusted HTTPS reverse proxy for LAN/public access"
        );
    }
    Ok(())
}

pub async fn not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "not found")
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use agent_remote_protocol::{
        AgentMessagePhase, ClientCommand, CommandId, ConversationId, ConversationState, DeviceId,
        EffortOption, HostId, ModelOption, ProjectId, ProviderHealth, ProviderId, ProviderState,
        ServerMessage, SessionOption, SessionOptionValue, SessionSummary, TimelineItemKind, decode,
        encode,
    };
    use anyhow::Result;
    use async_trait::async_trait;
    use futures_util::{SinkExt, StreamExt};
    use tokio::sync::broadcast;
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{Message as ClientMessage, client::IntoClientRequest},
    };

    use super::*;
    use crate::{
        app::AppService,
        attachments::{AttachmentStore, DEFAULT_MAX_IMAGE_BYTES},
        providers::{
            AgentProvider, CommandAck, CreateSession, InterruptSession, NativeSession,
            ProviderCapabilities, ProviderEvent, ProviderEventKind, ProviderRegistry,
            ResolveApproval, ResumeSession, SendMessage, SetSessionOption, SteerMessage,
        },
        storage::Storage,
    };

    type TestSocket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    struct DirectHarness {
        _temp: tempfile::TempDir,
        address: SocketAddr,
        host_id: HostId,
        device_id: DeviceId,
        device_token: String,
        project_id: ProjectId,
        provider: Arc<LoopProvider>,
        task: tokio::task::JoinHandle<Result<()>>,
    }

    struct LoopProvider {
        events: broadcast::Sender<ProviderEvent>,
        projects:
            Mutex<HashMap<agent_remote_protocol::ConversationId, agent_remote_protocol::ProjectId>>,
        sends: AtomicUsize,
        interruptions: AtomicUsize,
        session_list_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
        session_list_started: tokio::sync::Notify,
        session_list_calls: AtomicUsize,
        option_calls: Mutex<Vec<(String, String)>>,
        option_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
        option_started: tokio::sync::Notify,
    }

    impl LoopProvider {
        fn new() -> Arc<Self> {
            let (events, _) = broadcast::channel(16);
            Arc::new(Self {
                events,
                projects: Mutex::new(HashMap::new()),
                sends: AtomicUsize::new(0),
                interruptions: AtomicUsize::new(0),
                session_list_gate: Mutex::new(None),
                session_list_started: tokio::sync::Notify::new(),
                session_list_calls: AtomicUsize::new(0),
                option_calls: Mutex::new(Vec::new()),
                option_gate: Mutex::new(None),
                option_started: tokio::sync::Notify::new(),
            })
        }
    }

    #[async_trait]
    impl AgentProvider for LoopProvider {
        fn id(&self) -> ProviderId {
            ProviderId::Codex
        }
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                supports_session_list: true,
                supports_resume: true,
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
        async fn list_models(&self, _project: &crate::Project) -> Result<Vec<ModelOption>> {
            Ok(vec![ModelOption {
                id: "mock-model".to_owned(),
                display_name: "Mock Model".to_owned(),
                effort_options: vec![EffortOption {
                    id: "high".to_owned(),
                    display_name: "High".to_owned(),
                }],
                default_effort: Some("high".to_owned()),
            }])
        }
        async fn list_sessions(&self, _project: &crate::Project) -> Result<Vec<SessionSummary>> {
            self.session_list_calls.fetch_add(1, Ordering::SeqCst);
            let gate = self
                .session_list_gate
                .lock()
                .expect("session list gate mutex")
                .clone();
            if let Some(gate) = gate {
                self.session_list_started.notify_one();
                gate.acquire().await.expect("session list gate").forget();
            }
            Ok(Vec::new())
        }
        async fn create_session(&self, request: CreateSession) -> Result<NativeSession> {
            self.projects
                .lock()
                .expect("projects mutex")
                .insert(request.conversation_id, request.project.id);
            Ok(NativeSession {
                native_session_id: format!("native-{}", request.conversation_id),
                title: "Direct loop".to_owned(),
                selected_model: request.model,
                selected_effort: request.effort,
                session_options: vec![
                    SessionOption {
                        id: "alpha".to_owned(),
                        display_name: "Alpha".to_owned(),
                        category: None,
                        current_value: "a0".to_owned(),
                        values: vec![
                            SessionOptionValue {
                                value: "a0".to_owned(),
                                display_name: "A0".to_owned(),
                            },
                            SessionOptionValue {
                                value: "a1".to_owned(),
                                display_name: "A1".to_owned(),
                            },
                        ],
                    },
                    SessionOption {
                        id: "beta".to_owned(),
                        display_name: "Beta".to_owned(),
                        category: None,
                        current_value: "b0".to_owned(),
                        values: vec![
                            SessionOptionValue {
                                value: "b0".to_owned(),
                                display_name: "B0".to_owned(),
                            },
                            SessionOptionValue {
                                value: "b1".to_owned(),
                                display_name: "B1".to_owned(),
                            },
                        ],
                    },
                ],
            })
        }
        async fn resume_session(&self, request: ResumeSession) -> Result<NativeSession> {
            self.projects
                .lock()
                .expect("projects mutex")
                .insert(request.conversation_id, request.project.id);
            Ok(NativeSession {
                native_session_id: request.native_session_id,
                title: "Direct loop".to_owned(),
                selected_model: request.model,
                selected_effort: request.effort,
                session_options: Vec::new(),
            })
        }
        async fn send_message(&self, request: SendMessage) -> Result<CommandAck> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            let project_id =
                self.projects.lock().expect("projects mutex")[&request.conversation_id];
            for kind in [
                ProviderEventKind::AgentTextDelta {
                    provider_item_id: "answer".to_owned(),
                    phase: AgentMessagePhase::Final,
                    delta: "mock result".to_owned(),
                },
                ProviderEventKind::Completed,
            ] {
                let _ = self.events.send(ProviderEvent {
                    provider: ProviderId::Codex,
                    project_id,
                    conversation_id: request.conversation_id,
                    kind,
                });
            }
            Ok(CommandAck)
        }
        async fn steer(&self, _request: SteerMessage) -> Result<CommandAck> {
            Ok(CommandAck)
        }
        async fn interrupt(&self, _request: InterruptSession) -> Result<CommandAck> {
            self.interruptions.fetch_add(1, Ordering::SeqCst);
            Ok(CommandAck)
        }
        async fn resolve_approval(&self, _request: ResolveApproval) -> Result<CommandAck> {
            Ok(CommandAck)
        }
        async fn set_session_option(&self, request: SetSessionOption) -> Result<CommandAck> {
            self.option_calls
                .lock()
                .expect("option calls mutex")
                .push((request.option_id.clone(), request.value));
            let gate = (request.option_id == "alpha")
                .then(|| self.option_gate.lock().expect("option gate mutex").clone())
                .flatten();
            if let Some(gate) = gate {
                self.option_started.notify_one();
                gate.acquire().await.expect("option gate").forget();
            }
            Ok(CommandAck)
        }
    }

    async fn receive_server(socket: &mut TestSocket) -> ServerMessage {
        loop {
            match socket
                .next()
                .await
                .expect("socket open")
                .expect("read socket")
            {
                ClientMessage::Binary(bytes) => {
                    return decode(&bytes).expect("decode server message");
                }
                ClientMessage::Ping(bytes) => {
                    socket.send(ClientMessage::Pong(bytes)).await.expect("pong")
                }
                other => panic!("unexpected websocket message: {other:?}"),
            }
        }
    }

    async fn send_client(socket: &mut TestSocket, command: &ClientCommand) {
        socket
            .send(ClientMessage::Binary(
                encode(command).expect("encode command").into(),
            ))
            .await
            .expect("send command");
    }

    async fn connect_direct(address: SocketAddr) -> TestSocket {
        let mut request = format!("ws://{address}/ws")
            .into_client_request()
            .expect("request");
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            WS_SUBPROTOCOL.parse().expect("protocol"),
        );
        connect_async(request).await.expect("connect").0
    }

    async fn authenticated_direct() -> (DirectHarness, TestSocket) {
        let temp = tempfile::tempdir().expect("temp dir");
        let project_root = temp.path().join("project");
        fs::create_dir(&project_root).expect("project dir");
        let storage = Arc::new(Storage::open(temp.path().join("state.db")).expect("storage"));
        let project = storage
            .add_project(&project_root, Some("Project"), &[ProviderId::Codex])
            .expect("project");
        let pairing = storage.create_pairing_token().expect("pair token");
        let host_id: HostId = storage.host_id().expect("host id");
        let provider = LoopProvider::new();
        let provider_trait: Arc<dyn AgentProvider> = provider.clone();
        let service = AppService::new(
            storage,
            AttachmentStore::new(temp.path().join("attachments"), DEFAULT_MAX_IMAGE_BYTES)
                .expect("attachments"),
            ProviderRegistry::new([provider_trait]),
            "test-host".to_owned(),
        )
        .expect("service");
        let web = temp.path().join("web");
        fs::create_dir(&web).expect("web dir");
        fs::write(web.join("index.html"), "<!doctype html>").expect("web fixture");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        let task = tokio::spawn(serve(listener, service, web));

        let mut socket = connect_direct(address).await;
        send_client(
            &mut socket,
            &ClientCommand::Pair {
                host_id,
                pair_token: pairing.token,
                device_name: "test client".to_owned(),
            },
        )
        .await;
        send_client(&mut socket, &ClientCommand::GetSnapshot).await;
        let (device_id, device_token) = match receive_server(&mut socket).await {
            ServerMessage::Paired {
                device_id,
                device_token,
                ..
            } => (device_id, device_token),
            other => panic!("expected Paired, got {other:?}"),
        };
        assert!(matches!(
            receive_server(&mut socket).await,
            ServerMessage::Snapshot { .. }
        ));
        (
            DirectHarness {
                _temp: temp,
                address,
                host_id,
                device_id,
                device_token,
                project_id: project.id,
                provider,
                task,
            },
            socket,
        )
    }

    async fn reconnect_direct(harness: &DirectHarness) -> TestSocket {
        let mut socket = connect_direct(harness.address).await;
        send_client(
            &mut socket,
            &ClientCommand::Authenticate {
                host_id: harness.host_id,
                device_id: harness.device_id,
                device_token: harness.device_token.clone(),
            },
        )
        .await;
        assert_eq!(
            receive_server(&mut socket).await,
            ServerMessage::Authenticated {
                host_id: harness.host_id,
                device_id: harness.device_id,
            }
        );
        socket
    }

    async fn create_conversation(socket: &mut TestSocket, project_id: ProjectId) -> ConversationId {
        let command_id = CommandId::new();
        send_client(
            socket,
            &ClientCommand::CreateConversation {
                command_id,
                project_id,
                provider: ProviderId::Codex,
                native_session_id: None,
                model: Some("mock-model".to_owned()),
                effort: Some("high".to_owned()),
            },
        )
        .await;
        let mut accepted = false;
        let mut conversation_id = None;
        while !accepted || conversation_id.is_none() {
            match receive_server(socket).await {
                ServerMessage::CommandAccepted {
                    command_id: accepted_id,
                } if accepted_id == command_id => accepted = true,
                ServerMessage::ConversationUpserted { conversation } => {
                    conversation_id = Some(conversation.id);
                }
                _ => {}
            }
        }
        conversation_id.expect("created conversation")
    }

    #[tokio::test]
    async fn authenticated_direct_websocket_completes_a_message_loop_once() {
        let (harness, mut socket) = authenticated_direct().await;
        let conversation_id = create_conversation(&mut socket, harness.project_id).await;

        let send_id = CommandId::new();
        let send = ClientCommand::SendMessage {
            command_id: send_id,
            attempt: 0,
            conversation_id,
            client_message_id: Some("direct-test".to_owned()),
            text: "hello".to_owned(),
            attachments: Vec::new(),
        };
        send_client(&mut socket, &send).await;
        let mut saw_final = false;
        let mut saw_completed = false;
        while !(saw_final && saw_completed) {
            match receive_server(&mut socket).await {
                ServerMessage::TimelineItemUpserted { item } => {
                    if matches!(item.kind, TimelineItemKind::AgentMessage { ref text, .. } if text == "mock result")
                    {
                        saw_final = true;
                    }
                }
                ServerMessage::ConversationUpserted { conversation }
                    if conversation.state == ConversationState::Completed =>
                {
                    saw_completed = true
                }
                _ => {}
            }
        }
        send_client(&mut socket, &send).await;
        loop {
            if matches!(receive_server(&mut socket).await, ServerMessage::CommandAccepted { command_id } if command_id == send_id)
            {
                break;
            }
        }
        assert_eq!(harness.provider.sends.load(Ordering::SeqCst), 1);
        harness.task.abort();
    }

    #[tokio::test]
    async fn slow_direct_command_does_not_block_other_commands_or_updates() {
        let (harness, mut socket) = authenticated_direct().await;
        let conversation_id = create_conversation(&mut socket, harness.project_id).await;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *harness
            .provider
            .session_list_gate
            .lock()
            .expect("session list gate mutex") = Some(Arc::clone(&gate));

        let sync_id = CommandId::new();
        send_client(
            &mut socket,
            &ClientCommand::SyncProject {
                command_id: sync_id,
                project_id: harness.project_id,
                provider: ProviderId::Codex,
            },
        )
        .await;
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            harness.provider.session_list_started.notified(),
        )
        .await
        .expect("sync command did not reach provider");

        let interrupt_id = CommandId::new();
        send_client(
            &mut socket,
            &ClientCommand::Interrupt {
                command_id: interrupt_id,
                conversation_id,
            },
        )
        .await;
        let _ = harness.provider.events.send(ProviderEvent {
            provider: ProviderId::Codex,
            project_id: harness.project_id,
            conversation_id,
            kind: ProviderEventKind::AgentTextDelta {
                provider_item_id: "during-sync".to_owned(),
                phase: AgentMessagePhase::Final,
                delta: "update during sync".to_owned(),
            },
        });

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            let mut interrupt_accepted = false;
            let mut update_received = false;
            while !interrupt_accepted || !update_received {
                match receive_server(&mut socket).await {
                    ServerMessage::CommandAccepted { command_id } if command_id == interrupt_id => {
                        interrupt_accepted = true;
                    }
                    ServerMessage::TimelineItemUpserted { item }
                        if item.conversation_id == conversation_id
                            && matches!(
                                item.kind,
                                TimelineItemKind::AgentMessage { ref text, .. }
                                    if text == "update during sync"
                            ) =>
                    {
                        update_received = true;
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("slow command blocked an independent response or update");
        assert_eq!(harness.provider.interruptions.load(Ordering::SeqCst), 1);

        gate.add_permits(1);
        loop {
            if matches!(
                receive_server(&mut socket).await,
                ServerMessage::ProjectSyncCompleted { command_id, .. } if command_id == sync_id
            ) {
                break;
            }
        }
        harness.task.abort();
    }

    #[tokio::test]
    async fn slow_project_sync_does_not_block_send_or_steer() {
        let (harness, mut socket) = authenticated_direct().await;
        let conversation_id = create_conversation(&mut socket, harness.project_id).await;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *harness
            .provider
            .session_list_gate
            .lock()
            .expect("session list gate mutex") = Some(Arc::clone(&gate));
        send_client(
            &mut socket,
            &ClientCommand::SyncProject {
                command_id: CommandId::new(),
                project_id: harness.project_id,
                provider: ProviderId::Codex,
            },
        )
        .await;
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            harness.provider.session_list_started.notified(),
        )
        .await
        .expect("sync command did not reach provider");

        let send_id = CommandId::new();
        send_client(
            &mut socket,
            &ClientCommand::SendMessage {
                command_id: send_id,
                attempt: 0,
                conversation_id,
                client_message_id: Some("send-during-sync".to_owned()),
                text: "hello".to_owned(),
                attachments: Vec::new(),
            },
        )
        .await;
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if matches!(
                    receive_server(&mut socket).await,
                    ServerMessage::CommandAccepted { command_id } if command_id == send_id
                ) {
                    break;
                }
            }
        })
        .await
        .expect("slow project sync blocked send");

        let steer_id = CommandId::new();
        send_client(
            &mut socket,
            &ClientCommand::Steer {
                command_id: steer_id,
                conversation_id,
                text: "continue".to_owned(),
            },
        )
        .await;
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if matches!(
                    receive_server(&mut socket).await,
                    ServerMessage::CommandAccepted { command_id } if command_id == steer_id
                ) {
                    break;
                }
            }
        })
        .await
        .expect("slow project sync blocked steer");
        assert_eq!(harness.provider.sends.load(Ordering::SeqCst), 1);

        gate.add_permits(1);
        harness.task.abort();
    }

    #[tokio::test]
    async fn authenticated_sessions_apply_conversation_mutations_in_arrival_order() {
        let (harness, mut first_socket) = authenticated_direct().await;
        let conversation_id = create_conversation(&mut first_socket, harness.project_id).await;
        let mut second_socket = reconnect_direct(&harness).await;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *harness
            .provider
            .option_gate
            .lock()
            .expect("option gate mutex") = Some(Arc::clone(&gate));
        let alpha_id = CommandId::new();
        let beta_id = CommandId::new();

        send_client(
            &mut first_socket,
            &ClientCommand::SetSessionOption {
                command_id: alpha_id,
                conversation_id,
                option_id: "alpha".to_owned(),
                value: "a1".to_owned(),
            },
        )
        .await;
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            harness.provider.option_started.notified(),
        )
        .await
        .expect("first option did not reach provider");
        send_client(
            &mut second_socket,
            &ClientCommand::SetSessionOption {
                command_id: beta_id,
                conversation_id,
                option_id: "beta".to_owned(),
                value: "b1".to_owned(),
            },
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        assert_eq!(
            harness
                .provider
                .option_calls
                .lock()
                .expect("option calls mutex")
                .as_slice(),
            &[("alpha".to_owned(), "a1".to_owned())]
        );
        let interrupt_id = CommandId::new();
        send_client(
            &mut second_socket,
            &ClientCommand::Interrupt {
                command_id: interrupt_id,
                conversation_id,
            },
        )
        .await;
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if matches!(
                    receive_server(&mut second_socket).await,
                    ServerMessage::CommandAccepted { command_id } if command_id == interrupt_id
                ) {
                    break;
                }
            }
        })
        .await
        .expect("interrupt was blocked by a conversation mutation");
        assert_eq!(harness.provider.interruptions.load(Ordering::SeqCst), 1);

        gate.add_permits(1);
        loop {
            if matches!(
                receive_server(&mut first_socket).await,
                ServerMessage::CommandAccepted { command_id } if command_id == alpha_id
            ) {
                break;
            }
        }
        loop {
            if matches!(
                receive_server(&mut second_socket).await,
                ServerMessage::CommandAccepted { command_id } if command_id == beta_id
            ) {
                break;
            }
        }
        assert_eq!(
            harness
                .provider
                .option_calls
                .lock()
                .expect("option calls mutex")
                .as_slice(),
            &[
                ("alpha".to_owned(), "a1".to_owned()),
                ("beta".to_owned(), "b1".to_owned()),
            ]
        );

        send_client(&mut first_socket, &ClientCommand::GetSnapshot).await;
        let conversation = loop {
            if let ServerMessage::Snapshot { snapshot } = receive_server(&mut first_socket).await {
                break snapshot
                    .conversations
                    .into_iter()
                    .find(|conversation| conversation.id == conversation_id)
                    .expect("conversation in snapshot");
            }
        };
        assert!(
            conversation
                .session_options
                .iter()
                .any(|option| { option.id == "alpha" && option.current_value == "a1" })
        );
        assert!(
            conversation
                .session_options
                .iter()
                .any(|option| { option.id == "beta" && option.current_value == "b1" })
        );
        harness.task.abort();
    }

    #[tokio::test]
    async fn direct_disconnect_keeps_command_running_for_exact_replay() {
        let (harness, mut socket) = authenticated_direct().await;
        let baseline_calls = harness.provider.session_list_calls.load(Ordering::SeqCst);
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *harness
            .provider
            .session_list_gate
            .lock()
            .expect("session list gate mutex") = Some(Arc::clone(&gate));
        let command_id = CommandId::new();
        let command = ClientCommand::SyncProject {
            command_id,
            project_id: harness.project_id,
            provider: ProviderId::Codex,
        };

        send_client(&mut socket, &command).await;
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            harness.provider.session_list_started.notified(),
        )
        .await
        .expect("sync command did not reach provider");
        socket.close(None).await.expect("close direct client");
        drop(socket);
        gate.add_permits(1);

        let mut socket = reconnect_direct(&harness).await;
        send_client(&mut socket, &command).await;
        let replay = loop {
            match receive_server(&mut socket).await {
                response @ ServerMessage::ProjectSyncCompleted {
                    command_id: replayed_id,
                    ..
                } if replayed_id == command_id => break response,
                ServerMessage::CommandRejected {
                    command_id: Some(rejected_id),
                    code,
                    message,
                } if rejected_id == command_id => {
                    panic!("command was not completed after disconnect: {code}: {message}")
                }
                _ => {}
            }
        };
        assert!(matches!(
            replay,
            ServerMessage::ProjectSyncCompleted {
                command_id: replayed_id,
                ..
            } if replayed_id == command_id
        ));
        assert_eq!(
            harness.provider.session_list_calls.load(Ordering::SeqCst),
            baseline_calls + 1
        );
        harness.task.abort();
    }
}
