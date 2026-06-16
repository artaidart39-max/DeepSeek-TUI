use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use deepseek_protocol::EventFrame;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HookEvent {
    ResponseStart {
        response_id: String,
    },
    ResponseDelta {
        response_id: String,
        delta: String,
    },
    ResponseEnd {
        response_id: String,
    },
    ToolLifecycle {
        response_id: String,
        tool_name: String,
        phase: String,
        payload: Value,
    },
    JobLifecycle {
        job_id: String,
        phase: String,
        progress: Option<u8>,
        detail: Option<String>,
    },
    ApprovalLifecycle {
        approval_id: String,
        phase: String,
        reason: Option<String>,
    },
    GenericEventFrame {
        frame: EventFrame,
    },
}

impl HookEvent {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({"type":"serialization_error"}))
    }
}

#[async_trait]
pub trait HookSink: Send + Sync {
    async fn emit(&self, event: &HookEvent) -> Result<()>;
}

#[derive(Default)]
pub struct StdoutHookSink;

#[async_trait]
impl HookSink for StdoutHookSink {
    async fn emit(&self, event: &HookEvent) -> Result<()> {
        println!("{}", event.to_json());
        Ok(())
    }
}

pub struct JsonlHookSink {
    path: PathBuf,
}

impl JsonlHookSink {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

#[async_trait]
impl HookSink for JsonlHookSink {
    async fn emit(&self, event: &HookEvent) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("failed to create hook log directory {}", parent.display())
            })?;
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .with_context(|| format!("failed to open hook log {}", self.path.display()))?;
        let payload = json!({
            "at": Utc::now().to_rfc3339(),
            "event": event
        });
        let encoded = serde_json::to_string(&payload).context("failed to encode hook event")?;
        file.write_all(encoded.as_bytes())
            .await
            .context("failed to write hook event")?;
        file.write_all(b"\n")
            .await
            .context("failed to write hook event newline")?;
        Ok(())
    }
}

pub struct WebhookHookSink {
    url: String,
    client: reqwest::Client,
}

impl WebhookHookSink {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl HookSink for WebhookHookSink {
    async fn emit(&self, event: &HookEvent) -> Result<()> {
        let mut retries = 0usize;
        loop {
            let resp = self
                .client
                .post(&self.url)
                .json(&json!({
                    "at": Utc::now().to_rfc3339(),
                    "event": event,
                }))
                .send()
                .await;
            match resp {
                Ok(response) if response.status().is_success() => return Ok(()),
                Ok(response) => {
                    if retries >= 2 {
                        anyhow::bail!("webhook returned non-success status {}", response.status());
                    }
                }
                Err(err) => {
                    if retries >= 2 {
                        return Err(err).context("webhook request failed");
                    }
                }
            }
            retries += 1;
            tokio::time::sleep(std::time::Duration::from_millis(200 * retries as u64)).await;
        }
    }
}

#[derive(Default, Clone)]
pub struct HookDispatcher {
    sinks: Vec<Arc<dyn HookSink>>,
}

impl HookDispatcher {
    pub fn add_sink(&mut self, sink: Arc<dyn HookSink>) {
        self.sinks.push(sink);
    }

