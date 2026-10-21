use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCount {
    pub provider: String,
    pub count: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SqliteProviderCount {
    pub provider: String,
    pub archived: bool,
    pub count: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedWarning {
    pub provider: String,
    pub count: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BackupSummary {
    pub name: String,
    pub path: String,
    pub created_at: String,
    pub target_provider: String,
    pub changed_session_count: u32,
    /// `sync` (rollout/SQLite rewrite or clone) or `repair` (config.toml only).
    pub kind: String,
}

/// A provider id that sessions still reference but Codex cannot resolve:
/// neither built in nor defined under `[model_providers.*]`. Resuming such a
/// session fails with "Model provider `<id>` not found".
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MissingProvider {
    pub provider: String,
    pub rollouts: u32,
    pub archived: u32,
    pub sqlite_threads: u32,
    /// The root `model_provider` of config.toml itself points at it.
    pub is_current: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RepairResult {
    pub backup_dir: String,
    pub config_toml_path: String,
    pub added_providers: Vec<String>,
    /// Provider whose settings were copied; `None` = current OpenAI login.
    pub template: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSyncStatus {
    pub codex_home: String,
    pub current_provider: String,
    pub current_provider_implicit: bool,
    pub config_toml_path: String,
    pub config_toml_exists: bool,
    pub configured_providers: Vec<String>,
    pub rollout_stats: Vec<ProviderCount>,
    pub archived_stats: Vec<ProviderCount>,
    pub sqlite_stats: Vec<SqliteProviderCount>,
    pub sqlite_path: String,
    pub sqlite_exists: bool,
    pub global_state_path: String,
    pub global_state_exists: bool,
    pub mismatched_rollouts: u32,
    pub mismatched_archived: u32,
    pub mismatched_sqlite_threads: u32,
    pub encrypted_warnings: Vec<EncryptedWarning>,
    pub backups: Vec<BackupSummary>,
    pub missing_providers: Vec<MissingProvider>,
    /// config.toml exists but is not valid TOML (Codex cannot load it either).
    pub config_error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub backup_dir: String,
    pub target_provider: String,
    pub updated_rollouts: u32,
    pub updated_sqlite_rows: u32,
    pub global_state_updated: bool,
    pub config_updated: bool,
    pub skipped_locked: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CloneResult {
    pub backup_dir: String,
    pub target_provider: String,
    /// Number of sessions successfully cloned.
    pub cloned: u32,
    /// Source file paths that couldn't be cloned (missing/locked/no thread row).
    pub skipped: Vec<String>,
    /// New session ids created by the clone.
    pub new_session_ids: Vec<String>,
    /// Subset of clones whose source carried `encrypted_content` — these may
    /// display fine but can fail to resume/compact across providers/accounts.
    pub encrypted_session_ids: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RestoreOptions {
    #[serde(default = "default_true")]
    pub include_config: bool,
    #[serde(default = "default_true")]
    pub include_db: bool,
    #[serde(default = "default_true")]
    pub include_sessions: bool,
    #[serde(default = "default_true")]
    pub include_global_state: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub restored_files: u32,
    pub restored_sessions: u32,
}
