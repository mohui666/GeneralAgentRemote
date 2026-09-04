use std::{env, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use agent_remote_host::{
    Project, Storage,
    app::{AppService, default_data_root},
    attachments::{AttachmentStore, DEFAULT_MAX_IMAGE_BYTES},
    providers::{
        AgentProvider, BUILT_IN_PROVIDER_IDS, CreateSession, ProviderEventKind, ProviderRegistry,
        ResolveApproval, SendMessage, built_in_providers,
    },
    transport::{
        direct,
        relay::{RelayClientConfig, run_reconnecting},
    },
};
use agent_remote_protocol::{ConversationId, ProjectId, ProviderId, ProviderState};
use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use qrcode::{QrCode, render::unicode};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "agent-remote-host", version, about)]
struct Cli {
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    Pair(PairArgs),
    Device {
        #[command(subcommand)]
        command: DeviceCommand,
    },
    Serve(ServeArgs),
    ProviderSmoke {
        #[arg(long = "provider", value_enum)]
        providers: Vec<ProviderArg>,
    },
}

#[derive(Debug, Subcommand)]
enum ProjectCommand {
    Add {
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long = "provider", value_enum)]
        providers: Vec<ProviderArg>,
    },
    SetProviders {
        project_id: String,
        #[arg(long = "provider", value_enum, required = true)]
        providers: Vec<ProviderArg>,
    },
    List,
    Remove {
        project_id: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ProviderArg {
    Codex,
    Grok,
    #[value(name = "claude-code", alias = "claude")]
    ClaudeCode,
    #[value(name = "gemini-cli", alias = "gemini")]
    GeminiCli,
    #[value(name = "copilot-cli", alias = "copilot")]
    CopilotCli,
    #[value(name = "opencode")]
    OpenCode,
    Cursor,
    Cline,
    Goose,
    Junie,
    #[value(name = "qwen-code", alias = "qwen")]
    QwenCode,
    #[value(name = "kimi-cli", alias = "kimi")]
    KimiCli,
    #[value(name = "kiro-cli", alias = "kiro")]
    KiroCli,
    #[value(name = "mistral-vibe", alias = "vibe")]
    MistralVibe,
    #[value(name = "qoder-cli", alias = "qoder")]
    QoderCli,
    #[value(name = "auggie", alias = "augment")]
    Auggie,
    #[value(name = "factory-droid", alias = "droid")]
    FactoryDroid,
    Devin,
    #[value(name = "codebuddy", alias = "codebuddy-code")]
    CodeBuddy,
    #[value(name = "glm-agent", alias = "glm")]
    GlmAgent,
    #[value(name = "kilo-code", alias = "kilo")]
    KiloCode,
    Amp,
}

impl From<ProviderArg> for ProviderId {
    fn from(provider: ProviderArg) -> Self {
        match provider {
            ProviderArg::Codex => Self::Codex,
            ProviderArg::Grok => Self::Grok,
            ProviderArg::ClaudeCode => Self::ClaudeCode,
            ProviderArg::GeminiCli => Self::GeminiCli,
            ProviderArg::CopilotCli => Self::CopilotCli,
            ProviderArg::OpenCode => Self::OpenCode,
            ProviderArg::Cursor => Self::Cursor,
            ProviderArg::Cline => Self::Cline,
            ProviderArg::Goose => Self::Goose,
            ProviderArg::Junie => Self::Junie,
            ProviderArg::QwenCode => Self::QwenCode,
            ProviderArg::KimiCli => Self::KimiCli,
            ProviderArg::KiroCli => Self::KiroCli,
            ProviderArg::MistralVibe => Self::MistralVibe,
            ProviderArg::QoderCli => Self::QoderCli,
            ProviderArg::Auggie => Self::Auggie,
            ProviderArg::FactoryDroid => Self::FactoryDroid,
            ProviderArg::Devin => Self::Devin,
            ProviderArg::CodeBuddy => Self::CodeBuddy,
            ProviderArg::GlmAgent => Self::GlmAgent,
            ProviderArg::KiloCode => Self::KiloCode,
            ProviderArg::Amp => Self::Amp,
        }
    }
}

#[derive(Debug, Args)]
struct PairArgs {
    #[arg(long)]
    relay: bool,

    #[arg(long)]
    base_url: Option<String>,
}

#[derive(Debug, Subcommand)]
enum DeviceCommand {
    List,
    Revoke { device_id: String },
}

#[derive(Debug, Args)]
struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1:7437")]
    listen: SocketAddr,

    #[arg(long)]
    web_root: Option<PathBuf>,

    #[arg(long)]
    dev_insecure: bool,

    #[arg(long)]
    relay_url: Option<String>,

    #[arg(long, default_value = "AGENT_REMOTE_RELAY_TOKEN")]
    relay_token_env: String,

    #[arg(long)]
    relay_dev_insecure: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing()?;
    let cli = Cli::parse();
    let data_root = cli.data_dir.map_or_else(default_data_root, Ok)?;
    match cli.command {
        Command::Project { command } => project_command(&data_root, command),
        Command::Pair(args) => pair_command(&data_root, args),
        Command::Device { command } => device_command(&data_root, command),
        Command::Serve(args) => serve_command(data_root, args).await,
        Command::ProviderSmoke { providers } => {
            provider_smoke(providers.into_iter().map(ProviderId::from).collect()).await
        }
    }
}

