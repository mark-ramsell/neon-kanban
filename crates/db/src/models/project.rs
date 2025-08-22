use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use thiserror::Error;
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("Project not found")]
    ProjectNotFound,
    #[error("Project with git repository path already exists")]
    GitRepoPathExists,
    #[error("Failed to check existing git repository path: {0}")]
    GitRepoCheckFailed(String),
    #[error("Failed to create project: {0}")]
    CreateFailed(String),
    #[error("Invalid branch prefix configuration: {0}")]
    InvalidBranchPrefixConfig(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct BranchPrefixConfig {
    pub feature: String,
    pub bugfix: String,
    pub hotfix: String,
    pub chore: String,
    pub default: String,
}

impl Default for BranchPrefixConfig {
    fn default() -> Self {
        Self {
            feature: "feature".to_string(),
            bugfix: "bugfix".to_string(),
            hotfix: "hotfix".to_string(),
            chore: "chore".to_string(),
            default: "vk".to_string(),
        }
    }
}

impl BranchPrefixConfig {
    pub fn get_prefix(&self, task_type: &str) -> &str {
        match task_type.to_lowercase().as_str() {
            "feature" => &self.feature,
            "bugfix" | "bug" => &self.bugfix,
            "hotfix" => &self.hotfix,
            "chore" => &self.chore,
            _ => &self.default,
        }
    }

    pub fn from_json(json_str: &str) -> Result<Self, ProjectError> {
        serde_json::from_str(json_str)
            .map_err(|e| ProjectError::InvalidBranchPrefixConfig(e.to_string()))
    }

    pub fn to_json(&self) -> Result<String, ProjectError> {
        serde_json::to_string(self)
            .map_err(|e| ProjectError::InvalidBranchPrefixConfig(e.to_string()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub git_repo_path: PathBuf,
    pub setup_script: Option<String>,
    pub dev_script: Option<String>,
    pub cleanup_script: Option<String>,
    pub copy_files: Option<String>,
    pub branch_prefix_config: BranchPrefixConfig,

    #[ts(type = "Date")]
    pub created_at: DateTime<Utc>,
    #[ts(type = "Date")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
struct ProjectRow {
    pub id: Uuid,
    pub name: String,
    pub git_repo_path: PathBuf,
    pub setup_script: Option<String>,
    pub dev_script: Option<String>,
    pub cleanup_script: Option<String>,
    pub copy_files: Option<String>,
    pub branch_prefix_config: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<ProjectRow> for Project {
    fn from(row: ProjectRow) -> Self {
        let branch_prefix_config = row
            .branch_prefix_config
            .as_deref()
            .and_then(|config| BranchPrefixConfig::from_json(config).ok())
            .unwrap_or_default();

        Self {
            id: row.id,
            name: row.name,
            git_repo_path: row.git_repo_path,
            setup_script: row.setup_script,
            dev_script: row.dev_script,
            cleanup_script: row.cleanup_script,
            copy_files: row.copy_files,
            branch_prefix_config,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Deserialize, TS)]
pub struct CreateProject {
    pub name: String,
    pub git_repo_path: String,
    pub use_existing_repo: bool,
    pub setup_script: Option<String>,
    pub dev_script: Option<String>,
    pub cleanup_script: Option<String>,
    pub copy_files: Option<String>,
    pub branch_prefix_config: Option<BranchPrefixConfig>,
}

#[derive(Debug, Deserialize, TS)]
pub struct UpdateProject {
    pub name: Option<String>,
    pub git_repo_path: Option<String>,
    pub setup_script: Option<String>,
    pub dev_script: Option<String>,
    pub cleanup_script: Option<String>,
    pub copy_files: Option<String>,
    pub branch_prefix_config: Option<BranchPrefixConfig>,
}

#[derive(Debug, Serialize, TS)]
pub struct ProjectWithBranch {
    pub id: Uuid,
    pub name: String,
    pub git_repo_path: PathBuf,
    pub setup_script: Option<String>,
    pub dev_script: Option<String>,
    pub cleanup_script: Option<String>,
    pub copy_files: Option<String>,
    pub branch_prefix_config: BranchPrefixConfig,
    pub current_branch: Option<String>,

    #[ts(type = "Date")]
    pub created_at: DateTime<Utc>,
    #[ts(type = "Date")]
    pub updated_at: DateTime<Utc>,
}

impl ProjectWithBranch {
    pub fn from_project(project: Project, current_branch: Option<String>) -> Self {
        Self {
            id: project.id,
            name: project.name,
            git_repo_path: project.git_repo_path,
            setup_script: project.setup_script,
            dev_script: project.dev_script,
            cleanup_script: project.cleanup_script,
            copy_files: project.copy_files,
            branch_prefix_config: project.branch_prefix_config,
            current_branch,
            created_at: project.created_at,
            updated_at: project.updated_at,
        }
    }
}

#[derive(Debug, Serialize, TS)]
pub struct SearchResult {
    pub path: String,
    pub is_file: bool,
    pub match_type: SearchMatchType,
}

#[derive(Debug, Serialize, TS)]
pub enum SearchMatchType {
    FileName,
    DirectoryName,
    FullPath,
}

impl Project {
    pub async fn find_all(pool: &SqlitePool) -> Result<Vec<Self>, sqlx::Error> {
        let rows = sqlx::query_as!(
            ProjectRow,
            r#"SELECT id as "id!: Uuid", name, git_repo_path, setup_script, dev_script, cleanup_script, copy_files, branch_prefix_config, created_at as "created_at!: DateTime<Utc>", updated_at as "updated_at!: DateTime<Utc>" FROM projects ORDER BY created_at DESC"#
        )
        .fetch_all(pool)
        .await?;
        
        Ok(rows.into_iter().map(Project::from).collect())
    }

    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        let row = sqlx::query_as!(
            ProjectRow,
            r#"SELECT id as "id!: Uuid", name, git_repo_path, setup_script, dev_script, cleanup_script, copy_files, branch_prefix_config, created_at as "created_at!: DateTime<Utc>", updated_at as "updated_at!: DateTime<Utc>" FROM projects WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await?;
        
        Ok(row.map(Project::from))
    }

    pub async fn find_by_git_repo_path(
        pool: &SqlitePool,
        git_repo_path: &str,
    ) -> Result<Option<Self>, sqlx::Error> {
        let row = sqlx::query_as!(
            ProjectRow,
            r#"SELECT id as "id!: Uuid", name, git_repo_path, setup_script, dev_script, cleanup_script, copy_files, branch_prefix_config, created_at as "created_at!: DateTime<Utc>", updated_at as "updated_at!: DateTime<Utc>" FROM projects WHERE git_repo_path = $1"#,
            git_repo_path
        )
        .fetch_optional(pool)
        .await?;
        
        Ok(row.map(Project::from))
    }

    pub async fn find_by_git_repo_path_excluding_id(
        pool: &SqlitePool,
        git_repo_path: &str,
        exclude_id: Uuid,
    ) -> Result<Option<Self>, sqlx::Error> {
        let row = sqlx::query_as!(
            ProjectRow,
            r#"SELECT id as "id!: Uuid", name, git_repo_path, setup_script, dev_script, cleanup_script, copy_files, branch_prefix_config, created_at as "created_at!: DateTime<Utc>", updated_at as "updated_at!: DateTime<Utc>" FROM projects WHERE git_repo_path = $1 AND id != $2"#,
            git_repo_path,
            exclude_id
        )
        .fetch_optional(pool)
        .await?;
        
        Ok(row.map(Project::from))
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateProject,
        project_id: Uuid,
    ) -> Result<Self, sqlx::Error> {
        let default_config = BranchPrefixConfig::default();
        let branch_config = data.branch_prefix_config.as_ref()
            .unwrap_or(&default_config);
        let branch_config_json = branch_config.to_json()
            .map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
            
        let row = sqlx::query_as!(
            ProjectRow,
            r#"INSERT INTO projects (id, name, git_repo_path, setup_script, dev_script, cleanup_script, copy_files, branch_prefix_config) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id as "id!: Uuid", name, git_repo_path, setup_script, dev_script, cleanup_script, copy_files, branch_prefix_config, created_at as "created_at!: DateTime<Utc>", updated_at as "updated_at!: DateTime<Utc>""#,
            project_id,
            data.name,
            data.git_repo_path,
            data.setup_script,
            data.dev_script,
            data.cleanup_script,
            data.copy_files,
            branch_config_json
        )
        .fetch_one(pool)
        .await?;
        
        Ok(Project::from(row))
    }

    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        name: String,
        git_repo_path: String,
        setup_script: Option<String>,
        dev_script: Option<String>,
        cleanup_script: Option<String>,
        copy_files: Option<String>,
    ) -> Result<Self, sqlx::Error> {
        let row = sqlx::query_as!(
            ProjectRow,
            r#"UPDATE projects SET name = $2, git_repo_path = $3, setup_script = $4, dev_script = $5, cleanup_script = $6, copy_files = $7 WHERE id = $1 RETURNING id as "id!: Uuid", name, git_repo_path, setup_script, dev_script, cleanup_script, copy_files, branch_prefix_config, created_at as "created_at!: DateTime<Utc>", updated_at as "updated_at!: DateTime<Utc>""#,
            id,
            name,
            git_repo_path,
            setup_script,
            dev_script,
            cleanup_script,
            copy_files
        )
        .fetch_one(pool)
        .await?;
        
        Ok(Project::from(row))
    }

    pub async fn update_with_branch_config(
        pool: &SqlitePool,
        id: Uuid,
        name: String,
        git_repo_path: String,
        setup_script: Option<String>,
        dev_script: Option<String>,
        cleanup_script: Option<String>,
        copy_files: Option<String>,
        branch_prefix_config: Option<BranchPrefixConfig>,
    ) -> Result<Self, sqlx::Error> {
        let branch_config_json = if let Some(config) = branch_prefix_config {
            Some(config.to_json().map_err(|e| sqlx::Error::Encode(Box::new(e)))?)
        } else {
            None
        };
            
        let row = sqlx::query_as!(
            ProjectRow,
            r#"UPDATE projects SET name = $2, git_repo_path = $3, setup_script = $4, dev_script = $5, cleanup_script = $6, copy_files = $7, branch_prefix_config = COALESCE($8, branch_prefix_config) WHERE id = $1 RETURNING id as "id!: Uuid", name, git_repo_path, setup_script, dev_script, cleanup_script, copy_files, branch_prefix_config, created_at as "created_at!: DateTime<Utc>", updated_at as "updated_at!: DateTime<Utc>""#,
            id,
            name,
            git_repo_path,
            setup_script,
            dev_script,
            cleanup_script,
            copy_files,
            branch_config_json
        )
        .fetch_one(pool)
        .await?;
        
        Ok(Project::from(row))
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM projects WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn exists(pool: &SqlitePool, id: Uuid) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!(
            r#"
                SELECT COUNT(*) as "count!: i64"
                FROM projects
                WHERE id = $1
            "#,
            id
        )
        .fetch_one(pool)
        .await?;

        Ok(result.count > 0)
    }
}
