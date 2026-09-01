use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::Value;

fn run_binary(path: &str, input: &str) -> Vec<Value> {
    let mut child = Command::new(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn mock binary");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input.as_bytes())
        .expect("write JSONL input");
    let output = child.wait_with_output().expect("wait for mock binary");
    assert!(output.status.success(), "mock failed: {output:?}");
    String::from_utf8(output.stdout)
        .expect("UTF-8 stdout")
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSONL output"))
        .collect()
}

#[test]
fn codex_mock_binary_runs_over_stdio() {
    let messages = run_binary(
        env!("CARGO_BIN_EXE_mock-codex-app-server"),
        "{\"method\":\"initialize\",\"id\":1,\"params\":{\"clientInfo\":{\"name\":\"test\",\"version\":\"0.1.0\"}}}\n",
    );
    assert_eq!(messages[0]["id"], 1);
    assert_eq!(messages[0]["result"]["platformOs"], "windows");
}

#[test]
fn grok_mock_binary_runs_over_stdio() {
    let messages = run_binary(
        env!("CARGO_BIN_EXE_mock-grok-acp"),
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":1,\"clientCapabilities\":{},\"clientInfo\":{\"name\":\"test\",\"version\":\"0.1.0\"}}}\n",
    );
    assert_eq!(messages[0]["jsonrpc"], "2.0");
    assert_eq!(messages[0]["result"]["protocolVersion"], 1);
}
