//! Host-outbound relay tunnel. The Relay forwards opaque application CBOR.

use std::{collections::HashMap, sync::Arc, time::Duration};

use agent_remote_protocol::{RelayFrame, decode, encode};
use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use uuid::Uuid;

use crate::{
    app::AppService,
    transport::session::{
        ApplicationSession, AuthRateLimiter, AuthenticatedSession, CommandSchedule,
    },
};

const RELAY_SUBPROTOCOL: &str = "agent-remote-relay.cbor.v1";

#[derive(Clone)]
pub struct RelayClientConfig {
    pub url: String,
    pub access_token: String,
    pub dev_insecure: bool,
}

struct RelayApplicationClient {
    generation: u64,
    session: ApplicationSession,
    ordered_tx: Option<mpsc::UnboundedSender<Vec<u8>>>,
}

struct RelayCommandResponse {
    connection_id: Uuid,
    generation: u64,
    payload: Vec<u8>,
}

pub async fn run_reconnecting(service: Arc<AppService>, config: RelayClientConfig) -> ! {
    let mut delay = Duration::from_secs(1);
    loop {
        if let Err(error) = run_once(Arc::clone(&service), &config).await {
            tracing::warn!(error = %error, retry_seconds = delay.as_secs(), "relay connection ended");
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(30));
    }
}