    pub async fn emit(&self, event: HookEvent) {
        for sink in &self.sinks {
            let _ = sink.emit(&event).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn hook_event_response_start_serializes_with_type_tag() {
        let event = HookEvent::ResponseStart {
            response_id: "resp-1".to_string(),
        };
        let json = event.to_json();
        assert_eq!(json["type"], "response_start");
        assert_eq!(json["response_id"], "resp-1");
    }

    #[test]
    fn hook_event_response_delta_round_trips() {
        let event = HookEvent::ResponseDelta {
            response_id: "resp-2".to_string(),
            delta: "hello world".to_string(),
        };
        let serialized = serde_json::to_string(&event).unwrap();
        let deserialized: HookEvent = serde_json::from_str(&serialized).unwrap();
        match deserialized {
            HookEvent::ResponseDelta { response_id, delta } => {
                assert_eq!(response_id, "resp-2");
                assert_eq!(delta, "hello world");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn hook_event_tool_lifecycle_includes_payload() {
        let event = HookEvent::ToolLifecycle {
            response_id: "resp-3".to_string(),
            tool_name: "read_file".to_string(),
            phase: "started".to_string(),
            payload: json!({"path": "/tmp/foo.txt"}),
        };
        let json = event.to_json();
        assert_eq!(json["type"], "tool_lifecycle");
        assert_eq!(json["tool_name"], "read_file");
        assert_eq!(json["phase"], "started");
        assert_eq!(json["payload"]["path"], "/tmp/foo.txt");
    }

    #[test]
    fn hook_event_job_lifecycle_optional_fields() {
        let event = HookEvent::JobLifecycle {
            job_id: "job-1".to_string(),
            phase: "completed".to_string(),
            progress: Some(100),
            detail: None,
        };
        let json = event.to_json();
        assert_eq!(json["type"], "job_lifecycle");
        assert_eq!(json["progress"], 100);
        assert!(json["detail"].is_null());
    }

    #[test]
    fn hook_event_approval_lifecycle_with_reason() {
        let event = HookEvent::ApprovalLifecycle {
            approval_id: "approval-1".to_string(),
            phase: "denied".to_string(),
            reason: Some("dangerous command".to_string()),
        };
        let json = event.to_json();
        assert_eq!(json["type"], "approval_lifecycle");
        assert_eq!(json["reason"], "dangerous command");
    }

    #[test]
    fn hook_event_generic_event_frame_wraps_frame() {
        let frame = EventFrame::TurnComplete {
            turn_id: "turn-42".to_string(),
        };
        let event = HookEvent::GenericEventFrame {
            frame: frame.clone(),
        };
        let json = event.to_json();
        assert_eq!(json["type"], "generic_event_frame");
        assert!(json["frame"].is_object());
    }

    #[test]
    fn hook_event_response_end_round_trips() {
        let event = HookEvent::ResponseEnd {
            response_id: "resp-end".to_string(),
        };
        let serialized = serde_json::to_string(&event).unwrap();
        let deserialized: HookEvent = serde_json::from_str(&serialized).unwrap();
        match deserialized {
            HookEvent::ResponseEnd { response_id } => {
                assert_eq!(response_id, "resp-end");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    struct CollectorSink {
        events: Mutex<Vec<Value>>,
    }

    impl CollectorSink {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl HookSink for CollectorSink {
        async fn emit(&self, event: &HookEvent) -> Result<()> {
            self.events.lock().unwrap().push(event.to_json());
            Ok(())
        }
    }

    #[tokio::test]
    async fn dispatcher_emits_to_all_sinks() {
        let sink_a = Arc::new(CollectorSink::new());
        let sink_b = Arc::new(CollectorSink::new());
        let mut dispatcher = HookDispatcher::default();
        dispatcher.add_sink(sink_a.clone());
        dispatcher.add_sink(sink_b.clone());

        dispatcher
            .emit(HookEvent::ResponseStart {
                response_id: "r1".to_string(),
            })
            .await;

        assert_eq!(sink_a.events.lock().unwrap().len(), 1);
        assert_eq!(sink_b.events.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn dispatcher_with_no_sinks_does_not_panic() {
        let dispatcher = HookDispatcher::default();
        dispatcher
            .emit(HookEvent::ResponseEnd {
                response_id: "r2".to_string(),
            })
            .await;
    }

    struct FailingSink;

    #[async_trait]
    impl HookSink for FailingSink {
        async fn emit(&self, _event: &HookEvent) -> Result<()> {
            anyhow::bail!("sink failure")
        }
    }

    #[tokio::test]
    async fn dispatcher_continues_after_sink_failure() {
        let good_sink = Arc::new(CollectorSink::new());
        let mut dispatcher = HookDispatcher::default();
        dispatcher.add_sink(Arc::new(FailingSink));
        dispatcher.add_sink(good_sink.clone());

        dispatcher
            .emit(HookEvent::ResponseStart {
                response_id: "r3".to_string(),
            })
            .await;

        assert_eq!(good_sink.events.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn jsonl_sink_writes_to_file() {
        let dir = std::env::temp_dir().join(format!(
            "deepseek_hooks_test_{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let log_path = dir.join("hooks.jsonl");
        let sink = JsonlHookSink::new(log_path.clone());

        sink.emit(&HookEvent::ResponseStart {
            response_id: "file-test".to_string(),
        })
        .await
        .unwrap();

        let contents = tokio::fs::read_to_string(&log_path).await.unwrap();
        assert!(contents.contains("file-test"));
        assert!(contents.contains("response_start"));
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