fn project_command(data_root: &std::path::Path, command: ProjectCommand) -> Result<()> {
    let storage = Storage::open(data_root.join("state.db"))?;
    match command {
        ProjectCommand::Add {
            path,
            name,
            providers,
        } => {
            let providers = if providers.is_empty() {
                BUILT_IN_PROVIDER_IDS.to_vec()
            } else {
                providers.into_iter().map(ProviderId::from).collect()
            };
            let project = storage.add_project(path, name.as_deref(), &providers)?;
            println!(
                "{}\t{}\t{}",
                project.id,
                project.display_name,
                project.canonical_path.display()
            );
        }
        ProjectCommand::SetProviders {
            project_id,
            providers,
        } => {
            let project_id = ProjectId(parse_uuid(&project_id, "project")?);
            let providers = providers
                .into_iter()
                .map(ProviderId::from)
                .collect::<Vec<_>>();
            let project = storage.set_project_providers(project_id, &providers)?;
            println!(
                "updated {}\t{}",
                project.id,
                project
                    .enabled_providers
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
        ProjectCommand::List => {
            for project in storage.list_projects()? {
                let providers = project
                    .enabled_providers
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                let validity = if project.canonical_path.is_dir() {
                    "valid"
                } else {
                    "invalid"
                };
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    project.id,
                    project.display_name,
                    providers,
                    validity,
                    project.canonical_path.display()
                );
            }
        }
        ProjectCommand::Remove { project_id } => {
            let project_id = ProjectId(parse_uuid(&project_id, "project")?);
            if !storage.remove_project(project_id)? {
                bail!("project {project_id} was not found");
            }
            println!("removed {project_id}");
        }
    }
    Ok(())
}

fn pair_command(data_root: &std::path::Path, args: PairArgs) -> Result<()> {
    let storage = Storage::open(data_root.join("state.db"))?;
    let host_id = storage.host_id()?;
    let pairing = storage.create_pairing_token()?;
    let base_url = match (args.relay, args.base_url) {
        (true, Some(url)) => url,
        (true, None) => bail!("--relay requires --base-url https://relay.example.com"),
        (false, Some(url)) => url,
        (false, None) => "http://127.0.0.1:7437".to_owned(),
    };
    let relay_fragment = if args.relay { "&relay=1" } else { "" };
    let url = format!(
        "{}/#host={host_id}&pair={}{}",
        base_url.trim_end_matches('/'),
        pairing.token,
        relay_fragment
    );
    let qr = QrCode::new(url.as_bytes())?;
    let rendered = qr.render::<unicode::Dense1x2>().quiet_zone(true).build();
    println!("Pair code: {}", pairing.short_code);
    println!("Expires: {} (Unix ms)", pairing.expires_at_ms);
    println!("{url}\n{rendered}");
    Ok(())
}

fn device_command(data_root: &std::path::Path, command: DeviceCommand) -> Result<()> {
    let storage = Storage::open(data_root.join("state.db"))?;
    match command {
        DeviceCommand::List => {
            for device in storage.list_devices()? {
                println!(
                    "{}\t{}\tcreated={}\tlast_seen={}",
                    device.id,
                    device.name,
                    device.created_at_ms,
                    device
                        .last_seen_at_ms
                        .map_or_else(|| "never".to_owned(), |value| value.to_string())
                );
            }
        }
        DeviceCommand::Revoke { device_id } => {
            let device_id = agent_remote_protocol::DeviceId(parse_uuid(&device_id, "device")?);
            if !storage.revoke_device(device_id)? {
                bail!("device {device_id} was not found or was already revoked");
            }
            println!("revoked {device_id}");
        }
    }
    Ok(())
}

async fn serve_command(data_root: PathBuf, args: ServeArgs) -> Result<()> {
    direct::public_plaintext_rejected(args.listen, args.dev_insecure)?;
    let storage = Arc::new(Storage::open(data_root.join("state.db"))?);
    let attachments = AttachmentStore::new(data_root.join("attachments"), DEFAULT_MAX_IMAGE_BYTES)?;
    let registry = ProviderRegistry::new(built_in_providers());
    let host_name = env::var("COMPUTERNAME")
        .or_else(|_| env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Agent Remote Host".to_owned());
    let service = AppService::new(storage, attachments, registry, host_name)?;
    service.start_provider_event_pumps();

    if let Some(url) = args.relay_url {
        let token = env::var(&args.relay_token_env)
            .with_context(|| format!("{} is not set", args.relay_token_env))?;
        if token.is_empty() {
            bail!("{} must not be empty", args.relay_token_env);
        }
        let relay_service = Arc::clone(&service);
        tokio::spawn(run_reconnecting(
            relay_service,
            RelayClientConfig {
                url,
                access_token: token,
                dev_insecure: args.relay_dev_insecure,
            },
        ));
    }

    let web_root = args.web_root.map_or_else(default_web_root, Ok)?;
    let listener = TcpListener::bind(args.listen).await?;
    println!(
        "Host {} listening on http://{}",
        service.host_id(),
        listener.local_addr()?
    );
    direct::serve(listener, service, web_root).await
}

async fn provider_smoke(selected_providers: Vec<ProviderId>) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let enabled_providers = if selected_providers.is_empty() {
        BUILT_IN_PROVIDER_IDS.to_vec()
    } else {
        selected_providers
    };
    let providers = built_in_providers()
        .into_iter()
        .filter(|provider| enabled_providers.contains(&provider.id()))
        .collect::<Vec<_>>();
    let project = Project {
        id: ProjectId::new(),
        display_name: "Agent Remote smoke".to_owned(),
        canonical_path: temp.path().canonicalize()?,
        enabled_providers,
    };
    let mut failed = false;
    for provider in providers {
        match smoke_one(Arc::clone(&provider), &project).await {
            Ok(SmokeResult::Pass(detail)) => println!("PASS {}: {detail}", provider.id()),
            Ok(SmokeResult::Skip(reason)) => println!("SKIP {}: {reason}", provider.id()),
            Err(error) if smoke_precondition_unavailable(&error) => {
                println!("SKIP {}: {error:#}", provider.id());
            }
            Err(error) => {
                failed = true;
                println!("FAIL {}: {error:#}", provider.id());
            }
        }
    }
    if failed {
        bail!("one or more installed/authenticated Provider smoke tests failed");
    }
    Ok(())
}

enum SmokeResult {
    Pass(String),
    Skip(String),
}

async fn smoke_one(provider: Arc<dyn AgentProvider>, project: &Project) -> Result<SmokeResult> {
    let health = provider.health().await;
    match health.state {
        ProviderState::NotInstalled | ProviderState::NotAuthenticated => {
            return Ok(SmokeResult::Skip(health.detail.unwrap_or_else(|| {
                format!("Provider state is {:?}", health.state)
            })));
        }
        ProviderState::Ready => {}
        other => bail!(
            "Provider is not smoke-testable: {other:?}: {}",
            health.detail.unwrap_or_default()
        ),
    }

    let models = provider.list_models(project).await?;
    let selected_model = models.first().map(|model| model.id.clone());
    let selected_effort = models.first().and_then(|model| {
        model
            .default_effort
            .clone()
            .or_else(|| model.effort_options.first().map(|effort| effort.id.clone()))
    });
    let conversation_id = ConversationId::new();
    let mut events = provider.subscribe();
    let native = provider
        .create_session(CreateSession {
            conversation_id,
            project: project.clone(),
            model: selected_model.clone(),
            effort: selected_effort.clone(),
        })
        .await?;
    provider
        .send_message(SendMessage {
            conversation_id,
            project: project.clone(),
            native_session_id: native.native_session_id,
            client_message_id: Some("provider-smoke".to_owned()),
            text: "Reply with exactly AGENT_REMOTE_SMOKE_OK. Do not run commands or modify files."
                .to_owned(),
            attachments: Vec::new(),
            model: selected_model,
            effort: selected_effort,
            permission_mode: None,
        })
        .await?;

    let provider_for_wait = Arc::clone(&provider);
    let result = tokio::time::timeout(Duration::from_secs(180), async move {
        let mut final_text = String::new();
        loop {
            let event = events.recv().await?;
            if event.conversation_id != conversation_id {
                continue;
            }
            match event.kind {
                ProviderEventKind::AgentTextDelta {
                    phase: agent_remote_protocol::AgentMessagePhase::Final,
                    delta,
                    ..
                } => final_text.push_str(&delta),
                ProviderEventKind::AgentTextSnapshot {
                    phase: agent_remote_protocol::AgentMessagePhase::Final,
                    text,
                    ..
                } => final_text = text,
                ProviderEventKind::Approval {
                    provider_request_id,
                    options,
                    ..
                } => {
                    let reject = options
                        .iter()
                        .find(|option| {
                            let id = option.id.to_ascii_lowercase();
                            id.contains("decline") || id.contains("reject") || id.contains("deny")
                        })
                        .ok_or_else(|| {
                            anyhow!("smoke received an approval without a reject option")
                        })?;
                    provider_for_wait
                        .resolve_approval(ResolveApproval {
                            conversation_id,
                            provider_request_id,
                            option_id: reject.id.clone(),
                        })
                        .await?;
                }
                ProviderEventKind::Completed => return Ok::<_, anyhow::Error>(final_text),
                ProviderEventKind::Failed { code, message, .. } => bail!("{code}: {message}"),
                ProviderEventKind::Crashed { message } => bail!("Provider crashed: {message}"),
                ProviderEventKind::Interrupted => bail!("smoke was interrupted"),
                _ => {}
            }
        }
    })
    .await
    .context("Provider smoke timed out")??;
    if !result.contains("AGENT_REMOTE_SMOKE_OK") {
        bail!("final answer did not contain AGENT_REMOTE_SMOKE_OK: {result}");
    }
    Ok(SmokeResult::Pass(format!(
        "temporary project completed with {}",
        result.trim()
    )))
}

fn smoke_precondition_unavailable(error: &anyhow::Error) -> bool {
    let message = format!("{error:#}").to_ascii_lowercase();
    [
        "not authenticated",
        "authentication required",
        "login required",
        "payment required",
        "usage balance exhausted",
        "quota exceeded",
        "insufficient quota",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

fn parse_uuid(value: &str, kind: &str) -> Result<Uuid> {
    Uuid::parse_str(value).with_context(|| format!("invalid {kind} id: {value}"))
}

fn default_web_root() -> Result<PathBuf> {
    let current = env::current_dir()?.join("dist/web");
    if current.is_dir() {
        return Ok(current);
    }
    let executable = env::current_exe()?;
    Ok(executable
        .parent()
        .context("Host executable has no parent directory")?
        .join("../web"))
}

fn init_tracing() -> Result<()> {
    let filter = match env::var("RUST_LOG") {
        Ok(value) => EnvFilter::try_new(value)?,
        Err(env::VarError::NotPresent) => EnvFilter::new("agent_remote_host=info"),
        Err(error) => return Err(error.into()),
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init()
        .map_err(|error| anyhow!("initialize tracing: {error}"))
}
