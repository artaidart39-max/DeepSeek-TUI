use deepseek_protocol::{
    AppRequest, AppResponse, AskForApproval, Envelope, EventFrame, LocalShellParams,
    McpStartupCompleteEvent, McpStartupFailure, McpStartupStatus, McpStartupUpdateEvent,
    NetworkPolicyAmendment, NetworkPolicyRuleAction, PromptRequest, PromptResponse,
    ResponseChannel, ReviewDecision, ThreadForkParams, ThreadListParams, ThreadReadParams,
    ThreadRequest, ThreadResponse, ThreadResumeParams, ThreadSetNameParams, ThreadStartParams,
    ThreadStatus, ToolKind, ToolOutput, ToolPayload,
};
use serde_json::json;

#[test]
fn thread_resume_params_round_trip() {
    let request = ThreadRequest::Resume(ThreadResumeParams {
        thread_id: "thread-123".to_string(),
        history: None,
        path: None,
        model: Some("deepseek-v4-pro".to_string()),
        model_provider: Some("deepseek".to_string()),
        cwd: None,
        approval_policy: Some("on-request".to_string()),
        sandbox: Some("workspace-write".to_string()),
        config: None,
        base_instructions: Some("base".to_string()),
        developer_instructions: Some("dev".to_string()),
        personality: Some("default".to_string()),
        persist_extended_history: true,
    });

    let encoded = serde_json::to_string(&request).expect("serialize request");
    let decoded: ThreadRequest = serde_json::from_str(&encoded).expect("deserialize request");
    match decoded {
        ThreadRequest::Resume(params) => {
            assert_eq!(params.thread_id, "thread-123");
            assert_eq!(params.model.as_deref(), Some("deepseek-v4-pro"));
            assert!(params.persist_extended_history);
        }
        other => panic!("unexpected request: {other:?}"),
    }
}

#[test]
fn thread_list_params_defaults_are_serializable() {
    let request = ThreadRequest::List(ThreadListParams {
        include_archived: false,
        limit: Some(20),
    });
    let encoded = serde_json::to_string_pretty(&request).expect("serialize list request");
    assert!(encoded.contains("include_archived"));
}

#[test]
fn event_frame_serialization_contains_expected_tag() {
    let frame = EventFrame::TurnComplete {
        turn_id: "turn-1".to_string(),
    };
    let encoded = serde_json::to_string(&frame).expect("serialize frame");
    assert!(encoded.contains("turn_complete"));
}

// --- ThreadStatus ---

#[test]
fn thread_status_all_variants_round_trip() {
    let variants = [
        ThreadStatus::Running,
        ThreadStatus::Idle,
        ThreadStatus::Completed,
        ThreadStatus::Failed,
        ThreadStatus::Paused,
        ThreadStatus::Archived,
    ];
    for status in &variants {
        let json = serde_json::to_string(status).unwrap();
        let parsed: ThreadStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(&parsed, status);
    }
}

// --- Envelope ---

#[test]
fn envelope_round_trips_with_thread_id() {
    let envelope = Envelope {
        request_id: "req-1".to_string(),
        thread_id: Some("thread-99".to_string()),
        body: json!({"action": "test"}),
    };
    let json = serde_json::to_string(&envelope).unwrap();
    let parsed: Envelope<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.request_id, "req-1");
    assert_eq!(parsed.thread_id.as_deref(), Some("thread-99"));
}

#[test]
fn envelope_omits_none_thread_id() {
    let envelope = Envelope {
        request_id: "req-2".to_string(),
        thread_id: None,
        body: "hello",
    };
    let json = serde_json::to_string(&envelope).unwrap();
    assert!(!json.contains("thread_id"));
}

// --- ThreadRequest variants ---

