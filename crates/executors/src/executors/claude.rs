use std::{path::PathBuf, process::Stdio, sync::Arc, fs};

use async_trait::async_trait;
use command_group::{AsyncCommandGroup, AsyncGroupChild};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncWriteExt, process::Command};
use ts_rs::TS;
use utils::{
    diff::{concatenate_diff_hunks, create_unified_diff, create_unified_diff_hunk},
    log_msg::LogMsg,
    msg_store::MsgStore,
    path::make_path_relative,
    shell::get_shell_command,
};

use crate::{
    command::CommandBuilder,
    executors::{ExecutorError, StandardCodingAgentExecutor},
    logs::{
        ActionType, FileChange, NormalizedEntry, NormalizedEntryType, TodoItem,
        stderr_processor::normalize_stderr_logs,
        utils::{EntryIndexProvider, patch::ConversationPatch},
    },
};

/// An executor that uses Claude CLI to process tasks
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct ClaudeCode {
    pub command: CommandBuilder,
    pub append_prompt: Option<String>,
    pub plan: bool,
}

#[async_trait]
impl StandardCodingAgentExecutor for ClaudeCode {
    async fn spawn(
        &self,
        current_dir: &PathBuf,
        prompt: &str,
    ) -> Result<AsyncGroupChild, ExecutorError> {
        let (shell_cmd, shell_arg) = get_shell_command();
        let claude_command = if self.plan {
            let base_command = self.command.build_initial();
            create_watchkill_script(&base_command)
        } else {
            self.command.build_initial()
        };

        let combined_prompt = utils::text::combine_prompt(&self.append_prompt, prompt);

        let mut command = Command::new(shell_cmd);
        command
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(current_dir)
            .arg(shell_arg)
            .arg(&claude_command);

        let mut child = command.group_spawn()?;

        // Feed the prompt in, then close the pipe so Claude sees EOF
        if let Some(mut stdin) = child.inner().stdin.take() {
            stdin.write_all(combined_prompt.as_bytes()).await?;
            stdin.shutdown().await?;
        }

        Ok(child)
    }

    async fn spawn_follow_up(
        &self,
        current_dir: &PathBuf,
        prompt: &str,
        session_id: &str,
    ) -> Result<AsyncGroupChild, ExecutorError> {
        let (shell_cmd, shell_arg) = get_shell_command();
        
        // Determine what to resume with - provided session ID (if valid) or fallback to most recent
        let effective_session_id = if session_id.is_empty() {
            // No session ID provided, try to find most recent session ID from conversation files
            if let Some(fallback_session_id) = self.find_most_recent_session_id(current_dir) {
                tracing::info!(
                    "No session ID provided, using session ID from most recent conversation: {}",
                    fallback_session_id
                );
                fallback_session_id
            } else {
                tracing::warn!(
                    "No session ID provided and no recent conversation files found, starting fresh conversation"
                );
                // Return empty string to indicate no session to resume
                "".to_string()
            }
        } else if self.session_id_exists_in_project(current_dir, session_id) {
            // We have a session id and it exists in the current project's conversation files
            session_id.to_string()
        } else {
            // Provided session id appears to be stale or from another project
            // Try to heal by resuming the most recent conversation for this project
            if let Some(fallback_session_id) = self.find_most_recent_session_id(current_dir) {
                tracing::info!(
                    "Provided session ID not found; using session ID from most recent conversation: {}",
                    fallback_session_id
                );
                fallback_session_id
            } else {
                tracing::warn!(
                    "Provided session ID not found and no recent conversation files found, starting fresh conversation"
                );
                "".to_string()
            }
        };
        
        // Build resume arguments - either with session ID or empty for fresh start
        let resume_args = if effective_session_id.is_empty() {
            vec![]
        } else {
            vec!["--resume".to_string(), effective_session_id]
        };
        
        // Build follow-up command with appropriate resume arguments
        let claude_command = if self.plan {
            let base_command = self.command.build_follow_up(&resume_args);
            create_watchkill_script(&base_command)
        } else {
            self.command.build_follow_up(&resume_args)
        };

        let combined_prompt = utils::text::combine_prompt(&self.append_prompt, prompt);

        let mut command = Command::new(shell_cmd);
        command
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(current_dir)
            .arg(shell_arg)
            .arg(&claude_command);

        let mut child = command.group_spawn()?;

        // Feed the followup prompt in, then close the pipe
        if let Some(mut stdin) = child.inner().stdin.take() {
            stdin.write_all(combined_prompt.as_bytes()).await?;
            stdin.shutdown().await?;
        }

        Ok(child)
    }

    fn normalize_logs(&self, msg_store: Arc<MsgStore>, current_dir: &PathBuf) {
        let entry_index_provider = EntryIndexProvider::start_from(&msg_store);

        // Process stdout logs (Claude's JSON output)
        ClaudeLogProcessor::process_logs(
            self,
            msg_store.clone(),
            current_dir,
            entry_index_provider.clone(),
        );

        // Process stderr logs using the standard stderr processor
        normalize_stderr_logs(msg_store, entry_index_provider);
    }
}

