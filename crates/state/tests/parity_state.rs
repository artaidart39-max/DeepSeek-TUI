use std::path::PathBuf;

use deepseek_state::{
    DynamicToolRecord, JobStateRecord, JobStateStatus, SessionSource, StateStore,
    ThreadListFilters, ThreadMetadata, ThreadStatus,
};
use serde_json::json;

fn temp_state_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "deepseek_state_test_{}_{}_{}.db",
        label,
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ))
}

fn sample_thread(id: &str) -> ThreadMetadata {
    let now = chrono::Utc::now().timestamp();
    ThreadMetadata {
        id: id.to_string(),
        rollout_path: None,
        preview: "test preview".to_string(),
        ephemeral: false,
        model_provider: "deepseek".to_string(),
        created_at: now,
        updated_at: now,
        status: ThreadStatus::Running,
        path: None,
        cwd: PathBuf::from("/tmp"),
        cli_version: "0.0.0-test".to_string(),
        source: SessionSource::Interactive,
        name: None,
        sandbox_policy: None,
        approval_mode: None,
        archived: false,
        archived_at: None,
        git_sha: None,
        git_branch: None,
        git_origin_url: None,
        memory_mode: None,
    }
}

#[test]
fn upsert_and_resume_thread_metadata() {
    let path = temp_state_path("upsert_resume");
    let store = StateStore::open(Some(path.clone())).expect("open state store");
    let now = chrono::Utc::now().timestamp();
    let thread = ThreadMetadata {
        id: "thread-test-1".to_string(),
        rollout_path: Some(PathBuf::from("/tmp/rollout.jsonl")),
        preview: "hello".to_string(),
        ephemeral: false,
        model_provider: "deepseek".to_string(),
        created_at: now,
        updated_at: now,
        status: ThreadStatus::Running,
        path: Some(PathBuf::from("/tmp/project")),
        cwd: PathBuf::from("/tmp/project"),
        cli_version: "0.0.0-test".to_string(),
        source: SessionSource::Interactive,
        name: Some("Test Thread".to_string()),
        sandbox_policy: Some("workspace-write".to_string()),
        approval_mode: Some("on-request".to_string()),
        archived: false,
        archived_at: None,
        git_sha: None,
        git_branch: None,
        git_origin_url: None,
        memory_mode: Some("extended".to_string()),
    };
    store.upsert_thread(&thread).expect("upsert thread");

    let loaded = store
        .get_thread("thread-test-1")
        .expect("read thread")
        .expect("thread must exist");
    assert_eq!(loaded.id, "thread-test-1");
    assert_eq!(loaded.name.as_deref(), Some("Test Thread"));
    assert_eq!(loaded.memory_mode.as_deref(), Some("extended"));
    assert_eq!(
        loaded.rollout_path,
        Some(PathBuf::from("/tmp/rollout.jsonl"))
    );

    store
        .mark_archived("thread-test-1")
        .expect("archive thread");
    let archived = store
        .get_thread("thread-test-1")
        .expect("read archived thread")
        .expect("thread exists after archive");
    assert!(archived.archived);

    let listed = store
        .list_threads(ThreadListFilters {
            include_archived: true,
            limit: Some(10),
        })
        .expect("list threads");
    assert!(!listed.is_empty());
}

// --- Messages CRUD ---

#[test]
fn append_and_list_messages() {
    let path = temp_state_path("messages");
    let store = StateStore::open(Some(path)).unwrap();
    let thread = sample_thread("thread-msg-1");
    store.upsert_thread(&thread).unwrap();

    let id1 = store
        .append_message("thread-msg-1", "user", "Hello", None)
        .unwrap();
    let id2 = store
        .append_message(
            "thread-msg-1",
            "assistant",
            "Hi there",
            Some(json!({"model": "v4"})),
        )
        .unwrap();

    assert!(id2 > id1);

    let messages = store.list_messages("thread-msg-1", None).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "Hello");
    assert!(messages[0].item.is_none());
    assert_eq!(messages[1].role, "assistant");
    assert!(messages[1].item.is_some());
}

#[test]
fn clear_messages_removes_all() {
    let path = temp_state_path("clear_messages");
    let store = StateStore::open(Some(path)).unwrap();
    let thread = sample_thread("thread-clear-1");
    store.upsert_thread(&thread).unwrap();

    store
        .append_message("thread-clear-1", "user", "msg1", None)
        .unwrap();
    store
        .append_message("thread-clear-1", "user", "msg2", None)
        .unwrap();

    let deleted = store.clear_messages("thread-clear-1").unwrap();
    assert_eq!(deleted, 2);

    let messages = store.list_messages("thread-clear-1", None).unwrap();
    assert!(messages.is_empty());
}

