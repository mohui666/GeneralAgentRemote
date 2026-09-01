use agent_client_protocol::schema::v1::{
    InitializeResponse, NewSessionResponse, RequestPermissionRequest, SessionUpdate,
};
use agent_remote_testkit::{
    EngineOutput, JsonlEngine, MockCodexAppServer, MockGrokAcp, MockScenario, drive_lines,
};
use serde_json::{Value, json};

fn json_messages(output: &[EngineOutput]) -> Vec<&Value> {
    output
        .iter()
        .filter_map(|output| match output {
            EngineOutput::Json(message) => Some(message),
            EngineOutput::Exit(_) => None,
        })
        .collect()
}

fn initialize_codex(engine: &mut MockCodexAppServer) {
    drive_lines(
        engine,
        [
            r#"{"method":"initialize","id":1,"params":{"clientInfo":{"name":"test","title":"Test","version":"0.1.0"},"capabilities":{}}}"#,
            r#"{"method":"initialized"}"#,
        ],
    )
    .expect("initialize Codex mock");
}

fn start_codex_thread(engine: &mut MockCodexAppServer, cwd: &str) -> String {
    let request = json!({"method": "thread/start", "id": 2, "params": {"cwd": cwd}});
    let output = engine
        .receive_line(&request.to_string())
        .expect("start Codex thread");
    json_messages(&output)[0]
        .pointer("/result/thread/id")
        .and_then(Value::as_str)
        .expect("thread id")
        .to_owned()
}

fn initialize_grok(engine: &mut MockGrokAcp) {
    engine
        .receive_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":true,"writeTextFile":true},"terminal":true},"clientInfo":{"name":"test","title":"Test","version":"0.1.0"}}}"#,
        )
        .expect("initialize Grok mock");
}

fn start_grok_session(engine: &mut MockGrokAcp, cwd: &str) -> String {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "session/new",
        "params": {"cwd": cwd, "mcpServers": []}
    });
    let output = engine
        .receive_line(&request.to_string())
        .expect("start Grok session");
    let message = json_messages(&output)[0];
    let response = serde_json::from_value::<NewSessionResponse>(message["result"].clone())
        .expect("Grok new-session result matches ACP v1");
    response.session_id.to_string()
}

#[test]
fn codex_initializes_and_advertises_dynamic_models_without_jsonrpc_field() {
    let mut engine = MockCodexAppServer::default();
    let output = drive_lines(
        &mut engine,
        [
            r#"{"method":"initialize","id":1,"params":{"clientInfo":{"name":"test","version":"0.1.0"}}}"#,
            r#"{"method":"initialized"}"#,
            r#"{"method":"model/list","id":2,"params":{"includeHidden":false}}"#,
        ],
    )
    .expect("drive Codex model discovery");
    let messages = json_messages(&output);

    assert_eq!(
        messages[0].pointer("/result/platformOs"),
        Some(&json!("windows"))
    );
    assert!(
        messages
            .iter()
            .all(|message| message.get("jsonrpc").is_none())
    );
    assert_eq!(
        messages[1].pointer("/result/data/0/id"),
        Some(&json!("mock-codex-dynamic"))
    );
    assert_eq!(
        messages[1].pointer("/result/data/0/defaultReasoningEffort"),
        Some(&json!("high"))
    );
    assert_eq!(
        messages[1].pointer("/result/data/0/hidden"),
        Some(&json!(false))
    );
    assert_eq!(
        messages[1]
            .pointer("/result/data")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );
}

