//! Move sessions between machines: export selected sources/projects into one
//! zip archive, preview it on the target machine, then import with optional
//! project path remapping.
//!
//! Exports and imports can touch gigabytes, so both run as background jobs
//! that stream file by file and report progress; the caller polls
//! [`job_status`]. Tauri passes real paths; the web server stores uploads and
//! finished exports under the app directory (see [`upload_path`]).

mod archive;
mod export;
mod import;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

pub use import::inspect_archive;

// ── Requests ──

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub sources: Vec<SourceSelection>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSelection {
    pub source: String,
    /// `None` exports every project of the source.
    #[serde(default)]
    pub project_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictPolicy {
    /// Keep the local session; the archived copy is not imported.
    #[default]
    Skip,
    /// Move the local copy into a backup directory, then import.
    Overwrite,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRef {
    pub source: String,
    pub project_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathMapping {
    pub source: String,
    pub project_id: String,
    /// Absolute project directory on this machine.
    pub target_path: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    #[serde(default)]
    pub conflict: ConflictPolicy,
    /// `None` imports every project in the archive.
    #[serde(default)]
    pub projects: Option<Vec<ProjectRef>>,
    #[serde(default)]
    pub path_mappings: Vec<PathMapping>,
}

// ── Preview ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub app_version: String,
    pub created_at: String,
    pub host_os: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub sources: Vec<PreviewSource>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSource {
    pub source: String,
    pub session_count: usize,
    pub projects: Vec<PreviewProject>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewProject {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    /// The original project directory exists on this machine.
    pub path_exists: bool,
    pub session_count: usize,
    /// Sessions that already exist locally (without remapping).
    pub conflict_count: usize,
    pub total_bytes: u64,
}

// ── Job state ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobKind {
    Export,
    Import,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub output_path: String,
    pub file_name: String,
    pub project_count: usize,
    pub session_count: usize,
    pub file_count: u64,
    pub total_bytes: u64,
    pub archive_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceReport {
    pub source: String,
    pub imported: usize,
    pub overwritten: usize,
    pub skipped: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub sources: Vec<SourceReport>,
    pub bookmarks_imported: usize,
    pub codex_threads_imported: usize,
    /// Where overwritten local files were moved to, if any.
    pub backup_dir: Option<String>,
    /// Per-session failures and non-fatal merge problems (capped).
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferJob {
    pub id: String,
    pub kind: JobKind,
    pub state: JobState,
    /// `scanning` → `copying` → `merging` → `done`.
    pub phase: String,
    pub processed_files: u64,
    pub total_files: u64,
    pub processed_bytes: u64,
    pub total_bytes: u64,
    pub current_item: Option<String>,
    pub error: Option<String>,
    pub export_summary: Option<ExportSummary>,
    pub import_report: Option<ImportReport>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

pub(crate) struct JobHandle {
    status: Mutex<TransferJob>,
    cancel: AtomicBool,
}

impl JobHandle {
    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub(crate) fn update(&self, f: impl FnOnce(&mut TransferJob)) {
        f(&mut self.status.lock());
    }

    pub(crate) fn set_phase(&self, phase: &str) {
        self.update(|job| {
            job.phase = phase.to_string();
            job.current_item = None;
        });
    }

    pub(crate) fn add_bytes(&self, bytes: u64) {
        self.update(|job| job.processed_bytes += bytes);
    }
}

pub(crate) const CANCELLED: &str = "已取消";

/// Stream `reader` into `writer` in chunks, reporting bytes and honouring
/// cancellation between chunks.
pub(crate) fn copy_with_progress(
    reader: &mut impl std::io::Read,
    writer: &mut impl std::io::Write,
    job: &JobHandle,
) -> std::io::Result<u64> {
    let mut buffer = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        if job.cancelled() {
            return Err(std::io::Error::other(CANCELLED));
        }
        let read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        writer.write_all(&buffer[..read])?;
        total += read as u64;
        job.add_bytes(read as u64);
    }
    Ok(total)
}

/// Regular files below `dir`, recursively. Links are skipped: a session must
/// never pull in files from outside its own directory.
pub(crate) fn walk_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    files
}

fn jobs() -> &'static Mutex<HashMap<String, Arc<JobHandle>>> {
    static JOBS: OnceLock<Mutex<HashMap<String, Arc<JobHandle>>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Start a job unless another transfer is still running: two imports writing
/// the same index files at once would race.
fn spawn_job(
    kind: JobKind,
    run: impl FnOnce(&JobHandle) -> Result<(), String> + Send + 'static,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let handle = Arc::new(JobHandle {
        status: Mutex::new(TransferJob {
            id: id.clone(),
            kind,
            state: JobState::Running,
            phase: "scanning".to_string(),
            processed_files: 0,
            total_files: 0,
            processed_bytes: 0,
            total_bytes: 0,
            current_item: None,
            error: None,
            export_summary: None,
            import_report: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
        }),
        cancel: AtomicBool::new(false),
    });
    {
        let mut all = jobs().lock();
        if all
            .values()
            .any(|job| job.status.lock().state == JobState::Running)
        {
            return Err("已有导入或导出任务正在进行，请等待其完成".to_string());
        }
        // Finished jobs are only kept so the UI can read the final status.
        all.retain(|_, job| {
            let status = job.status.lock();
            status.state == JobState::Running
                || status.finished_at.as_deref().is_some_and(|finished| {
                    chrono::DateTime::parse_from_rfc3339(finished).is_ok_and(|at| {
                        chrono::Utc::now().signed_duration_since(at)
                            < chrono::Duration::hours(24)
                    })
                })
        });
        all.insert(id.clone(), Arc::clone(&handle));
    }
    std::thread::Builder::new()
        .name(format!("asv-transfer-{id}"))
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&handle)))
                .unwrap_or_else(|_| Err("传输任务异常终止".to_string()));
            handle.update(|job| {
                job.finished_at = Some(chrono::Utc::now().to_rfc3339());
                job.current_item = None;
                match result {
                    Ok(()) => {
                        job.state = JobState::Completed;
                        job.phase = "done".to_string();
                    }
                    Err(error) if error == CANCELLED || handle.cancelled() => {
                        job.state = JobState::Cancelled;
                    }
                    Err(error) => {
                        job.state = JobState::Failed;
                        job.error = Some(error);
                    }
                }
            });
        })
        .map_err(|e| format!("无法启动传输任务：{e}"))?;
    Ok(id)
}