#[test]
fn thread_request_create_round_trip() {
    let req = ThreadRequest::Create {
        metadata: json!({"key": "val"}),
    };
    let json = serde_json::to_string(&req).unwrap();
    let parsed: ThreadRequest = serde_json::from_str(&json).unwrap();
    match parsed {
        ThreadRequest::Create { metadata } => assert_eq!(metadata["key"], "val"),
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn thread_request_start_round_trip() {
    let req = ThreadRequest::Start(ThreadStartParams {
        model: Some("deepseek-v4-flash".to_string()),
        model_provider: None,
        cwd: None,
        persist_extended_history: false,
    });
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("start"));
    let parsed: ThreadRequest = serde_json::from_str(&json).unwrap();
    match parsed {
        ThreadRequest::Start(params) => {
            assert_eq!(params.model.as_deref(), Some("deepseek-v4-flash"));
            assert!(!params.persist_extended_history);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn thread_request_fork_round_trip() {
    let req = ThreadRequest::Fork(ThreadForkParams {
        thread_id: "parent-1".to_string(),
        path: None,
        model: None,
        model_provider: None,
        cwd: None,
        approval_policy: None,
        sandbox: None,
        config: None,
        base_instructions: None,
        developer_instructions: None,
        persist_extended_history: true,
    });
    let json = serde_json::to_string(&req).unwrap();
    let parsed: ThreadRequest = serde_json::from_str(&json).unwrap();
    match parsed {
        ThreadRequest::Fork(params) => {
            assert_eq!(params.thread_id, "parent-1");
            assert!(params.persist_extended_history);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn thread_request_read_round_trip() {
    let req = ThreadRequest::Read(ThreadReadParams {
        thread_id: "t-read".to_string(),
    });
    let json = serde_json::to_string(&req).unwrap();
    let parsed: ThreadRequest = serde_json::from_str(&json).unwrap();
    match parsed {
        ThreadRequest::Read(params) => assert_eq!(params.thread_id, "t-read"),
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn thread_request_set_name_round_trip() {
    let req = ThreadRequest::SetName(ThreadSetNameParams {
        thread_id: "t-1".to_string(),
        name: "My Session".to_string(),
    });
    let json = serde_json::to_string(&req).unwrap();
    let parsed: ThreadRequest = serde_json::from_str(&json).unwrap();
    match parsed {
        ThreadRequest::SetName(params) => {
            assert_eq!(params.name, "My Session");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn thread_request_archive_unarchive_message() {
    for req in [
        ThreadRequest::Archive {
            thread_id: "t-a".to_string(),
        },
        ThreadRequest::Unarchive {
            thread_id: "t-u".to_string(),
        },
        ThreadRequest::Message {
            thread_id: "t-m".to_string(),
            input: "hello".to_string(),
        },
    ] {
        let json = serde_json::to_string(&req).unwrap();
        let _: ThreadRequest = serde_json::from_str(&json).unwrap();
    }
}

// --- AppRequest / AppResponse ---

#[test]
fn app_request_all_variants_serialize() {
    let variants = [
        serde_json::to_string(&AppRequest::Capabilities).unwrap(),
        serde_json::to_string(&AppRequest::ConfigGet {
            key: "k".to_string(),
        })
        .unwrap(),
        serde_json::to_string(&AppRequest::ConfigSet {
            key: "k".to_string(),
            value: "v".to_string(),
        })
        .unwrap(),
        serde_json::to_string(&AppRequest::ConfigUnset {
            key: "k".to_string(),
        })
        .unwrap(),
        serde_json::to_string(&AppRequest::ConfigList).unwrap(),
        serde_json::to_string(&AppRequest::Models).unwrap(),
        serde_json::to_string(&AppRequest::ThreadLoadedList).unwrap(),
    ];
    for json in &variants {
        let _: AppRequest = serde_json::from_str(json).unwrap();
    }
}

#[test]
fn app_response_round_trip() {
    let resp = AppResponse {
        ok: true,
        data: json!({"models": ["deepseek-v4-pro"]}),
        events: vec![],
    };
    let json = serde_json::to_string(&resp).unwrap();
    let parsed: AppResponse = serde_json::from_str(&json).unwrap();
    assert!(parsed.ok);
    assert!(parsed.events.is_empty());
}

// --- PromptRequest / PromptResponse ---

#[test]
fn prompt_request_round_trip() {
    let req = PromptRequest {
        thread_id: Some("t1".to_string()),
        prompt: "explain this".to_string(),
        model: Some("deepseek-v4-pro".to_string()),
    };
    let json = serde_json::to_string(&req).unwrap();
    let parsed: PromptRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.prompt, "explain this");
}

#[test]
fn prompt_response_round_trip() {
    let resp = PromptResponse {
        output: "result text".to_string(),
        model: "deepseek-v4-flash".to_string(),
        events: vec![],
    };
    let json = serde_json::to_string(&resp).unwrap();
    let parsed: PromptResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.model, "deepseek-v4-flash");
}

// --- ToolPayload ---

#[test]
fn tool_payload_function_round_trip() {
    let payload = ToolPayload::Function {
        arguments: r#"{"file":"test.rs"}"#.to_string(),
    };
    let json = serde_json::to_string(&payload).unwrap();
    assert!(json.contains("function"));
    let parsed: ToolPayload = serde_json::from_str(&json).unwrap();
    match parsed {
        ToolPayload::Function { arguments } => assert!(arguments.contains("test.rs")),
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn tool_payload_local_shell_round_trip() {
    let payload = ToolPayload::LocalShell {
        params: LocalShellParams {
            command: "ls -la".to_string(),
            cwd: Some("/tmp".to_string()),
            timeout_ms: Some(5000),
        },
    };
    let json = serde_json::to_string(&payload).unwrap();
    let parsed: ToolPayload = serde_json::from_str(&json).unwrap();
    match parsed {
        ToolPayload::LocalShell { params } => {
            assert_eq!(params.command, "ls -la");
            assert_eq!(params.timeout_ms, Some(5000));
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn tool_payload_mcp_round_trip() {
    let payload = ToolPayload::Mcp {
        server: "srv".to_string(),
        tool: "ping".to_string(),
        raw_arguments: json!({"key": "val"}),
        raw_tool_call_id: Some("tc-1".to_string()),
    };
    let json = serde_json::to_string(&payload).unwrap();
    let parsed: ToolPayload = serde_json::from_str(&json).unwrap();
    match parsed {
        ToolPayload::Mcp {
            server,
            tool,
            raw_tool_call_id,
            ..
        } => {
            assert_eq!(server, "srv");
            assert_eq!(tool, "ping");
            assert_eq!(raw_tool_call_id.as_deref(), Some("tc-1"));
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

// --- ToolOutput ---

#[test]
fn tool_output_function_round_trip() {
    let output = ToolOutput::Function {
        body: Some(json!({"result": 42})),
        success: true,
    };
    let json = serde_json::to_string(&output).unwrap();
    let parsed: ToolOutput = serde_json::from_str(&json).unwrap();
    match parsed {
        ToolOutput::Function { body, success } => {
            assert!(success);
            assert_eq!(body.unwrap()["result"], 42);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn tool_output_mcp_round_trip() {
    let output = ToolOutput::Mcp {
        result: json!({"status": "ok"}),
    };
    let json = serde_json::to_string(&output).unwrap();
    let parsed: ToolOutput = serde_json::from_str(&json).unwrap();
    match parsed {
        ToolOutput::Mcp { result } => assert_eq!(result["status"], "ok"),
        other => panic!("wrong variant: {other:?}"),
    }
}

// --- ReviewDecision ---

#[test]
fn review_decision_all_variants_round_trip() {
    let decisions = [
        ReviewDecision::Approved,
        ReviewDecision::ApprovedExecpolicyAmendment,
        ReviewDecision::ApprovedForSession,
        ReviewDecision::NetworkPolicyAmendment {
            host: "example.com".to_string(),
            action: NetworkPolicyRuleAction::Allow,
        },
        ReviewDecision::Denied,
        ReviewDecision::Abort,
    ];
    for decision in &decisions {
        let json = serde_json::to_string(decision).unwrap();
        let parsed: ReviewDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(&parsed, decision);
    }
}

// --- AskForApproval ---

#[test]
fn ask_for_approval_variants_round_trip() {
    let variants = [
        AskForApproval::UnlessTrusted,
        AskForApproval::OnFailure,
        AskForApproval::OnRequest,
        AskForApproval::Reject {
            sandbox_approval: true,
            rules: false,
            mcp_elicitations: true,
        },
        AskForApproval::Never,
    ];
    for variant in &variants {
        let json = serde_json::to_string(variant).unwrap();
        let parsed: AskForApproval = serde_json::from_str(&json).unwrap();
        assert_eq!(&parsed, variant);
    }
}

// --- ToolKind ---

#[test]
fn tool_kind_round_trip() {
    for kind in [ToolKind::Function, ToolKind::Mcp] {
        let json = serde_json::to_string(&kind).unwrap();
        let parsed: ToolKind = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, kind);
    }
}

// --- ResponseChannel ---

#[test]
fn response_channel_default_is_text() {
    let channel = ResponseChannel::default();
    assert!(channel.is_text());
}

#[test]
fn response_channel_reasoning_is_not_text() {
    assert!(!ResponseChannel::Reasoning.is_text());
}

// --- EventFrame variants ---

#[test]
fn event_frame_response_delta_with_reasoning_channel() {
    let frame = EventFrame::ResponseDelta {
        response_id: "r1".to_string(),
        delta: "thinking...".to_string(),
        channel: ResponseChannel::Reasoning,
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("reasoning"));
    let parsed: EventFrame = serde_json::from_str(&json).unwrap();
    match parsed {
        EventFrame::ResponseDelta { channel, .. } => {
            assert_eq!(channel, ResponseChannel::Reasoning);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn event_frame_response_delta_text_channel_omitted() {
    let frame = EventFrame::ResponseDelta {
        response_id: "r2".to_string(),
        delta: "output".to_string(),
        channel: ResponseChannel::Text,
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(
        !json.contains("channel"),
        "text channel should be omitted via skip_serializing_if"
    );
}

#[test]
fn event_frame_tool_call_start_round_trip() {
    let frame = EventFrame::ToolCallStart {
        response_id: "r1".to_string(),
        tool_name: "read_file".to_string(),
        arguments: json!({"path": "/foo"}),
    };
    let json = serde_json::to_string(&frame).unwrap();
    let parsed: EventFrame = serde_json::from_str(&json).unwrap();
    match parsed {
        EventFrame::ToolCallStart { tool_name, .. } => assert_eq!(tool_name, "read_file"),
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn event_frame_exec_command_lifecycle() {
    let begin = EventFrame::ExecCommandBegin {
        command: "cargo test".to_string(),
        cwd: "/project".to_string(),
    };
    let delta = EventFrame::ExecCommandOutputDelta {
        command: "cargo test".to_string(),
        delta: "running 5 tests\n".to_string(),
    };
    let end = EventFrame::ExecCommandEnd {
        command: "cargo test".to_string(),
        exit_code: 0,
    };
    for frame in [&begin, &delta, &end] {
        let json = serde_json::to_string(frame).unwrap();
        let _: EventFrame = serde_json::from_str(&json).unwrap();
    }
}

#[test]
fn event_frame_mcp_startup_update() {
    let frame = EventFrame::McpStartupUpdate {
        update: McpStartupUpdateEvent {
            server_name: "test".to_string(),
            status: McpStartupStatus::Ready,
        },
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("mcp_startup_update"));
}

#[test]
fn event_frame_mcp_startup_complete() {
    let frame = EventFrame::McpStartupComplete {
        summary: McpStartupCompleteEvent {
            ready: vec!["srv1".to_string()],
            failed: vec![McpStartupFailure {
                server_name: "srv2".to_string(),
                error: "timeout".to_string(),
            }],
            cancelled: vec![],
        },
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("srv1"));
    assert!(json.contains("timeout"));
}

#[test]
fn event_frame_error_variant() {
    let frame = EventFrame::Error {
        response_id: "err-1".to_string(),
        message: "something broke".to_string(),
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("error"));
    assert!(json.contains("something broke"));
}

#[test]
fn event_frame_turn_aborted() {
    let frame = EventFrame::TurnAborted {
        turn_id: "turn-x".to_string(),
        reason: "user cancelled".to_string(),
    };
    let json = serde_json::to_string(&frame).unwrap();
    let parsed: EventFrame = serde_json::from_str(&json).unwrap();
    match parsed {
        EventFrame::TurnAborted { reason, .. } => assert_eq!(reason, "user cancelled"),
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn event_frame_patch_apply_lifecycle() {
    let begin = EventFrame::PatchApplyBegin {
        path: "src/main.rs".to_string(),
    };
    let end = EventFrame::PatchApplyEnd {
        path: "src/main.rs".to_string(),
        ok: true,
    };
    for frame in [&begin, &end] {
        let json = serde_json::to_string(frame).unwrap();
        let _: EventFrame = serde_json::from_str(&json).unwrap();
    }
}

// --- NetworkPolicyAmendment ---

#[test]
fn network_policy_amendment_round_trip() {
    let amendment = NetworkPolicyAmendment {
        host: "api.example.com".to_string(),
        action: NetworkPolicyRuleAction::Deny,
    };
    let json = serde_json::to_string(&amendment).unwrap();
    let parsed: NetworkPolicyAmendment = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.host, "api.example.com");
    assert_eq!(parsed.action, NetworkPolicyRuleAction::Deny);
}

// --- ThreadResponse ---

#[test]
fn thread_response_optional_fields_omitted_when_none() {
    let resp = ThreadResponse {
        thread_id: "t1".to_string(),
        status: "ok".to_string(),
        thread: None,
        threads: vec![],
        model: None,
        model_provider: None,
        cwd: None,
        approval_policy: None,
        sandbox: None,
        events: vec![],
        data: json!({}),
    };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(!json.contains("\"model\""));
    assert!(!json.contains("\"cwd\""));
}
