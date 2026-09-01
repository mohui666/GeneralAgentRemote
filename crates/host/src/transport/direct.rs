use std::{net::SocketAddr, path::PathBuf, sync::Arc};

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
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};

use crate::{
    app::AppService,
    transport::session::{ApplicationSession, AuthRateLimiter},
};

pub const WS_SUBPROTOCOL: &str = "agent-remote.cbor.v1";
const MAX_WS_MESSAGE_BYTES: usize = 12 * 1024 * 1024;

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
    loop {
        tokio::select! {
            incoming = receiver.next() => {
                let Some(incoming) = incoming else { break };
                match incoming {
                    Ok(Message::Binary(bytes)) => {
                        let response = session.process(&bytes).await;
                        if sender.send(Message::Binary(response.into())).await.is_err() {
                            break;
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
        AgentMessagePhase, ClientCommand, CommandId, ConversationState, EffortOption, HostId,
        ModelOption, ProviderHealth, ProviderId, ProviderState, ServerMessage, SessionSummary,
        TimelineItemKind, decode, encode,
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

    struct LoopProvider {
        events: broadcast::Sender<ProviderEvent>,
        projects:
            Mutex<HashMap<agent_remote_protocol::ConversationId, agent_remote_protocol::ProjectId>>,
        sends: AtomicUsize,
    }

    impl LoopProvider {
        fn new() -> Arc<Self> {
            let (events, _) = broadcast::channel(16);
            Arc::new(Self {
                events,
                projects: Mutex::new(HashMap::new()),
                sends: AtomicUsize::new(0),
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
            Ok(CommandAck)
        }
        async fn resolve_approval(&self, _request: ResolveApproval) -> Result<CommandAck> {
            Ok(CommandAck)
        }
        async fn set_session_option(&self, _request: SetSessionOption) -> Result<CommandAck> {
            Ok(CommandAck)
        }
    }

    async fn receive_server(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> ServerMessage {
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

    async fn send_client(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        command: &ClientCommand,
    ) {
        socket
            .send(ClientMessage::Binary(
                encode(command).expect("encode command").into(),
            ))
            .await
            .expect("send command");
    }

    #[tokio::test]
    async fn authenticated_direct_websocket_completes_a_message_loop_once() {
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

        let mut request = format!("ws://{address}/ws")
            .into_client_request()
            .expect("request");
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            WS_SUBPROTOCOL.parse().expect("protocol"),
        );
        let (mut socket, _) = connect_async(request).await.expect("connect");
        send_client(
            &mut socket,
            &ClientCommand::Pair {
                host_id,
                pair_token: pairing.token,
                device_name: "test client".to_owned(),
            },
        )
        .await;
        assert!(matches!(
            receive_server(&mut socket).await,
            ServerMessage::Paired { .. }
        ));

        let create_id = CommandId::new();
        send_client(
            &mut socket,
            &ClientCommand::CreateConversation {
                command_id: create_id,
                project_id: project.id,
                provider: ProviderId::Codex,
                native_session_id: None,
                model: Some("mock-model".to_owned()),
                effort: Some("high".to_owned()),
            },
        )
        .await;
        assert_eq!(
            receive_server(&mut socket).await,
            ServerMessage::CommandAccepted {
                command_id: create_id
            }
        );
        let conversation_id = loop {
            if let ServerMessage::ConversationUpserted { conversation } =
                receive_server(&mut socket).await
            {
                break conversation.id;
            }
        };

        let send_id = CommandId::new();
        let send = ClientCommand::SendMessage {
            command_id: send_id,
            conversation_id,
            text: "hello".to_owned(),
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
        assert_eq!(provider.sends.load(Ordering::SeqCst), 1);
        task.abort();
    }
}