#[test]
fn codex_lists_reads_and_resumes_only_matching_cwd_threads() {
    let mut engine = MockCodexAppServer::default();
    initialize_codex(&mut engine);
    let alpha = start_codex_thread(&mut engine, r"C:\project-alpha");
    let beta = start_codex_thread(&mut engine, r"C:\project-beta");

    let lines = [
        json!({"method": "thread/list", "id": 3, "params": {"cwd": r"C:\project-alpha"}}),
        json!({"method": "thread/read", "id": 4, "params": {"threadId": alpha, "includeTurns": true}}),
        json!({"method": "thread/resume", "id": 5, "params": {"threadId": beta, "cwd": r"C:\project-alpha"}}),
    ]
    .map(|message| message.to_string());
    let output = drive_lines(&mut engine, lines).expect("drive Codex session lifecycle");
    let messages = json_messages(&output);

    assert_eq!(
        messages[0].pointer("/result/data/0/cwd"),
        Some(&json!(r"C:\project-alpha"))
    );
    assert_eq!(
        messages[0]
            .pointer("/result/data")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        messages[1].pointer("/result/thread/id"),
        Some(&json!(alpha))
    );
    assert_eq!(messages[2].pointer("/error/code"), Some(&json!(-32602)));
}

#[test]
fn codex_complete_turn_streams_structured_events_image_and_terminal_completion() {
    let mut engine = MockCodexAppServer::default();
    initialize_codex(&mut engine);
    let thread_id = start_codex_thread(&mut engine, r"C:\project-alpha");
    let request = json!({
        "method": "turn/start",
        "id": 3,
        "params": {
            "threadId": thread_id,
            "input": [{"type": "text", "text": "complete", "text_elements": []}],
            "model": "mock-codex-dynamic",
            "effort": "ultra"
        }
    });
    let output = engine
        .receive_line(&request.to_string())
        .expect("run complete Codex turn");
    let messages = json_messages(&output);
    let methods = messages
        .iter()
        .filter_map(|message| message.get("method").and_then(Value::as_str))
        .collect::<Vec<_>>();

    for method in [
        "turn/started",
        "turn/plan/updated",
        "item/mcpToolCall/progress",
        "item/commandExecution/outputDelta",
        "item/fileChange/patchUpdated",
        "item/agentMessage/delta",
        "turn/completed",
    ] {
        assert!(methods.contains(&method), "missing {method}");
    }
    assert!(
        messages
            .iter()
            .any(|message| { message.pointer("/params/item/type") == Some(&json!("imageView")) })
    );
    assert_eq!(
        messages
            .iter()
            .find(|message| message.get("method") == Some(&json!("turn/completed")))
            .and_then(|message| message.pointer("/params/turn/status")),
        Some(&json!("completed"))
    );
    let merged = messages
        .iter()
        .filter(|message| message.get("method") == Some(&json!("item/agentMessage/delta")))
        .filter_map(|message| message.pointer("/params/delta").and_then(Value::as_str))
        .collect::<String>();
    assert!(merged.contains("Mock Codex completed the requested work."));
}

#[test]
fn codex_approval_allows_or_declines_then_continues() {
    for (decision, expected_status) in [("accept", "completed"), ("decline", "declined")] {
        let mut engine = MockCodexAppServer::new(MockScenario::Approval);
        initialize_codex(&mut engine);
        let thread_id = start_codex_thread(&mut engine, r"C:\project-alpha");
        let turn = json!({
            "method": "turn/start",
            "id": 3,
            "params": {"threadId": thread_id, "input": [{"type": "text", "text": "needs permission", "text_elements": []}]}
        });
        let first = engine
            .receive_line(&turn.to_string())
            .expect("request Codex approval");
        let approval = json_messages(&first)
            .into_iter()
            .find(|message| {
                message.get("method") == Some(&json!("item/commandExecution/requestApproval"))
            })
            .expect("approval request");
        let approval_id = approval.get("id").cloned().expect("approval id");
        let reply = json!({"id": approval_id, "result": {"decision": decision}});
        let continued = engine
            .receive_line(&reply.to_string())
            .expect("continue Codex turn");
        let messages = json_messages(&continued);

        assert!(messages.iter().any(|message| {
            message.pointer("/params/item/type") == Some(&json!("commandExecution"))
                && message.pointer("/params/item/status") == Some(&json!(expected_status))
        }));
        assert!(
            messages
                .iter()
                .any(|message| message.get("method") == Some(&json!("turn/completed")))
        );
    }
}