#[test]
fn list_messages_respects_limit() {
    let path = temp_state_path("messages_limit");
    let store = StateStore::open(Some(path)).unwrap();
    let thread = sample_thread("thread-lim-1");
    store.upsert_thread(&thread).unwrap();

    for i in 0..5 {
        store
            .append_message("thread-lim-1", "user", &format!("msg {i}"), None)
            .unwrap();
    }

    let messages = store.list_messages("thread-lim-1", Some(3)).unwrap();
    assert_eq!(messages.len(), 3);
}

// --- Checkpoints ---

#[test]
fn save_and_load_checkpoint() {
    let path = temp_state_path("checkpoints");
    let store = StateStore::open(Some(path)).unwrap();
    let thread = sample_thread("thread-cp-1");
    store.upsert_thread(&thread).unwrap();

    let state = json!({"progress": 50, "turn": 3});
    store
        .save_checkpoint("thread-cp-1", "cp-1", &state)
        .unwrap();

    let loaded = store
        .load_checkpoint("thread-cp-1", Some("cp-1"))
        .unwrap()
        .unwrap();
    assert_eq!(loaded.checkpoint_id, "cp-1");
    assert_eq!(loaded.state["progress"], 50);
}

#[test]
fn load_latest_checkpoint_returns_some_when_multiple_exist() {
    let path = temp_state_path("latest_cp");
    let store = StateStore::open(Some(path)).unwrap();
    let thread = sample_thread("thread-lcp-1");
    store.upsert_thread(&thread).unwrap();

    store
        .save_checkpoint("thread-lcp-1", "cp-a", &json!({"v": 1}))
        .unwrap();
    store
        .save_checkpoint("thread-lcp-1", "cp-b", &json!({"v": 2}))
        .unwrap();

    let latest = store
        .load_checkpoint("thread-lcp-1", None)
        .unwrap()
        .unwrap();
    // Both share the same second-precision timestamp, so either may be returned
    assert!(
        latest.checkpoint_id == "cp-a" || latest.checkpoint_id == "cp-b",
        "expected one of the saved checkpoints, got: {}",
        latest.checkpoint_id
    );
}

#[test]
fn list_checkpoints() {
    let path = temp_state_path("list_cp");
    let store = StateStore::open(Some(path)).unwrap();
    let thread = sample_thread("thread-listcp-1");
    store.upsert_thread(&thread).unwrap();

    store
        .save_checkpoint("thread-listcp-1", "a", &json!({}))
        .unwrap();
    store
        .save_checkpoint("thread-listcp-1", "b", &json!({}))
        .unwrap();

    let list = store.list_checkpoints("thread-listcp-1", None).unwrap();
    assert_eq!(list.len(), 2);
}

#[test]
fn delete_checkpoint() {
    let path = temp_state_path("delete_cp");
    let store = StateStore::open(Some(path)).unwrap();
    let thread = sample_thread("thread-dcp-1");
    store.upsert_thread(&thread).unwrap();

    store
        .save_checkpoint("thread-dcp-1", "to-delete", &json!({}))
        .unwrap();
    store
        .delete_checkpoint("thread-dcp-1", "to-delete")
        .unwrap();

    let loaded = store
        .load_checkpoint("thread-dcp-1", Some("to-delete"))
        .unwrap();
    assert!(loaded.is_none());
}

// --- Jobs ---

#[test]
fn upsert_and_get_job() {
    let path = temp_state_path("jobs");
    let store = StateStore::open(Some(path)).unwrap();

    let job = JobStateRecord {
        id: "job-1".to_string(),
        name: "build".to_string(),
        status: JobStateStatus::Running,
        progress: Some(50),
        detail: Some("compiling".to_string()),
        created_at: 1000,
        updated_at: 2000,
    };
    store.upsert_job(&job).unwrap();

    let loaded = store.get_job("job-1").unwrap().unwrap();
    assert_eq!(loaded.name, "build");
    assert_eq!(loaded.status, JobStateStatus::Running);
    assert_eq!(loaded.progress, Some(50));
    assert_eq!(loaded.detail.as_deref(), Some("compiling"));
}

