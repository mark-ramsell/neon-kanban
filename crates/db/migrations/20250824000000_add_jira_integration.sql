PRAGMA foreign_keys = ON;

-- Jira integration configuration table
CREATE TABLE jira_configs (
    id               BLOB PRIMARY KEY,
    user_config_id   TEXT NOT NULL,
    -- CRITICAL: Store cloudid, not just site_url
    cloudid          TEXT NOT NULL,        -- e.g., "11223344-a1b2-3b33-c444-def123456789"
    site_name        TEXT NOT NULL,        -- Human-readable site name
    site_url         TEXT NOT NULL,        -- e.g., "mycompany.atlassian.net" (for display)
    -- OAuth app credentials (static per installation)
    client_id        TEXT NOT NULL,
    client_secret    TEXT NOT NULL,        -- Encrypted
    -- User-specific tokens (dynamic)
    access_token     TEXT,                 -- OAuth access token (encrypted)
    refresh_token    TEXT,                 -- OAuth refresh token (encrypted)
    token_expires_at TEXT,                 -- 1-hour from issuance (3600 seconds)
    granted_scopes   TEXT NOT NULL,        -- Actual granted scopes from token response
    is_active        BOOLEAN NOT NULL DEFAULT TRUE,
    created_at       TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at       TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    UNIQUE(user_config_id, cloudid)        -- One config per user per site
);

-- Cache Jira projects per cloudid
CREATE TABLE jira_projects (
    id               BLOB PRIMARY KEY,
    jira_config_id   BLOB NOT NULL,
    jira_project_id  TEXT NOT NULL,        -- Jira's internal project ID
    project_key      TEXT NOT NULL,        -- Project key like "PROJ"
    project_name     TEXT NOT NULL,
    project_type     TEXT,                 -- "software", "service_desk", etc.
    cached_at        TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    FOREIGN KEY (jira_config_id) REFERENCES jira_configs(id) ON DELETE CASCADE
);

-- Indexes for efficient lookups
CREATE INDEX idx_jira_configs_user ON jira_configs(user_config_id);
CREATE INDEX idx_jira_configs_cloudid ON jira_configs(cloudid);
CREATE INDEX idx_jira_projects_config ON jira_projects(jira_config_id);