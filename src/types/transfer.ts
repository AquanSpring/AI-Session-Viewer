// Session archive export / import (mirrors session-core `transfer`).

export interface TransferSourceSelection {
  source: string;
  /** Omit to export every project of the source. */
  projectIds?: string[];
}

export interface TransferExportRequest {
  sources: TransferSourceSelection[];
}

export type TransferConflictPolicy = "skip" | "overwrite";

export interface TransferProjectRef {
  source: string;
  projectId: string;
}

export interface TransferPathMapping {
  source: string;
  projectId: string;
  targetPath: string;
}

export interface TransferImportOptions {
  conflict: TransferConflictPolicy;
  /** Omit to import every project. */
  projects?: TransferProjectRef[];
  pathMappings: TransferPathMapping[];
}

export interface TransferPreviewProject {
  id: string;
  name: string;
  path: string | null;
  pathExists: boolean;
  sessionCount: number;
  conflictCount: number;
  totalBytes: number;
}

export interface TransferPreviewSource {
  source: string;
  sessionCount: number;
  projects: TransferPreviewProject[];
}

export interface TransferPreview {
  appVersion: string;
  createdAt: string;
  hostOs: string;
  fileCount: number;
  totalBytes: number;
  sources: TransferPreviewSource[];
}

export type TransferJobState = "running" | "completed" | "failed" | "cancelled";

export interface TransferExportSummary {
  outputPath: string;
  fileName: string;
  projectCount: number;
  sessionCount: number;
  fileCount: number;
  totalBytes: number;
  archiveBytes: number;
}

export interface TransferSourceReport {
  source: string;
  imported: number;
  overwritten: number;
  skipped: number;
  failed: number;
}

export interface TransferImportReport {
  sources: TransferSourceReport[];
  bookmarksImported: number;
  codexThreadsImported: number;
  backupDir: string | null;
  errors: string[];
}

export interface TransferJob {
  id: string;
  kind: "export" | "import";
  state: TransferJobState;
  phase: "scanning" | "copying" | "merging" | "finishing" | "done";
  processedFiles: number;
  totalFiles: number;
  processedBytes: number;
  totalBytes: number;
  currentItem: string | null;
  error: string | null;
  exportSummary: TransferExportSummary | null;
  importReport: TransferImportReport | null;
  startedAt: string;
  finishedAt: string | null;
}