#[test]
fn list_jobs_ordered_by_updated_at() {
    let path = temp_state_path("list_jobs");
    let store = StateStore::open(Some(path)).unwrap();

    for (i, status) in [
        JobStateStatus::Queued,
        JobStateStatus::Running,
        JobStateStatus::Completed,
    ]
    .iter()
    .enumerate()
    {
        store
            .upsert_job(&JobStateRecord {
                id: format!("job-{i}"),
                name: format!("task-{i}"),
                status: status.clone(),
                progress: None,
                detail: None,
                created_at: 1000,
                updated_at: (i as i64 + 1) * 1000,
            })
            .unwrap();
    }

    let jobs = store.list_jobs(Some(10)).unwrap();
    assert_eq!(jobs.len(), 3);
    assert!(jobs[0].updated_at >= jobs[1].updated_at);
}

#[test]
fn delete_job() {
    let path = temp_state_path("delete_job");
    let store = StateStore::open(Some(path)).unwrap();

    store
        .upsert_job(&JobStateRecord {
            id: "job-del".to_string(),
            name: "deletable".to_string(),
            status: JobStateStatus::Completed,
            progress: Some(100),
            detail: None,
            created_at: 1000,
            updated_at: 1000,
        })
        .unwrap();

    store.delete_job("job-del").unwrap();
    assert!(store.get_job("job-del").unwrap().is_none());
}

#[test]
fn job_status_round_trip_all_variants() {
    let path = temp_state_path("job_status_rt");
    let store = StateStore::open(Some(path)).unwrap();

    for (i, status) in [
        JobStateStatus::Queued,
        JobStateStatus::Running,
        JobStateStatus::Completed,
        JobStateStatus::Failed,
        JobStateStatus::Cancelled,
    ]
    .iter()
    .enumerate()
    {
        let job = JobStateRecord {
            id: format!("job-rt-{i}"),
            name: "test".to_string(),
            status: status.clone(),
            progress: None,
            detail: None,
            created_at: 1000,
            updated_at: 1000,
        };
        store.upsert_job(&job).unwrap();
        let loaded = store.get_job(&format!("job-rt-{i}")).unwrap().unwrap();
        assert_eq!(loaded.status, *status);
    }
}

// --- Dynamic tools ---

#[test]
fn persist_and_get_dynamic_tools() {
    let path = temp_state_path("dynamic_tools");
    let store = StateStore::open(Some(path)).unwrap();
    let thread = sample_thread("thread-dt-1");
    store.upsert_thread(&thread).unwrap();

    let tools = vec![
        DynamicToolRecord {
            position: 0,
            name: "custom_search".to_string(),
            description: Some("Search the codebase".to_string()),
            input_schema: json!({"type": "object", "properties": {"query": {"type": "string"}}}),
        },
        DynamicToolRecord {
            position: 1,
            name: "custom_lint".to_string(),
            description: None,
            input_schema: json!({"type": "object"}),
        },
    ];

    store.persist_dynamic_tools("thread-dt-1", &tools).unwrap();

    let loaded = store.get_dynamic_tools("thread-dt-1").unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].name, "custom_search");
    assert_eq!(loaded[1].name, "custom_lint");
    assert!(loaded[0].description.is_some());
    assert!(loaded[1].description.is_none());
}

#[test]
fn persist_dynamic_tools_replaces_old_set() {
    let path = temp_state_path("dynamic_tools_replace");
    let store = StateStore::open(Some(path)).unwrap();
    let thread = sample_thread("thread-dtr-1");
    store.upsert_thread(&thread).unwrap();

    store
        .persist_dynamic_tools(
            "thread-dtr-1",
            &[DynamicToolRecord {
                position: 0,
                name: "old_tool".to_string(),
                description: None,
                input_schema: json!({}),
            }],
        )
        .unwrap();

    store
        .persist_dynamic_tools(
            "thread-dtr-1",
            &[DynamicToolRecord {
                position: 0,
                name: "new_tool".to_string(),
                description: None,
                input_schema: json!({}),
            }],
        )
        .unwrap();

    let loaded = store.get_dynamic_tools("thread-dtr-1").unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].name, "new_tool");
}

// --- Memory mode ---

#[test]
fn set_and_get_thread_memory_mode() {
    let path = temp_state_path("memory_mode");
    let store = StateStore::open(Some(path)).unwrap();
    let thread = sample_thread("thread-mm-1");
    store.upsert_thread(&thread).unwrap();

    assert!(
        store
            .get_thread_memory_mode("thread-mm-1")
            .unwrap()
            .is_none()
    );

    store
        .set_thread_memory_mode("thread-mm-1", Some("extended"))
        .unwrap();
    assert_eq!(
        store
            .get_thread_memory_mode("thread-mm-1")
            .unwrap()
            .as_deref(),
        Some("extended")
    );

    store.set_thread_memory_mode("thread-mm-1", None).unwrap();
    assert!(
        store
            .get_thread_memory_mode("thread-mm-1")
            .unwrap()
            .is_none()
    );
}

