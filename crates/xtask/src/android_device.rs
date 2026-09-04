use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::{Value, json};

const PACKAGE: &str = "dev.agentremote.messenger";
const ACTIVITY: &str = "dev.agentremote.messenger/.MainActivity";
const UI_DUMP_PATH: &str = "/data/local/tmp/general-agent-remote-window.xml";
const DEFAULT_PORT: u16 = 7437;
const DEFAULT_WAIT_SECONDS: u64 = 15;
const DEFAULT_LOG_SECONDS: u64 = 15;
const SCENARIO_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Args)]
pub(crate) struct AndroidDeviceArgs {
    /// Target device from `adb devices`. Required when more than one device is connected.
    #[arg(long, global = true)]
    pub(crate) serial: Option<String>,
    /// Emit one JSON object on stdout. Failures use exit status 1 and `{ "ok": false, ... }`.
    #[arg(long, global = true)]
    pub(crate) json: bool,
    #[command(subcommand)]
    command: AndroidDeviceCommand,
}

#[derive(Debug, Subcommand)]
enum AndroidDeviceCommand {
    /// Check adb, select one ready device, and inspect the installed app.
    Doctor,
    /// Build, install, reverse the Host port, and launch the debug app.
    Prepare {
        /// Uninstall the app first so credentials and cached state start empty.
        #[arg(long)]
        fresh: bool,
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// Save a screenshot, UI hierarchy, and display metadata.
    Inspect {
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Interact only with explicit Compose test tags/resource IDs.
    #[command(subcommand)]
    Ui(AndroidUiCommand),
    /// Run a bounded, named device workflow.
    Scenario {
        #[arg(long, value_enum)]
        name: ScenarioName,
        #[arg(long, value_enum, default_value = "mock")]
        mode: ScenarioMode,
    },
    /// Capture app logcat output for a bounded duration.
    Logs {
        #[arg(long, default_value_t = DEFAULT_LOG_SECONDS)]
        duration: u64,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Save a PNG screenshot.
    Capture {
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum AndroidUiCommand {
    /// Print the currently visible accessibility hierarchy.
    Dump,
    /// Tap the center of one exact stable ID.
    Click {
        #[arg(long)]
        id: String,
    },
    /// Focus one exact stable ID, replace its visible text, and type a value.
    Text {
        #[arg(long)]
        id: String,
        #[arg(long)]
        value: String,
    },
    /// Poll one exact stable ID until it reaches the requested state.
    Wait {
        #[arg(long)]
        id: String,
        #[arg(long, value_enum)]
        state: UiWaitState,
        #[arg(long, default_value_t = DEFAULT_WAIT_SECONDS)]
        timeout: u64,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum UiWaitState {
    Visible,
    Gone,
    Enabled,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ScenarioMode {
    Mock,
    Real,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ScenarioName {
    ProjectTree,
    Send,
    Reconnect,
    Layout,
    SendLatency,
}

impl AndroidDeviceArgs {
    pub(crate) fn command_name(&self) -> &'static str {
        match &self.command {
            AndroidDeviceCommand::Doctor => "doctor",
            AndroidDeviceCommand::Prepare { .. } => "prepare",
            AndroidDeviceCommand::Inspect { .. } => "inspect",
            AndroidDeviceCommand::Ui(AndroidUiCommand::Dump) => "ui dump",
            AndroidDeviceCommand::Ui(AndroidUiCommand::Click { .. }) => "ui click",
            AndroidDeviceCommand::Ui(AndroidUiCommand::Text { .. }) => "ui text",
            AndroidDeviceCommand::Ui(AndroidUiCommand::Wait { .. }) => "ui wait",
            AndroidDeviceCommand::Scenario { .. } => "scenario",
            AndroidDeviceCommand::Logs { .. } => "logs",
            AndroidDeviceCommand::Capture { .. } => "capture",
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct CommandReport {
    ok: bool,
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    serial: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    artifacts: Vec<String>,
    details: Value,
}

impl CommandReport {
    fn new(command: &str, serial: Option<&str>, details: Value) -> Self {
        Self {
            ok: true,
            command: command.to_owned(),
            serial: serial.map(str::to_owned),
            artifacts: Vec::new(),
            details,
        }
    }

    fn with_artifacts(mut self, artifacts: impl IntoIterator<Item = PathBuf>) -> Self {
        self.artifacts = artifacts
            .into_iter()
            .map(|path| path.display().to_string())
            .collect();
        self
    }

    pub(crate) fn emit(&self, json_output: bool) -> Result<()> {
        if json_output {
            println!("{}", serde_json::to_string(self)?);
            return Ok(());
        }
        println!("PASS android-device {}", self.command);
        if let Some(serial) = &self.serial {
            println!("device: {serial}");
        }
        for artifact in &self.artifacts {
            println!("artifact: {artifact}");
        }
        if self.details != Value::Null {
            println!("{}", serde_json::to_string_pretty(&self.details)?);
        }
        Ok(())
    }
}

pub(crate) fn emit_error(command: &str, error: &anyhow::Error) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string(&json!({
            "ok": false,
            "command": command,
            "error": format!("{error:#}"),
        }))?
    );
    Ok(())
}

#[derive(Debug)]
struct AdbOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

trait AdbRunner {
    fn output(&mut self, args: &[String]) -> Result<AdbOutput>;

    fn output_for(&mut self, args: &[String], duration: Duration) -> Result<AdbOutput>;
}

struct SystemAdb {
    program: String,
}

impl SystemAdb {
    fn from_environment() -> Self {
        Self {
            program: env::var("ADB").unwrap_or_else(|_| "adb".to_owned()),
        }
    }

    fn command(&self, args: &[String]) -> Command {
        let mut command = Command::new(&self.program);
        command.args(args);
        command
    }
}

impl AdbRunner for SystemAdb {
    fn output(&mut self, args: &[String]) -> Result<AdbOutput> {
        let output = self
            .command(args)
            .output()
            .with_context(|| format!("start {}", self.program))?;
        Ok(AdbOutput {
            success: output.status.success(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    fn output_for(&mut self, args: &[String], duration: Duration) -> Result<AdbOutput> {
        let mut command = self.command(args);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .with_context(|| format!("start {}", self.program))?;
        let deadline = Instant::now() + duration;
        loop {
            if child.try_wait()?.is_some() {
                let output = child.wait_with_output()?;
                return Ok(AdbOutput {
                    success: output.status.success(),
                    stdout: output.stdout,
                    stderr: output.stderr,
                });
            }
            if Instant::now() >= deadline {
                child.kill().context("stop adb logcat")?;
                let output = child.wait_with_output()?;
                return Ok(AdbOutput {
                    success: true,
                    stdout: output.stdout,
                    stderr: output.stderr,
                });
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct DeviceEntry {
    serial: String,
    state: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    properties: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
struct Bounds {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl Bounds {
    fn center(self) -> Result<(i32, i32)> {
        if self.right <= self.left || self.bottom <= self.top {
            bail!("stable ID has empty bounds")
        }
        Ok(((self.left + self.right) / 2, (self.top + self.bottom) / 2))
    }

    fn contains(self, other: Self) -> bool {
        other.left >= self.left
            && other.top >= self.top
            && other.right <= self.right
            && other.bottom <= self.bottom
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct UiNode {
    resource_id: String,
    content_description: String,
    text: String,
    class: String,
    enabled: bool,
    clickable: bool,
    bounds: Bounds,
}

pub(crate) fn run(
    root: &Path,
    args: &AndroidDeviceArgs,
    build_android: &mut dyn FnMut() -> Result<()>,
) -> Result<CommandReport> {
    let mut adb = SystemAdb::from_environment();
    run_with(root, args, build_android, &mut adb)
}

fn run_with(
    root: &Path,
    args: &AndroidDeviceArgs,
    build_android: &mut dyn FnMut() -> Result<()>,
    adb: &mut dyn AdbRunner,
) -> Result<CommandReport> {
    match &args.command {
        AndroidDeviceCommand::Doctor => doctor(adb, args.serial.as_deref()),
        AndroidDeviceCommand::Prepare { fresh, port } => prepare(
            root,
            adb,
            args.serial.as_deref(),
            *fresh,
            *port,
            build_android,
        ),
        AndroidDeviceCommand::Inspect { output } => {
            inspect(root, adb, args.serial.as_deref(), output.as_deref())
        }
        AndroidDeviceCommand::Ui(command) => ui(adb, args.serial.as_deref(), command),
        AndroidDeviceCommand::Scenario { name, mode } => {
            scenario(root, adb, args.serial.as_deref(), *name, *mode)
        }
        AndroidDeviceCommand::Logs { duration, output } => logs(
            root,
            adb,
            args.serial.as_deref(),
            *duration,
            output.as_deref(),
        ),
        AndroidDeviceCommand::Capture { output } => {
            capture(root, adb, args.serial.as_deref(), output.as_deref())
        }
    }
}

fn checked(adb: &mut dyn AdbRunner, args: Vec<String>, label: &str) -> Result<Vec<u8>> {
    let output = adb.output(&args).with_context(|| label.to_owned())?;
    if !output.success {
        bail!("{label} failed: {}", adb_failure_detail(&output));
    }
    Ok(output.stdout)
}

fn checked_for(
    adb: &mut dyn AdbRunner,
    args: Vec<String>,
    duration: Duration,
    label: &str,
) -> Result<Vec<u8>> {
    let output = adb
        .output_for(&args, duration)
        .with_context(|| label.to_owned())?;
    if !output.success {
        bail!("{label} failed: {}", adb_failure_detail(&output));
    }
    Ok(output.stdout)
}

fn adb_failure_detail(output: &AdbOutput) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = [stdout.trim(), stderr.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if detail.is_empty() {
        "adb returned a non-zero status".to_owned()
    } else {
        detail
    }
}

fn serial_args(serial: &str, command: &[&str]) -> Vec<String> {
    let mut args = vec!["-s".to_owned(), serial.to_owned()];
    args.extend(command.iter().map(|value| (*value).to_owned()));
    args
}

fn parse_devices(output: &str) -> Vec<DeviceEntry> {
    output
        .lines()
        .skip_while(|line| !line.starts_with("List of devices attached"))
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            let state = fields.next()?;
            let properties = fields
                .filter_map(|field| field.split_once(':'))
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect();
            Some(DeviceEntry {
                serial: serial.to_owned(),
                state: state.to_owned(),
                properties,
            })
        })
        .collect()
}

fn list_devices(adb: &mut dyn AdbRunner) -> Result<Vec<DeviceEntry>> {
    let bytes = checked(adb, vec!["devices".into(), "-l".into()], "adb devices")?;
    Ok(parse_devices(&String::from_utf8_lossy(&bytes)))
}

fn resolve_serial(
    adb: &mut dyn AdbRunner,
    requested: Option<&str>,
) -> Result<(String, Vec<DeviceEntry>)> {
    let devices = list_devices(adb)?;
    if let Some(serial) = requested {
        let entry = devices
            .iter()
            .find(|device| device.serial == serial)
            .ok_or_else(|| anyhow!("device {serial:?} was not reported by adb"))?;
        if entry.state != "device" {
            bail!("device {serial:?} is {}", entry.state);
        }
        return Ok((serial.to_owned(), devices));
    }
    let ready = devices
        .iter()
        .filter(|device| device.state == "device")
        .collect::<Vec<_>>();
    match ready.as_slice() {
        [device] => Ok((device.serial.clone(), devices)),
        [] if devices.is_empty() => bail!("no Android device is connected"),
        [] => bail!(
            "no ready Android device; adb states: {}",
            devices
                .iter()
                .map(|device| format!("{}={}", device.serial, device.state))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => bail!("multiple Android devices are ready; pass --serial <id>"),
    }
}

fn doctor(adb: &mut dyn AdbRunner, requested: Option<&str>) -> Result<CommandReport> {
    let version = checked(adb, vec!["version".into()], "adb version")?;
    let (serial, devices) = resolve_serial(adb, requested)?;
    let state = checked(
        adb,
        serial_args(&serial, &["get-state"]),
        "read device state",
    )?;
    if String::from_utf8_lossy(&state).trim() != "device" {
        bail!("device {serial:?} did not report the ready state");
    }
    let manufacturer = shell_text(adb, &serial, &["getprop", "ro.product.manufacturer"])?;
    let model = shell_text(adb, &serial, &["getprop", "ro.product.model"])?;
    let sdk = shell_text(adb, &serial, &["getprop", "ro.build.version.sdk"])?;
    let package_path = shell_text_allow_failure(adb, &serial, &["pm", "path", PACKAGE])?;
    let installed = package_path.starts_with("package:");
    let adb_version = String::from_utf8_lossy(&version)
        .lines()
        .next()
        .unwrap_or("unknown")
        .trim()
        .to_owned();
    Ok(CommandReport::new(
        "doctor",
        Some(&serial),
        json!({
            "adbVersion": adb_version,
            "manufacturer": manufacturer,
            "model": model,
            "sdk": sdk,
            "appInstalled": installed,
            "package": PACKAGE,
            "devices": devices,
        }),
    ))
}

fn prepare(
    root: &Path,
    adb: &mut dyn AdbRunner,
    requested: Option<&str>,
    fresh: bool,
    port: u16,
    build_android: &mut dyn FnMut() -> Result<()>,
) -> Result<CommandReport> {
    if port == 0 {
        bail!("--port must be between 1 and 65535");
    }
    let (serial, _) = resolve_serial(adb, requested)?;
    build_android().context("build Android debug APK")?;
    let apk = root.join("dist/android/agent-remote-debug.apk");
    if !apk.is_file() {
        bail!("Android build did not produce {}", apk.display());
    }

    let installed =
        shell_text_allow_failure(adb, &serial, &["pm", "path", PACKAGE])?.starts_with("package:");
    if fresh && installed {
        checked(
            adb,
            serial_args(&serial, &["uninstall", PACKAGE]),
            "uninstall existing Android app",
        )?;
    }
    let install_args = vec![
        "-s".to_owned(),
        serial.clone(),
        "install".to_owned(),
        "-r".to_owned(),
        apk.display().to_string(),
    ];
    checked(adb, install_args, "install Android debug APK")?;
    let port_mapping = format!("tcp:{port}");
    checked(
        adb,
        vec![
            "-s".to_owned(),
            serial.clone(),
            "reverse".to_owned(),
            port_mapping.clone(),
            port_mapping.clone(),
        ],
        "configure adb reverse",
    )?;
    checked(
        adb,
        serial_args(&serial, &["shell", "am", "start", "-W", "-n", ACTIVITY]),
        "launch Android app",
    )?;
    Ok(CommandReport::new(
        "prepare",
        Some(&serial),
        json!({
            "fresh": fresh,
            "port": port,
            "package": PACKAGE,
            "activity": ACTIVITY,
        }),
    )
    .with_artifacts([apk]))
}

fn shell_text(adb: &mut dyn AdbRunner, serial: &str, command: &[&str]) -> Result<String> {
    let mut parts = vec!["shell"];
    parts.extend(command);
    let bytes = checked(adb, serial_args(serial, &parts), "run Android shell query")?;
    Ok(String::from_utf8_lossy(&bytes).trim().to_owned())
}

fn shell_text_allow_failure(
    adb: &mut dyn AdbRunner,
    serial: &str,
    command: &[&str],
) -> Result<String> {
    let mut parts = vec!["shell"];
    parts.extend(command);
    let output = adb
        .output(&serial_args(serial, &parts))
        .context("run Android shell query")?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn resolve_output(root: &Path, supplied: Option<&Path>, default: &str) -> PathBuf {
    match supplied {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => root.join(path),
        None => root.join(default),
    }
}

fn ensure_output(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))
}

fn screenshot(adb: &mut dyn AdbRunner, serial: &str) -> Result<Vec<u8>> {
    let bytes = checked(
        adb,
        serial_args(serial, &["exec-out", "screencap", "-p"]),
        "capture Android screenshot",
    )?;
    if bytes.is_empty() {
        bail!("adb returned an empty screenshot");
    }
    Ok(bytes)
}

fn dump_ui_xml(adb: &mut dyn AdbRunner, serial: &str) -> Result<String> {
    checked(
        adb,
        serial_args(serial, &["shell", "uiautomator", "dump", UI_DUMP_PATH]),
        "dump Android UI hierarchy",
    )?;
    let bytes = checked(
        adb,
        serial_args(serial, &["exec-out", "cat", UI_DUMP_PATH]),
        "read Android UI hierarchy",
    )?;
    let xml = String::from_utf8(bytes).context("Android UI hierarchy was not UTF-8")?;
    if !xml.contains("<hierarchy") {
        bail!("adb returned an invalid UI hierarchy");
    }
    Ok(xml)
}

fn inspect(
    root: &Path,
    adb: &mut dyn AdbRunner,
    requested: Option<&str>,
    supplied_output: Option<&Path>,
) -> Result<CommandReport> {
    let (serial, _) = resolve_serial(adb, requested)?;
    let output = resolve_output(root, supplied_output, "dist/android-device/inspect");
    ensure_output(&output)?;
    let screen_path = output.join("screen.png");
    let hierarchy_path = output.join("window.xml");
    let metadata_path = output.join("device.json");
    let screen = screenshot(adb, &serial)?;
    let xml = dump_ui_xml(adb, &serial)?;
    let nodes = parse_ui_nodes(&xml)?;
    let size = shell_text(adb, &serial, &["wm", "size"])?;
    let density = shell_text(adb, &serial, &["wm", "density"])?;
    fs::write(&screen_path, screen).with_context(|| format!("write {}", screen_path.display()))?;
    fs::write(&hierarchy_path, xml)
        .with_context(|| format!("write {}", hierarchy_path.display()))?;
    let metadata = json!({
        "serial": serial,
        "size": size,
        "density": density,
        "visibleNodeCount": nodes.len(),
    });
    fs::write(&metadata_path, serde_json::to_vec_pretty(&metadata)?)
        .with_context(|| format!("write {}", metadata_path.display()))?;
    Ok(
        CommandReport::new("inspect", Some(&serial), metadata).with_artifacts([
            screen_path,
            hierarchy_path,
            metadata_path,
        ]),
    )
}

fn capture(
    root: &Path,
    adb: &mut dyn AdbRunner,
    requested: Option<&str>,
    supplied_output: Option<&Path>,
) -> Result<CommandReport> {
    let (serial, _) = resolve_serial(adb, requested)?;
    let output = resolve_output(root, supplied_output, "dist/android-device/capture");
    ensure_output(&output)?;
    let path = output.join("screen.png");
    fs::write(&path, screenshot(adb, &serial)?)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(
        CommandReport::new("capture", Some(&serial), json!({ "package": PACKAGE }))
            .with_artifacts([path]),
    )
}

fn parse_ui_nodes(xml: &str) -> Result<Vec<UiNode>> {
    let mut nodes = Vec::new();
    let mut remaining = xml;
    while let Some(start) = remaining.find("<node ") {
        remaining = &remaining[start + 6..];
        let end = remaining
            .find('>')
            .ok_or_else(|| anyhow!("malformed UI hierarchy node"))?;
        let tag = &remaining[..end];
        nodes.push(UiNode {
            resource_id: xml_attribute(tag, "resource-id"),
            content_description: xml_attribute(tag, "content-desc"),
            text: xml_attribute(tag, "text"),
            class: xml_attribute(tag, "class"),
            enabled: xml_attribute(tag, "enabled") == "true",
            clickable: xml_attribute(tag, "clickable") == "true",
            bounds: parse_bounds(&xml_attribute(tag, "bounds"))?,
        });
        remaining = &remaining[end + 1..];
    }
    Ok(nodes)
}

fn xml_attribute(tag: &str, name: &str) -> String {
    let marker = format!(" {name}=\"");
    let Some(start) = tag.find(&marker) else {
        return String::new();
    };
    let value = &tag[start + marker.len()..];
    let Some(end) = value.find('"') else {
        return String::new();
    };
    unescape_xml(&value[..end])
}

fn unescape_xml(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn parse_bounds(value: &str) -> Result<Bounds> {
    let numbers = value
        .split(|character: char| !character.is_ascii_digit() && character != '-')
        .filter(|part| !part.is_empty())
        .map(str::parse::<i32>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    match numbers.as_slice() {
        [left, top, right, bottom] => Ok(Bounds {
            left: *left,
            top: *top,
            right: *right,
            bottom: *bottom,
        }),
        _ => bail!("invalid UI bounds {value:?}"),
    }
}

fn resource_id_matches(resource_id: &str, stable_id: &str) -> bool {
    resource_id == stable_id
        || resource_id
            .rsplit_once("/")
            .is_some_and(|(_, suffix)| suffix == stable_id)
}

fn node_matches(node: &UiNode, stable_id: &str) -> bool {
    resource_id_matches(&node.resource_id, stable_id)
        || node
            .content_description
            .split(',')
            .map(str::trim)
            .any(|value| value == stable_id)
}

fn find_node<'a>(nodes: &'a [UiNode], stable_id: &str) -> Result<&'a UiNode> {
    let matches = nodes
        .iter()
        .filter(|node| node_matches(node, stable_id))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [node] => Ok(*node),
        [] => bail!("stable UI ID {stable_id:?} is not visible"),
        _ => bail!("stable UI ID {stable_id:?} is not unique"),
    }
}

fn tap(adb: &mut dyn AdbRunner, serial: &str, node: &UiNode) -> Result<()> {
    let (x, y) = node.bounds.center()?;
    checked(
        adb,
        serial_args(
            serial,
            &["shell", "input", "tap", &x.to_string(), &y.to_string()],
        ),
        "tap stable Android UI target",
    )?;
    Ok(())
}

fn click_id(adb: &mut dyn AdbRunner, serial: &str, stable_id: &str) -> Result<UiNode> {
    let xml = dump_ui_xml(adb, serial)?;
    let nodes = parse_ui_nodes(&xml)?;
    let node = find_node(&nodes, stable_id)?.clone();
    if !node.enabled {
        bail!("stable UI ID {stable_id:?} is disabled");
    }
    tap(adb, serial, &node)?;
    Ok(node)
}

fn encode_adb_input_text(value: &str) -> String {
    let mut encoded = String::new();
    for character in value.chars() {
        match character {
            ' ' => encoded.push_str("%s"),
            '\n' => encoded.push_str("\\n"),
            '\t' => encoded.push_str("\\t"),
            '&' | '<' | '>' | '|' | ';' | '(' | ')' | '$' | '`' | '\\' | '"' | '\'' | '*' | '?'
            | '[' | ']' | '{' | '}' | '!' | '#' => {
                encoded.push('\\');
                encoded.push(character);
            }
            _ => encoded.push(character),
        }
    }
    encoded
}

fn replace_text(
    adb: &mut dyn AdbRunner,
    serial: &str,
    stable_id: &str,
    value: &str,
) -> Result<UiNode> {
    let xml = dump_ui_xml(adb, serial)?;
    let nodes = parse_ui_nodes(&xml)?;
    let node = find_node(&nodes, stable_id)?.clone();
    if !node.enabled {
        bail!("stable UI ID {stable_id:?} is disabled");
    }
    tap(adb, serial, &node)?;
    if !node.text.is_empty() {
        checked(
            adb,
            serial_args(serial, &["shell", "input", "keyevent", "KEYCODE_MOVE_END"]),
            "move Android text cursor",
        )?;
        let mut command = vec!["shell", "input", "keyevent"];
        command.extend(std::iter::repeat_n(
            "KEYCODE_DEL",
            node.text.chars().count(),
        ));
        checked(
            adb,
            serial_args(serial, &command),
            "clear Android text field",
        )?;
    }
    if !value.is_empty() {
        let encoded = encode_adb_input_text(value);
        checked(
            adb,
            serial_args(serial, &["shell", "input", "text", &encoded]),
            "type Android text",
        )?;
    }
    Ok(node)
}

fn wait_for_node(
    adb: &mut dyn AdbRunner,
    serial: &str,
    stable_id: &str,
    state: UiWaitState,
    timeout: Duration,
) -> Result<(Option<UiNode>, Duration)> {
    let started = Instant::now();
    loop {
        let xml = dump_ui_xml(adb, serial)?;
        let nodes = parse_ui_nodes(&xml)?;
        let matches = nodes
            .iter()
            .filter(|node| node_matches(node, stable_id))
            .cloned()
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            bail!("stable UI ID {stable_id:?} is not unique");
        }
        let node = matches.into_iter().next();
        let reached = match state {
            UiWaitState::Visible => node.is_some(),
            UiWaitState::Gone => node.is_none(),
            UiWaitState::Enabled => node.as_ref().is_some_and(|node| node.enabled),
        };
        if reached {
            return Ok((node, started.elapsed()));
        }
        if started.elapsed() >= timeout {
            bail!(
                "timed out after {:.1}s waiting for {stable_id:?} to become {}",
                timeout.as_secs_f64(),
                wait_state_name(state)
            );
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn wait_state_name(state: UiWaitState) -> &'static str {
    match state {
        UiWaitState::Visible => "visible",
        UiWaitState::Gone => "gone",
        UiWaitState::Enabled => "enabled",
    }
}

fn ui(
    adb: &mut dyn AdbRunner,
    requested: Option<&str>,
    command: &AndroidUiCommand,
) -> Result<CommandReport> {
    let (serial, _) = resolve_serial(adb, requested)?;
    match command {
        AndroidUiCommand::Dump => {
            let nodes = parse_ui_nodes(&dump_ui_xml(adb, &serial)?)?;
            let stable_ids = nodes
                .iter()
                .filter(|node| !node.resource_id.is_empty())
                .map(|node| node.resource_id.clone())
                .collect::<BTreeSet<_>>();
            Ok(CommandReport::new(
                "ui dump",
                Some(&serial),
                json!({
                    "nodeCount": nodes.len(),
                    "stableIds": stable_ids,
                    "nodes": nodes,
                }),
            ))
        }
        AndroidUiCommand::Click { id } => {
            let node = click_id(adb, &serial, id)?;
            Ok(CommandReport::new(
                "ui click",
                Some(&serial),
                json!({ "id": id, "bounds": node.bounds }),
            ))
        }
        AndroidUiCommand::Text { id, value } => {
            let node = replace_text(adb, &serial, id, value)?;
            Ok(CommandReport::new(
                "ui text",
                Some(&serial),
                json!({
                    "id": id,
                    "bounds": node.bounds,
                    "characterCount": value.chars().count(),
                }),
            ))
        }
        AndroidUiCommand::Wait { id, state, timeout } => {
            let (node, elapsed) =
                wait_for_node(adb, &serial, id, *state, Duration::from_secs(*timeout))?;
            Ok(CommandReport::new(
                "ui wait",
                Some(&serial),
                json!({
                    "id": id,
                    "state": wait_state_name(*state),
                    "elapsedMs": elapsed.as_millis(),
                    "node": node,
                }),
            ))
        }
    }
}

fn logs(
    root: &Path,
    adb: &mut dyn AdbRunner,
    requested: Option<&str>,
    duration_seconds: u64,
    supplied_output: Option<&Path>,
) -> Result<CommandReport> {
    let (serial, _) = resolve_serial(adb, requested)?;
    let output = resolve_output(root, supplied_output, "dist/android-device/logs");
    ensure_output(&output)?;
    let path = output.join("logcat.txt");
    let pid = shell_text_allow_failure(adb, &serial, &["pidof", PACKAGE])?;
    let mut command = serial_args(&serial, &["logcat", "-v", "threadtime"]);
    if let Some(pid) = pid
        .split_whitespace()
        .next()
        .filter(|value| !value.is_empty())
    {
        command.push(format!("--pid={pid}"));
    } else {
        command.extend([
            "GAR.SendTrace:V".to_owned(),
            "AndroidRuntime:E".to_owned(),
            "*:S".to_owned(),
        ]);
    }
    let bytes = if duration_seconds == 0 {
        command.push("-d".to_owned());
        checked(adb, command, "capture Android logs")?
    } else {
        checked_for(
            adb,
            command,
            Duration::from_secs(duration_seconds),
            "capture Android logs",
        )?
    };
    fs::write(&path, &bytes).with_context(|| format!("write {}", path.display()))?;
    Ok(CommandReport::new(
        "logs",
        Some(&serial),
        json!({
            "durationSeconds": duration_seconds,
            "byteCount": bytes.len(),
            "package": PACKAGE,
        }),
    )
    .with_artifacts([path]))
}

fn scenario(
    root: &Path,
    adb: &mut dyn AdbRunner,
    requested: Option<&str>,
    name: ScenarioName,
    mode: ScenarioMode,
) -> Result<CommandReport> {
    let (serial, _) = resolve_serial(adb, requested)?;
    match name {
        ScenarioName::ProjectTree => scenario_project_tree(root, adb, &serial, mode),
        ScenarioName::Send => scenario_send(adb, &serial, mode, false),
        ScenarioName::Reconnect => scenario_reconnect(root, adb, &serial, mode),
        ScenarioName::Layout => scenario_layout(root, adb, &serial, mode),
        ScenarioName::SendLatency => scenario_send(adb, &serial, mode, true),
    }
}

fn scenario_name(name: ScenarioName) -> &'static str {
    match name {
        ScenarioName::ProjectTree => "project-tree",
        ScenarioName::Send => "send",
        ScenarioName::Reconnect => "reconnect",
        ScenarioName::Layout => "layout",
        ScenarioName::SendLatency => "send-latency",
    }
}

fn scenario_mode(mode: ScenarioMode) -> &'static str {
    match mode {
        ScenarioMode::Mock => "mock",
        ScenarioMode::Real => "real",
    }
}

fn scenario_directory(root: &Path, name: ScenarioName) -> Result<PathBuf> {
    let directory = root
        .join("dist/android-device/scenarios")
        .join(scenario_name(name));
    ensure_output(&directory)?;
    Ok(directory)
}

fn save_scenario_state(
    adb: &mut dyn AdbRunner,
    serial: &str,
    directory: &Path,
    stem: &str,
) -> Result<(PathBuf, PathBuf, Vec<UiNode>)> {
    let screen_path = directory.join(format!("{stem}.png"));
    let xml_path = directory.join(format!("{stem}.xml"));
    let xml = dump_ui_xml(adb, serial)?;
    let nodes = parse_ui_nodes(&xml)?;
    fs::write(&xml_path, xml).with_context(|| format!("write {}", xml_path.display()))?;
    fs::write(&screen_path, screenshot(adb, serial)?)
        .with_context(|| format!("write {}", screen_path.display()))?;
    Ok((screen_path, xml_path, nodes))
}

fn save_scenario_state_from_dump(
    adb: &mut dyn AdbRunner,
    serial: &str,
    directory: &Path,
    stem: &str,
    xml: &str,
    nodes: Vec<UiNode>,
) -> Result<(PathBuf, PathBuf, Vec<UiNode>)> {
    let screen_path = directory.join(format!("{stem}.png"));
    let xml_path = directory.join(format!("{stem}.xml"));
    fs::write(&xml_path, xml).with_context(|| format!("write {}", xml_path.display()))?;
    fs::write(&screen_path, screenshot(adb, serial)?)
        .with_context(|| format!("write {}", screen_path.display()))?;
    Ok((screen_path, xml_path, nodes))
}

fn stable_resource_id(node: &UiNode) -> &str {
    node.resource_id
        .rsplit_once('/')
        .map_or(node.resource_id.as_str(), |(_, suffix)| suffix)
}

fn stable_ids_with_prefix(nodes: &[UiNode], prefix: &str) -> Vec<String> {
    nodes
        .iter()
        .map(stable_resource_id)
        .filter(|resource| resource.starts_with(prefix))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn project_conversation_ids(nodes: &[UiNode], project_id: &str) -> Vec<String> {
    let mut inside_project = false;
    let mut conversations = Vec::new();
    for node in nodes {
        let resource_id = stable_resource_id(node);
        if resource_id.starts_with("gar.project.") && !resource_id.ends_with(".toggle") {
            if inside_project {
                break;
            }
            inside_project = resource_id == project_id;
            continue;
        }
        if inside_project && resource_id.starts_with("gar.conversation.") {
            conversations.push(resource_id.to_owned());
        }
    }
    conversations
}

fn project_tree_signature(nodes: &[UiNode], project_id: &str) -> Vec<(String, Bounds)> {
    let toggle_id = format!("{project_id}.toggle");
    let conversation_ids = project_conversation_ids(nodes, project_id)
        .into_iter()
        .collect::<BTreeSet<_>>();
    nodes
        .iter()
        .filter_map(|node| {
            let resource_id = stable_resource_id(node);
            (resource_id == project_id
                || resource_id == toggle_id
                || conversation_ids.contains(resource_id))
            .then(|| (resource_id.to_owned(), node.bounds))
        })
        .collect()
}

fn wait_for_project_children(
    adb: &mut dyn AdbRunner,
    serial: &str,
    project_id: &str,
    expected_visible: bool,
    timeout: Duration,
) -> Result<(String, Vec<UiNode>, Duration)> {
    let started = Instant::now();
    let toggle_id = format!("{project_id}.toggle");
    let mut previous_signature = None;
    loop {
        let xml = dump_ui_xml(adb, serial)?;
        let nodes = parse_ui_nodes(&xml)?;
        find_node(&nodes, &toggle_id)?;
        let conversations = project_conversation_ids(&nodes, project_id);
        let reached = conversations.is_empty() != expected_visible;
        let signature = project_tree_signature(&nodes, project_id);
        if reached && previous_signature.as_ref() == Some(&signature) {
            return Ok((xml, nodes, started.elapsed()));
        }
        previous_signature = reached.then_some(signature);
        if started.elapsed() >= timeout {
            let expected = if expected_visible { "visible" } else { "gone" };
            bail!(
                "timed out after {:.1}s waiting for {project_id:?} conversation children to become {expected} and settle",
                timeout.as_secs_f64()
            );
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn scenario_project_tree(
    root: &Path,
    adb: &mut dyn AdbRunner,
    serial: &str,
    mode: ScenarioMode,
) -> Result<CommandReport> {
    let directory = scenario_directory(root, ScenarioName::ProjectTree)?;
    let initial = parse_ui_nodes(&dump_ui_xml(adb, serial)?)?;
    if stable_ids_with_prefix(&initial, "gar.project.").is_empty() {
        click_id(adb, serial, "gar.drawer.open")?;
    }
    let (before_screen, before_xml, before_nodes) =
        save_scenario_state(adb, serial, &directory, "tree-before")?;
    let project_ids = stable_ids_with_prefix(&before_nodes, "gar.project.")
        .into_iter()
        .filter(|id| !id.ends_with(".toggle"))
        .collect::<Vec<_>>();
    let selected_project = project_ids
        .first()
        .ok_or_else(|| anyhow!("project tree exposes no gar.project.<projectId> node"))?
        .clone();
    let children_before = project_conversation_ids(&before_nodes, &selected_project);
    let expected_children_visible = children_before.is_empty();
    let toggle_id = format!("{selected_project}.toggle");
    click_id(adb, serial, &toggle_id)?;
    let (settled_xml, settled_nodes, settled_after) = wait_for_project_children(
        adb,
        serial,
        &selected_project,
        expected_children_visible,
        SCENARIO_TIMEOUT,
    )?;
    let (after_screen, after_xml, after_nodes) = save_scenario_state_from_dump(
        adb,
        serial,
        &directory,
        "tree-after",
        &settled_xml,
        settled_nodes,
    )?;
    let conversations = project_conversation_ids(&after_nodes, &selected_project);
    Ok(CommandReport::new(
        "scenario",
        Some(serial),
        json!({
            "name": scenario_name(ScenarioName::ProjectTree),
            "mode": scenario_mode(mode),
            "projectCount": project_ids.len(),
            "selectedProjectId": selected_project,
            "toggleId": toggle_id,
            "expectedConversationChildrenVisible": expected_children_visible,
            "settledAfterMs": settled_after.as_millis(),
            "visibleConversationCountAfterToggle": conversations.len(),
            "conversationIds": conversations,
        }),
    )
    .with_artifacts([before_screen, before_xml, after_screen, after_xml]))
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SendTraceEvent {
    command_id: String,
    client_message_id: String,
    stage: String,
    elapsed_ms: u64,
}

fn logcat_send_trace(adb: &mut dyn AdbRunner, serial: &str) -> Result<Vec<String>> {
    let bytes = checked(
        adb,
        serial_args(
            serial,
            &["logcat", "-d", "-v", "raw", "GAR.SendTrace:I", "*:S"],
        ),
        "read Android send trace",
    )?;
    Ok(String::from_utf8_lossy(&bytes)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

fn parse_send_trace(line: &str) -> Option<SendTraceEvent> {
    let payload = &line[line.find('{')?..];
    let value: Value = serde_json::from_str(payload).ok()?;
    let string = |camel: &str, snake: &str| {
        value
            .get(camel)
            .or_else(|| value.get(snake))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let number = |camel: &str, snake: &str| {
        value
            .get(camel)
            .or_else(|| value.get(snake))
            .and_then(Value::as_u64)
    };
    Some(SendTraceEvent {
        command_id: string("commandId", "command_id")?,
        client_message_id: string("clientMessageId", "client_message_id")?,
        stage: string("stage", "stage")?,
        elapsed_ms: number("elapsedMs", "elapsed_ms")?,
    })
}

fn wait_for_send_trace(
    adb: &mut dyn AdbRunner,
    serial: &str,
    baseline: &BTreeSet<String>,
    timeout: Duration,
) -> Result<Vec<SendTraceEvent>> {
    let started = Instant::now();
    let mut correlation: Option<(String, String)> = None;
    loop {
        let lines = logcat_send_trace(adb, serial)?;
        let new_events = lines
            .iter()
            .filter(|line| !baseline.contains(*line))
            .filter_map(|line| parse_send_trace(line))
            .collect::<Vec<_>>();
        if correlation.is_none() {
            correlation = new_events
                .iter()
                .rev()
                .find(|event| event.stage == "click")
                .or_else(|| new_events.last())
                .map(|event| (event.command_id.clone(), event.client_message_id.clone()));
        }
        if let Some((command_id, client_message_id)) = &correlation {
            let correlated = new_events
                .iter()
                .filter(|event| {
                    event.command_id == *command_id && event.client_message_id == *client_message_id
                })
                .cloned()
                .collect::<Vec<_>>();
            if correlated
                .iter()
                .any(|event| event.stage == "first_provider_event")
            {
                return Ok(correlated);
            }
        }
        if started.elapsed() >= timeout {
            bail!(
                "send trace did not reach first_provider_event within {:.1}s",
                timeout.as_secs_f64()
            );
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn open_new_conversation_composer(
    adb: &mut dyn AdbRunner,
    serial: &str,
    timeout: Duration,
) -> Result<Duration> {
    let started = Instant::now();
    let initial = parse_ui_nodes(&dump_ui_xml(adb, serial)?)?;
    if find_node(&initial, "gar.conversation.new").is_err() {
        let drawer = match find_node(&initial, "gar.drawer.open") {
            Ok(node) if node.enabled => node.clone(),
            _ => wait_for_node(
                adb,
                serial,
                "gar.drawer.open",
                UiWaitState::Enabled,
                timeout,
            )?
            .0
            .ok_or_else(|| anyhow!("gar.drawer.open became enabled without a visible node"))?,
        };
        tap(adb, serial, &drawer)?;
    }
    let new_conversation = wait_for_node(
        adb,
        serial,
        "gar.conversation.new",
        UiWaitState::Enabled,
        timeout.saturating_sub(started.elapsed()),
    )?
    .0
    .ok_or_else(|| anyhow!("gar.conversation.new became enabled without a visible node"))?;
    tap(adb, serial, &new_conversation)?;
    wait_for_node(
        adb,
        serial,
        "gar.composer.input",
        UiWaitState::Enabled,
        timeout.saturating_sub(started.elapsed()),
    )?;
    Ok(started.elapsed())
}

fn scenario_send(
    adb: &mut dyn AdbRunner,
    serial: &str,
    mode: ScenarioMode,
    latency: bool,
) -> Result<CommandReport> {
    let input_id = "gar.composer.input";
    let send_id = "gar.composer.send";
    let composer_prepared_after = open_new_conversation_composer(adb, serial, SCENARIO_TIMEOUT)?;
    let baseline = logcat_send_trace(adb, serial)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let prompt = match mode {
        ScenarioMode::Mock => "Android device mock send check",
        ScenarioMode::Real => "Reply with OK for the Android device send check.",
    };
    replace_text(adb, serial, input_id, prompt)?;
    wait_for_node(adb, serial, send_id, UiWaitState::Enabled, SCENARIO_TIMEOUT)?;
    click_id(adb, serial, send_id)?;
    let events = wait_for_send_trace(adb, serial, &baseline, SCENARIO_TIMEOUT)?;
    let first = events
        .first()
        .ok_or_else(|| anyhow!("send trace was empty"))?;
    let elapsed_ms = events
        .iter()
        .find(|event| event.stage == "first_provider_event")
        .map(|event| event.elapsed_ms)
        .ok_or_else(|| anyhow!("send trace omitted first_provider_event"))?;
    let required_stages = [
        "click",
        "local_pending",
        "websocket_write",
        "host_received",
        "provider_received",
        "first_provider_event",
    ];
    let observed = events
        .iter()
        .map(|event| event.stage.as_str())
        .collect::<BTreeSet<_>>();
    let missing = required_stages
        .iter()
        .filter(|stage| !observed.contains(**stage))
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!("send trace omitted stages: {}", missing.join(", "));
    }
    Ok(CommandReport::new(
        "scenario",
        Some(serial),
        json!({
            "name": if latency { "send-latency" } else { "send" },
            "mode": scenario_mode(mode),
            "composerPreparedAfterMs": composer_prepared_after.as_millis(),
            "commandId": first.command_id,
            "clientMessageId": first.client_message_id,
            "firstProviderEventMs": elapsed_ms,
            "stages": events,
        }),
    ))
}

fn connection_status_is_online(nodes: &[UiNode]) -> bool {
    let Ok(status) = find_node(nodes, "gar.connection.status") else {
        return false;
    };
    let mut labels = vec![status.text.as_str(), status.content_description.as_str()];
    labels.extend(
        nodes
            .iter()
            .filter(|node| status.bounds.contains(node.bounds))
            .flat_map(|node| [node.text.as_str(), node.content_description.as_str()]),
    );
    labels.into_iter().any(|label| {
        let lower = label.to_lowercase();
        !lower.contains("离线")
            && !lower.contains("断开")
            && !lower.contains("offline")
            && !lower.contains("disconnected")
            && (lower.contains("已连接")
                || lower.contains("host 在线")
                || lower.trim() == "在线"
                || lower.contains("connected"))
    })
}

fn wait_for_online(adb: &mut dyn AdbRunner, serial: &str, timeout: Duration) -> Result<Duration> {
    let started = Instant::now();
    loop {
        let nodes = parse_ui_nodes(&dump_ui_xml(adb, serial)?)?;
        if connection_status_is_online(&nodes) {
            return Ok(started.elapsed());
        }
        if started.elapsed() >= timeout {
            bail!("app did not report an authenticated online connection after relaunch");
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn scenario_reconnect(
    root: &Path,
    adb: &mut dyn AdbRunner,
    serial: &str,
    mode: ScenarioMode,
) -> Result<CommandReport> {
    let directory = scenario_directory(root, ScenarioName::Reconnect)?;
    let (before_screen, before_xml, _) =
        save_scenario_state(adb, serial, &directory, "before-relaunch")?;
    checked(
        adb,
        serial_args(serial, &["shell", "am", "force-stop", PACKAGE]),
        "stop Android app for reconnect scenario",
    )?;
    checked(
        adb,
        serial_args(serial, &["shell", "am", "start", "-W", "-n", ACTIVITY]),
        "relaunch Android app",
    )?;
    let elapsed = wait_for_online(adb, serial, SCENARIO_TIMEOUT)?;
    let (after_screen, after_xml, _) =
        save_scenario_state(adb, serial, &directory, "after-reconnect")?;
    Ok(CommandReport::new(
        "scenario",
        Some(serial),
        json!({
            "name": scenario_name(ScenarioName::Reconnect),
            "mode": scenario_mode(mode),
            "authenticatedOnlineMs": elapsed.as_millis(),
        }),
    )
    .with_artifacts([before_screen, before_xml, after_screen, after_xml]))
}

fn hierarchy_extent(nodes: &[UiNode]) -> Result<(i32, i32)> {
    let root = nodes
        .first()
        .ok_or_else(|| anyhow!("UI hierarchy has no visible nodes"))?;
    let width = root.bounds.right - root.bounds.left;
    let height = root.bounds.bottom - root.bounds.top;
    if width <= 0 || height <= 0 {
        bail!("UI hierarchy has no visible screen bounds");
    }
    Ok((width, height))
}

fn validate_hierarchy_bounds(nodes: &[UiNode], width: i32, height: i32) -> Result<()> {
    let invalid = nodes.iter().find(|node| {
        node.bounds.left < 0
            || node.bounds.top < 0
            || node.bounds.right > width
            || node.bounds.bottom > height
    });
    if let Some(node) = invalid {
        bail!(
            "UI node {:?} exceeds {}x{} screen bounds",
            node.resource_id,
            width,
            height
        );
    }
    Ok(())
}

fn set_rotation(adb: &mut dyn AdbRunner, serial: &str, rotation: &str) -> Result<()> {
    checked(
        adb,
        serial_args(
            serial,
            &[
                "shell",
                "settings",
                "put",
                "system",
                "user_rotation",
                rotation,
            ],
        ),
        "set Android display rotation",
    )?;
    Ok(())
}

fn set_fixed_to_user_rotation(adb: &mut dyn AdbRunner, serial: &str, value: &str) -> Result<()> {
    checked(
        adb,
        serial_args(
            serial,
            &["shell", "cmd", "window", "fixed-to-user-rotation", value],
        ),
        "set Android fixed-to-user rotation mode",
    )?;
    Ok(())
}

fn wait_for_orientation(
    adb: &mut dyn AdbRunner,
    serial: &str,
    landscape: bool,
    timeout: Duration,
) -> Result<(String, Vec<UiNode>, i32, i32)> {
    let started = Instant::now();
    loop {
        let xml = dump_ui_xml(adb, serial)?;
        let nodes = parse_ui_nodes(&xml)?;
        let (width, height) = hierarchy_extent(&nodes)?;
        if (width > height) == landscape {
            validate_hierarchy_bounds(&nodes, width, height)?;
            return Ok((xml, nodes, width, height));
        }
        if started.elapsed() >= timeout {
            bail!(
                "display did not enter {} orientation",
                if landscape { "landscape" } else { "portrait" }
            );
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn scenario_layout(
    root: &Path,
    adb: &mut dyn AdbRunner,
    serial: &str,
    mode: ScenarioMode,
) -> Result<CommandReport> {
    let directory = scenario_directory(root, ScenarioName::Layout)?;
    let original_auto = shell_text(
        adb,
        serial,
        &["settings", "get", "system", "accelerometer_rotation"],
    )?;
    let original_rotation =
        shell_text(adb, serial, &["settings", "get", "system", "user_rotation"])?;
    let original_fixed_rotation =
        shell_text(adb, serial, &["cmd", "window", "fixed-to-user-rotation"])?;
    checked(
        adb,
        serial_args(
            serial,
            &[
                "shell",
                "settings",
                "put",
                "system",
                "accelerometer_rotation",
                "0",
            ],
        ),
        "disable Android auto-rotation for layout scenario",
    )?;
    set_fixed_to_user_rotation(adb, serial, "enabled")?;

    let run_result: Result<_> = (|| {
        set_rotation(adb, serial, "0")?;
        let (_, _, portrait_width, portrait_height) =
            wait_for_orientation(adb, serial, false, Duration::from_secs(8))?;
        thread::sleep(Duration::from_millis(750));
        let portrait_xml = dump_ui_xml(adb, serial)?;
        let portrait_xml_path = directory.join("portrait.xml");
        let portrait_screen_path = directory.join("portrait.png");
        fs::write(&portrait_xml_path, portrait_xml)
            .with_context(|| format!("write {}", portrait_xml_path.display()))?;
        fs::write(&portrait_screen_path, screenshot(adb, serial)?)
            .with_context(|| format!("write {}", portrait_screen_path.display()))?;

        set_rotation(adb, serial, "1")?;
        let (_, _, landscape_width, landscape_height) =
            wait_for_orientation(adb, serial, true, Duration::from_secs(8))?;
        thread::sleep(Duration::from_millis(750));
        let landscape_xml = dump_ui_xml(adb, serial)?;
        let landscape_xml_path = directory.join("landscape.xml");
        let landscape_screen_path = directory.join("landscape.png");
        fs::write(&landscape_xml_path, landscape_xml)
            .with_context(|| format!("write {}", landscape_xml_path.display()))?;
        fs::write(&landscape_screen_path, screenshot(adb, serial)?)
            .with_context(|| format!("write {}", landscape_screen_path.display()))?;
        Ok((
            portrait_width,
            portrait_height,
            landscape_width,
            landscape_height,
            portrait_screen_path,
            portrait_xml_path,
            landscape_screen_path,
            landscape_xml_path,
        ))
    })();

    let restore_rotation = set_rotation(adb, serial, &original_rotation);
    let restore_fixed_rotation = set_fixed_to_user_rotation(adb, serial, &original_fixed_rotation);
    let restore_auto = checked(
        adb,
        serial_args(
            serial,
            &[
                "shell",
                "settings",
                "put",
                "system",
                "accelerometer_rotation",
                &original_auto,
            ],
        ),
        "restore Android auto-rotation",
    )
    .map(|_| ());
    let (
        portrait_width,
        portrait_height,
        landscape_width,
        landscape_height,
        portrait_screen,
        portrait_xml,
        landscape_screen,
        landscape_xml,
    ) = run_result?;
    restore_rotation?;
    restore_fixed_rotation?;
    restore_auto?;

    Ok(CommandReport::new(
        "scenario",
        Some(serial),
        json!({
            "name": scenario_name(ScenarioName::Layout),
            "mode": scenario_mode(mode),
            "portrait": { "width": portrait_width, "height": portrait_height },
            "landscape": { "width": landscape_width, "height": landscape_height },
            "boundsValid": true,
        }),
    )
    .with_artifacts([
        portrait_screen,
        portrait_xml,
        landscape_screen,
        landscape_xml,
    ]))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use clap::Parser;
    use tempfile::tempdir;

    use super::*;
    use crate::Cli;

    const SERIAL: &str = "FAKE-DEVICE";

    struct FakeAdb {
        calls: Vec<Vec<String>>,
        dumps: VecDeque<String>,
        logs: VecDeque<String>,
        installed: bool,
        rotation: u8,
        fixed_to_user_rotation: String,
    }

    impl Default for FakeAdb {
        fn default() -> Self {
            Self {
                calls: Vec::new(),
                dumps: VecDeque::new(),
                logs: VecDeque::new(),
                installed: true,
                rotation: 0,
                fixed_to_user_rotation: "default".into(),
            }
        }
    }

    impl FakeAdb {
        fn ok(text: impl Into<Vec<u8>>) -> AdbOutput {
            AdbOutput {
                success: true,
                stdout: text.into(),
                stderr: Vec::new(),
            }
        }

        fn command<'a>(&self, args: &'a [String]) -> &'a [String] {
            if args.first().is_some_and(|arg| arg == "-s") {
                &args[2..]
            } else {
                args
            }
        }

        fn has_call(&self, expected: &[&str]) -> bool {
            self.calls.iter().any(|args| {
                let command = self.command(args);
                command
                    .iter()
                    .map(String::as_str)
                    .eq(expected.iter().copied())
            })
        }

        fn default_dump(&self) -> String {
            if self.rotation == 1 {
                hierarchy(
                    2400,
                    1080,
                    &[node(
                        "gar.connection.status",
                        "已连接 Fake Host",
                        true,
                        "[0,0][600,80]",
                    )],
                )
            } else {
                hierarchy(
                    1080,
                    2400,
                    &[node(
                        "gar.connection.status",
                        "已连接 Fake Host",
                        true,
                        "[0,0][600,80]",
                    )],
                )
            }
        }
    }

    impl AdbRunner for FakeAdb {
        fn output(&mut self, args: &[String]) -> Result<AdbOutput> {
            self.calls.push(args.to_vec());
            let command = self.command(args);
            let strings = command.iter().map(String::as_str).collect::<Vec<_>>();
            let output = match strings.as_slice() {
                ["version"] => "Android Debug Bridge version 1.0.41\n".into(),
                ["devices", "-l"] => format!(
                    "List of devices attached\n{SERIAL}\tdevice product:fake model:Pixel_9 device:fake transport_id:1\n"
                )
                .into_bytes(),
                ["get-state"] => b"device\n".to_vec(),
                ["shell", "getprop", "ro.product.manufacturer"] => b"Google\n".to_vec(),
                ["shell", "getprop", "ro.product.model"] => b"Pixel 9\n".to_vec(),
                ["shell", "getprop", "ro.build.version.sdk"] => b"36\n".to_vec(),
                ["shell", "pm", "path", PACKAGE] if self.installed => {
                    format!("package:/data/app/{PACKAGE}/base.apk\n").into_bytes()
                }
                ["shell", "pm", "path", PACKAGE] => Vec::new(),
                ["uninstall", PACKAGE] => {
                    self.installed = false;
                    b"Success\n".to_vec()
                }
                ["install", "-r", _] => {
                    self.installed = true;
                    b"Success\n".to_vec()
                }
                ["reverse", _, _] => Vec::new(),
                ["shell", "am", "start", "-W", "-n", ACTIVITY] => b"Status: ok\n".to_vec(),
                ["shell", "am", "force-stop", PACKAGE] => Vec::new(),
                ["shell", "uiautomator", "dump", UI_DUMP_PATH] => {
                    b"UI hierchary dumped\n".to_vec()
                }
                ["exec-out", "cat", UI_DUMP_PATH] => self
                    .dumps
                    .pop_front()
                    .unwrap_or_else(|| self.default_dump())
                    .into_bytes(),
                ["exec-out", "screencap", "-p"] => b"\x89PNG\r\n\x1a\nFAKE".to_vec(),
                ["shell", "wm", "size"] => b"Physical size: 1080x2400\n".to_vec(),
                ["shell", "wm", "density"] => b"Physical density: 420\n".to_vec(),
                ["shell", "pidof", PACKAGE] => b"1234\n".to_vec(),
                ["shell", "settings", "get", "system", "accelerometer_rotation"] => {
                    b"1\n".to_vec()
                }
                ["shell", "settings", "get", "system", "user_rotation"] => {
                    self.rotation.to_string().into_bytes()
                }
                ["shell", "cmd", "window", "fixed-to-user-rotation"] => {
                    self.fixed_to_user_rotation.clone().into_bytes()
                }
                ["shell", "cmd", "window", "fixed-to-user-rotation", value] => {
                    self.fixed_to_user_rotation = (*value).to_owned();
                    Vec::new()
                }
                ["shell", "settings", "put", "system", "user_rotation", value] => {
                    self.rotation = value.parse().unwrap_or(0);
                    Vec::new()
                }
                ["shell", "settings", "put", "system", "accelerometer_rotation", _] => {
                    Vec::new()
                }
                ["logcat", "-d", "-v", "raw", "GAR.SendTrace:I", "*:S"] => self
                    .logs
                    .pop_front()
                    .unwrap_or_default()
                    .into_bytes(),
                ["shell", "input", ..] | ["logcat", ..] => Vec::new(),
                other => panic!("unhandled fake adb command: {other:?}"),
            };
            Ok(Self::ok(output))
        }

        fn output_for(&mut self, args: &[String], _duration: Duration) -> Result<AdbOutput> {
            self.calls.push(args.to_vec());
            Ok(Self::ok(b"fake timed log\n".to_vec()))
        }
    }

    fn node(id: &str, text: &str, enabled: bool, bounds: &str) -> String {
        format!(
            r#"<node index="0" text="{text}" resource-id="{id}" class="android.view.View" package="{PACKAGE}" content-desc="" clickable="true" enabled="{enabled}" bounds="{bounds}" />"#
        )
    }

    fn hierarchy(width: i32, height: i32, children: &[String]) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?><hierarchy rotation="0"><node index="0" text="" resource-id="" class="android.widget.FrameLayout" package="{PACKAGE}" content-desc="" clickable="false" enabled="true" bounds="[0,0][{width},{height}]">{}</node></hierarchy>"#,
            children.join("")
        )
    }

    fn args(command: AndroidDeviceCommand) -> AndroidDeviceArgs {
        AndroidDeviceArgs {
            serial: Some(SERIAL.to_owned()),
            json: true,
            command,
        }
    }

    fn no_build() -> Result<()> {
        Ok(())
    }

    #[test]
    fn clap_accepts_every_documented_android_device_command() {
        let commands = [
            vec![
                "xtask",
                "android-device",
                "doctor",
                "--serial",
                SERIAL,
                "--json",
            ],
            vec![
                "xtask",
                "android-device",
                "prepare",
                "--serial",
                SERIAL,
                "--fresh",
                "--port",
                "7437",
                "--json",
            ],
            vec![
                "xtask",
                "android-device",
                "inspect",
                "--output",
                "artifacts",
                "--json",
            ],
            vec!["xtask", "android-device", "ui", "dump", "--json"],
            vec![
                "xtask",
                "android-device",
                "ui",
                "click",
                "--id",
                "gar.composer.send",
                "--json",
            ],
            vec![
                "xtask",
                "android-device",
                "ui",
                "text",
                "--id",
                "gar.composer.input",
                "--value",
                "hello",
                "--json",
            ],
            vec![
                "xtask",
                "android-device",
                "ui",
                "wait",
                "--id",
                "gar.send.retry",
                "--state",
                "gone",
                "--timeout",
                "2",
                "--json",
            ],
            vec![
                "xtask",
                "android-device",
                "logs",
                "--duration",
                "5",
                "--output",
                "logs",
                "--json",
            ],
            vec![
                "xtask",
                "android-device",
                "capture",
                "--output",
                "capture",
                "--json",
            ],
        ];
        for command in commands {
            Cli::try_parse_from(command).expect("documented command must parse");
        }
        for name in [
            "project-tree",
            "send",
            "reconnect",
            "layout",
            "send-latency",
        ] {
            for mode in ["mock", "real"] {
                Cli::try_parse_from([
                    "xtask",
                    "android-device",
                    "scenario",
                    "--name",
                    name,
                    "--mode",
                    mode,
                    "--json",
                ])
                .expect("documented scenario must parse");
            }
        }
        Cli::try_parse_from(["xtask", "android-device", "scenario", "--name", "send"])
            .expect("scenario mode defaults to mock");
    }

    #[test]
    fn parses_devices_and_requires_explicit_serial_for_multiple_ready_devices() {
        let parsed = parse_devices(&format!(
            "List of devices attached\n{SERIAL}\tdevice product:fake model:Pixel_9\nSECOND\toffline transport_id:2\n"
        ));
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].properties["model"], "Pixel_9");

        let mut adb = FakeAdb::default();
        let (serial, _) = resolve_serial(&mut adb, None).expect("one ready fake device");
        assert_eq!(serial, SERIAL);
    }

    #[test]
    fn parses_compose_resource_ids_without_falling_back_to_titles() {
        let xml = hierarchy(
            1080,
            2400,
            &[
                node("gar.composer.send", "发送", true, "[900,2200][1040,2340]"),
                node(
                    "dev.agentremote.messenger:id/gar.send.retry",
                    "重试",
                    false,
                    "[700,2100][900,2200]",
                ),
            ],
        );
        let nodes = parse_ui_nodes(&xml).expect("valid fake hierarchy");
        assert!(find_node(&nodes, "gar.composer.send").is_ok());
        assert!(find_node(&nodes, "gar.send.retry").is_ok());
        assert!(find_node(&nodes, "发送").is_err());
    }

    #[test]
    fn layout_validation_uses_root_screen_bounds() {
        let xml = hierarchy(
            1080,
            2400,
            &[node(
                "gar.composer.send",
                "",
                true,
                "[1000,2200][1200,2340]",
            )],
        );
        let nodes = parse_ui_nodes(&xml).expect("valid fake hierarchy");
        let (width, height) = hierarchy_extent(&nodes).expect("root extent");
        assert_eq!((width, height), (1080, 2400));
        assert!(validate_hierarchy_bounds(&nodes, width, height).is_err());
    }

    #[test]
    fn doctor_uses_fake_adb_and_reports_install_state() {
        let mut adb = FakeAdb::default();
        let report = doctor(&mut adb, None).expect("doctor");
        assert_eq!(report.serial.as_deref(), Some(SERIAL));
        assert_eq!(report.details["appInstalled"], true);
        assert!(adb.has_call(&["devices", "-l"]));
        assert!(adb.has_call(&["shell", "pm", "path", PACKAGE]));
    }

    #[test]
    fn prepare_builds_installs_reverses_and_launches_with_fake_adb() {
        let directory = tempdir().expect("tempdir");
        let apk_dir = directory.path().join("dist/android");
        fs::create_dir_all(&apk_dir).expect("apk dir");
        let mut built = false;
        let mut build = || {
            built = true;
            fs::write(apk_dir.join("agent-remote-debug.apk"), b"fake apk")?;
            Ok(())
        };
        let mut adb = FakeAdb::default();
        let report = prepare(
            directory.path(),
            &mut adb,
            Some(SERIAL),
            true,
            7437,
            &mut build,
        )
        .expect("prepare");
        assert!(built);
        assert_eq!(report.details["fresh"], true);
        assert!(adb.has_call(&["uninstall", PACKAGE]));
        assert!(adb.has_call(&["reverse", "tcp:7437", "tcp:7437"]));
        assert!(adb.has_call(&["shell", "am", "start", "-W", "-n", ACTIVITY]));
    }

    #[test]
    fn inspect_capture_and_logs_write_artifacts_using_fake_adb() {
        let directory = tempdir().expect("tempdir");
        let mut adb = FakeAdb::default();
        let inspect_output = directory.path().join("inspect");
        inspect(
            directory.path(),
            &mut adb,
            Some(SERIAL),
            Some(&inspect_output),
        )
        .expect("inspect");
        assert!(inspect_output.join("screen.png").is_file());
        assert!(inspect_output.join("window.xml").is_file());
        assert!(inspect_output.join("device.json").is_file());

        let capture_output = directory.path().join("capture");
        capture(
            directory.path(),
            &mut adb,
            Some(SERIAL),
            Some(&capture_output),
        )
        .expect("capture");
        assert!(capture_output.join("screen.png").is_file());

        let log_output = directory.path().join("logs");
        logs(
            directory.path(),
            &mut adb,
            Some(SERIAL),
            1,
            Some(&log_output),
        )
        .expect("logs");
        assert_eq!(
            fs::read_to_string(log_output.join("logcat.txt")).expect("read logs"),
            "fake timed log\n"
        );
    }

    #[test]
    fn ui_commands_locate_exact_ids_and_escape_typed_text() {
        let tagged = hierarchy(
            1080,
            2400,
            &[node(
                "gar.composer.input",
                "old",
                true,
                "[40,2100][880,2320]",
            )],
        );
        let mut adb = FakeAdb::default();
        adb.dumps.push_back(tagged.clone());
        let report = ui(
            &mut adb,
            Some(SERIAL),
            &AndroidUiCommand::Text {
                id: "gar.composer.input".to_owned(),
                value: "hello world!".to_owned(),
            },
        )
        .expect("type");
        assert_eq!(report.details["characterCount"], 12);
        assert!(adb.has_call(&["shell", "input", "text", "hello%sworld\\!"]));

        adb.dumps.push_back(tagged);
        ui(
            &mut adb,
            Some(SERIAL),
            &AndroidUiCommand::Click {
                id: "gar.composer.input".to_owned(),
            },
        )
        .expect("click");
        assert!(adb.has_call(&["shell", "input", "tap", "460", "2210"]));
    }

    #[test]
    fn ui_wait_handles_visible_enabled_and_gone_states() {
        let enabled = hierarchy(
            1080,
            2400,
            &[node("gar.composer.send", "", true, "[900,2200][1040,2340]")],
        );
        let empty = hierarchy(1080, 2400, &[]);
        let mut adb = FakeAdb::default();
        adb.dumps.push_back(enabled);
        wait_for_node(
            &mut adb,
            SERIAL,
            "gar.composer.send",
            UiWaitState::Enabled,
            Duration::ZERO,
        )
        .expect("enabled");
        adb.dumps.push_back(empty);
        wait_for_node(
            &mut adb,
            SERIAL,
            "gar.send.retry",
            UiWaitState::Gone,
            Duration::ZERO,
        )
        .expect("gone");
    }

    #[test]
    fn send_latency_scenario_opens_a_new_conversation_and_requires_correlated_stages() {
        let drawer = hierarchy(
            1080,
            2400,
            &[node("gar.drawer.open", "", true, "[0,0][100,100]")],
        );
        let new_conversation = hierarchy(
            1080,
            2400,
            &[node("gar.conversation.new", "", true, "[40,200][1040,340]")],
        );
        let input = hierarchy(
            1080,
            2400,
            &[node("gar.composer.input", "", true, "[40,2100][880,2320]")],
        );
        let send = hierarchy(
            1080,
            2400,
            &[node("gar.composer.send", "", true, "[900,2200][1040,2340]")],
        );
        let mut adb = FakeAdb::default();
        adb.dumps.extend([
            drawer,
            new_conversation,
            input.clone(),
            input,
            send.clone(),
            send,
        ]);
        adb.logs.push_back(String::new());
        let stages = [
            ("click", 0),
            ("local_pending", 4),
            ("websocket_write", 8),
            ("host_received", 15),
            ("provider_received", 21),
            ("first_provider_event", 47),
        ]
        .into_iter()
        .map(|(stage, elapsed)| {
            json!({
                "commandId": "command-1",
                "clientMessageId": "message-1",
                "stage": stage,
                "elapsedMs": elapsed,
            })
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
        adb.logs.push_back(stages);
        let report = scenario_send(&mut adb, SERIAL, ScenarioMode::Mock, true)
            .expect("send latency scenario");
        assert_eq!(report.details["firstProviderEventMs"], 47);
        assert_eq!(report.details["commandId"], "command-1");
        assert!(adb.has_call(&["shell", "input", "tap", "50", "50"]));
        assert!(adb.has_call(&["shell", "input", "tap", "540", "270"]));
    }

    #[test]
    fn project_tree_reconnect_and_layout_scenarios_use_stable_ids_and_fake_adb() {
        let directory = tempdir().expect("tempdir");
        let drawer = node("gar.drawer.open", "", true, "[0,0][100,100]");
        let project = node(
            "gar.project.11111111-1111-1111-1111-111111111111",
            "",
            true,
            "[0,100][800,220]",
        );
        let project_toggle = node(
            "gar.project.11111111-1111-1111-1111-111111111111.toggle",
            "",
            true,
            "[0,100][100,220]",
        );
        let conversation = node(
            "gar.conversation.22222222-2222-2222-2222-222222222222",
            "",
            true,
            "[40,220][800,340]",
        );
        let mut adb = FakeAdb::default();
        let expanded = hierarchy(
            1080,
            2400,
            &[
                project.clone(),
                project_toggle.clone(),
                conversation.clone(),
            ],
        );
        let collapsed = hierarchy(1080, 2400, &[project.clone(), project_toggle.clone()]);
        adb.dumps.extend([
            hierarchy(1080, 2400, std::slice::from_ref(&drawer)),
            hierarchy(1080, 2400, std::slice::from_ref(&drawer)),
            expanded.clone(),
            expanded,
            hierarchy(
                1080,
                2400,
                &[project.clone(), project_toggle.clone(), conversation],
            ),
            collapsed.clone(),
            collapsed,
        ]);
        let project_report =
            scenario_project_tree(directory.path(), &mut adb, SERIAL, ScenarioMode::Mock)
                .expect("project tree scenario");
        assert_eq!(project_report.details["projectCount"], 1);
        assert_eq!(
            project_report.details["visibleConversationCountAfterToggle"],
            0
        );
        assert_eq!(
            project_report.details["expectedConversationChildrenVisible"],
            false
        );

        let reconnect_report =
            scenario_reconnect(directory.path(), &mut adb, SERIAL, ScenarioMode::Real)
                .expect("reconnect scenario");
        assert_eq!(reconnect_report.details["mode"], "real");
        assert!(adb.has_call(&["shell", "am", "force-stop", PACKAGE]));

        let layout_report = scenario_layout(directory.path(), &mut adb, SERIAL, ScenarioMode::Mock)
            .expect("layout scenario");
        assert_eq!(layout_report.details["portrait"]["width"], 1080);
        assert_eq!(layout_report.details["landscape"]["width"], 2400);
        assert_eq!(adb.rotation, 0);
        assert_eq!(adb.fixed_to_user_rotation, "default");
    }

    #[test]
    fn command_dispatch_covers_dump_and_zero_duration_logs_without_real_adb() {
        let directory = tempdir().expect("tempdir");
        let mut adb = FakeAdb::default();
        let mut build = no_build;
        let dump = run_with(
            directory.path(),
            &args(AndroidDeviceCommand::Ui(AndroidUiCommand::Dump)),
            &mut build,
            &mut adb,
        )
        .expect("ui dump dispatch");
        assert_eq!(dump.command, "ui dump");
        let report = run_with(
            directory.path(),
            &args(AndroidDeviceCommand::Logs {
                duration: 0,
                output: None,
            }),
            &mut build,
            &mut adb,
        )
        .expect("log dispatch");
        assert_eq!(report.details["durationSeconds"], 0);
    }
}
