use std::{
    collections::HashMap,
    fs,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use agent_remote_host::{
    Project, Storage,
    app::AppService,
    attachments::{AttachmentStore, DEFAULT_MAX_IMAGE_BYTES},
    providers::{
        AgentProvider, CommandAck, CreateSession, InterruptSession, NativeSession,
        ProviderCapabilities, ProviderEvent, ProviderEventKind, ProviderRegistry, ResolveApproval,
        ResumeSession, SendMessage, SetSessionOption, SteerMessage,
    },
    transport::{
        direct::WS_SUBPROTOCOL,
        relay::{RelayClientConfig, run_once, run_reconnecting},
    },
};
use agent_remote_protocol::{
    AgentMessagePhase, ClientCommand, CommandId, ConversationId, ConversationState, EffortOption,
    HostId, ModelOption, ProjectId, ProviderHealth, ProviderId, ProviderState, ServerMessage,
    SessionSummary, TimelineItemKind, decode, encode,
};
use agent_remote_relay::{DEFAULT_CHANNEL_CAPACITY, RelayState, router as relay_router};
use anyhow::Result;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use tokio::{net::TcpListener, sync::broadcast};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

type ClientSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct LoopProvider {
    events: broadcast::Sender<ProviderEvent>,
    projects: Mutex<HashMap<ConversationId, ProjectId>>,
    sends: AtomicUsize,
    interruptions: AtomicUsize,
    session_list_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    session_list_started: tokio::sync::Notify,
    session_list_calls: AtomicUsize,
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

    async fn list_models(&self, _project: &Project) -> Result<Vec<ModelOption>> {
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

    async fn list_sessions(&self, _project: &Project) -> Result<Vec<SessionSummary>> {
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
            title: "Relay loop".to_owned(),
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
            title: "Relay loop".to_owned(),
            selected_model: request.model,
            selected_effort: request.effort,
            session_options: Vec::new(),
        })
    }

    async fn send_message(&self, request: SendMessage) -> Result<CommandAck> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        let project_id = self.projects.lock().expect("projects mutex")[&request.conversation_id];
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

    async fn set_session_option(&self, _request: SetSessionOption) -> Result<CommandAck> {
        Ok(CommandAck)
    }
}

async fn connect_client(address: SocketAddr, host_id: HostId) -> ClientSocket {
    let mut request = format!("ws://{address}/client/{host_id}")
        .into_client_request()
        .expect("client websocket request");
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        WS_SUBPROTOCOL.parse().expect("application subprotocol"),
    );
    let (socket, response) = connect_async(request).await.expect("connect relay client");
    assert_eq!(
        response
            .headers()
            .get("Sec-WebSocket-Protocol")
            .and_then(|value| value.to_str().ok()),
        Some(WS_SUBPROTOCOL)
    );
    socket
}

async fn receive_server(socket: &mut ClientSocket) -> ServerMessage {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket
                .next()
                .await
                .expect("client websocket open")
                .expect("read client websocket")
            {
                Message::Binary(bytes) => return decode(&bytes).expect("decode server message"),
                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.expect("pong"),
                other => panic!("unexpected client websocket message: {other:?}"),
            }
        }
    })
    .await
    .expect("server message timeout")
}

async fn connect_online(address: SocketAddr, host_id: HostId) -> ClientSocket {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let mut socket = connect_client(address, host_id).await;
            match receive_server(&mut socket).await {
                ServerMessage::HostStatus {
                    host_id: status_host,
                    online: true,
                    message: None,
                } => {
                    assert_eq!(status_host, host_id);
                    return socket;
                }
                ServerMessage::HostStatus { online: false, .. } => {
                    let _ = socket.close(None).await;
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                other => panic!("expected HostStatus, got {other:?}"),
            }
        }
    })
    .await
    .expect("Host did not register with relay")
}

async fn send_client(socket: &mut ClientSocket, command: &ClientCommand) {
    socket
        .send(Message::Binary(
            encode(command).expect("encode client command").into(),
        ))
        .await
        .expect("send client command");
}