// --- Archive / Unarchive ---

#[test]
fn archive_and_unarchive_thread() {
    let path = temp_state_path("archive_unarchive");
    let store = StateStore::open(Some(path)).unwrap();
    let thread = sample_thread("thread-au-1");
    store.upsert_thread(&thread).unwrap();

    store.mark_archived("thread-au-1").unwrap();
    let t = store.get_thread("thread-au-1").unwrap().unwrap();
    assert!(t.archived);
    assert_eq!(t.status, ThreadStatus::Archived);

    store.mark_unarchived("thread-au-1").unwrap();
    let t = store.get_thread("thread-au-1").unwrap().unwrap();
    assert!(!t.archived);
}

// --- Delete thread ---

#[test]
fn delete_thread() {
    let path = temp_state_path("delete_thread");
    let store = StateStore::open(Some(path)).unwrap();
    let thread = sample_thread("thread-del-1");
    store.upsert_thread(&thread).unwrap();

    store.delete_thread("thread-del-1").unwrap();
    assert!(store.get_thread("thread-del-1").unwrap().is_none());
}

// --- List filters ---

#[test]
fn list_threads_excludes_archived_by_default() {
    let path = temp_state_path("list_filter_archived");
    let store = StateStore::open(Some(path)).unwrap();

    let mut t1 = sample_thread("thread-lf-1");
    t1.name = Some("Active".to_string());
    store.upsert_thread(&t1).unwrap();

    let mut t2 = sample_thread("thread-lf-2");
    t2.name = Some("Archived".to_string());
    store.upsert_thread(&t2).unwrap();
    store.mark_archived("thread-lf-2").unwrap();

    let non_archived = store
        .list_threads(ThreadListFilters {
            include_archived: false,
            limit: Some(50),
        })
        .unwrap();
    assert_eq!(non_archived.len(), 1);
    assert_eq!(non_archived[0].id, "thread-lf-1");

    let all = store
        .list_threads(ThreadListFilters {
            include_archived: true,
            limit: Some(50),
        })
        .unwrap();
    assert_eq!(all.len(), 2);
}

// --- Get nonexistent thread ---

#[test]
fn get_nonexistent_thread_returns_none() {
    let path = temp_state_path("nonexistent");
    let store = StateStore::open(Some(path)).unwrap();
    assert!(store.get_thread("no-such-id").unwrap().is_none());
}

// --- Session index (thread name lookup) ---

#[test]
fn find_thread_name_by_id() {
    let path = temp_state_path("thread_name_lookup");
    let store = StateStore::open(Some(path)).unwrap();
    let mut thread = sample_thread("thread-name-1");
    thread.name = Some("My Session".to_string());
    store.upsert_thread(&thread).unwrap();

    let name = store.find_thread_name_by_id("thread-name-1").unwrap();
    assert_eq!(name.as_deref(), Some("My Session"));
}

#[test]
fn find_thread_names_by_ids() {
    let path = temp_state_path("thread_names_batch");
    let store = StateStore::open(Some(path)).unwrap();

    let mut t1 = sample_thread("tn-1");
    t1.name = Some("First".to_string());
    store.upsert_thread(&t1).unwrap();

    let mut t2 = sample_thread("tn-2");
    t2.name = Some("Second".to_string());
    store.upsert_thread(&t2).unwrap();

    let names = store
        .find_thread_names_by_ids(&[
            "tn-1".to_string(),
            "tn-2".to_string(),
            "tn-missing".to_string(),
        ])
        .unwrap();
    assert_eq!(names.get("tn-1").unwrap().as_deref(), Some("First"));
    assert_eq!(names.get("tn-2").unwrap().as_deref(), Some("Second"));
    assert!(names.get("tn-missing").unwrap().is_none());
}

// --- Rollout path ---

#[test]
fn find_rollout_path_by_id() {
    let path = temp_state_path("rollout_path");
    let store = StateStore::open(Some(path)).unwrap();
    let mut thread = sample_thread("thread-rp-1");
    thread.rollout_path = Some(PathBuf::from("/data/rollout.jsonl"));
    store.upsert_thread(&thread).unwrap();

    let rp = store.find_rollout_path_by_id("thread-rp-1").unwrap();
    assert_eq!(rp, Some(PathBuf::from("/data/rollout.jsonl")));
}

#[test]
fn find_rollout_path_missing_returns_none() {
    let path = temp_state_path("rollout_missing");
    let store = StateStore::open(Some(path)).unwrap();
    assert!(store.find_rollout_path_by_id("nope").unwrap().is_none());
}
