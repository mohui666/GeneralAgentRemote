use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use agent_remote_protocol::{
    HostId, RelayFrame, ServerMessage, decode_relay, encode, encode_relay,
};
use axum::{
    Router,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{any, get},
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{RwLock, mpsc};
use tower_http::services::{ServeDir, ServeFile};
use tracing::{debug, info, warn};
use uuid::Uuid;

pub const DEFAULT_CHANNEL_CAPACITY: usize = 64;
const MAX_RELAY_MESSAGE_BYTES: usize = 12 * 1024 * 1024;
const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(10);
const APP_SUBPROTOCOL: &str = "agent-remote.cbor.v4";
const RELAY_SUBPROTOCOL: &str = "agent-remote-relay.cbor.v1";

#[derive(Clone)]
pub struct RelayState {
    inner: Arc<RwLock<Registry>>,
    access_token: Arc<str>,
    channel_capacity: usize,
}

#[derive(Default)]
struct Registry {
    hosts: HashMap<HostId, HostEntry>,
}

struct HostEntry {
    generation: Uuid,
    outbound: mpsc::Sender<RelayFrame>,
    clients: HashMap<Uuid, mpsc::Sender<ClientOutbound>>,
}

#[derive(Debug)]
enum ClientOutbound {
    Payload(Vec<u8>),
    HostOffline(String),
    Close(String),
}

#[derive(Clone, Copy)]
struct ClientLease {
    host_id: HostId,
    generation: Uuid,
    connection_id: Uuid,
}

#[derive(Debug)]
enum OpenClientError {
    Offline,
    HostBusy,
}

enum ClientRoute {
    Delivered,
    Missing,
    Slow,
}

impl RelayState {
    pub fn new(access_token: impl Into<String>, channel_capacity: usize) -> Self {
        assert!(
            channel_capacity > 0,
            "relay channel capacity must be non-zero"
        );
        Self {
            inner: Arc::new(RwLock::new(Registry::default())),
            access_token: Arc::from(access_token.into()),
            channel_capacity,
        }
    }

    async fn register_host(
        &self,
        host_id: HostId,
        outbound: mpsc::Sender<RelayFrame>,
    ) -> Option<Uuid> {
        let mut registry = self.inner.write().await;
        if registry.hosts.contains_key(&host_id) {
            return None;
        }
        let generation = Uuid::new_v4();
        registry.hosts.insert(
            host_id,
            HostEntry {
                generation,
                outbound,
                clients: HashMap::new(),
            },
        );
        Some(generation)
    }

    async fn unregister_host(
        &self,
        host_id: HostId,
        generation: Uuid,
    ) -> Vec<mpsc::Sender<ClientOutbound>> {
        let mut registry = self.inner.write().await;
        let is_current = registry
            .hosts
            .get(&host_id)
            .is_some_and(|host| host.generation == generation);
        if !is_current {
            return Vec::new();
        }
        registry
            .hosts
            .remove(&host_id)
            .map(|host| host.clients.into_values().collect())
            .unwrap_or_default()
    }

    async fn open_client(
        &self,
        host_id: HostId,
        client_outbound: mpsc::Sender<ClientOutbound>,
    ) -> Result<(ClientLease, mpsc::Sender<RelayFrame>), OpenClientError> {
        let connection_id = Uuid::new_v4();
        let (lease, host_outbound) = {
            let mut registry = self.inner.write().await;
            let host = registry
                .hosts
                .get_mut(&host_id)
                .ok_or(OpenClientError::Offline)?;
            let lease = ClientLease {
                host_id,
                generation: host.generation,
                connection_id,
            };
            host.clients.insert(connection_id, client_outbound);
            (lease, host.outbound.clone())
        };

        let open = RelayFrame::OpenClient {
            host_id,
            connection_id,
        };
        if let Err(error) = host_outbound.try_send(open) {
            self.remove_client(lease).await;
            return Err(match error {
                mpsc::error::TrySendError::Full(_) => OpenClientError::HostBusy,
                mpsc::error::TrySendError::Closed(_) => OpenClientError::Offline,
            });
        }
        Ok((lease, host_outbound))
    }

    async fn remove_client(&self, lease: ClientLease) {
        let mut registry = self.inner.write().await;
        if let Some(host) = registry.hosts.get_mut(&lease.host_id)
            && host.generation == lease.generation
        {
            host.clients.remove(&lease.connection_id);
        }
    }

    async fn route_to_client(
        &self,
        host_id: HostId,
        generation: Uuid,
        connection_id: Uuid,
        outbound: ClientOutbound,
    ) -> ClientRoute {
        let sender = {
            let registry = self.inner.read().await;
            let Some(host) = registry.hosts.get(&host_id) else {
                return ClientRoute::Missing;
            };
            if host.generation != generation {
                return ClientRoute::Missing;
            }
            let Some(sender) = host.clients.get(&connection_id) else {
                return ClientRoute::Missing;
            };
            sender.clone()
        };

        match sender.try_send(outbound) {
            Ok(()) => ClientRoute::Delivered,
            Err(error) => {
                self.remove_client(ClientLease {
                    host_id,
                    generation,
                    connection_id,
                })
                .await;
                match error {
                    mpsc::error::TrySendError::Full(_) => ClientRoute::Slow,
                    mpsc::error::TrySendError::Closed(_) => ClientRoute::Missing,
                }
            }
        }
    }
}

pub fn router(state: RelayState, web_dir: PathBuf) -> Router {
    let index = web_dir.join("index.html");
    let static_files = ServeDir::new(web_dir).fallback(ServeFile::new(index));
    Router::new()
        .route("/health", get(health))
        .route("/host", any(host_upgrade))
        .route("/client/{host_id}", any(client_upgrade))
        .fallback_service(static_files)
        .with_state(state)
}

async fn health() -> &'static str {
    "ok\n"
}

