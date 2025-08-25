use std::{collections::HashMap, str::FromStr, sync::Arc, time::Instant};

use anyhow::Error as AnyhowError;
use db::{
    DBService,
    models::{execution_process::ExecutionProcess, task::Task, task_attempt::TaskAttempt},
};
use serde::Serialize;
use serde_json::json;
use sqlx::{Error as SqlxError, sqlite::SqliteOperation};
use strum_macros::{Display, EnumString};
use thiserror::Error;
use tokio::{sync::RwLock, task::JoinHandle};
use ts_rs::TS;
use utils::msg_store::MsgStore;

#[derive(Debug, Error)]
pub enum EventError {
    #[error(transparent)]
    Sqlx(#[from] SqlxError),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
    #[error(transparent)]
    Other(#[from] AnyhowError), // Catches any unclassified errors
}

// Configuration constants for memory management
const MAX_ENTRY_COUNT: usize = 100_000;
const CLEANUP_BATCH_SIZE: usize = 10_000;
const MAX_ACTIVE_TASKS: usize = 1000;
const TASK_CLEANUP_INTERVAL_SECS: u64 = 300; // 5 minutes

#[derive(Clone)]
pub struct EventService {
    msg_store: Arc<MsgStore>,
    _db: DBService,
    entry_count: Arc<RwLock<usize>>,
    active_tasks: Arc<RwLock<HashMap<String, JoinHandle<()>>>>,
    last_cleanup: Arc<RwLock<Instant>>,
}

#[derive(EnumString, Display)]
enum HookTables {
    #[strum(to_string = "tasks")]
    Tasks,
    #[strum(to_string = "task_attempts")]
    TaskAttempts,
    #[strum(to_string = "execution_processes")]
    ExecutionProcesses,
}

#[derive(Serialize, TS)]
#[serde(tag = "type", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecordTypes {
    Task(Task),
    TaskAttempt(TaskAttempt),
    ExecutionProcess(ExecutionProcess),
    DeletedTask { rowid: i64 },
    DeletedTaskAttempt { rowid: i64 },
    DeletedExecutionProcess { rowid: i64 },
}

#[derive(Serialize, TS)]
pub struct EventPatchInner {
    db_op: String,
    record: RecordTypes,
}

#[derive(Serialize, TS)]
pub struct EventPatch {
    op: String,
    path: String,
    value: EventPatchInner,
}

impl EventService {
    /// Creates a new EventService that will work with a DBService configured with hooks
    pub fn new(db: DBService, msg_store: Arc<MsgStore>, entry_count: Arc<RwLock<usize>>) -> Self {
        Self {
            msg_store,
            _db: db,
            entry_count,
            active_tasks: Arc::new(RwLock::new(HashMap::new())),
            last_cleanup: Arc::new(RwLock::new(Instant::now())),
        }
    }

    /// Cleanup old tasks and reset entry count if needed
    async fn perform_cleanup(&self) -> Result<(), EventError> {
        let now = Instant::now();
        let mut last_cleanup = self.last_cleanup.write().await;

        // Only cleanup every TASK_CLEANUP_INTERVAL_SECS seconds
        if now.duration_since(*last_cleanup).as_secs() < TASK_CLEANUP_INTERVAL_SECS {
            return Ok(());
        }

        // Cleanup finished tasks
        let mut active_tasks = self.active_tasks.write().await;
        let mut completed_tasks = Vec::new();

        for (task_id, handle) in active_tasks.iter() {
            if handle.is_finished() {
                completed_tasks.push(task_id.clone());
            }
        }

        for task_id in completed_tasks {
            if let Some(handle) = active_tasks.remove(&task_id) {
                // Clean up the finished task
                let _ = handle.await;
                tracing::debug!("Cleaned up completed task: {}", task_id);
            }
        }

        // Reset entry count if it exceeds the limit
        let mut entry_count = self.entry_count.write().await;
        if *entry_count > MAX_ENTRY_COUNT {
            tracing::info!(
                "Resetting entry count from {} to {} to prevent memory leak",
                *entry_count,
                CLEANUP_BATCH_SIZE
            );
            *entry_count = CLEANUP_BATCH_SIZE;
        }

        *last_cleanup = now;

        tracing::debug!(
            "Cleanup completed. Active tasks: {}, Entry count: {}",
            active_tasks.len(),
            *entry_count
        );

        Ok(())
    }

    /// Check if we need to perform cleanup based on current state
    #[allow(dead_code)]
    async fn should_cleanup(&self) -> bool {
        let entry_count = *self.entry_count.read().await;
        let active_tasks_count = self.active_tasks.read().await.len();

        entry_count > MAX_ENTRY_COUNT || active_tasks_count > MAX_ACTIVE_TASKS
    }