#[test]
fn codex_supports_interrupt_failure_crash_and_unknown_events() {
    let mut interrupted = MockCodexAppServer::new(MockScenario::Approval);
    initialize_codex(&mut interrupted);
    let thread_id = start_codex_thread(&mut interrupted, r"C:\project-alpha");
    let turn = json!({"method": "turn/start", "id": 3, "params": {"threadId": thread_id, "input": [{"type": "text", "text": "wait", "text_elements": []}]}});
    let started = interrupted
        .receive_line(&turn.to_string())
        .expect("start interruptible turn");
    let turn_id = json_messages(&started)[0]
        .pointer("/result/turn/id")
        .and_then(Value::as_str)
        .expect("turn id")
        .to_owned();
    let interrupt = json!({"method": "turn/interrupt", "id": 4, "params": {"threadId": thread_id, "turnId": turn_id}});
    let output = interrupted
        .receive_line(&interrupt.to_string())
        .expect("interrupt turn");
    assert!(
        json_messages(&output).iter().any(|message| {
            message.pointer("/params/turn/status") == Some(&json!("interrupted"))
        })
    );

    for (scenario, expected) in [
        (MockScenario::Failure, "failed"),
        (MockScenario::Unknown, "completed"),
    ] {
        let mut engine = MockCodexAppServer::new(scenario);
        initialize_codex(&mut engine);
        let thread_id = start_codex_thread(&mut engine, r"C:\project-alpha");
        let turn = json!({"method": "turn/start", "id": 3, "params": {"threadId": thread_id, "input": [{"type": "text", "text": "run", "text_elements": []}]}});
        let output = engine
            .receive_line(&turn.to_string())
            .expect("run scenario");
        let messages = json_messages(&output);
        assert!(
            messages.iter().any(|message| {
                message.pointer("/params/turn/status") == Some(&json!(expected))
            })
        );
        if scenario == MockScenario::Unknown {
            assert!(
                messages
                    .iter()
                    .any(|message| message.get("method") == Some(&json!("mock/futureEvent")))
            );
        }
    }

    let mut crashed = MockCodexAppServer::new(MockScenario::Crash);
    initialize_codex(&mut crashed);
    let thread_id = start_codex_thread(&mut crashed, r"C:\project-alpha");
    let turn = json!({"method": "turn/start", "id": 3, "params": {"threadId": thread_id, "input": [{"type": "text", "text": "run", "text_elements": []}]}});
    let output = crashed
        .receive_line(&turn.to_string())
        .expect("run crash scenario");
    assert!(output.contains(&EngineOutput::Exit(86)));
}

#[test]
fn grok_initializes_with_acp_v1_and_vendor_dynamic_model_state() {
    let mut engine = MockGrokAcp::default();
    let output = engine
        .receive_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"test","version":"0.1.0"}}}"#,
        )
        .expect("initialize Grok");
    let message = json_messages(&output)[0];

    assert_eq!(message.get("jsonrpc"), Some(&json!("2.0")));
    assert_eq!(message.pointer("/result/protocolVersion"), Some(&json!(1)));
    assert_eq!(
        message.pointer("/result/_meta/agentVersion"),
        Some(&json!("1.0.13"))
    );
    assert_eq!(
        message.pointer("/result/_meta/modelState/availableModels/0/modelId"),
        Some(&json!("grok-4.6"))
    );
    assert_eq!(
        message
            .pointer("/result/_meta/modelState/availableModels/0/_meta/reasoningEfforts/0/value"),
        Some(&json!("xhigh"))
    );
    serde_json::from_value::<InitializeResponse>(message["result"].clone())
        .expect("Grok initialize result matches ACP v1");
}