#[tokio::test]
async fn relay_websocket_runs_the_authenticated_application_flow_once() {
    const RELAY_TOKEN: &str = "relay-test-token";

    let temp = tempfile::tempdir().expect("temp dir");
    let project_root = temp.path().join("project");
    let web_root = temp.path().join("web");
    fs::create_dir(&project_root).expect("project dir");
    fs::create_dir(&web_root).expect("web dir");
    fs::write(web_root.join("index.html"), "<!doctype html>").expect("web fixture");

    let storage = Arc::new(Storage::open(temp.path().join("state.db")).expect("storage"));
    let project = storage
        .add_project(&project_root, Some("Project"), &[ProviderId::Codex])
        .expect("project");
    let pairing = storage.create_pairing_token().expect("pair token");
    let host_id = storage.host_id().expect("host id");
    let provider = LoopProvider::new();
    let provider_trait: Arc<dyn AgentProvider> = provider.clone();
    let service = AppService::new(
        storage,
        AttachmentStore::new(temp.path().join("attachments"), DEFAULT_MAX_IMAGE_BYTES)
            .expect("attachments"),
        ProviderRegistry::new([provider_trait]),
        "relay-test-host".to_owned(),
    )
    .expect("service");
    service.start_provider_event_pumps();

    let relay_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("relay listener");
    let relay_address = relay_listener.local_addr().expect("relay address");
    let relay = relay_router(
        RelayState::new(RELAY_TOKEN, DEFAULT_CHANNEL_CAPACITY),
        web_root,
    );
    let relay_task = tokio::spawn(async move {
        axum::serve(relay_listener, relay)
            .await
            .expect("serve relay");
    });
    let host_service = Arc::clone(&service);
    let host_config = RelayClientConfig {
        url: format!("ws://{relay_address}/host"),
        access_token: RELAY_TOKEN.to_owned(),
        dev_insecure: true,
    };
    let running_host_config = host_config.clone();
    let host_task = tokio::spawn(async move { run_once(host_service, &running_host_config).await });

    let mut pairing_client = connect_online(relay_address, host_id).await;
    send_client(
        &mut pairing_client,
        &ClientCommand::Pair {
            host_id,
            pair_token: pairing.token,
            device_name: "relay test client".to_owned(),
        },
    )
    .await;
    let (device_id, device_token) = match receive_server(&mut pairing_client).await {
        ServerMessage::Paired {
            host_id: paired_host,
            device_id,
            device_token,
        } => {
            assert_eq!(paired_host, host_id);
            (device_id, device_token)
        }
        other => panic!("expected Paired, got {other:?}"),
    };
    pairing_client
        .close(None)
        .await
        .expect("close paired client");

    let mut client = connect_online(relay_address, host_id).await;
    send_client(
        &mut client,
        &ClientCommand::Authenticate {
            host_id,
            device_id,
            device_token: device_token.clone(),
        },
    )
    .await;
    assert_eq!(
        receive_server(&mut client).await,
        ServerMessage::Authenticated { host_id, device_id }
    );

    send_client(
        &mut client,
        &ClientCommand::GetSnapshot {
            metadata_only: false,
        },
    )
    .await;
    let snapshot = match receive_server(&mut client).await {
        ServerMessage::Snapshot { snapshot } => snapshot,
        other => panic!("expected Snapshot, got {other:?}"),
    };
    assert_eq!(snapshot.host_id, host_id);
    assert!(snapshot.projects.iter().any(|item| item.id == project.id));
    assert!(snapshot.provider_capabilities.iter().any(|capability| {
        capability.provider == ProviderId::Codex
            && capability.project_id == project.id
            && capability.health.state == ProviderState::Starting
    }));
    send_client(
        &mut client,
        &ClientCommand::RefreshProjects {
            provider: ProviderId::Codex,
        },
    )
    .await;
    let refreshed = loop {
        if let ServerMessage::ProjectsUpdated {
            provider: ProviderId::Codex,
            capabilities,
            ..
        } = receive_server(&mut client).await
        {
            break capabilities;
        }
    };
    assert!(refreshed.iter().any(|capability| {
        capability.project_id == project.id && capability.health.state == ProviderState::Ready
    }));

    let create_id = CommandId::new();
    send_client(
        &mut client,
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
    let mut create_accepted = false;
    let mut conversation_id = None;
    while !create_accepted || conversation_id.is_none() {
        match receive_server(&mut client).await {
            ServerMessage::CommandAccepted { command_id } if command_id == create_id => {
                create_accepted = true;
            }
            ServerMessage::ConversationUpserted { conversation }
                if conversation.project_id == project.id =>
            {
                conversation_id = Some(conversation.id);
            }
            _ => {}
        }
    }
    let conversation_id = conversation_id.expect("created conversation");

    let send_id = CommandId::new();
    let send = ClientCommand::SendMessage {
        command_id: send_id,
        attempt: 0,
        conversation_id,
        client_message_id: Some("relay-e2e".to_owned()),
        text: "hello through relay".to_owned(),
        attachments: Vec::new(),
    };
    send_client(&mut client, &send).await;
    let mut send_accepted = false;
    let mut saw_final = false;
    let mut saw_completed = false;
    while !(send_accepted && saw_final && saw_completed) {
        match receive_server(&mut client).await {
            ServerMessage::CommandAccepted { command_id } if command_id == send_id => {
                send_accepted = true;
            }
            ServerMessage::TimelineItemUpserted { item }
                if item.conversation_id == conversation_id
                    && matches!(
                        item.kind,
                        TimelineItemKind::AgentMessage { ref text, .. }
                            if text == "mock result"
                    ) =>
            {
                saw_final = true;
            }
            ServerMessage::ConversationUpserted { conversation }
                if conversation.id == conversation_id
                    && conversation.state == ConversationState::Completed =>
            {
                saw_completed = true;
            }
            _ => {}
        }
    }

    send_client(&mut client, &send).await;
    loop {
        if matches!(
            receive_server(&mut client).await,
            ServerMessage::CommandAccepted { command_id } if command_id == send_id
        ) {
            break;
        }
    }
    assert_eq!(provider.sends.load(Ordering::SeqCst), 1);

    let mut observer = connect_online(relay_address, host_id).await;
    send_client(
        &mut observer,
        &ClientCommand::Authenticate {
            host_id,
            device_id,
            device_token: device_token.clone(),
        },
    )
    .await;
    assert_eq!(
        receive_server(&mut observer).await,
        ServerMessage::Authenticated { host_id, device_id }
    );

    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *provider
        .session_list_gate
        .lock()
        .expect("session list gate mutex") = Some(Arc::clone(&gate));
    let sync_id = CommandId::new();
    send_client(
        &mut client,
        &ClientCommand::SyncProject {
            command_id: sync_id,
            project_id: project.id,
            provider: ProviderId::Codex,
        },
    )
    .await;
    tokio::time::timeout(
        Duration::from_secs(1),
        provider.session_list_started.notified(),
    )
    .await
    .expect("sync command did not reach provider");

    let interrupt_id = CommandId::new();
    send_client(
        &mut client,
        &ClientCommand::Interrupt {
            command_id: interrupt_id,
            conversation_id,
        },
    )
    .await;
    let _ = provider.events.send(ProviderEvent {
        provider: ProviderId::Codex,
        project_id: project.id,
        conversation_id,
        kind: ProviderEventKind::AgentTextDelta {
            provider_item_id: "relay-during-sync".to_owned(),
            phase: AgentMessagePhase::Final,
            delta: "relay update during sync".to_owned(),
        },
    });

    tokio::time::timeout(Duration::from_secs(1), async {
        let mut interrupt_accepted = false;
        let mut update_received = false;
        while !interrupt_accepted || !update_received {
            match receive_server(&mut client).await {
                ServerMessage::CommandAccepted { command_id } if command_id == interrupt_id => {
                    interrupt_accepted = true;
                }
                ServerMessage::TimelineItemUpserted { item }
                    if item.conversation_id == conversation_id
                        && matches!(
                            item.kind,
                            TimelineItemKind::AgentMessage { ref text, .. }
                                if text == "relay update during sync"
                        ) =>
                {
                    update_received = true;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("slow relay command blocked an independent response or update");
    assert_eq!(provider.interruptions.load(Ordering::SeqCst), 1);

    loop {
        match receive_server(&mut observer).await {
            ServerMessage::TimelineItemUpserted { item }
                if item.conversation_id == conversation_id
                    && matches!(
                        item.kind,
                        TimelineItemKind::AgentMessage { ref text, .. }
                            if text == "relay update during sync"
                    ) =>
            {
                break;
            }
            ServerMessage::CommandAccepted { command_id } if command_id == interrupt_id => {
                panic!("command response was routed to the wrong logical client")
            }
            _ => {}
        }
    }

    gate.add_permits(1);
    loop {
        if matches!(
            receive_server(&mut client).await,
            ServerMessage::ProjectSyncCompleted { command_id, .. } if command_id == sync_id
        ) {
            break;
        }
    }

    let baseline_calls = provider.session_list_calls.load(Ordering::SeqCst);
    let disconnect_gate = Arc::new(tokio::sync::Semaphore::new(0));
    *provider
        .session_list_gate
        .lock()
        .expect("session list gate mutex") = Some(Arc::clone(&disconnect_gate));
    let reconnect_id = CommandId::new();
    let reconnect_command = ClientCommand::SyncProject {
        command_id: reconnect_id,
        project_id: project.id,
        provider: ProviderId::Codex,
    };
    send_client(&mut client, &reconnect_command).await;
    tokio::time::timeout(
        Duration::from_secs(1),
        provider.session_list_started.notified(),
    )
    .await
    .expect("disconnect sync did not reach provider");

    host_task.abort();
    let _ = host_task.await;
    drop(client);
    drop(observer);
    disconnect_gate.add_permits(1);

    let restarted_service = Arc::clone(&service);
    let restarted_host_task = tokio::spawn(run_reconnecting(restarted_service, host_config));
    let mut reconnected = connect_online(relay_address, host_id).await;
    send_client(
        &mut reconnected,
        &ClientCommand::Authenticate {
            host_id,
            device_id,
            device_token,
        },
    )
    .await;
    assert_eq!(
        receive_server(&mut reconnected).await,
        ServerMessage::Authenticated { host_id, device_id }
    );
    send_client(&mut reconnected, &reconnect_command).await;
    match receive_server(&mut reconnected).await {
        ServerMessage::ProjectSyncCompleted { command_id, .. } => {
            assert_eq!(command_id, reconnect_id)
        }
        other => panic!("expected exact completed replay after relay reconnect, got {other:?}"),
    }
    assert_eq!(
        provider.session_list_calls.load(Ordering::SeqCst),
        baseline_calls + 1
    );

    restarted_host_task.abort();
    relay_task.abort();
}
