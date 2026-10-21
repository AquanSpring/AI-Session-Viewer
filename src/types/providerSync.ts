export interface ProviderCount {
  provider: string;
  count: number;
}

export interface SqliteProviderCount {
  provider: string;
  archived: boolean;
  count: number;
}

export interface EncryptedWarning {
  provider: string;
  count: number;
}

export interface BackupSummary {
  name: string;
  path: string;
  createdAt: string;
  targetProvider: string;
  changedSessionCount: number;
  /** `sync` (rollout / SQLite rewrite or clone) or `repair` (config.toml only). */
  kind: string;
}

export interface ProviderSyncStatus {
  codexHome: string;
  currentProvider: string;
  currentProviderImplicit: boolean;
  configTomlPath: string;
  configTomlExists: boolean;
  configuredProviders: string[];
  rolloutStats: ProviderCount[];
  archivedStats: ProviderCount[];
  sqliteStats: SqliteProviderCount[];
  sqlitePath: string;
  sqliteExists: boolean;
  globalStatePath: string;
  globalStateExists: boolean;
  mismatchedRollouts: number;
  mismatchedArchived: number;
  mismatchedSqliteThreads: number;
  encryptedWarnings: EncryptedWarning[];
  backups: BackupSummary[];
  missingProviders: MissingProvider[];
  /** config.toml exists but is not valid TOML. */
  configError: string | null;
}

export interface SyncResult {
  backupDir: string;
  targetProvider: string;
  updatedRollouts: number;
  updatedSqliteRows: number;
  globalStateUpdated: boolean;
  configUpdated: boolean;
  skippedLocked: string[];
}

export interface CloneResult {
  backupDir: string;
  targetProvider: string;
  cloned: number;
  skipped: string[];
  newSessionIds: string[];
  encryptedSessionIds: string[];
}

export interface RestoreOptions {
  includeConfig: boolean;
  includeDb: boolean;
  includeSessions: boolean;
  includeGlobalState: boolean;
}

export interface RestoreResult {
  restoredFiles: number;
  restoredSessions: number;
}

/** Provider id referenced by sessions that Codex cannot resolve (neither
 *  built in nor defined in `[model_providers.*]`). */
export interface MissingProvider {
  provider: string;
  rollouts: number;
  archived: number;
  sqliteThreads: number;
  /** The root `model_provider` of config.toml points at it. */
  isCurrent: boolean;
}

export interface RepairResult {
  backupDir: string;
  configTomlPath: string;
  addedProviders: string[];
  /** Provider whose settings were copied; null = current OpenAI login. */
  template: string | null;
}