#[test]
fn grok_lists_loads_resumes_and_uses_only_the_1013_vendor_model_method() {
    let mut engine = MockGrokAcp::default();
    initialize_grok(&mut engine);
    let alpha = start_grok_session(&mut engine, r"C:\project-alpha");
    let beta = start_grok_session(&mut engine, r"C:\project-beta");
    let lines = [
        json!({"jsonrpc": "2.0", "id": 3, "method": "session/list", "params": {"cwd": r"C:\project-alpha"}}),
        json!({"jsonrpc": "2.0", "id": 4, "method": "session/load", "params": {"sessionId": alpha, "cwd": r"C:\project-alpha", "mcpServers": []}}),
        json!({"jsonrpc": "2.0", "id": 5, "method": "session/resume", "params": {"sessionId": beta, "cwd": r"C:\project-alpha", "mcpServers": []}}),
        json!({"jsonrpc": "2.0", "id": 6, "method": "session/set_config_option", "params": {"sessionId": alpha, "configId": "model", "value": "grok-4.5"}}),
        json!({"jsonrpc": "2.0", "id": 7, "method": "session/set_model", "params": {"sessionId": alpha, "modelId": "grok-4.5", "_meta": {"reasoningEffort": "medium"}}}),
    ]
    .map(|message| message.to_string());
    let output = drive_lines(&mut engine, lines).expect("drive Grok session lifecycle");
    let messages = json_messages(&output);

    assert_eq!(
        messages[0]
            .pointer("/result/sessions")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    assert!(messages.iter().any(|message| {
        message.pointer("/params/update/sessionUpdate") == Some(&json!("user_message_chunk"))
    }));
    assert!(
        messages
            .iter()
            .any(|message| message.get("id") == Some(&json!(5))
                && message.pointer("/error/code") == Some(&json!(-32602)))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.get("id") == Some(&json!(6))
                && message.pointer("/error/code") == Some(&json!(-32601)))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.get("id") == Some(&json!(7))
                && message.get("result") == Some(&json!({})))
    );
}

#[test]
fn grok_complete_prompt_streams_plan_command_file_text_and_both_image_forms() {
    let mut engine = MockGrokAcp::default();
    initialize_grok(&mut engine);
    let session_id = start_grok_session(&mut engine, r"C:\project-alpha");
    let request = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "session/prompt",
        "params": {"sessionId": session_id, "prompt": [{"type": "text", "text": "complete"}]}
    });
    let output = engine
        .receive_line(&request.to_string())
        .expect("run complete Grok prompt");
    let messages = json_messages(&output);
    for update in messages
        .iter()
        .filter_map(|message| message.pointer("/params/update"))
    {
        serde_json::from_value::<SessionUpdate>(update.clone())
            .expect("known Grok update matches ACP v1");
    }
    let updates = messages
        .iter()
        .filter_map(|message| {
            message
                .pointer("/params/update/sessionUpdate")
                .and_then(Value::as_str)
        })
        .collect::<Vec<_>>();

    for update in [
        "plan",
        "agent_thought_chunk",
        "tool_call",
        "tool_call_update",
        "agent_message_chunk",
    ] {
        assert!(updates.contains(&update), "missing {update}");
    }
    assert!(messages.iter().any(|message| {
        message.pointer("/params/update/content/type") == Some(&json!("image"))
    }));
    assert!(messages.iter().any(|message| {
        message.pointer("/params/update/content/1/content/type") == Some(&json!("image"))
    }));
    assert_eq!(
        messages
            .last()
            .and_then(|message| message.pointer("/result/stopReason")),
        Some(&json!("end_turn"))
    );
    let merged = messages
        .iter()
        .filter(|message| {
            message.pointer("/params/update/sessionUpdate") == Some(&json!("agent_message_chunk"))
                && message.pointer("/params/update/content/type") == Some(&json!("text"))
        })
        .filter_map(|message| {
            message
                .pointer("/params/update/content/text")
                .and_then(Value::as_str)
        })
        .collect::<String>();
    assert_eq!(merged, "Mock Grok completed the requested work.");
}