pub fn job_status(id: &str) -> Option<TransferJob> {
    jobs().lock().get(id).map(|job| job.status.lock().clone())
}

pub fn cancel_job(id: &str) -> Result<(), String> {
    let job = jobs()
        .lock()
        .get(id)
        .cloned()
        .ok_or_else(|| "传输任务不存在".to_string())?;
    job.cancel.store(true, Ordering::Relaxed);
    Ok(())
}

// ── Entry points ──

/// Export to `output_path` (must end with `.zip`). `None` writes into the app
/// directory for the web server to stream back via [`export_file`].
pub fn start_export(request: ExportRequest, output_path: Option<PathBuf>) -> Result<String, String> {
    if request.sources.is_empty() {
        return Err("请至少选择一个数据源".to_string());
    }
    for selection in &request.sources {
        crate::paths::SessionSourceKind::parse(&selection.source)?;
    }
    let output = match output_path {
        Some(path) => {
            validate_archive_name(&path)?;
            match path.parent() {
                Some(parent) if parent.as_os_str().is_empty() || parent.is_dir() => {}
                _ => return Err(format!("目标目录不存在：{}", path.display())),
            }
            Some(path)
        }
        None => None,
    };
    cleanup_stale_files();
    spawn_job(JobKind::Export, move |job| {
        let output = match output {
            Some(path) => path,
            None => {
                let id = job.status.lock().id.clone();
                web_export_path(&id)?
            }
        };
        export::run(job, &request, &output)
    })
}

