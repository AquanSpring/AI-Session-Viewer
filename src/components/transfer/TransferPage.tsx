import { t } from "../../i18n/index.js";
import { useTranslation } from "react-i18next";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  AlertCircle,
  ArrowLeftRight,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Download,
  FolderOpen,
  Loader2,
  Upload,
  X,
} from "lucide-react";
import { api } from "../../services/api";
import { isRemoteNodeActive } from "../../services/nodeConfig";
import { useAppStore } from "../../stores/appStore";
import { SOURCE_OPTIONS } from "../layout/sourceOptions";
import type { ProjectEntry } from "../../types";
import type {
  TransferConflictPolicy,
  TransferJob,
  TransferPreview,
  TransferPreviewProject,
} from "../../types/transfer";
import { formatDateTime } from "../../utils/dateTime";

declare const __IS_TAURI__: boolean;

const POLL_MS = 500;

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 100 ? 0 : 1)} ${units[unit]}`;
}

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function sourceLabel(source: string): string {
  return SOURCE_OPTIONS.find((option) => option.id === source)?.label ?? source;
}

/** Desktop app on this machine: real file paths and native dialogs. */
function usesLocalFiles(): boolean {
  return __IS_TAURI__ && !isRemoteNodeActive();
}

function defaultArchiveName(): string {
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `ai-sessions-${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}.zip`;
}

const PHASE_LABELS: Record<string, string> = {
  get scanning() { return t("正在扫描会话..."); },
  get copying() { return t("正在复制文件..."); },
  get merging() { return t("正在合并索引与元数据..."); },
  get finishing() { return t("正在写入归档..."); },
  get done() { return t("已完成"); },
};

/** Poll a transfer job until it leaves the running state. */
function useTransferJob(onFinished: (job: TransferJob) => void) {
  const [jobId, setJobId] = useState<string | null>(null);
  const [job, setJob] = useState<TransferJob | null>(null);
  const finishedRef = useRef(onFinished);
  finishedRef.current = onFinished;

  useEffect(() => {
    if (!jobId) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      try {
        const next = await api.getTransferJob(jobId);
        if (stopped) return;
        setJob(next);
        if (next.state !== "running") {
          finishedRef.current(next);
          return;
        }
      } catch {
        // Transient polling errors: try again on the next tick.
      }
      if (!stopped) timer = setTimeout(tick, POLL_MS);
    };
    void tick();
    return () => {
      stopped = true;
      if (timer) clearTimeout(timer);
    };
  }, [jobId]);

  const start = (id: string) => {
    setJob(null);
    setJobId(id);
  };
  const reset = () => {
    setJobId(null);
    setJob(null);
  };
  return { jobId, job, start, reset, running: !!jobId && (!job || job.state === "running") };
}

function JobProgress({ job, onCancel }: { job: TransferJob | null; onCancel: () => void }) {
  const { t } = useTranslation();
  const percent = job && job.totalBytes > 0 ? Math.min(100, (job.processedBytes / job.totalBytes) * 100) : 0;
  return (
    <div className="rounded-md border border-border bg-muted/40 p-3 space-y-2">
      <div className="flex items-center gap-2 text-sm">
        <Loader2 className="w-4 h-4 animate-spin text-muted-foreground" />
        <span className="flex-1">{job ? PHASE_LABELS[job.phase] ?? job.phase : t("正在启动...")}</span>
        <button
          onClick={onCancel}
          className="px-2 py-1 text-xs rounded-md border border-border hover:bg-accent transition-colors"
        >
          {t("取消")}
        </button>
      </div>
      <div className="h-2 rounded-full bg-muted overflow-hidden">
        <div className="h-full bg-primary transition-all" style={{ width: `${percent}%` }} />
      </div>
      {job && job.totalFiles > 0 && (
        <div className="flex justify-between gap-2 text-xs text-muted-foreground">
          <span>
            {t("{{done}} / {{total}} 个文件", { done: job.processedFiles, total: job.totalFiles })}
            {" · "}
            {formatBytes(job.processedBytes)} / {formatBytes(job.totalBytes)}
          </span>
          <span>{percent.toFixed(0)}%</span>
        </div>
      )}
      {job?.currentItem && (
        <div className="truncate font-mono text-[11px] text-muted-foreground" title={job.currentItem}>
          {job.currentItem}
        </div>
      )}
    </div>
  );
}

function ErrorBanner({ message, onClose }: { message: string; onClose: () => void }) {
  return (
    <div className="rounded-md border border-destructive/40 bg-destructive/10 p-3 flex items-start gap-2">
      <AlertCircle className="w-4 h-4 text-destructive mt-0.5 shrink-0" />
      <div className="flex-1 text-sm text-destructive break-all">{message}</div>
      <button onClick={onClose} className="text-destructive shrink-0" aria-label={t("关闭")}>
        <X className="w-3.5 h-3.5" />
      </button>
    </div>
  );
}

// ── Export ──

type SourceChoice = { enabled: boolean; all: boolean; selected: Set<string> };

function ExportSection() {
  const { t } = useTranslation();
  const timeZone = useAppStore((state) => state.timeZone);
  const currentSource = useAppStore((state) => state.source);
  const [choices, setChoices] = useState<Record<string, SourceChoice>>(() =>
    Object.fromEntries(
      SOURCE_OPTIONS.map((option) => [
        option.id,
        { enabled: option.id === currentSource, all: true, selected: new Set<string>() },
      ]),
    ),
  );
  const [projects, setProjects] = useState<Record<string, ProjectEntry[] | "loading" | string>>({});
  const [expanded, setExpanded] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [savedPath, setSavedPath] = useState<string | null>(null);

  const { job, start, reset, running } = useTransferJob((finished) => {
    if (finished.state === "failed") setError(finished.error ?? t("导出失败"));
    if (finished.state === "completed" && finished.exportSummary && !usesLocalFiles()) {
      void api
        .downloadTransferExport(finished.id, finished.exportSummary.fileName)
        .catch((e) => setError(errorText(e)));
    }
  });

  const loadProjects = (source: string) => {
    if (projects[source] && typeof projects[source] !== "string") return;
    setProjects((prev) => ({ ...prev, [source]: "loading" }));
    api
      .getProjects(source)
      .then((list) => setProjects((prev) => ({ ...prev, [source]: list })))
      .catch((e) => setProjects((prev) => ({ ...prev, [source]: errorText(e) })));
  };

  const update = (source: string, patch: Partial<SourceChoice>) =>
    setChoices((prev) => ({ ...prev, [source]: { ...prev[source], ...patch } }));

  const toggleProject = (source: string, id: string) => {
    const selected = new Set(choices[source].selected);
    if (selected.has(id)) selected.delete(id);
    else selected.add(id);
    update(source, { selected });
  };

  const request = useMemo(
    () => ({
      sources: Object.entries(choices)
        .filter(([, choice]) => choice.enabled && (choice.all || choice.selected.size > 0))
        .map(([source, choice]) => ({
          source,
          projectIds: choice.all ? undefined : [...choice.selected],
        })),
    }),
    [choices],
  );

  const handleExport = async () => {
    setError(null);
    setSavedPath(null);
    reset();
    try {
      let outputPath: string | null = null;
      if (usesLocalFiles()) {
        const { save } = await import("@tauri-apps/plugin-dialog");
        const picked = await save({
          defaultPath: defaultArchiveName(),
          filters: [{ name: t("会话归档"), extensions: ["zip"] }],
        });
        if (!picked) return;
        outputPath = picked.toLowerCase().endsWith(".zip") ? picked : `${picked}.zip`;
        setSavedPath(outputPath);
      }
      start(await api.startTransferExport(request, outputPath));
    } catch (e) {
      setError(errorText(e));
    }
  };

  const summary = job?.state === "completed" ? job.exportSummary : null;

  return (
    <section className="rounded-lg border border-border bg-card p-4 space-y-3">
      <h2 className="text-sm font-semibold flex items-center gap-1.5">
        <Download className="w-4 h-4" />
        {t("导出会话")}
      </h2>
      <p className="text-xs text-muted-foreground">
        {t("把所选数据源的原始会话文件（含子代理、附件、别名、标签与收藏）打包成一个 .zip，在另一台电脑上导入即可继续查看和续聊。")}
      </p>

      <div className="space-y-1.5">
        {SOURCE_OPTIONS.map((option) => {
          const choice = choices[option.id];
          const list = projects[option.id];
          const isOpen = expanded === option.id;
          return (
            <div key={option.id} className="rounded-md border border-border">
              <div className="flex flex-wrap items-center gap-3 px-3 py-2">
                <label className="flex items-center gap-2 text-sm min-w-28">
                  <input
                    type="checkbox"
                    checked={choice.enabled}
                    disabled={running}
                    onChange={(e) => update(option.id, { enabled: e.target.checked })}
                  />
                  <option.icon className={`w-4 h-4 ${option.iconClass}`} />
                  {option.label}
                </label>
                {choice.enabled && (
                  <>
                    <label className="flex items-center gap-1.5 text-xs">
                      <input
                        type="radio"
                        checked={choice.all}
                        disabled={running}
                        onChange={() => update(option.id, { all: true })}
                      />
                      {t("全部项目")}
                    </label>
                    <label className="flex items-center gap-1.5 text-xs">
                      <input
                        type="radio"
                        checked={!choice.all}
                        disabled={running}
                        onChange={() => {
                          update(option.id, { all: false });
                          setExpanded(option.id);
                          loadProjects(option.id);
                        }}
                      />
                      {t("指定项目")}
                      {!choice.all && (
                        <span className="text-muted-foreground">
                          {t("（已选 {{count}} 个）", { count: choice.selected.size })}
                        </span>
                      )}
                    </label>
                    {!choice.all && (
                      <button
                        onClick={() => setExpanded(isOpen ? null : option.id)}
                        className="ml-auto text-xs text-muted-foreground hover:text-foreground flex items-center gap-0.5"
                      >
                        {isOpen ? <ChevronDown className="w-3.5 h-3.5" /> : <ChevronRight className="w-3.5 h-3.5" />}
                        {isOpen ? t("收起") : t("展开")}
                      </button>
                    )}
                  </>
                )}
              </div>
              {choice.enabled && !choice.all && isOpen && (
                <div className="border-t border-border max-h-64 overflow-y-auto px-3 py-2 space-y-1">
                  {list === "loading" || list === undefined ? (
                    <div className="flex items-center gap-2 text-xs text-muted-foreground">
                      <Loader2 className="w-3.5 h-3.5 animate-spin" />
                      {t("加载项目...")}
                    </div>
                  ) : typeof list === "string" ? (
                    <div className="text-xs text-destructive break-all">{list}</div>
                  ) : list.length === 0 ? (
                    <div className="text-xs text-muted-foreground">{t("没有项目")}</div>
                  ) : (
                    <>
                      <div className="flex gap-3 pb-1 text-xs">
                        <button
                          className="text-muted-foreground hover:text-foreground"
                          onClick={() => update(option.id, { selected: new Set(list.map((p) => p.id)) })}
                        >
                          {t("全选")}
                        </button>
                        <button
                          className="text-muted-foreground hover:text-foreground"
                          onClick={() => update(option.id, { selected: new Set() })}
                        >
                          {t("全不选")}
                        </button>
                      </div>
                      {list.map((project) => (
                        <label key={project.id} className="flex items-center gap-2 text-xs py-0.5">
                          <input
                            type="checkbox"
                            checked={choice.selected.has(project.id)}
                            disabled={running}
                            onChange={() => toggleProject(option.id, project.id)}
                          />
                          <span className="truncate">{project.alias || project.shortName}</span>
                          <span className="truncate font-mono text-muted-foreground" title={project.displayPath}>
                            {project.displayPath}
                          </span>
                          <span className="ml-auto shrink-0 text-muted-foreground">
                            {t("{{count}} 个会话", { count: project.sessionCount })}
                          </span>
                        </label>
                      ))}
                    </>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>

      <p className="text-xs text-warning">
        {t("会话中可能包含代码、命令输出、密钥或环境变量，请妥善保管导出的文件。")}
      </p>

      {error && <ErrorBanner message={error} onClose={() => setError(null)} />}

      {running ? (
        <JobProgress job={job} onCancel={() => job && void api.cancelTransferJob(job.id)} />
      ) : (
        <button
          onClick={handleExport}
          disabled={request.sources.length === 0}
          className="w-full px-3 py-2 text-sm rounded-md bg-primary text-primary-foreground hover:bg-primary/90 disabled:opacity-50 transition-colors flex items-center justify-center gap-1.5"
        >
          <Download className="w-4 h-4" />
          {usesLocalFiles() ? t("选择保存位置并导出") : t("导出并下载")}
        </button>
      )}

      {job?.state === "cancelled" && <p className="text-xs text-muted-foreground">{t("导出已取消")}</p>}

      {summary && (
        <div className="rounded-md border border-success/40 bg-success/10 p-3 space-y-1 text-sm">
          <div className="flex items-center gap-1.5 text-success">
            <CheckCircle2 className="w-4 h-4" />
            {t("导出完成：{{projects}} 个项目，{{sessions}} 个会话", {
              projects: summary.projectCount,
              sessions: summary.sessionCount,
            })}
          </div>
          <div className="text-xs text-muted-foreground">
            {t("原始大小 {{raw}}，压缩后 {{zip}}", {
              raw: formatBytes(summary.totalBytes),
              zip: formatBytes(summary.archiveBytes),
            })}
            {job?.finishedAt && ` · ${formatDateTime(job.finishedAt, timeZone)}`}
          </div>
          {usesLocalFiles() ? (
            <div className="text-xs font-mono break-all">{savedPath ?? summary.outputPath}</div>
          ) : (
            <button
              onClick={() =>
                job &&
                void api
                  .downloadTransferExport(job.id, summary.fileName)
                  .catch((e) => setError(errorText(e)))
              }
              className="text-xs underline hover:no-underline"
            >
              {t("没有开始下载？点此重新下载")}
            </button>
          )}
        </div>
      )}
    </section>
  );
}

// ── Import ──

const projectKey = (source: string, id: string) => `${source}\u0000${id}`;

function ImportSection() {
  const { t } = useTranslation();
  const timeZone = useAppStore((state) => state.timeZone);
  const loadProjects = useAppStore((state) => state.loadProjects);
  const [handle, setHandle] = useState<string | null>(null);
  const [fileLabel, setFileLabel] = useState<string>("");
  const [uploading, setUploading] = useState<{ loaded: number; total: number } | null>(null);
  const [preview, setPreview] = useState<TransferPreview | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [targets, setTargets] = useState<Record<string, string>>({});
  const [conflict, setConflict] = useState<TransferConflictPolicy>("skip");
  const [error, setError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const handleRef = useRef<string | null>(null);
  handleRef.current = handle;

  const { job, start, reset, running } = useTransferJob((finished) => {
    if (finished.state === "failed") setError(finished.error ?? t("导入失败"));
    // The server deletes the upload once the job ends.
    setHandle(null);
    void loadProjects();
  });

  // Drop an inspected-but-not-imported web upload when leaving the page.
  useEffect(
    () => () => {
      if (handleRef.current && !usesLocalFiles()) {
        void api.discardTransferArchive(handleRef.current).catch(() => {});
      }
    },
    [],
  );

  const clearArchive = () => {
    if (handle && !usesLocalFiles()) void api.discardTransferArchive(handle).catch(() => {});
    setHandle(null);
    setPreview(null);
    setFileLabel("");
    setSelected(new Set());
    setTargets({});
  };

  const openArchive = async (archive: File | string, label: string) => {
    clearArchive();
    reset();
    setError(null);
    setFileLabel(label);
    try {
      if (!usesLocalFiles()) setUploading({ loaded: 0, total: typeof archive === "string" ? 0 : archive.size });
      const nextHandle = await api.prepareTransferArchive(archive, (loaded, total) =>
        setUploading({ loaded, total }),
      );
      setUploading(null);
      setHandle(nextHandle);
      const nextPreview = await api.inspectTransferArchive(nextHandle);
      setPreview(nextPreview);
      setSelected(
        new Set(
          nextPreview.sources.flatMap((source) =>
            source.projects.map((project) => projectKey(source.source, project.id)),
          ),
        ),
      );
    } catch (e) {
      setUploading(null);
      setError(errorText(e));
    }
  };

  const pickArchive = async () => {
    if (!usesLocalFiles()) {
      fileInputRef.current?.click();
      return;
    }
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({
      multiple: false,
      directory: false,
      filters: [{ name: t("会话归档"), extensions: ["zip"] }],
    });
    if (typeof picked === "string") {
      await openArchive(picked, picked);
    }
  };

  const pickTargetDir = async (key: string) => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ directory: true, multiple: false, title: t("选择本机项目目录") });
    if (typeof picked === "string") setTargets((prev) => ({ ...prev, [key]: picked }));
  };

  const toggle = (key: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

  const allProjects = useMemo(
    () =>
      preview?.sources.flatMap((source) =>
        source.projects.map((project) => ({ source: source.source, project })),
      ) ?? [],
    [preview],
  );
  const chosen = allProjects.filter(({ source, project }) => selected.has(projectKey(source, project.id)));
  const chosenSessions = chosen.reduce((sum, { project }) => sum + project.sessionCount, 0);
  const chosenConflicts = chosen.reduce((sum, { project }) => sum + project.conflictCount, 0);

  const handleImport = async () => {
    if (!handle) return;
    setError(null);
    try {
      const pathMappings = chosen.flatMap(({ source, project }) => {
        const target = targets[projectKey(source, project.id)]?.trim();
        return target && target !== project.path ? [{ source, projectId: project.id, targetPath: target }] : [];
      });
      const everything = chosen.length === allProjects.length;
      start(
        await api.startTransferImport(handle, {
          conflict,
          projects: everything ? undefined : chosen.map(({ source, project }) => ({ source, projectId: project.id })),
          pathMappings,
        }),
      );
    } catch (e) {
      setError(errorText(e));
    }
  };

  const report = job?.importReport;

  const renderProject = (source: string, project: TransferPreviewProject) => {
    const key = projectKey(source, project.id);
    const target = targets[key] ?? "";
    const missing = !!project.path && !project.pathExists;
    return (
      <div key={key} className="px-3 py-2 space-y-1.5">
        <label className="flex items-center gap-2 text-xs">
          <input type="checkbox" checked={selected.has(key)} disabled={running} onChange={() => toggle(key)} />
          <span className="font-medium truncate">{project.name}</span>
          <span className="ml-auto shrink-0 text-muted-foreground">
            {t("{{count}} 个会话", { count: project.sessionCount })} · {formatBytes(project.totalBytes)}
          </span>
        </label>
        <div className="pl-5 space-y-1">
          {project.path ? (
            <div className="flex items-center gap-2 text-[11px]">
              <span className="truncate font-mono text-muted-foreground" title={project.path}>
                {project.path}
              </span>
              {project.pathExists ? (
                <span className="shrink-0 rounded px-1.5 py-0.5 bg-success/15 text-success">{t("本机存在")}</span>
              ) : (
                <span className="shrink-0 rounded px-1.5 py-0.5 bg-warning/15 text-warning">{t("本机不存在")}</span>
              )}
              {project.conflictCount > 0 && (
                <span className="shrink-0 rounded px-1.5 py-0.5 bg-muted text-muted-foreground">
                  {t("{{count}} 个已存在", { count: project.conflictCount })}
                </span>
              )}
            </div>
          ) : (
            <div className="text-[11px] text-muted-foreground">{t("虚拟分组，按原样导入")}</div>
          )}
          {project.path && selected.has(key) && (
            <div className="flex items-center gap-1.5">
              <input
                type="text"
                value={target}
                disabled={running}
                onChange={(e) => setTargets((prev) => ({ ...prev, [key]: e.target.value }))}
                placeholder={missing ? t("本机项目路径（留空则保持原路径）") : t("改为其他本机路径（可选）")}
                className={`flex-1 min-w-0 bg-muted border rounded px-2 py-1 text-[11px] font-mono ${
                  missing && !target ? "border-warning/50" : "border-border"
                }`}
              />
              {usesLocalFiles() && (
                <button
                  onClick={() => void pickTargetDir(key)}
                  disabled={running}
                  className="shrink-0 p-1 rounded border border-border hover:bg-accent/50"
                  title={t("选择目录")}
                >
                  <FolderOpen className="w-3.5 h-3.5" />
                </button>
              )}
            </div>
          )}
        </div>
      </div>
    );
  };

  return (
    <section className="rounded-lg border border-border bg-card p-4 space-y-3">
      <h2 className="text-sm font-semibold flex items-center gap-1.5">
        <Upload className="w-4 h-4" />
        {t("导入会话")}
      </h2>
      <p className="text-xs text-muted-foreground">
        {t("选择在其他电脑导出的 .zip。导入只会新增会话，不会删除本机已有内容；项目路径不同时可以改成本机目录，会话里记录的工作目录会一并改写，以便在本机续聊。")}
      </p>

      <input
        ref={fileInputRef}
        type="file"
        accept=".zip,application/zip"
        className="hidden"
        onChange={(e) => {
          const file = e.target.files?.[0];
          e.target.value = "";
          if (file) void openArchive(file, file.name);
        }}
      />
      <div className="flex items-center gap-2">
        <button
          onClick={() => void pickArchive()}
          disabled={running || !!uploading}
          className="shrink-0 flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-border bg-card hover:bg-accent/50 transition-colors disabled:opacity-50"
        >
          <FolderOpen className="w-3.5 h-3.5" />
          {t("选择归档文件")}
        </button>
        {fileLabel && <span className="truncate text-xs font-mono text-muted-foreground">{fileLabel}</span>}
      </div>

      {uploading && (
        <div className="space-y-1">
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
            {uploading.total > 0
              ? t("正在上传 {{loaded}} / {{total}}", {
                  loaded: formatBytes(uploading.loaded),
                  total: formatBytes(uploading.total),
                })
              : t("正在读取归档...")}
          </div>
          {uploading.total > 0 && (
            <div className="h-1.5 rounded-full bg-muted overflow-hidden">
              <div
                className="h-full bg-primary transition-all"
                style={{ width: `${(uploading.loaded / uploading.total) * 100}%` }}
              />
            </div>
          )}
        </div>
      )}

      {error && <ErrorBanner message={error} onClose={() => setError(null)} />}

      {preview && !report && (
        <>
          <div className="text-xs text-muted-foreground">
            {t("导出于 {{time}}（{{os}}，v{{version}}），共 {{files}} 个文件，{{size}}", {
              time: formatDateTime(preview.createdAt, timeZone),
              os: preview.hostOs,
              version: preview.appVersion,
              files: preview.fileCount,
              size: formatBytes(preview.totalBytes),
            })}
          </div>
          <div className="space-y-2">
            {preview.sources.map((source) => (
              <div key={source.source} className="rounded-md border border-border">
                <div className="flex items-center justify-between px-3 py-2 border-b border-border text-sm">
                  <span className="font-medium">{sourceLabel(source.source)}</span>
                  <span className="text-xs text-muted-foreground">
                    {t("{{projects}} 个项目，{{sessions}} 个会话", {
                      projects: source.projects.length,
                      sessions: source.sessionCount,
                    })}
                  </span>
                </div>
                <div className="max-h-80 overflow-y-auto divide-y divide-border">
                  {source.projects.map((project) => renderProject(source.source, project))}
                </div>
              </div>
            ))}
          </div>

          <div className="space-y-1">
            <div className="text-xs text-muted-foreground">{t("本机已存在同一会话时")}</div>
            <label className="flex items-center gap-2 text-xs">
              <input type="radio" checked={conflict === "skip"} disabled={running} onChange={() => setConflict("skip")} />
              {t("跳过，保留本机版本（推荐）")}
            </label>
            <label className="flex items-center gap-2 text-xs">
              <input
                type="radio"
                checked={conflict === "overwrite"}
                disabled={running}
                onChange={() => setConflict("overwrite")}
              />
              {t("用归档覆盖（本机版本先移入备份目录）")}
            </label>
          </div>

          {running ? (
            <JobProgress job={job} onCancel={() => job && void api.cancelTransferJob(job.id)} />
          ) : (
            <button
              onClick={handleImport}
              disabled={chosen.length === 0 || !handle}
              className="w-full px-3 py-2 text-sm rounded-md bg-primary text-primary-foreground hover:bg-primary/90 disabled:opacity-50 transition-colors flex items-center justify-center gap-1.5"
            >
              <Upload className="w-4 h-4" />
              {chosenConflicts > 0
                ? t("导入 {{count}} 个会话（{{conflicts}} 个已存在）", { count: chosenSessions, conflicts: chosenConflicts })
                : t("导入 {{count}} 个会话", { count: chosenSessions })}
            </button>
          )}
        </>
      )}

      {job?.state === "cancelled" && !report && <p className="text-xs text-muted-foreground">{t("导入已取消")}</p>}

      {report && (
        <div
          className={`rounded-md border p-3 space-y-2 text-sm ${
            job?.state === "completed" ? "border-success/40 bg-success/10" : "border-warning/40 bg-warning/10"
          }`}
        >
          <div className="flex items-center gap-1.5">
            <CheckCircle2 className="w-4 h-4" />
            {job?.state === "cancelled" ? t("导入已取消，以下为取消前的结果") : t("导入完成")}
          </div>
          <ul className="text-xs space-y-0.5">
            {report.sources.map((source) => (
              <li key={source.source}>
                {sourceLabel(source.source)}：
                {t("新增 {{imported}}，覆盖 {{overwritten}}，跳过 {{skipped}}，失败 {{failed}}", {
                  imported: source.imported,
                  overwritten: source.overwritten,
                  skipped: source.skipped,
                  failed: source.failed,
                })}
              </li>
            ))}
            {report.bookmarksImported > 0 && <li>{t("收藏 {{count}} 条", { count: report.bookmarksImported })}</li>}
            {report.codexThreadsImported > 0 && (
              <li>{t("Codex 线程记录 {{count}} 条", { count: report.codexThreadsImported })}</li>
            )}
          </ul>
          {report.backupDir && (
            <div className="text-xs text-muted-foreground">
              {t("被覆盖的本机文件已备份到：")}
              <span className="font-mono break-all">{report.backupDir}</span>
            </div>
          )}
          {report.errors.length > 0 && (
            <details className="text-xs">
              <summary className="cursor-pointer text-destructive">
                {t("{{count}} 条问题", { count: report.errors.length })}
              </summary>
              <ul className="mt-1 space-y-0.5 font-mono break-all text-muted-foreground">
                {report.errors.map((message, index) => (
                  <li key={index}>{message}</li>
                ))}
              </ul>
            </details>
          )}
          <p className="text-xs text-muted-foreground">
            {t("Codex 会话导入后可正常查看；含加密推理内容的会话在不同账号或 provider 下续聊可能失败。")}
          </p>
          <button
            onClick={() => {
              reset();
              clearArchive();
            }}
            className="text-xs underline hover:no-underline"
          >
            {t("导入另一个归档")}
          </button>
        </div>
      )}
    </section>
  );
}

export function TransferPage() {
  const { t } = useTranslation();
  return (
    <div className="workspace-page h-full overflow-y-auto">
      <div className="space-y-4">
        <header>
          <h1 className="text-xl font-semibold flex items-center gap-2">
            <ArrowLeftRight className="w-5 h-5 text-info" />
            {t("会话迁移")}
          </h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {t("在电脑之间搬运 AI 会话：在旧电脑导出为一个压缩包，在新电脑导入。")}
          </p>
        </header>
        <div className="grid grid-cols-1 xl:grid-cols-2 gap-4 items-start">
          <ExportSection />
          <ImportSection />
        </div>
      </div>
    </div>
  );
}