pub async fn run_once(service: Arc<AppService>, config: &RelayClientConfig) -> Result<()> {
    validate_url(&config.url, config.dev_insecure)?;
    let mut request = config
        .url
        .as_str()
        .into_client_request()
        .context("invalid relay WebSocket URL")?;
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        RELAY_SUBPROTOCOL
            .parse()
            .expect("static subprotocol header"),
    );
    let (socket, response) = connect_async(request).await.context("connect relay")?;
    if response
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|value| value.to_str().ok())
        != Some(RELAY_SUBPROTOCOL)
    {
        bail!("relay did not negotiate {RELAY_SUBPROTOCOL}");
    }

    let (mut sink, mut stream) = socket.split();
    send_frame(
        &mut sink,
        RelayFrame::RegisterHost {
            host_id: service.host_id(),
            access_token: config.access_token.clone(),
        },
    )
    .await?;
    let registered = next_frame(&mut stream).await?;
    match registered {
        RelayFrame::HostRegistered { host_id } if host_id == service.host_id() => {}
        RelayFrame::Error { code, message } => bail!("relay rejected Host: {code}: {message}"),
        other => bail!("unexpected relay registration response: {other:?}"),
    }
    tracing::info!(relay = %config.url, host_id = %service.host_id(), "Host registered with relay");

    let limiter = Arc::new(AuthRateLimiter::default());
    let mut clients: HashMap<Uuid, RelayApplicationClient> = HashMap::new();
    let mut next_client_generation = 0_u64;
    let (response_tx, mut response_rx) = mpsc::unbounded_channel();
    let mut updates = service.subscribe();
    loop {
        tokio::select! {
            incoming = stream.next() => {
                let Some(incoming) = incoming else { bail!("relay closed the WebSocket") };
                match incoming? {
                    Message::Binary(bytes) => {
                        let frame = decode::<RelayFrame>(&bytes)?;
                        match frame {
                            RelayFrame::OpenClient { host_id, connection_id } if host_id == service.host_id() => {
                                next_client_generation += 1;
                                clients.insert(connection_id, RelayApplicationClient {
                                    generation: next_client_generation,
                                    session: ApplicationSession::new(
                                        Arc::clone(&service),
                                        Arc::clone(&limiter),
                                        format!("relay:{connection_id}"),
                                    ),
                                    ordered_tx: None,
                                });
                                send_frame(&mut sink, RelayFrame::ClientOpened { connection_id }).await?;
                            }
                            RelayFrame::Payload { connection_id, payload } => {
                                if let Some(client) = clients.get_mut(&connection_id) {
                                    if let Some(authenticated) = client.session.authenticated() {
                                        let generation = client.generation;
                                        match AuthenticatedSession::schedule(&payload) {
                                            CommandSchedule::Concurrent => {
                                                let response_tx = response_tx.clone();
                                                tokio::spawn(async move {
                                                    let payload = authenticated.process(&payload).await;
                                                    let _ = response_tx.send(RelayCommandResponse {
                                                        connection_id,
                                                        generation,
                                                        payload,
                                                    });
                                                });
                                            }
                                            CommandSchedule::Ordered => {
                                                let ordered_tx = client.ordered_tx.get_or_insert_with(|| {
                                                    spawn_ordered_worker(
                                                        authenticated,
                                                        connection_id,
                                                        generation,
                                                        response_tx.clone(),
                                                    )
                                                });
                                                if ordered_tx.send(payload).is_err() {
                                                    clients.remove(&connection_id);
                                                }
                                            }
                                        }
                                    } else {
                                        let response = client.session.process(&payload).await;
                                        send_frame(&mut sink, RelayFrame::Payload { connection_id, payload: response }).await?;
                                    }
                                } else {
                                    send_frame(&mut sink, RelayFrame::Close {
                                        connection_id,
                                        reason: "logical client is not open".to_owned(),
                                    }).await?;
                                }
                            }
                            RelayFrame::Close { connection_id, .. } => {
                                clients.remove(&connection_id);
                            }
                            RelayFrame::Error { code, message } => bail!("relay error {code}: {message}"),
                            _ => {
                                send_frame(&mut sink, RelayFrame::Error {
                                    code: "unexpected_frame".to_owned(),
                                    message: "frame is not valid for a registered Host".to_owned(),
                                }).await?;
                            }
                        }
                    }
                    Message::Ping(payload) => sink.send(Message::Pong(payload)).await?,
                    Message::Pong(_) => {}
                    Message::Close(_) => bail!("relay closed the WebSocket"),
                    Message::Text(_) | Message::Frame(_) => bail!("relay sent a non-binary protocol frame"),
                }
            }
            response = response_rx.recv() => {
                let Some(response) = response else { bail!("relay command response channel closed") };
                let is_current = clients
                    .get(&response.connection_id)
                    .is_some_and(|client| client.generation == response.generation);
                if is_current {
                    send_frame(&mut sink, RelayFrame::Payload {
                        connection_id: response.connection_id,
                        payload: response.payload,
                    }).await?;
                }
            }
            update = updates.recv() => {
                match update {
                    Ok(update) => {
                        let payload = encode(&update)?;
                        let authenticated = clients
                            .iter()
                            .filter_map(|(id, client)| client.session.is_authenticated().then_some(*id))
                            .collect::<Vec<_>>();
                        for connection_id in authenticated {
                            send_frame(&mut sink, RelayFrame::Payload {
                                connection_id,
                                payload: payload.clone(),
                            }).await?;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        for connection_id in clients.keys().copied().collect::<Vec<_>>() {
                            send_frame(&mut sink, RelayFrame::Close {
                                connection_id,
                                reason: "Host event channel lagged; reconnect and request a snapshot".to_owned(),
                            }).await?;
                        }
                        clients.clear();
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => bail!("Host update channel closed"),
                }
            }
        }
    }
}

fn spawn_ordered_worker(
    session: AuthenticatedSession,
    connection_id: Uuid,
    generation: u64,
    response_tx: mpsc::UnboundedSender<RelayCommandResponse>,
) -> mpsc::UnboundedSender<Vec<u8>> {
    let (command_tx, mut command_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        while let Some(payload) = command_rx.recv().await {
            let payload = session.process(&payload).await;
            let _ = response_tx.send(RelayCommandResponse {
                connection_id,
                generation,
                payload,
            });
        }
    });
    command_tx
}

fn validate_url(url: &str, dev_insecure: bool) -> Result<()> {
    if url.starts_with("wss://") {
        return Ok(());
    }
    if dev_insecure && url.starts_with("ws://127.0.0.1") {
        return Ok(());
    }
    bail!("Relay URL must use wss://; only ws://127.0.0.1 is allowed with --relay-dev-insecure")
}

async fn send_frame<S>(sink: &mut S, frame: RelayFrame) -> Result<()>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    sink.send(Message::Binary(encode(&frame)?.into())).await?;
    Ok(())
}

async fn next_frame<S>(stream: &mut S) -> Result<RelayFrame>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match stream
            .next()
            .await
            .context("relay closed during registration")??
        {
            Message::Binary(bytes) => return Ok(decode(&bytes)?),
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => bail!("relay closed during registration"),
            Message::Text(_) | Message::Frame(_) => {
                bail!("relay registration response was not binary CBOR")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_relay_requires_wss() {
        assert!(validate_url("wss://relay.example/host", false).is_ok());
        assert!(validate_url("ws://relay.example/host", true).is_err());
        assert!(validate_url("ws://127.0.0.1:8443/host", true).is_ok());
    }
}