#[test]
fn grok_permission_allows_or_rejects_then_continues() {
    for (option_id, expected_status) in [("allow-once", "completed"), ("reject-once", "failed")] {
        let mut engine = MockGrokAcp::new(MockScenario::Approval);
        initialize_grok(&mut engine);
        let session_id = start_grok_session(&mut engine, r"C:\project-alpha");
        let prompt = json!({"jsonrpc": "2.0", "id": 3, "method": "session/prompt", "params": {"sessionId": session_id, "prompt": [{"type": "text", "text": "needs permission"}]}});
        let first = engine
            .receive_line(&prompt.to_string())
            .expect("request Grok permission");
        let request = json_messages(&first)
            .into_iter()
            .find(|message| message.get("method") == Some(&json!("session/request_permission")))
            .expect("permission request");
        serde_json::from_value::<RequestPermissionRequest>(request["params"].clone())
            .expect("Grok permission request matches ACP v1");
        let permission_id = request.get("id").cloned().expect("permission id");
        let reply = json!({"jsonrpc": "2.0", "id": permission_id, "result": {"outcome": {"outcome": "selected", "optionId": option_id}}});
        let continued = engine
            .receive_line(&reply.to_string())
            .expect("continue Grok prompt");
        let messages = json_messages(&continued);

        assert!(messages.iter().any(|message| {
            message.pointer("/params/update/toolCallId") == Some(&json!("mock-grok-command-1"))
                && message.pointer("/params/update/status") == Some(&json!(expected_status))
        }));
        assert_eq!(
            messages
                .last()
                .and_then(|message| message.pointer("/result/stopReason")),
            Some(&json!("end_turn"))
        );
    }
}

#[test]
fn grok_supports_cancel_failure_crash_and_unknown_updates() {
    let mut cancelled = MockGrokAcp::new(MockScenario::Approval);
    initialize_grok(&mut cancelled);
    let session_id = start_grok_session(&mut cancelled, r"C:\project-alpha");
    let prompt = json!({"jsonrpc": "2.0", "id": 3, "method": "session/prompt", "params": {"sessionId": session_id, "prompt": [{"type": "text", "text": "wait"}]}});
    cancelled
        .receive_line(&prompt.to_string())
        .expect("start cancellable prompt");
    let cancel =
        json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session_id}});
    let output = cancelled
        .receive_line(&cancel.to_string())
        .expect("cancel Grok prompt");
    assert_eq!(
        json_messages(&output)
            .last()
            .and_then(|message| message.pointer("/result/stopReason")),
        Some(&json!("cancelled"))
    );

    for scenario in [MockScenario::Failure, MockScenario::Unknown] {
        let mut engine = MockGrokAcp::new(scenario);
        initialize_grok(&mut engine);
        let session_id = start_grok_session(&mut engine, r"C:\project-alpha");
        let prompt = json!({"jsonrpc": "2.0", "id": 3, "method": "session/prompt", "params": {"sessionId": session_id, "prompt": [{"type": "text", "text": "run"}]}});
        let output = engine
            .receive_line(&prompt.to_string())
            .expect("run Grok scenario");
        let messages = json_messages(&output);
        if scenario == MockScenario::Failure {
            assert!(
                messages
                    .iter()
                    .any(|message| message.pointer("/error/message")
                        == Some(&json!("mock Grok prompt failed")))
            );
        } else {
            assert!(
                messages
                    .iter()
                    .any(|message| message.pointer("/params/update/sessionUpdate")
                        == Some(&json!("future_mock_update")))
            );
        }
    }

    let mut crashed = MockGrokAcp::new(MockScenario::Crash);
    initialize_grok(&mut crashed);
    let session_id = start_grok_session(&mut crashed, r"C:\project-alpha");
    let prompt = json!({"jsonrpc": "2.0", "id": 3, "method": "session/prompt", "params": {"sessionId": session_id, "prompt": [{"type": "text", "text": "run"}]}});
    let output = crashed
        .receive_line(&prompt.to_string())
        .expect("run Grok crash");
    assert!(output.contains(&EngineOutput::Exit(87)));
}
