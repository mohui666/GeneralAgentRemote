use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

const TRUNK_VERSION: &str = "0.21.14";

#[derive(Parser)]
#[command(about = "Agent Remote Messenger build and development tasks")]
struct Cli {
    #[command(subcommand)]
    command: Task,
}

#[derive(Subcommand)]
enum Task {
    Build,
    Test,
    Android {
        #[arg(long)]
        release: bool,
    },
    Web {
        #[arg(long)]
        release: bool,
    },
    DevHost {
        #[arg(last = true)]
        args: Vec<String>,
    },
    DevRelay {
        #[arg(last = true)]
        args: Vec<String>,
    },
    ProviderSmoke,
}

fn main() -> Result<()> {
    let root = workspace_root()?;
    env::set_current_dir(&root)?;
    match Cli::parse().command {
        Task::Build => build(&root),
        Task::Test => test(&root),
        Task::Android { release } => build_android(&root, release),
        Task::Web { release } => build_web(&root, release),
        Task::DevHost { args } => {
            build_web(&root, false)?;
            run_cargo(
                &root,
                &[
                    "run",
                    "-p",
                    "agent-remote-host",
                    "--",
                    "serve",
                    "--web-root",
                    "dist/web",
                ],
                &args,
            )
        }
        Task::DevRelay { args } => {
            build_web(&root, false)?;
            run_cargo(
                &root,
                &[
                    "run",
                    "-p",
                    "agent-remote-relay",
                    "--",
                    "serve",
                    "--web-dir",
                    "dist/web",
                    "--dev-insecure",
                ],
                &args,
            )
        }
        Task::ProviderSmoke => run_cargo(
            &root,
            &["run", "-p", "agent-remote-host", "--", "provider-smoke"],
            &[],
        ),
    }
}

fn build(root: &Path) -> Result<()> {
    build_web(root, true)?;
    run(
        root,
        "cargo",
        &[
            "build",
            "--release",
            "-p",
            "agent-remote-host",
            "-p",
            "agent-remote-relay",
        ],
    )?;
    let bin_dir = root.join("dist/bin");
    fs::create_dir_all(&bin_dir)?;
    for binary in ["agent-remote-host", "agent-remote-relay"] {
        let source = root.join("target/release").join(executable_name(binary));
        publish_binary(&source, &bin_dir.join(executable_name(binary)))?;
    }
    println!("Release package: {}", root.join("dist").display());
    Ok(())
}

fn publish_binary(source: &Path, destination: &Path) -> Result<()> {
    match fs::copy(source, destination) {
        Ok(_) => Ok(()),
        #[cfg(unix)]
        Err(copy_error) => {
            let staged = destination.with_extension(format!("next-{}", std::process::id()));
            fs::copy(source, &staged).with_context(|| {
                format!(
                    "stage {} after direct copy failed: {copy_error}",
                    source.display()
                )
            })?;
            fs::rename(&staged, destination)
                .with_context(|| format!("publish {}", destination.display()))?;
            Ok(())
        }
        #[cfg(not(unix))]
        Err(error) => Err(error).with_context(|| format!("copy {}", source.display())),
    }
}

fn test(root: &Path) -> Result<()> {
    run(root, "cargo", &["fmt", "--all", "--check"])?;
    run(
        root,
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run(root, "cargo", &["test", "--workspace"])?;
    build_web(root, true)?;
    println!(
        "All automated gates passed. Real Provider smoke tests are separate: cargo xtask provider-smoke"
    );
    Ok(())
}

fn build_web(root: &Path, release: bool) -> Result<()> {
    ensure_wasm_target(root)?;
    let trunk = ensure_trunk(root)?;
    let mut command = Command::new(trunk);
    command
        .current_dir(root)
        .env_remove("NO_COLOR")
        .env("TRUNK_COLOR", "never")
        .args(["build", "web/index.html", "--dist", "dist/web"]);
    if release {
        command.arg("--release");
    }
    run_command(command, "Trunk web build")
}

fn build_android(root: &Path, release: bool) -> Result<()> {
    let android = root.join("android");
    let wrapper = android.join(if cfg!(windows) {
        "gradlew.bat"
    } else {
        "gradlew"
    });
    if !wrapper.is_file() {
        bail!("Android Gradle Wrapper is missing: {}", wrapper.display());
    }
    let task = if release {
        "assembleRelease"
    } else {
        "assembleDebug"
    };
    let mut command = Command::new(&wrapper);
    command
        .current_dir(&android)
        .args(["--no-daemon", "testDebugUnitTest", task]);
    run_command(command, "Android build")?;

    let variant = if release { "release" } else { "debug" };
    let apk_name = if release {
        "app-release-unsigned.apk"
    } else {
        "app-debug.apk"
    };
    let source = android
        .join("app/build/outputs/apk")
        .join(variant)
        .join(apk_name);
    let output = root.join("dist/android");
    fs::create_dir_all(&output)?;
    let destination_name = if release {
        "agent-remote-release-unsigned.apk"
    } else {
        "agent-remote-debug.apk"
    };
    let destination = output.join(destination_name);
    fs::copy(&source, &destination).with_context(|| format!("copy {}", source.display()))?;
    println!("Android APK: {}", destination.display());
    Ok(())
}

fn ensure_wasm_target(root: &Path) -> Result<()> {
    let output = Command::new("rustup")
        .current_dir(root)
        .args(["target", "list", "--installed"])
        .output()
        .context("run rustup target list")?;
    if !output.status.success() {
        bail!("rustup target list failed");
    }
    let installed = String::from_utf8_lossy(&output.stdout);
    if !installed
        .lines()
        .any(|line| line.trim() == "wasm32-unknown-unknown")
    {
        bail!(
            "wasm32 target is missing; run: rustup target add wasm32-unknown-unknown --toolchain stable"
        );
    }
    Ok(())
}

fn ensure_trunk(root: &Path) -> Result<PathBuf> {
    let tools_root = root.join("target/xtask-tools");
    let executable = tools_root.join("bin").join(executable_name("trunk"));
    if executable.is_file() {
        return Ok(executable);
    }
    println!(
        "Installing pinned Trunk {TRUNK_VERSION} into {}",
        tools_root.display()
    );
    let mut command = Command::new("cargo");
    command.current_dir(root).args([
        "install",
        "--locked",
        "--version",
        TRUNK_VERSION,
        "--root",
        tools_root.to_str().context("non-UTF-8 tools path")?,
        "trunk",
    ]);
    run_command(command, "install pinned Trunk")?;
    Ok(executable)
}

fn run_cargo(root: &Path, fixed: &[&str], extra: &[String]) -> Result<()> {
    let mut command = Command::new("cargo");
    command.current_dir(root).args(fixed).args(extra);
    run_command(command, "cargo task")
}

fn run(root: &Path, program: &str, args: &[&str]) -> Result<()> {
    let mut command = Command::new(program);
    command.current_dir(root).args(args);
    run_command(command, program)
}

fn run_command(mut command: Command, label: &str) -> Result<()> {
    let status = command.status().with_context(|| format!("start {label}"))?;
    if !status.success() {
        bail!("{label} failed with {status}");
    }
    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Ok(manifest
        .parent()
        .and_then(Path::parent)
        .context("xtask must live under crates/xtask")?
        .to_path_buf())
}

fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}