    /// Static cleanup method for use in hooks
    async fn cleanup_if_needed(
        active_tasks: Arc<RwLock<HashMap<String, JoinHandle<()>>>>,
        entry_count: Arc<RwLock<usize>>,
        last_cleanup: Arc<RwLock<Instant>>,
    ) -> Result<(), EventError> {
        let now = Instant::now();
        let mut last_cleanup_guard = last_cleanup.write().await;

        // Only cleanup every TASK_CLEANUP_INTERVAL_SECS seconds
        if now.duration_since(*last_cleanup_guard).as_secs() < TASK_CLEANUP_INTERVAL_SECS {
            return Ok(());
        }

        // Cleanup finished tasks
        let mut active_tasks_guard = active_tasks.write().await;
        let mut completed_tasks = Vec::new();

        for (task_id, handle) in active_tasks_guard.iter() {
            if handle.is_finished() {
                completed_tasks.push(task_id.clone());
            }
        }

        for task_id in completed_tasks {
            active_tasks_guard.remove(&task_id);
        }

        // Reset entry count if it exceeds the limit
        let mut entry_count_guard = entry_count.write().await;
        if *entry_count_guard > MAX_ENTRY_COUNT {
            tracing::info!(
                "Resetting entry count from {} to {} to prevent memory leak",
                *entry_count_guard,
                CLEANUP_BATCH_SIZE
            );
            *entry_count_guard = CLEANUP_BATCH_SIZE;
        }

        *last_cleanup_guard = now;

        tracing::debug!(
            "Hook cleanup completed. Active tasks: {}, Entry count: {}",
            active_tasks_guard.len(),
            *entry_count_guard
        );

        Ok(())
    }

