use anyhow::Error;
use executors::profile::ProfileVariantLabel;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
pub use v4::{EditorConfig, EditorType, GitHubConfig, NotificationConfig, SoundFile, ThemeMode};

use crate::services::config::versions::v4;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Config {
    pub config_version: String,
    pub theme: ThemeMode,
    pub profile: ProfileVariantLabel,
    pub disclaimer_acknowledged: bool,
    pub onboarding_acknowledged: bool,
    pub github_login_acknowledged: bool,
    pub telemetry_acknowledged: bool,
    pub notifications: NotificationConfig,
    pub editor: EditorConfig,
    pub github: GitHubConfig,
    pub jira: JiraConfig,
    pub analytics_enabled: Option<bool>,
    pub workspace_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct JiraConfig {
    pub enabled: bool,
    pub default_client_id: Option<String>,
    pub default_client_secret: Option<String>,
    pub auto_sync_enabled: bool,
    pub sync_interval_minutes: u32, // Default: 15 minutes
}

impl Default for JiraConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_client_id: option_env!("JIRA_CLIENT_ID").map(|s| s.to_string()),
            default_client_secret: option_env!("JIRA_CLIENT_SECRET").map(|s| s.to_string()),
            auto_sync_enabled: false,
            sync_interval_minutes: 15,
        }
    }
}

impl Config {
    pub fn from_previous_version(raw_config: &str) -> Result<Self, Error> {
        let old_config = match serde_json::from_str::<v4::Config>(raw_config) {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::error!("❌ Failed to parse config: {}", e);
                tracing::error!("   at line {}, column {}", e.line(), e.column());
                return Err(e.into());
            }
        };

        Ok(Self {
            config_version: "v5".to_string(),
            theme: old_config.theme,
            profile: old_config.profile,
            disclaimer_acknowledged: old_config.disclaimer_acknowledged,
            onboarding_acknowledged: old_config.onboarding_acknowledged,
            github_login_acknowledged: old_config.github_login_acknowledged,
            telemetry_acknowledged: old_config.telemetry_acknowledged,
            notifications: old_config.notifications,
            editor: old_config.editor,
            github: old_config.github,
            jira: JiraConfig::default(),
            analytics_enabled: old_config.analytics_enabled,
            workspace_dir: old_config.workspace_dir,
        })
    }
}

impl From<String> for Config {
    fn from(raw_config: String) -> Self {
        if let Ok(config) = serde_json::from_str::<Config>(&raw_config)
            && config.config_version == "v5"
        {
            return config;
        }

        match Self::from_previous_version(&raw_config) {
            Ok(config) => {
                tracing::info!("Config upgraded to v5");
                config
            }
            Err(e) => {
                tracing::warn!("Config migration failed: {}, using default", e);
                Self::default()
            }
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_version: "v5".to_string(),
            theme: ThemeMode::System,
            profile: ProfileVariantLabel::default("claude-code".to_string()),
            disclaimer_acknowledged: false,
            onboarding_acknowledged: false,
            github_login_acknowledged: false,
            telemetry_acknowledged: false,
            notifications: NotificationConfig::default(),
            editor: EditorConfig::default(),
            github: GitHubConfig::default(),
            jira: JiraConfig::default(),
            analytics_enabled: None,
            workspace_dir: None,
        }
    }
}