async fn host_upgrade(State(state): State<RelayState>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(MAX_RELAY_MESSAGE_BYTES)
        .max_frame_size(MAX_RELAY_MESSAGE_BYTES)
        .protocols([RELAY_SUBPROTOCOL])
        .on_upgrade(move |socket| host_socket(state, socket))
}

async fn client_upgrade(
    State(state): State<RelayState>,
    Path(raw_host_id): Path<String>,
    ws: WebSocketUpgrade,
) -> Response {
    let host_id = match Uuid::parse_str(&raw_host_id) {
        Ok(id) => HostId(id),
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid host id").into_response(),
    };
    ws.max_message_size(MAX_RELAY_MESSAGE_BYTES)
        .max_frame_size(MAX_RELAY_MESSAGE_BYTES)
        .protocols([APP_SUBPROTOCOL])
        .on_upgrade(move |socket| client_socket(state, host_id, socket))
}

async fn host_socket(state: RelayState, mut socket: WebSocket) {
    let registration = match tokio::time::timeout(REGISTRATION_TIMEOUT, socket.recv()).await {
        Ok(Some(Ok(Message::Binary(bytes)))) => decode_relay(&bytes),
        Ok(Some(Ok(_))) => {
            send_relay_error(
                &mut socket,
                "binary_required",
                "registration must be a binary CBOR frame",
            )
            .await;
            return;
        }
        Ok(Some(Err(error))) => {
            debug!(%error, "host websocket failed during registration");
            return;
        }
        Ok(None) => return,
        Err(_) => {
            send_relay_error(
                &mut socket,
                "registration_timeout",
                "host registration timed out",
            )
            .await;
            return;
        }
    };

    let (host_id, access_token) = match registration {
        Ok(RelayFrame::RegisterHost {
            host_id,
            access_token,
        }) => (host_id, access_token),
        Ok(_) => {
            send_relay_error(
                &mut socket,
                "registration_required",
                "first frame must register the host",
            )
            .await;
            return;
        }
        Err(error) => {
            send_relay_error(&mut socket, "invalid_frame", &error.to_string()).await;
            return;
        }
    };

    if access_token != state.access_token.as_ref() {
        send_relay_error(
            &mut socket,
            "invalid_access_token",
            "relay access token was rejected",
        )
        .await;
        return;
    }

    let (outbound_tx, mut outbound_rx) = mpsc::channel(state.channel_capacity);
    let Some(generation) = state.register_host(host_id, outbound_tx).await else {
        send_relay_error(
            &mut socket,
            "host_already_online",
            "this host id is already connected",
        )
        .await;
        return;
    };

    if send_relay(&mut socket, RelayFrame::HostRegistered { host_id })
        .await
        .is_err()
    {
        state.unregister_host(host_id, generation).await;
        return;
    }

    info!(%host_id, "host connected");
    let (mut sink, mut stream) = socket.split();
    loop {
        tokio::select! {
            inbound = stream.next() => {
                let Some(inbound) = inbound else { break };
                match inbound {
                    Ok(Message::Binary(bytes)) => {
                        let frame = match decode_relay(&bytes) {
                            Ok(frame) => frame,
                            Err(error) => {
                                let _ = send_relay_sink(&mut sink, RelayFrame::Error {
                                    code: "invalid_frame".to_owned(),
                                    message: error.to_string(),
                                }).await;
                                break;
                            }
                        };
                        if let Some(reply) = handle_host_frame(&state, host_id, generation, frame).await
                            && send_relay_sink(&mut sink, reply).await.is_err()
                        {
                            break;
                        }
                    }
                    Ok(Message::Ping(bytes)) => {
                        if sink.send(Message::Pong(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Ok(Message::Pong(_)) => {}
                    Ok(Message::Close(_)) => break,
                    Ok(Message::Text(_)) => {
                        let _ = send_relay_sink(&mut sink, RelayFrame::Error {
                            code: "binary_required".to_owned(),
                            message: "relay frames must use binary CBOR".to_owned(),
                        }).await;
                        break;
                    }
                    Err(error) => {
                        debug!(%host_id, %error, "host websocket read failed");
                        break;
                    }
                }
            }
            outbound = outbound_rx.recv() => {
                let Some(frame) = outbound else { break };
                if send_relay_sink(&mut sink, frame).await.is_err() {
                    break;
                }
            }
        }
    }

    let clients = state.unregister_host(host_id, generation).await;
    for client in clients {
        let _ = client.try_send(ClientOutbound::HostOffline(
            "Host disconnected from relay".to_owned(),
        ));
    }
    info!(%host_id, "host disconnected");
}

async fn handle_host_frame(
    state: &RelayState,
    host_id: HostId,
    generation: Uuid,
    frame: RelayFrame,
) -> Option<RelayFrame> {
    match frame {
        RelayFrame::ClientOpened { .. } => None,
        RelayFrame::Payload {
            connection_id,
            payload,
        } => match state
            .route_to_client(
                host_id,
                generation,
                connection_id,
                ClientOutbound::Payload(payload),
            )
            .await
        {
            ClientRoute::Delivered => None,
            ClientRoute::Missing => Some(RelayFrame::Close {
                connection_id,
                reason: "client is no longer connected".to_owned(),
            }),
            ClientRoute::Slow => Some(RelayFrame::Close {
                connection_id,
                reason: "slow client disconnected; reconnect and request a snapshot".to_owned(),
            }),
        },
        RelayFrame::Close {
            connection_id,
            reason,
        } => {
            let _ = state
                .route_to_client(
                    host_id,
                    generation,
                    connection_id,
                    ClientOutbound::Close(reason),
                )
                .await;
            None
        }
        RelayFrame::RegisterHost { .. }
        | RelayFrame::HostRegistered { .. }
        | RelayFrame::OpenClient { .. }
        | RelayFrame::HostAvailability { .. }
        | RelayFrame::Error { .. } => Some(RelayFrame::Error {
            code: "unexpected_host_frame".to_owned(),
            message: "frame is not valid from a registered host".to_owned(),
        }),
    }
}

async fn client_socket(state: RelayState, host_id: HostId, socket: WebSocket) {
    let (client_tx, mut client_rx) = mpsc::channel(state.channel_capacity);
    let (lease, host_tx) = match state.open_client(host_id, client_tx).await {
        Ok(opened) => opened,
        Err(OpenClientError::Offline) => {
            send_offline_and_close(socket, host_id, "Host is offline").await;
            return;
        }
        Err(OpenClientError::HostBusy) => {
            send_offline_and_close(
                socket,
                host_id,
                "Host relay connection is busy; reconnect and request a snapshot",
            )
            .await;
            return;
        }
    };

    let (mut sink, mut stream) = socket.split();
    if send_server_sink(
        &mut sink,
        ServerMessage::HostStatus {
            host_id,
            online: true,
            message: None,
        },
    )
    .await
    .is_err()
    {
        state.remove_client(lease).await;
        return;
    }

    loop {
        tokio::select! {
            inbound = stream.next() => {
                let Some(inbound) = inbound else { break };
                match inbound {
                    Ok(Message::Binary(payload)) => {
                        let frame = RelayFrame::Payload {
                            connection_id: lease.connection_id,
                            payload: payload.to_vec(),
                        };
                        if host_tx.try_send(frame).is_err() {
                            let _ = send_server_sink(&mut sink, ServerMessage::HostStatus {
                                host_id,
                                online: false,
                                message: Some("Host relay connection is busy; reconnect and request a snapshot".to_owned()),
                            }).await;
                            break;
                        }
                    }
                    Ok(Message::Ping(bytes)) => {
                        if sink.send(Message::Pong(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Ok(Message::Pong(_)) => {}
                    Ok(Message::Close(_)) => break,
                    Ok(Message::Text(_)) => {
                        let _ = send_server_sink(&mut sink, ServerMessage::ProtocolError {
                            supported_version: agent_remote_protocol::PROTOCOL_VERSION,
                            message: "application messages must use binary CBOR".to_owned(),
                        }).await;
                        break;
                    }
                    Err(error) => {
                        debug!(%host_id, connection_id = %lease.connection_id, %error, "client websocket read failed");
                        break;
                    }
                }
            }
            outbound = client_rx.recv() => {
                match outbound {
                    Some(ClientOutbound::Payload(payload)) => {
                        if sink.send(Message::Binary(payload.into())).await.is_err() {
                            break;
                        }
                    }
                    Some(ClientOutbound::HostOffline(message)) => {
                        let _ = send_server_sink(&mut sink, ServerMessage::HostStatus {
                            host_id,
                            online: false,
                            message: Some(message),
                        }).await;
                        break;
                    }
                    Some(ClientOutbound::Close(reason)) => {
                        debug!(%host_id, connection_id = %lease.connection_id, %reason, "host closed logical client");
                        break;
                    }
                    None => break,
                }
            }
        }
    }

    state.remove_client(lease).await;
    if host_tx
        .try_send(RelayFrame::Close {
            connection_id: lease.connection_id,
            reason: "client disconnected".to_owned(),
        })
        .is_err()
    {
        debug!(%host_id, connection_id = %lease.connection_id, "host was unavailable while closing logical client");
    }
    let _ = sink.send(Message::Close(None)).await;
}

async fn send_offline_and_close(mut socket: WebSocket, host_id: HostId, message: &str) {
    let _ = send_server(
        &mut socket,
        ServerMessage::HostStatus {
            host_id,
            online: false,
            message: Some(message.to_owned()),
        },
    )
    .await;
    let _ = socket.send(Message::Close(None)).await;
}

async fn send_relay(socket: &mut WebSocket, frame: RelayFrame) -> Result<(), ()> {
    let payload = encode_relay(&frame).map_err(|error| {
        warn!(%error, "failed to encode relay frame");
    })?;
    socket
        .send(Message::Binary(payload.into()))
        .await
        .map_err(|_| ())
}

async fn send_relay_sink<S>(sink: &mut S, frame: RelayFrame) -> Result<(), ()>
where
    S: futures_util::Sink<Message> + Unpin,
{
    let payload = encode_relay(&frame).map_err(|error| {
        warn!(%error, "failed to encode relay frame");
    })?;
    sink.send(Message::Binary(payload.into()))
        .await
        .map_err(|_| ())
}

async fn send_relay_error(socket: &mut WebSocket, code: &str, message: &str) {
    let _ = send_relay(
        socket,
        RelayFrame::Error {
            code: code.to_owned(),
            message: message.to_owned(),
        },
    )
    .await;
    let _ = socket.send(Message::Close(None)).await;
}

async fn send_server(socket: &mut WebSocket, message: ServerMessage) -> Result<(), ()> {
    let payload = encode(&message).map_err(|error| {
        warn!(%error, "failed to encode server message");
    })?;
    socket
        .send(Message::Binary(payload.into()))
        .await
        .map_err(|_| ())
}

async fn send_server_sink<S>(sink: &mut S, message: ServerMessage) -> Result<(), ()>
where
    S: futures_util::Sink<Message> + Unpin,
{
    let payload = encode(&message).map_err(|error| {
        warn!(%error, "failed to encode server message");
    })?;
    sink.send(Message::Binary(payload.into()))
        .await
        .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use std::{fs, net::SocketAddr};

    use agent_remote_protocol::{ClientCommand, PROTOCOL_VERSION, decode};
    use futures_util::{SinkExt, StreamExt};
    use tempfile::TempDir;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        task::JoinHandle,
    };
    use tokio_tungstenite::{
        WebSocketStream, connect_async,
        tungstenite::{Message as TungsteniteMessage, client::IntoClientRequest},
    };

    use super::*;

    struct TestRelay {
        address: SocketAddr,
        task: JoinHandle<()>,
        _web: TempDir,
    }

    impl Drop for TestRelay {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn start_relay(token: &str, capacity: usize) -> TestRelay {
        let web = tempfile::tempdir().expect("create web fixture");
        fs::write(
            web.path().join("index.html"),
            "<!doctype html><title>relay</title>",
        )
        .expect("write web fixture");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind relay");
        let address = listener.local_addr().expect("relay address");
        let app = router(RelayState::new(token, capacity), web.path().to_owned());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve relay");
        });
        TestRelay {
            address,
            task,
            _web: web,
        }
    }

    async fn connect(
        address: SocketAddr,
        path: &str,
        protocol: &str,
    ) -> WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
        let mut request = format!("ws://{address}{path}")
            .into_client_request()
            .expect("websocket request");
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            protocol.parse().expect("subprotocol header"),
        );
        let (socket, response) = connect_async(request).await.expect("connect websocket");
        assert_eq!(
            response
                .headers()
                .get("Sec-WebSocket-Protocol")
                .and_then(|value| value.to_str().ok()),
            Some(protocol)
        );
        socket
    }

    async fn http_get(address: SocketAddr, path: &str) -> String {
        let mut stream = TcpStream::connect(address).await.expect("connect http");
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .expect("write http request");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .await
            .expect("read http response");
        response
    }

    async fn send_frame(
        socket: &mut WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
        frame: RelayFrame,
    ) {
        socket
            .send(TungsteniteMessage::Binary(
                encode_relay(&frame).expect("encode frame").into(),
            ))
            .await
            .expect("send frame");
    }

    async fn receive_binary(
        socket: &mut WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ) -> Vec<u8> {
        loop {
            match socket
                .next()
                .await
                .expect("websocket open")
                .expect("read message")
            {
                TungsteniteMessage::Binary(bytes) => return bytes.to_vec(),
                TungsteniteMessage::Ping(bytes) => {
                    socket
                        .send(TungsteniteMessage::Pong(bytes))
                        .await
                        .expect("send pong");
                }
                other => panic!("expected binary websocket message, got {other:?}"),
            }
        }
    }

    async fn register_host(
        relay: &TestRelay,
        host_id: HostId,
        token: &str,
    ) -> WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
        let mut host = connect(relay.address, "/host", RELAY_SUBPROTOCOL).await;
        send_frame(
            &mut host,
            RelayFrame::RegisterHost {
                host_id,
                access_token: token.to_owned(),
            },
        )
        .await;
        let registered =
            decode_relay(&receive_binary(&mut host).await).expect("decode registration");
        assert_eq!(registered, RelayFrame::HostRegistered { host_id });
        host
    }

    #[tokio::test]
    async fn bridges_opaque_application_payloads_in_both_directions() {
        let relay = start_relay("secret", 8).await;
        let host_id = HostId::new();
        let mut host = register_host(&relay, host_id, "secret").await;
        let mut client = connect(
            relay.address,
            &format!("/client/{host_id}"),
            APP_SUBPROTOCOL,
        )
        .await;

        let online: ServerMessage =
            decode(&receive_binary(&mut client).await).expect("decode online");
        assert_eq!(
            online,
            ServerMessage::HostStatus {
                host_id,
                online: true,
                message: None,
            }
        );

        let opened = decode_relay(&receive_binary(&mut host).await).expect("decode open");
        let connection_id = match opened {
            RelayFrame::OpenClient {
                host_id: opened_host,
                connection_id,
            } => {
                assert_eq!(opened_host, host_id);
                connection_id
            }
            other => panic!("expected OpenClient, got {other:?}"),
        };

        let application_payload = encode(&ClientCommand::GetSnapshot).expect("encode app command");
        client
            .send(TungsteniteMessage::Binary(
                application_payload.clone().into(),
            ))
            .await
            .expect("send client payload");
        let forwarded = decode_relay(&receive_binary(&mut host).await).expect("decode payload");
        assert_eq!(
            forwarded,
            RelayFrame::Payload {
                connection_id,
                payload: application_payload,
            }
        );

        let response_payload = encode(&ServerMessage::ProtocolError {
            supported_version: PROTOCOL_VERSION,
            message: "test response".to_owned(),
        })
        .expect("encode app response");
        send_frame(
            &mut host,
            RelayFrame::Payload {
                connection_id,
                payload: response_payload.clone(),
            },
        )
        .await;
        assert_eq!(receive_binary(&mut client).await, response_payload);
    }

    #[tokio::test]
    async fn serves_health_and_the_static_web_entrypoint() {
        let relay = start_relay("secret", 8).await;
        let health = http_get(relay.address, "/health").await;
        assert!(health.starts_with("HTTP/1.1 200 OK"));
        assert!(health.ends_with("\r\nok\n"));

        let index = http_get(relay.address, "/").await;
        assert!(index.starts_with("HTTP/1.1 200 OK"));
        assert!(index.contains("<!doctype html><title>relay</title>"));
    }

    #[tokio::test]
    async fn rejects_an_invalid_host_access_token() {
        let relay = start_relay("right-token", 8).await;
        let host_id = HostId::new();
        let mut host = connect(relay.address, "/host", RELAY_SUBPROTOCOL).await;
        send_frame(
            &mut host,
            RelayFrame::RegisterHost {
                host_id,
                access_token: "wrong-token".to_owned(),
            },
        )
        .await;
        let error = decode_relay(&receive_binary(&mut host).await).expect("decode error");
        assert!(matches!(error, RelayFrame::Error { code, .. } if code == "invalid_access_token"));

        let mut client = connect(
            relay.address,
            &format!("/client/{host_id}"),
            APP_SUBPROTOCOL,
        )
        .await;
        let status: ServerMessage =
            decode(&receive_binary(&mut client).await).expect("decode status");
        assert!(matches!(
            status,
            ServerMessage::HostStatus { online: false, .. }
        ));
    }

    #[tokio::test]
    async fn notifies_clients_when_the_host_disconnects() {
        let relay = start_relay("secret", 8).await;
        let host_id = HostId::new();
        let mut host = register_host(&relay, host_id, "secret").await;
        let mut client = connect(
            relay.address,
            &format!("/client/{host_id}"),
            APP_SUBPROTOCOL,
        )
        .await;
        let _: ServerMessage = decode(&receive_binary(&mut client).await).expect("decode online");
        let _ = decode_relay(&receive_binary(&mut host).await).expect("decode open");

        host.close(None).await.expect("close host");
        let offline: ServerMessage =
            decode(&receive_binary(&mut client).await).expect("decode offline");
        assert_eq!(
            offline,
            ServerMessage::HostStatus {
                host_id,
                online: false,
                message: Some("Host disconnected from relay".to_owned()),
            }
        );
    }

    #[tokio::test]
    async fn removes_a_client_when_its_bounded_channel_is_full() {
        let state = RelayState::new("secret", 1);
        let host_id = HostId::new();
        let (host_tx, mut host_rx) = mpsc::channel(1);
        let generation = state
            .register_host(host_id, host_tx)
            .await
            .expect("register host");
        let (client_tx, _client_rx) = mpsc::channel(1);
        let (lease, _) = state
            .open_client(host_id, client_tx)
            .await
            .expect("open client");
        assert!(matches!(
            host_rx.recv().await,
            Some(RelayFrame::OpenClient { .. })
        ));

        assert!(matches!(
            state
                .route_to_client(
                    host_id,
                    generation,
                    lease.connection_id,
                    ClientOutbound::Payload(vec![1]),
                )
                .await,
            ClientRoute::Delivered
        ));
        assert!(matches!(
            state
                .route_to_client(
                    host_id,
                    generation,
                    lease.connection_id,
                    ClientOutbound::Payload(vec![2]),
                )
                .await,
            ClientRoute::Slow
        ));
        assert!(matches!(
            state
                .route_to_client(
                    host_id,
                    generation,
                    lease.connection_id,
                    ClientOutbound::Payload(vec![3]),
                )
                .await,
            ClientRoute::Missing
        ));
    }
}
