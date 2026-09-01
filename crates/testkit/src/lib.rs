//! Reusable JSONL protocol engines used by provider and end-to-end tests.

pub mod codex;
pub mod grok;
mod stdio;

use anyhow::{Context, Result};
use serde_json::Value;

pub use codex::MockCodexAppServer;
pub use grok::MockGrokAcp;
pub use stdio::{RunOutcome, run_stdio};

pub const MOCK_PROTOCOL_VERSION: &str = "v0.1";
pub const SCENARIO_MARKER_APPROVAL: &str = "mock:approval";
pub const SCENARIO_MARKER_FAILURE: &str = "mock:failure";
pub const SCENARIO_MARKER_CRASH: &str = "mock:crash";
pub const SCENARIO_MARKER_UNKNOWN: &str = "mock:unknown";

/// Terminal behavior selected for a mock prompt/turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MockScenario {
    #[default]
    Complete,
    Approval,
    Failure,
    Crash,
    Unknown,
}

impl MockScenario {
    pub fn from_name(name: &str) -> Result<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "complete" | "happy" | "happy_path" => Ok(Self::Complete),
            "approval" | "permission" => Ok(Self::Approval),
            "failure" | "fail" => Ok(Self::Failure),
            "crash" => Ok(Self::Crash),
            "unknown" | "future" => Ok(Self::Unknown),
            value => anyhow::bail!(
                "unknown mock scenario {value:?}; expected complete, approval, failure, crash, or unknown"
            ),
        }
    }

    pub fn from_prompt(prompt: &str, fallback: Self) -> Self {
        if prompt.contains(SCENARIO_MARKER_APPROVAL) {
            Self::Approval
        } else if prompt.contains(SCENARIO_MARKER_FAILURE) {
            Self::Failure
        } else if prompt.contains(SCENARIO_MARKER_CRASH) {
            Self::Crash
        } else if prompt.contains(SCENARIO_MARKER_UNKNOWN) {
            Self::Unknown
        } else {
            fallback
        }
    }
}

/// A line produced by an engine, or an intentional mock process exit.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineOutput {
    Json(Value),
    Exit(i32),
}

/// Stateful protocol engine shared by the stdio binaries and Rust tests.
pub trait JsonlEngine {
    fn receive(&mut self, message: Value) -> Result<Vec<EngineOutput>>;

    fn receive_line(&mut self, line: &str) -> Result<Vec<EngineOutput>> {
        let message = serde_json::from_str(line).context("invalid JSONL input")?;
        self.receive(message)
    }
}

/// Drives an engine with input lines without spawning a child process.
pub fn drive_lines<E, I, S>(engine: &mut E, lines: I) -> Result<Vec<EngineOutput>>
where
    E: JsonlEngine,
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut output = Vec::new();
    for line in lines {
        output.extend(engine.receive_line(line.as_ref())?);
    }
    Ok(output)
}

pub(crate) fn request_id(message: &Value) -> Option<Value> {
    message.get("id").cloned()
}

pub(crate) fn requested_method(message: &Value) -> Option<&str> {
    message.get("method").and_then(Value::as_str)
}

pub(crate) fn scenario_from_env() -> Result<MockScenario> {
    match std::env::var("AGENT_REMOTE_MOCK_SCENARIO") {
        Ok(value) => MockScenario::from_name(&value),
        Err(std::env::VarError::NotPresent) => Ok(MockScenario::Complete),
        Err(error) => Err(error).context("AGENT_REMOTE_MOCK_SCENARIO is not valid Unicode"),
    }
}
