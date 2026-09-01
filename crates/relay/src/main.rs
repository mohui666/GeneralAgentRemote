use std::{env, net::SocketAddr, path::PathBuf};

use agent_remote_relay::{DEFAULT_CHANNEL_CAPACITY, RelayState, router};
use anyhow::{Context, Result, bail};
use axum_server::tls_rustls::RustlsConfig;
use clap::{Args, Parser, Subcommand};
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "agent-remote-relay", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve(ServeArgs),
}

#[derive(Debug, Args)]
struct ServeArgs {
    #[arg(long, default_value = "0.0.0.0:8443")]
    listen: SocketAddr,

    #[arg(long)]
    tls_cert: Option<PathBuf>,

    #[arg(long)]
    tls_key: Option<PathBuf>,

    #[arg(long)]
    web_dir: Option<PathBuf>,

    #[arg(long, default_value = "AGENT_REMOTE_RELAY_TOKEN")]
    access_token_env: String,

    #[arg(long)]
    dev_insecure: bool,

    #[arg(long, conflicts_with = "dev_insecure")]
    behind_proxy: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = match env::var("RUST_LOG") {
        Ok(value) => EnvFilter::try_new(value).context("RUST_LOG is not a valid tracing filter")?,
        Err(env::VarError::NotPresent) => EnvFilter::new("agent_remote_relay=info"),
        Err(error) => return Err(error).context("failed to read RUST_LOG"),
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init()
        .map_err(|error| anyhow::anyhow!("failed to initialize tracing: {error}"))?;

    match Cli::parse().command {
        Command::Serve(args) => serve(args).await,
    }
}

async fn serve(args: ServeArgs) -> Result<()> {
    let access_token = env::var(&args.access_token_env)
        .with_context(|| format!("{} is not set", args.access_token_env))?;
    if access_token.is_empty() {
        bail!("{} must not be empty", args.access_token_env);
    }

    let web_dir = args.web_dir.unwrap_or(default_web_dir()?);
    let state = RelayState::new(access_token, DEFAULT_CHANNEL_CAPACITY);
    let app = router(state, web_dir);

    if args.dev_insecure {
        warn!(listen = %args.listen, "relay is serving plaintext HTTP/WebSocket in explicit development mode");
        let listener = TcpListener::bind(args.listen).await?;
        info!(listen = %listener.local_addr()?, "relay listening");
        axum::serve(listener, app).await?;
        return Ok(());
    }

    if args.behind_proxy {
        if !args.listen.ip().is_loopback() {
            bail!("--behind-proxy requires a loopback --listen address");
        }
        info!(listen = %args.listen, "relay listening behind a trusted HTTPS reverse proxy");
        let listener = TcpListener::bind(args.listen).await?;
        axum::serve(listener, app).await?;
        return Ok(());
    }

    let (cert, key) = match (args.tls_cert, args.tls_key) {
        (Some(cert), Some(key)) => (cert, key),
        _ => bail!(
            "--tls-cert and --tls-key are required unless --dev-insecure or --behind-proxy is set"
        ),
    };

    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("a rustls crypto provider was already installed"))?;
    let tls = RustlsConfig::from_pem_file(cert, key)
        .await
        .context("failed to load relay TLS certificate or key")?;
    info!(listen = %args.listen, "relay listening with rustls");
    axum_server::bind_rustls(args.listen, tls)
        .serve(app.into_make_service())
        .await?;
    Ok(())
}

fn default_web_dir() -> Result<PathBuf> {
    let executable = env::current_exe().context("failed to locate relay executable")?;
    let directory = executable
        .parent()
        .context("relay executable has no parent directory")?;
    Ok(directory.join("../web"))
}