/// Import an archive. With `delete_after`, the archive is removed when the
/// job ends (web uploads).
pub fn start_import(
    archive: PathBuf,
    options: ImportOptions,
    delete_after: bool,
) -> Result<String, String> {
    validate_archive_name(&archive)?;
    if !archive.is_file() {
        return Err(format!("归档不存在：{}", archive.display()));
    }
    for mapping in &options.path_mappings {
        crate::paths::SessionSourceKind::parse(&mapping.source)?;
        if !Path::new(mapping.target_path.trim()).is_absolute() {
            return Err(format!(
                "项目路径必须是本机绝对路径：{}",
                mapping.target_path
            ));
        }
    }
    spawn_job(JobKind::Import, move |job| {
        let result = import::run(job, &archive, &options);
        if delete_after {
            let _ = std::fs::remove_file(&archive);
        }
        result
    })
}

pub fn validate_archive_name(path: &Path) -> Result<(), String> {
    let is_zip = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"));
    if is_zip {
        Ok(())
    } else {
        Err("会话归档必须是 .zip 文件".to_string())
    }
}

// ── Web server storage ──

fn transfer_dir(kind: &str) -> Result<PathBuf, String> {
    let dir = crate::app_dir::get_app_dir()
        .ok_or("无法确定应用数据目录")?
        .join("transfer")
        .join(kind);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建传输目录失败：{e}"))?;
    Ok(dir)
}

fn validate_storage_id(id: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| "无效的传输 ID".to_string())
}

fn web_export_path(job_id: &str) -> Result<PathBuf, String> {
    validate_storage_id(job_id)?;
    Ok(transfer_dir("exports")?.join(format!("{job_id}.zip")))
}

/// Finished export of a web job, with the download file name.
pub fn export_file(job_id: &str) -> Result<(PathBuf, String), String> {
    let job = job_status(job_id).ok_or("传输任务不存在")?;
    let summary = match (job.kind, job.state, job.export_summary) {
        (JobKind::Export, JobState::Completed, Some(summary)) => summary,
        _ => return Err("导出尚未完成".to_string()),
    };
    let path = web_export_path(job_id)?;
    if Path::new(&summary.output_path) != path || !path.is_file() {
        return Err("导出文件不存在".to_string());
    }
    Ok((path, summary.file_name))
}

pub fn delete_export_file(job_id: &str) -> Result<(), String> {
    let path = web_export_path(job_id)?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("删除导出文件失败：{e}")),
    }
}

/// Fresh upload slot: `(upload_id, path)`.
pub fn new_upload() -> Result<(String, PathBuf), String> {
    cleanup_stale_files();
    let id = uuid::Uuid::new_v4().to_string();
    let path = transfer_dir("uploads")?.join(format!("{id}.zip"));
    Ok((id, path))
}

pub fn upload_path(upload_id: &str) -> Result<PathBuf, String> {
    validate_storage_id(upload_id)?;
    let path = transfer_dir("uploads")?.join(format!("{upload_id}.zip"));
    if path.is_file() {
        Ok(path)
    } else {
        Err("上传的归档不存在或已过期，请重新选择文件".to_string())
    }
}

pub fn delete_upload(upload_id: &str) -> Result<(), String> {
    let path = upload_path(upload_id)?;
    std::fs::remove_file(path).map_err(|e| format!("删除上传文件失败：{e}"))
}

/// Drop web uploads/exports older than a day (abandoned downloads, closed tabs).
fn cleanup_stale_files() {
    const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
    let Some(app_dir) = crate::app_dir::get_app_dir() else {
        return;
    };
    for kind in ["exports", "uploads"] {
        let Ok(entries) = std::fs::read_dir(app_dir.join("transfer").join(kind)) else {
            continue;
        };
        for entry in entries.flatten() {
            let stale = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                .is_some_and(|age| age > MAX_AGE);
            if stale {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}