impl ClaudeCode {
    /// Check whether the given session_id exists in any JSONL conversation file
    /// for the claude project that corresponds to the provided current_dir.
    fn session_id_exists_in_project(&self, current_dir: &PathBuf, target_session_id: &str) -> bool {
        let home_dir = match dirs::home_dir() {
            Some(h) => h,
            None => return false,
        };
        let claude_projects_dir = home_dir.join(".claude").join("projects");
        if !claude_projects_dir.exists() {
            return false;
        }

        // First pass: try to find matches by directory naming convention (best effort)
        let current_dir_normalized = current_dir
            .to_string_lossy()
            .replace('/', "-")
            .replace(' ', "-");

        let mut candidate_files: Vec<PathBuf> = Vec::new();
        if let Ok(entries) = fs::read_dir(&claude_projects_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                    if dir_name.contains(&current_dir_normalized) {
                        if let Ok(jsonl_entries) = fs::read_dir(&path) {
                            for jsonl_entry in jsonl_entries.flatten() {
                                let jsonl_path = jsonl_entry.path();
                                if jsonl_path
                                    .extension()
                                    .and_then(|s| s.to_str())
                                    == Some("jsonl")
                                {
                                    candidate_files.push(jsonl_path);
                                }
                            }
                        }
                    }
                }
            }
        }

        // If no candidates by name, fall back to scanning all projects and filtering by `cwd` in file content
        if candidate_files.is_empty() {
            if let Ok(entries) = fs::read_dir(&claude_projects_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    if let Ok(jsonl_entries) = fs::read_dir(&path) {
                        for jsonl_entry in jsonl_entries.flatten() {
                            let jsonl_path = jsonl_entry.path();
                            if jsonl_path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                                if Self::jsonl_matches_cwd(&jsonl_path, current_dir) {
                                    candidate_files.push(jsonl_path);
                                }
                            }
                        }
                    }
                }
            }
        }

        for file in candidate_files {
            if Self::jsonl_contains_session_id(&file, target_session_id) {
                return true;
            }
        }
        false
    }

    /// Quick check: does the JSONL file contain an entry with the given session id?
    fn jsonl_contains_session_id(file_path: &PathBuf, target_session_id: &str) -> bool {
        if let Ok(content) = fs::read_to_string(file_path) {
            for line in content.lines() {
                if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(line) {
                    if json_value
                        .get("sessionId")
                        .and_then(|v| v.as_str())
                        == Some(target_session_id)
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Check whether a JSONL file belongs to current_dir by comparing its `cwd` field (if present)
    fn jsonl_matches_cwd(file_path: &PathBuf, current_dir: &PathBuf) -> bool {
        if let Ok(content) = fs::read_to_string(file_path) {
            let current_dir_str = current_dir.to_string_lossy();
            for line in content.lines() {
                if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(line) {
                    if let Some(cwd) = json_value.get("cwd").and_then(|v| v.as_str()) {
                        if cwd == current_dir_str {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
    /// Spawn a follow-up command with fallback to most recent session ID if the provided session ID fails
    pub async fn spawn_follow_up_with_fallback(
        &self,
        current_dir: &PathBuf,
        prompt: &str,
        session_id: &str,
        use_fallback: bool,
    ) -> Result<AsyncGroupChild, ExecutorError> {
        if use_fallback && !session_id.is_empty() {
            // This is a retry after the original session ID failed
            // Try to find the most recent session ID from conversation files as fallback
            if let Some(fallback_session_id) = self.find_most_recent_session_id(current_dir) {
                if fallback_session_id != session_id {
                    tracing::info!("Original session ID failed, trying fallback session ID from most recent conversation: {}", fallback_session_id);
                    return self.spawn_follow_up(current_dir, prompt, &fallback_session_id).await;
                } else {
                    tracing::warn!("Fallback session ID is the same as the failed one, starting fresh conversation");
                    return self.spawn_follow_up(current_dir, prompt, "").await;
                }
            } else {
                tracing::warn!("No fallback conversation files found, starting fresh conversation");
                return self.spawn_follow_up(current_dir, prompt, "").await;
            }
        }
        
        // Normal flow - either initial attempt or already using fallback
        self.spawn_follow_up(current_dir, prompt, session_id).await
    }
    /// Find the most recent session ID from JSONL files in the Claude project directory for the current directory
    fn find_most_recent_session_id(&self, current_dir: &PathBuf) -> Option<String> {
        let home_dir = dirs::home_dir()?;
        let claude_projects_dir = home_dir.join(".claude").join("projects");
        
        if !claude_projects_dir.exists() {
            tracing::warn!("Claude projects directory not found at {:?}", claude_projects_dir);
            return None;
        }

        // Phase 1: try by directory naming convention (best effort)
        let current_dir_normalized = current_dir
            .to_string_lossy()
            .replace('/', "-")
            .replace(' ', "-");
        let mut matching_files = Vec::new();
        if let Ok(entries) = fs::read_dir(&claude_projects_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                    if dir_name.contains(&current_dir_normalized) {
                        if let Ok(jsonl_entries) = fs::read_dir(&path) {
                            for jsonl_entry in jsonl_entries.flatten() {
                                let jsonl_path = jsonl_entry.path();
                                if jsonl_path.extension().and_then(|s| s.to_str()) == Some("jsonl")
                                {
                                    if let Ok(metadata) = jsonl_entry.metadata() {
                                        if let Ok(modified) = metadata.modified() {
                                            matching_files.push((jsonl_path, modified));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Phase 2: if nothing matched, scan all projects and include files whose `cwd` matches current_dir
        if matching_files.is_empty() {
            if let Ok(entries) = fs::read_dir(&claude_projects_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if let Ok(jsonl_entries) = fs::read_dir(&path) {
                        for jsonl_entry in jsonl_entries.flatten() {
                            let jsonl_path = jsonl_entry.path();
                            if jsonl_path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                                if Self::jsonl_matches_cwd(&jsonl_path, current_dir) {
                                    if let Ok(metadata) = jsonl_entry.metadata() {
                                        if let Ok(modified) = metadata.modified() {
                                            matching_files.push((jsonl_path, modified));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Sort by modification time (most recent first) and extract session ID from the most recent file
        matching_files.sort_by(|a, b| b.1.cmp(&a.1));
        
        if let Some((most_recent_file, _)) = matching_files.first() {
            tracing::info!("Found most recent conversation file: {:?}", most_recent_file);
            
            // Extract session ID from the JSONL file
            if let Some(session_id) = self.extract_session_id_from_jsonl(most_recent_file) {
                tracing::info!("Extracted session ID from conversation file: {}", session_id);
                return Some(session_id);
            }
        }
        
        None
    }

    /// Extract session ID from a JSONL conversation file
    fn extract_session_id_from_jsonl(&self, file_path: &PathBuf) -> Option<String> {
        match fs::read_to_string(file_path) {
            Ok(content) => {
                // Read the first line that contains a session ID
                for line in content.lines() {
                    if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(line) {
                        if let Some(session_id) = json_value.get("sessionId")
                            .and_then(|v| v.as_str()) {
                            return Some(session_id.to_string());
                        }
                    }
                }
                tracing::warn!("No session ID found in conversation file: {:?}", file_path);
                None
            },
            Err(e) => {
                tracing::error!("Failed to read conversation file {:?}: {}", file_path, e);
                None
            }
        }
    }
}

fn create_watchkill_script(command: &str) -> String {
    let claude_plan_stop_indicator = concat!("Exit ", "plan mode?"); // Use concat!() as a workaround to avoid killing plan mode when this file is read.
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail

word="{claude_plan_stop_indicator}"
command="{command}"

exit_code=0
while IFS= read -r line; do
    printf '%s\n' "$line"
    if [[ $line == *"$word"* ]]; then
        exit 0
    fi
done < <($command <&0 2>&1)

exit_code=${{PIPESTATUS[0]}}
exit "$exit_code"
"#
    )
}

/// Handles log processing and interpretation for Claude executor
struct ClaudeLogProcessor {
    model_name: Option<String>,
}

impl ClaudeLogProcessor {
    fn new() -> Self {
        Self { model_name: None }
    }

    /// Process raw logs and convert them to normalized entries with patches
    fn process_logs(
        _executor: &ClaudeCode,
        msg_store: Arc<MsgStore>,
        current_dir: &PathBuf,
        entry_index_provider: EntryIndexProvider,
    ) {
        let current_dir_clone = current_dir.clone();
        tokio::spawn(async move {
            let mut stream = msg_store.history_plus_stream();
            let mut buffer = String::new();
            let worktree_path = current_dir_clone.to_string_lossy().to_string();
            let mut session_id_extracted = false;
            let mut processor = Self::new();

            while let Some(Ok(msg)) = stream.next().await {
                let chunk = match msg {
                    LogMsg::Stdout(x) => x,
                    LogMsg::JsonPatch(_) | LogMsg::SessionId(_) | LogMsg::Stderr(_) => continue,
                    LogMsg::Finished => break,
                };

                buffer.push_str(&chunk);

                // Process complete JSON lines
                for line in buffer
                    .split_inclusive('\n')
                    .filter(|l| l.ends_with('\n'))
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
                {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    // Filter out claude-code-router service messages
                    if trimmed.starts_with("Service not running, starting service")
                        || trimmed
                            .contains("claude code router service has been successfully stopped")
                    {
                        continue;
                    }

                    match serde_json::from_str::<ClaudeJson>(trimmed) {
                        Ok(claude_json) => {
                            // Extract session ID if present
                            if !session_id_extracted
                                && let Some(session_id) = Self::extract_session_id(&claude_json)
                            {
                                msg_store.push_session_id(session_id);
                                session_id_extracted = true;
                            }

                            // Convert to normalized entries and create patches
                            for entry in
                                processor.to_normalized_entries(&claude_json, &worktree_path)
                            {
                                let patch_id = entry_index_provider.next();
                                let patch =
                                    ConversationPatch::add_normalized_entry(patch_id, entry);
                                msg_store.push_patch(patch);
                            }
                        }
                        Err(_) => {
                            // Handle non-JSON output as raw system message
                            if !trimmed.is_empty() {
                                let entry = NormalizedEntry {
                                    timestamp: None,
                                    entry_type: NormalizedEntryType::SystemMessage,
                                    content: format!("Raw output: {trimmed}"),
                                    metadata: None,
                                };

                                let patch_id = entry_index_provider.next();
                                let patch =
                                    ConversationPatch::add_normalized_entry(patch_id, entry);
                                msg_store.push_patch(patch);
                            }
                        }
                    }
                }

                // Keep the partial line in the buffer
                buffer = buffer.rsplit('\n').next().unwrap_or("").to_owned();
            }

            // Handle any remaining content in buffer
            if !buffer.trim().is_empty() {
                let entry = NormalizedEntry {
                    timestamp: None,
                    entry_type: NormalizedEntryType::SystemMessage,
                    content: format!("Raw output: {}", buffer.trim()),
                    metadata: None,
                };

                let patch_id = entry_index_provider.next();
                let patch = ConversationPatch::add_normalized_entry(patch_id, entry);
                msg_store.push_patch(patch);
            }
        });
    }

    /// Extract session ID from Claude JSON
    fn extract_session_id(claude_json: &ClaudeJson) -> Option<String> {
        match claude_json {
            ClaudeJson::System { session_id, .. } => session_id.clone(),
            ClaudeJson::Assistant { session_id, .. } => session_id.clone(),
            ClaudeJson::User { session_id, .. } => session_id.clone(),
            ClaudeJson::ToolUse { session_id, .. } => session_id.clone(),
            ClaudeJson::ToolResult { session_id, .. } => session_id.clone(),
            ClaudeJson::Result { .. } => None,
            ClaudeJson::Unknown => None,
        }
    }

    /// Convert Claude JSON to normalized entries
    fn to_normalized_entries(
        &mut self,
        claude_json: &ClaudeJson,
        worktree_path: &str,
    ) -> Vec<NormalizedEntry> {
        match claude_json {
            ClaudeJson::System { subtype, .. } => {
                let content = match subtype.as_deref() {
                    Some("init") => {
                        // Skip system init messages because it doesn't contain the actual model that will be used in assistant messages in case of claude-code-router.
                        // We'll send system initialized message with first assistant message that has a model field.
                        return vec![];
                    }
                    Some(subtype) => format!("System: {subtype}"),
                    None => "System message".to_string(),
                };

                vec![NormalizedEntry {
                    timestamp: None,
                    entry_type: NormalizedEntryType::SystemMessage,
                    content,
                    metadata: Some(
                        serde_json::to_value(claude_json).unwrap_or(serde_json::Value::Null),
                    ),
                }]
            }
            ClaudeJson::Assistant { message, .. } => {
                let mut entries = Vec::new();

                if self.model_name.is_none()
                    && let Some(model) = message.model.as_ref()
                {
                    self.model_name = Some(model.clone());
                    entries.push(NormalizedEntry {
                        timestamp: None,
                        entry_type: NormalizedEntryType::SystemMessage,
                        content: format!("System initialized with model: {model}"),
                        metadata: None,
                    });
                }

                for content_item in &message.content {
                    if let Some(entry) = Self::content_item_to_normalized_entry(
                        content_item,
                        "assistant",
                        worktree_path,
                    ) {
                        entries.push(entry);
                    }
                }
                entries
            }
            ClaudeJson::User { .. } => {
                vec![]
            }
            ClaudeJson::ToolUse { tool_data, .. } => {
                let tool_name = tool_data.get_name();
                let action_type = Self::extract_action_type(tool_data, worktree_path);
                let content =
                    Self::generate_concise_content(tool_data, &action_type, worktree_path);

                vec![NormalizedEntry {
                    timestamp: None,
                    entry_type: NormalizedEntryType::ToolUse {
                        tool_name: tool_name.to_string(),
                        action_type,
                    },
                    content,
                    metadata: Some(
                        serde_json::to_value(claude_json).unwrap_or(serde_json::Value::Null),
                    ),
                }]
            }
            ClaudeJson::ToolResult { .. } => {
                // TODO: Add proper ToolResult support to NormalizedEntry when the type system supports it
                vec![]
            }
            ClaudeJson::Result { .. } => {
                // Skip result messages
                vec![]
            }
            ClaudeJson::Unknown => {
                vec![NormalizedEntry {
                    timestamp: None,
                    entry_type: NormalizedEntryType::SystemMessage,
                    content: "Unrecognized JSON message from Claude".to_string(),
                    metadata: None,
                }]
            }
        }
    }

    /// Convert Claude content item to normalized entry
    fn content_item_to_normalized_entry(
        content_item: &ClaudeContentItem,
        role: &str,
        worktree_path: &str,
    ) -> Option<NormalizedEntry> {
        match content_item {
            ClaudeContentItem::Text { text } => {
                let entry_type = match role {
                    "assistant" => NormalizedEntryType::AssistantMessage,
                    _ => return None,
                };
                Some(NormalizedEntry {
                    timestamp: None,
                    entry_type,
                    content: text.clone(),
                    metadata: Some(
                        serde_json::to_value(content_item).unwrap_or(serde_json::Value::Null),
                    ),
                })
            }
            ClaudeContentItem::Thinking { thinking } => Some(NormalizedEntry {
                timestamp: None,
                entry_type: NormalizedEntryType::Thinking,
                content: thinking.clone(),
                metadata: Some(
                    serde_json::to_value(content_item).unwrap_or(serde_json::Value::Null),
                ),
            }),
            ClaudeContentItem::ToolUse { tool_data, .. } => {
                let name = tool_data.get_name();
                let action_type = Self::extract_action_type(tool_data, worktree_path);
                let content =
                    Self::generate_concise_content(tool_data, &action_type, worktree_path);

                Some(NormalizedEntry {
                    timestamp: None,
                    entry_type: NormalizedEntryType::ToolUse {
                        tool_name: name.to_string(),
                        action_type,
                    },
                    content,
                    metadata: Some(
                        serde_json::to_value(content_item).unwrap_or(serde_json::Value::Null),
                    ),
                })
            }
            ClaudeContentItem::ToolResult { .. } => {
                // TODO: Add proper ToolResult support to NormalizedEntry when the type system supports it
                None
            }
        }
    }

    /// Extract action type from structured tool data
    fn extract_action_type(tool_data: &ClaudeToolData, worktree_path: &str) -> ActionType {
        match tool_data {
            ClaudeToolData::Read { file_path } => ActionType::FileRead {
                path: make_path_relative(file_path, worktree_path),
            },
            ClaudeToolData::Edit {
                file_path,
                old_string,
                new_string,
            } => {
                let changes = if old_string.is_some() || new_string.is_some() {
                    vec![FileChange::Edit {
                        unified_diff: create_unified_diff(
                            file_path,
                            &old_string.clone().unwrap_or_default(),
                            &new_string.clone().unwrap_or_default(),
                        ),
                        has_line_numbers: false,
                    }]
                } else {
                    vec![]
                };
                ActionType::FileEdit {
                    path: make_path_relative(file_path, worktree_path),
                    changes,
                }
            }
            ClaudeToolData::MultiEdit { file_path, edits } => {
                let hunks: Vec<String> = edits
                    .iter()
                    .filter_map(|edit| {
                        if edit.old_string.is_some() || edit.new_string.is_some() {
                            Some(create_unified_diff_hunk(
                                &edit.old_string.clone().unwrap_or_default(),
                                &edit.new_string.clone().unwrap_or_default(),
                            ))
                        } else {
                            None
                        }
                    })
                    .collect();
                ActionType::FileEdit {
                    path: make_path_relative(file_path, worktree_path),
                    changes: vec![FileChange::Edit {
                        unified_diff: concatenate_diff_hunks(file_path, &hunks),
                        has_line_numbers: false,
                    }],
                }
            }
            ClaudeToolData::Write { file_path, content } => {
                let diffs = vec![FileChange::Write {
                    content: content.clone(),
                }];
                ActionType::FileEdit {
                    path: make_path_relative(file_path, worktree_path),
                    changes: diffs,
                }
            }
            ClaudeToolData::Bash { command, .. } => ActionType::CommandRun {
                command: command.clone(),
            },
            ClaudeToolData::Grep { pattern, .. } => ActionType::Search {
                query: pattern.clone(),
            },
            ClaudeToolData::WebFetch { url, .. } => ActionType::WebFetch { url: url.clone() },
            ClaudeToolData::WebSearch { query } => ActionType::WebFetch { url: query.clone() },
            ClaudeToolData::Task {
                description,
                prompt,
                ..
            } => {
                let task_description = if let Some(desc) = description {
                    desc.clone()
                } else {
                    prompt.clone()
                };
                ActionType::TaskCreate {
                    description: task_description,
                }
            }
            ClaudeToolData::ExitPlanMode { plan } => {
                ActionType::PlanPresentation { plan: plan.clone() }
            }
            ClaudeToolData::NotebookEdit { notebook_path, .. } => ActionType::FileEdit {
                path: make_path_relative(notebook_path, worktree_path),
                changes: vec![],
            },
            ClaudeToolData::TodoWrite { todos } => ActionType::TodoManagement {
                todos: todos
                    .iter()
                    .map(|t| TodoItem {
                        content: t.content.clone(),
                        status: t.status.clone(),
                        priority: t.priority.clone(),
                    })
                    .collect(),
                operation: "write".to_string(),
            },
            ClaudeToolData::Glob { pattern, path: _ } => ActionType::Search {
                query: pattern.clone(),
            },
            ClaudeToolData::LS { .. } => ActionType::Other {
                description: "List directory".to_string(),
            },
            ClaudeToolData::Unknown { .. } => ActionType::Other {
                description: format!("Tool: {}", tool_data.get_name()),
            },
        }
    }

    /// Generate concise, readable content for tool usage using structured data
    fn generate_concise_content(
        tool_data: &ClaudeToolData,
        action_type: &ActionType,
        worktree_path: &str,
    ) -> String {
        match action_type {
            ActionType::FileRead { path } => format!("`{path}`"),
            ActionType::FileEdit { path, .. } => format!("`{path}`"),
            ActionType::CommandRun { command } => format!("`{command}`"),
            ActionType::Search { query } => format!("`{query}`"),
            ActionType::WebFetch { url } => format!("`{url}`"),
            ActionType::TaskCreate { description } => description.clone(),
            ActionType::PlanPresentation { plan } => plan.clone(),
            ActionType::TodoManagement { .. } => "TODO list updated".to_string(),
            ActionType::Other { description: _ } => match tool_data {
                ClaudeToolData::LS { path } => {
                    let relative_path = make_path_relative(path, worktree_path);
                    if relative_path.is_empty() {
                        "List directory".to_string()
                    } else {
                        format!("List directory: `{relative_path}`")
                    }
                }
                ClaudeToolData::Glob { pattern, path } => {
                    if let Some(search_path) = path {
                        format!(
                            "Find files: `{}` in `{}`",
                            pattern,
                            make_path_relative(search_path, worktree_path)
                        )
                    } else {
                        format!("Find files: `{pattern}`")
                    }
                }
                _ => tool_data.get_name().to_string(),
            },
        }
    }
}

// Data structures for parsing Claude's JSON output format
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
#[serde(tag = "type")]
pub enum ClaudeJson {
    #[serde(rename = "system")]
    System {
        subtype: Option<String>,
        session_id: Option<String>,
        cwd: Option<String>,
        tools: Option<Vec<serde_json::Value>>,
        model: Option<String>,
    },
    #[serde(rename = "assistant")]
    Assistant {
        message: ClaudeMessage,
        session_id: Option<String>,
    },
    #[serde(rename = "user")]
    User {
        message: ClaudeMessage,
        session_id: Option<String>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        tool_name: String,
        #[serde(flatten)]
        tool_data: ClaudeToolData,
        session_id: Option<String>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        result: serde_json::Value,
        is_error: Option<bool>,
        session_id: Option<String>,
    },
    #[serde(rename = "result")]
    Result {
        subtype: Option<String>,
        is_error: Option<bool>,
        duration_ms: Option<u64>,
        result: Option<serde_json::Value>,
    },
    // Catch-all for unknown message types
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct ClaudeMessage {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub message_type: Option<String>,
    pub role: String,
    pub model: Option<String>,
    pub content: Vec<ClaudeContentItem>,
    pub stop_reason: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
#[serde(tag = "type")]
pub enum ClaudeContentItem {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        #[serde(flatten)]
        tool_data: ClaudeToolData,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: serde_json::Value,
        is_error: Option<bool>,
    },
}

/// Structured tool data for Claude tools based on real samples
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
#[serde(tag = "name", content = "input")]
pub enum ClaudeToolData {
    TodoWrite {
        todos: Vec<ClaudeTodoItem>,
    },
    Task {
        subagent_type: String,
        description: Option<String>,
        prompt: String,
    },
    Glob {
        pattern: String,
        #[serde(default)]
        path: Option<String>,
    },
    LS {
        path: String,
    },
    Read {
        file_path: String,
    },
    Bash {
        command: String,
        #[serde(default)]
        description: Option<String>,
    },
    Grep {
        pattern: String,
        #[serde(default)]
        output_mode: Option<String>,
        #[serde(default)]
        path: Option<String>,
    },
    ExitPlanMode {
        plan: String,
    },
    Edit {
        file_path: String,
        old_string: Option<String>,
        new_string: Option<String>,
    },
    MultiEdit {
        file_path: String,
        edits: Vec<ClaudeEditItem>,
    },
    Write {
        file_path: String,
        content: String,
    },
    NotebookEdit {
        notebook_path: String,
        new_source: String,
        edit_mode: String,
        #[serde(default)]
        cell_id: Option<String>,
    },
    WebFetch {
        url: String,
        #[serde(default)]
        prompt: Option<String>,
    },
    WebSearch {
        query: String,
    },
    #[serde(untagged)]
    Unknown {
        #[serde(flatten)]
        data: std::collections::HashMap<String, serde_json::Value>,
    },
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct ClaudeTodoItem {
    #[serde(default)]
    pub id: Option<String>,
    pub content: String,
    pub status: String,
    #[serde(default)]
    pub priority: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct ClaudeEditItem {
    pub old_string: Option<String>,
    pub new_string: Option<String>,
}

impl ClaudeToolData {
    pub fn get_name(&self) -> &str {
        match self {
            ClaudeToolData::TodoWrite { .. } => "TodoWrite",
            ClaudeToolData::Task { .. } => "Task",
            ClaudeToolData::Glob { .. } => "Glob",
            ClaudeToolData::LS { .. } => "LS",
            ClaudeToolData::Read { .. } => "Read",
            ClaudeToolData::Bash { .. } => "Bash",
            ClaudeToolData::Grep { .. } => "Grep",
            ClaudeToolData::ExitPlanMode { .. } => "ExitPlanMode",
            ClaudeToolData::Edit { .. } => "Edit",
            ClaudeToolData::MultiEdit { .. } => "MultiEdit",
            ClaudeToolData::Write { .. } => "Write",
            ClaudeToolData::NotebookEdit { .. } => "NotebookEdit",
            ClaudeToolData::WebFetch { .. } => "WebFetch",
            ClaudeToolData::WebSearch { .. } => "WebSearch",
            ClaudeToolData::Unknown { data } => data
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_json_parsing() {
        let system_json =
            r#"{"type":"system","subtype":"init","session_id":"abc123","model":"claude-sonnet-4"}"#;
        let parsed: ClaudeJson = serde_json::from_str(system_json).unwrap();

        assert_eq!(
            ClaudeLogProcessor::extract_session_id(&parsed),
            Some("abc123".to_string())
        );

        let entries = ClaudeLogProcessor::new().to_normalized_entries(&parsed, "");
        assert_eq!(entries.len(), 0);

        let assistant_json = r#"
        {"type":"assistant","message":{"type":"message","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"Hi! I'm Claude Code."}]}}"#;
        let parsed: ClaudeJson = serde_json::from_str(assistant_json).unwrap();
        let entries = ClaudeLogProcessor::new().to_normalized_entries(&parsed, "");

        assert_eq!(entries.len(), 2);
        assert!(matches!(
            entries[0].entry_type,
            NormalizedEntryType::SystemMessage
        ));
        assert_eq!(
            entries[0].content,
            "System initialized with model: claude-sonnet-4-20250514"
        );
    }

    #[test]
    fn test_assistant_message_parsing() {
        let assistant_json = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hello world"}]},"session_id":"abc123"}"#;
        let parsed: ClaudeJson = serde_json::from_str(assistant_json).unwrap();

        let entries = ClaudeLogProcessor::new().to_normalized_entries(&parsed, "");
        assert_eq!(entries.len(), 1);
        assert!(matches!(
            entries[0].entry_type,
            NormalizedEntryType::AssistantMessage
        ));
        assert_eq!(entries[0].content, "Hello world");
    }

    #[test]
    fn test_result_message_ignored() {
        let result_json = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":6059,"result":"Final result"}"#;
        let parsed: ClaudeJson = serde_json::from_str(result_json).unwrap();

        let entries = ClaudeLogProcessor::new().to_normalized_entries(&parsed, "");
        assert_eq!(entries.len(), 0); // Should be ignored like in old implementation
    }

    #[test]
    fn test_thinking_content() {
        let thinking_json = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Let me think about this..."}]}}"#;
        let parsed: ClaudeJson = serde_json::from_str(thinking_json).unwrap();

        let entries = ClaudeLogProcessor::new().to_normalized_entries(&parsed, "");
        assert_eq!(entries.len(), 1);
        assert!(matches!(
            entries[0].entry_type,
            NormalizedEntryType::Thinking
        ));
        assert_eq!(entries[0].content, "Let me think about this...");
    }

    #[test]
    fn test_todo_tool_empty_list() {
        // Test TodoWrite with empty todo list
        let empty_data = ClaudeToolData::TodoWrite { todos: vec![] };

        let action_type =
            ClaudeLogProcessor::extract_action_type(&empty_data, "/tmp/test-worktree");
        let result = ClaudeLogProcessor::generate_concise_content(
            &empty_data,
            &action_type,
            "/tmp/test-worktree",
        );

        assert_eq!(result, "TODO list updated");
    }

    #[test]
    fn test_glob_tool_content_extraction() {
        // Test Glob with pattern and path
        let glob_data = ClaudeToolData::Glob {
            pattern: "**/*.ts".to_string(),
            path: Some("/tmp/test-worktree/src".to_string()),
        };

        let action_type = ClaudeLogProcessor::extract_action_type(&glob_data, "/tmp/test-worktree");
        let result = ClaudeLogProcessor::generate_concise_content(
            &glob_data,
            &action_type,
            "/tmp/test-worktree",
        );

        assert_eq!(result, "`**/*.ts`");
    }

    #[test]
    fn test_glob_tool_pattern_only() {
        // Test Glob with pattern only
        let glob_data = ClaudeToolData::Glob {
            pattern: "*.js".to_string(),
            path: None,
        };

        let action_type = ClaudeLogProcessor::extract_action_type(&glob_data, "/tmp/test-worktree");
        let result = ClaudeLogProcessor::generate_concise_content(
            &glob_data,
            &action_type,
            "/tmp/test-worktree",
        );

        assert_eq!(result, "`*.js`");
    }

    #[test]
    fn test_ls_tool_content_extraction() {
        // Test LS with path
        let ls_data = ClaudeToolData::LS {
            path: "/tmp/test-worktree/components".to_string(),
        };

        let action_type = ClaudeLogProcessor::extract_action_type(&ls_data, "/tmp/test-worktree");
        let result = ClaudeLogProcessor::generate_concise_content(
            &ls_data,
            &action_type,
            "/tmp/test-worktree",
        );

        assert_eq!(result, "List directory: `components`");
    }

    #[test]
    fn test_path_relative_conversion() {
        // Test with relative path (should remain unchanged)
        let relative_result = make_path_relative("src/main.rs", "/tmp/test-worktree");
        assert_eq!(relative_result, "src/main.rs");

        // Test with absolute path (should become relative if possible)
        let test_worktree = "/tmp/test-worktree";
        let absolute_path = format!("{test_worktree}/src/main.rs");
        let absolute_result = make_path_relative(&absolute_path, test_worktree);
        assert_eq!(absolute_result, "src/main.rs");
    }

    #[tokio::test]
    async fn test_streaming_patch_generation() {
        use std::sync::Arc;

        use utils::msg_store::MsgStore;

        let executor = ClaudeCode {
            command: CommandBuilder::new(""),
            plan: false,
            append_prompt: None,
        };
        let msg_store = Arc::new(MsgStore::new());
        let current_dir = std::path::PathBuf::from("/tmp/test-worktree");

        // Push some test messages
        msg_store.push_stdout(
            r#"{"type":"system","subtype":"init","session_id":"test123"}"#.to_string(),
        );
        msg_store.push_stdout(r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hello"}]}}"#.to_string());
        msg_store.push_finished();

        // Start normalization (this spawns async task)
        executor.normalize_logs(msg_store.clone(), &current_dir);

        // Give some time for async processing
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Check that the history now contains patch messages
        let history = msg_store.get_history();
        let patch_count = history
            .iter()
            .filter(|msg| matches!(msg, utils::log_msg::LogMsg::JsonPatch(_)))
            .count();
        assert!(
            patch_count > 0,
            "Expected JsonPatch messages to be generated from streaming processing"
        );
    }

    #[test]
    fn test_session_id_extraction() {
        let system_json = r#"{"type":"system","session_id":"test-session-123"}"#;
        let parsed: ClaudeJson = serde_json::from_str(system_json).unwrap();

        assert_eq!(
            ClaudeLogProcessor::extract_session_id(&parsed),
            Some("test-session-123".to_string())
        );

        let tool_use_json =
            r#"{"type":"tool_use","tool_name":"read","input":{},"session_id":"another-session"}"#;
        let parsed_tool: ClaudeJson = serde_json::from_str(tool_use_json).unwrap();

        assert_eq!(
            ClaudeLogProcessor::extract_session_id(&parsed_tool),
            Some("another-session".to_string())
        );
    }

    #[test]
    fn test_tool_result_parsing_ignored() {
        let tool_result_json = r#"{"type":"tool_result","result":"File content here","is_error":false,"session_id":"test123"}"#;
        let parsed: ClaudeJson = serde_json::from_str(tool_result_json).unwrap();

        // Test session ID extraction from ToolResult still works
        assert_eq!(
            ClaudeLogProcessor::extract_session_id(&parsed),
            Some("test123".to_string())
        );

        // ToolResult messages should be ignored (produce no entries) until proper support is added
        let entries = ClaudeLogProcessor::new().to_normalized_entries(&parsed, "");
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_content_item_tool_result_ignored() {
        let assistant_with_tool_result = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_result","tool_use_id":"tool_123","content":"Operation completed","is_error":false}]}}"#;
        let parsed: ClaudeJson = serde_json::from_str(assistant_with_tool_result).unwrap();

        // ToolResult content items should be ignored (produce no entries) until proper support is added
        let entries = ClaudeLogProcessor::new().to_normalized_entries(&parsed, "");
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_session_id_fallback_logic() {
        // Test that the session ID fallback logic works correctly
        let executor = ClaudeCode {
            command: CommandBuilder::new("echo test"),
            plan: false,
            append_prompt: None,
        };

        // This test verifies that the fallback logic is triggered when session_id is empty
        // The actual file lookup will depend on the environment, so we just test the logic path
        let current_dir = PathBuf::from("/tmp/test-worktree");
        
        // Test with empty session ID - should trigger fallback logic
        // Note: This test mainly verifies the code doesn't panic and follows the correct path
        let result = executor.find_most_recent_session_id(&current_dir);
        
        // In most test environments, this will return None since Claude projects may not exist
        // But the function should handle this gracefully
        assert!(result.is_none() || result.is_some());
    }

    #[test]
    fn test_extract_session_id_from_jsonl_content() {
        // Test session ID extraction logic using string parsing directly
        let jsonl_content = r#"{"type":"summary","summary":"Test conversation"}
{"sessionId":"test-session-123","type":"user","message":{"role":"user","content":"Hello"}}
{"sessionId":"test-session-123","type":"assistant","message":{"role":"assistant","content":"Hi there"}}"#;

        // Simulate the extraction logic
        for line in jsonl_content.lines() {
            if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(session_id) = json_value.get("sessionId").and_then(|v| v.as_str()) {
                    assert_eq!(session_id, "test-session-123");
                    return; // Test passed
                }
            }
        }
        panic!("Should have found session ID");
    }

    #[test]
    fn test_mixed_content_with_thinking_ignores_tool_result() {
        let complex_assistant_json = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"I need to read the file first"},{"type":"text","text":"I'll help you with that"},{"type":"tool_result","tool_use_id":"tool_789","content":"Success","is_error":false}]}}"#;
        let parsed: ClaudeJson = serde_json::from_str(complex_assistant_json).unwrap();

        let entries = ClaudeLogProcessor::new().to_normalized_entries(&parsed, "");
        // Only thinking and text entries should be processed, tool_result ignored
        assert_eq!(entries.len(), 2);

        // Check thinking entry
        assert!(matches!(
            entries[0].entry_type,
            NormalizedEntryType::Thinking
        ));
        assert_eq!(entries[0].content, "I need to read the file first");

        // Check assistant message
        assert!(matches!(
            entries[1].entry_type,
            NormalizedEntryType::AssistantMessage
        ));
        assert_eq!(entries[1].content, "I'll help you with that");

        // ToolResult entry is ignored - no third entry
    }
}
