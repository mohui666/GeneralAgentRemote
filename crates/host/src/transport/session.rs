use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use agent_remote_protocol::{
    ClientCommand, DeviceId, ProtocolError, ServerMessage, decode, encode,
};

use crate::app::AppService;

const AUTH_WINDOW: Duration = Duration::from_secs(60);
const MAX_AUTH_FAILURES: usize = 5;

#[derive(Default)]
pub struct AuthRateLimiter {
    failures: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl AuthRateLimiter {
    fn allows(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut failures = self.failures.lock().expect("auth limiter mutex poisoned");
        let attempts = failures.entry(key.to_owned()).or_default();
        while attempts
            .front()
            .is_some_and(|attempt| now.duration_since(*attempt) > AUTH_WINDOW)
        {
            attempts.pop_front();
        }
        attempts.len() < MAX_AUTH_FAILURES
    }

    fn record_failure(&self, key: &str) {
        self.failures
            .lock()
            .expect("auth limiter mutex poisoned")
            .entry(key.to_owned())
            .or_default()
            .push_back(Instant::now());
    }

    fn clear(&self, key: &str) {
        self.failures
            .lock()
            .expect("auth limiter mutex poisoned")
            .remove(key);
    }
}

pub struct ApplicationSession {
    service: Arc<AppService>,
    rate_limiter: Arc<AuthRateLimiter>,
    peer_key: String,
    authenticated_device: Option<DeviceId>,
}

#[derive(Clone)]
pub struct AuthenticatedSession {
    service: Arc<AppService>,
    device_id: DeviceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSchedule {
    Concurrent,
    Ordered,
}

impl ApplicationSession {
    pub fn new(
        service: Arc<AppService>,
        rate_limiter: Arc<AuthRateLimiter>,
        peer_key: String,
    ) -> Self {
        Self {
            service,
            rate_limiter,
            peer_key,
            authenticated_device: None,
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.authenticated_device.is_some()
    }

    pub fn authenticated(&self) -> Option<AuthenticatedSession> {
        self.authenticated_device
            .map(|device_id| AuthenticatedSession {
                service: Arc::clone(&self.service),
                device_id,
            })
    }

    pub async fn process(&mut self, bytes: &[u8]) -> Vec<u8> {
        let response = match decode::<ClientCommand>(bytes) {
            Ok(command) => match self.authenticated() {
                Some(session) => session.process_command(command).await,
                None => self.authenticate(command).await,
            },
            Err(error) => decode_rejected(error),
        };
        encode(&response).expect("server messages are serializable")
    }

    async fn authenticate(&mut self, command: ClientCommand) -> ServerMessage {
        if !self.rate_limiter.allows(&self.peer_key) {
            return ServerMessage::CommandRejected {
                command_id: None,
                code: "authentication_rate_limited".to_owned(),
                message: "Too many failed authentication attempts; try again shortly".to_owned(),
            };
        }
        match command {
            ClientCommand::Pair {
                host_id,
                pair_token,
                device_name,
            } if host_id == self.service.host_id() => {
                match self
                    .service
                    .exchange_pairing_token(&pair_token, &device_name)
                {
                    Ok(device) => {
                        self.authenticated_device = Some(device.id);
                        self.rate_limiter.clear(&self.peer_key);
                        ServerMessage::Paired {
                            host_id,
                            device_id: device.id,
                            device_token: device.token,
                        }
                    }
                    Err(error) => {
                        self.rate_limiter.record_failure(&self.peer_key);
                        auth_rejected(error.to_string())
                    }
                }
            }
            ClientCommand::Authenticate {
                host_id,
                device_id,
                device_token,
            } if host_id == self.service.host_id() => {
                match self.service.authenticate_device(device_id, &device_token) {
                    Ok(true) => {
                        self.authenticated_device = Some(device_id);
                        self.rate_limiter.clear(&self.peer_key);
                        ServerMessage::Authenticated { host_id, device_id }
                    }
                    Ok(false) => {
                        self.rate_limiter.record_failure(&self.peer_key);
                        auth_rejected("Device credential is invalid or revoked".to_owned())
                    }
                    Err(error) => auth_rejected(error.to_string()),
                }
            }
            ClientCommand::Pair { .. } | ClientCommand::Authenticate { .. } => {
                self.rate_limiter.record_failure(&self.peer_key);
                auth_rejected("Host ID does not match this Host".to_owned())
            }
            _ => auth_rejected("Authenticate or pair before sending commands".to_owned()),
        }
    }
}

impl AuthenticatedSession {
    pub fn schedule(bytes: &[u8]) -> CommandSchedule {
        match decode::<ClientCommand>(bytes) {
            Ok(
                ClientCommand::SyncProject { .. }
                | ClientCommand::CreateConversation { .. }
                | ClientCommand::StartConversation { .. }
                | ClientCommand::SendMessage { .. }
                | ClientCommand::Steer { .. }
                | ClientCommand::SetSessionOption { .. }
                | ClientCommand::RenameConversation { .. },
            ) => CommandSchedule::Ordered,
            _ => CommandSchedule::Concurrent,
        }
    }

    pub async fn process(&self, bytes: &[u8]) -> Vec<u8> {
        let response = match decode::<ClientCommand>(bytes) {
            Ok(command) => self.process_command(command).await,
            Err(error) => decode_rejected(error),
        };
        encode(&response).expect("server messages are serializable")
    }

    async fn process_command(&self, command: ClientCommand) -> ServerMessage {
        if matches!(
            command,
            ClientCommand::Authenticate { .. } | ClientCommand::Pair { .. }
        ) {
            return ServerMessage::CommandRejected {
                command_id: None,
                code: "already_authenticated".to_owned(),
                message: "This connection is already authenticated".to_owned(),
            };
        }
        let command_id = command.command_id();
        match self.service.execute_command(self.device_id, command).await {
            Ok(response) => response,
            Err(error) => ServerMessage::CommandRejected {
                command_id,
                code: "command_failed".to_owned(),
                message: error.to_string(),
            },
        }
    }
}

fn decode_rejected(error: ProtocolError) -> ServerMessage {
    match error {
        ProtocolError::Version { .. } => ServerMessage::ProtocolError {
            supported_version: agent_remote_protocol::PROTOCOL_VERSION,
            message: "Client protocol version is not supported".to_owned(),
        },
        error => ServerMessage::CommandRejected {
            command_id: None,
            code: "invalid_message".to_owned(),
            message: error.to_string(),
        },
    }
}

fn auth_rejected(message: String) -> ServerMessage {
    ServerMessage::CommandRejected {
        command_id: None,
        code: "authentication_failed".to_owned(),
        message,
    }
}