    /// Creates the hook function that should be used with DBService::new_with_after_connect
    pub fn create_hook(
        msg_store: Arc<MsgStore>,
        entry_count: Arc<RwLock<usize>>,
        db_service: DBService,
    ) -> impl for<'a> Fn(
        &'a mut sqlx::sqlite::SqliteConnection,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), sqlx::Error>> + Send + 'a>,
    > + Send
    + Sync
    + 'static {
        let active_tasks: Arc<RwLock<HashMap<String, JoinHandle<()>>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let last_cleanup = Arc::new(RwLock::new(Instant::now()));
        move |conn: &mut sqlx::sqlite::SqliteConnection| {
            let msg_store_for_hook = msg_store.clone();
            let entry_count_for_hook = entry_count.clone();
            let db_for_hook = db_service.clone();
            let active_tasks_for_hook = active_tasks.clone();
            let last_cleanup_for_hook = last_cleanup.clone();

            Box::pin(async move {
                let mut handle = conn.lock_handle().await?;
                let runtime_handle = tokio::runtime::Handle::current();
                handle.set_update_hook(move |hook: sqlx::sqlite::UpdateHookResult<'_>| {
                    let runtime_handle = runtime_handle.clone();
                    let entry_count_for_hook = entry_count_for_hook.clone();
                    let msg_store_for_hook = msg_store_for_hook.clone();
                    let db = db_for_hook.clone();
                    let active_tasks_for_hook = active_tasks_for_hook.clone();
                    let last_cleanup_for_hook = last_cleanup_for_hook.clone();

                    if let Ok(table) = HookTables::from_str(hook.table) {
                        let rowid = hook.rowid;

                        // Perform cleanup if needed (async spawn to avoid blocking)
                        let cleanup_tasks = active_tasks_for_hook.clone();
                        let cleanup_entry_count = entry_count_for_hook.clone();
                        let cleanup_last = last_cleanup_for_hook.clone();
                        runtime_handle.spawn(async move {
                            if let Err(e) = EventService::cleanup_if_needed(cleanup_tasks, cleanup_entry_count, cleanup_last).await {
                                tracing::error!("Hook cleanup failed: {:?}", e);
                            }
                        });

                        let task_id = format!("hook_{}_{}", hook.table, rowid);
                        let handle = runtime_handle.spawn(async move {
                            let record_type: RecordTypes = match (table, hook.operation.clone()) {
                                (HookTables::Tasks, SqliteOperation::Delete) => {
                                    RecordTypes::DeletedTask { rowid }
                                }
                                (HookTables::TaskAttempts, SqliteOperation::Delete) => {
                                    RecordTypes::DeletedTaskAttempt { rowid }
                                }
                                (HookTables::ExecutionProcesses, SqliteOperation::Delete) => {
                                    RecordTypes::DeletedExecutionProcess { rowid }
                                }
                                (HookTables::Tasks, _) => {
                                    match Task::find_by_rowid(&db.pool, rowid).await {
                                        Ok(Some(task)) => RecordTypes::Task(task),
                                        Ok(None) => RecordTypes::DeletedTask { rowid },
                                        Err(e) => {
                                            tracing::error!("Failed to fetch task: {:?}", e);
                                            return;
                                        }
                                    }
                                }
                                (HookTables::TaskAttempts, _) => {
                                    match TaskAttempt::find_by_rowid(&db.pool, rowid).await {
                                        Ok(Some(attempt)) => RecordTypes::TaskAttempt(attempt),
                                        Ok(None) => RecordTypes::DeletedTaskAttempt { rowid },
                                        Err(e) => {
                                            tracing::error!(
                                                "Failed to fetch task_attempt: {:?}",
                                                e
                                            );
                                            return;
                                        }
                                    }
                                }
                                (HookTables::ExecutionProcesses, _) => {
                                    match ExecutionProcess::find_by_rowid(&db.pool, rowid).await {
                                        Ok(Some(process)) => RecordTypes::ExecutionProcess(process),
                                        Ok(None) => RecordTypes::DeletedExecutionProcess { rowid },
                                        Err(e) => {
                                            tracing::error!(
                                                "Failed to fetch execution_process: {:?}",
                                                e
                                            );
                                            return;
                                        }
                                    }
                                }
                            };

                            let next_entry_count = {
                                let mut entry_count = entry_count_for_hook.write().await;
                                *entry_count += 1;

                                // Prevent unbounded growth - reset if too high
                                if *entry_count > MAX_ENTRY_COUNT {
                                    tracing::warn!("Entry count exceeded limit, resetting to prevent memory leak");
                                    *entry_count = CLEANUP_BATCH_SIZE;
                                }

                                *entry_count
                            };

                            let db_op: &str = match hook.operation {
                                SqliteOperation::Insert => "insert",
                                SqliteOperation::Delete => "delete",
                                SqliteOperation::Update => "update",
                                SqliteOperation::Unknown(_) => "unknown",
                            };

                            let event_patch: EventPatch = EventPatch {
                                op: "add".to_string(),
                                path: format!("/entries/{next_entry_count}"),
                                value: EventPatchInner {
                                    db_op: db_op.to_string(),
                                    record: record_type,
                                },
                            };

                            let patch =
                                serde_json::from_value(json!([
                                    serde_json::to_value(event_patch).unwrap()
                                ]))
                                .unwrap();

                            msg_store_for_hook.push_patch(patch);
                        });

                        // Track the spawned task for cleanup
                        let active_tasks_for_tracking = active_tasks_for_hook.clone();
                        let task_id_for_tracking = task_id.clone();
                        runtime_handle.spawn(async move {
                            let mut tasks = active_tasks_for_tracking.write().await;
                            tasks.insert(task_id_for_tracking, handle);

                            // Prevent unlimited task accumulation
                            if tasks.len() > MAX_ACTIVE_TASKS {
                                tracing::warn!("Active task limit exceeded: {}", tasks.len());
                            }
                        });
                    }
                });

                Ok(())
            })
        }
    }

    pub fn msg_store(&self) -> &Arc<MsgStore> {
        &self.msg_store
    }

    /// Get comprehensive memory usage statistics
    pub async fn get_memory_stats(&self) -> EventMemoryStats {
        let entry_count = *self.entry_count.read().await;
        let active_tasks_count = self.active_tasks.read().await.len();
        let msg_store_metrics = self.msg_store.get_memory_metrics();

        EventMemoryStats {
            entry_count,
            active_tasks_count,
            msg_store_metrics,
        }
    }

    /// Log comprehensive memory statistics
    pub async fn log_memory_stats(&self) {
        let stats = self.get_memory_stats().await;
        tracing::info!(
            "EventService memory stats - Entry count: {}, Active tasks: {}, MsgStore: {} messages/{} bytes",
            stats.entry_count,
            stats.active_tasks_count,
            stats.msg_store_metrics.total_messages,
            stats.msg_store_metrics.total_bytes
        );
    }

    /// Perform comprehensive cleanup including old messages
    pub async fn deep_cleanup(&self) -> Result<(), EventError> {
        // Perform standard cleanup first
        self.perform_cleanup().await?;

        // Clean up old messages (older than 1 hour)
        self.msg_store.cleanup_old_messages(3600);

        // Log statistics after cleanup
        self.log_memory_stats().await;

        Ok(())
    }
}

#[derive(Debug)]
pub struct EventMemoryStats {
    pub entry_count: usize,
    pub active_tasks_count: usize,
    pub msg_store_metrics: utils::msg_store::MemoryMetrics,
}
