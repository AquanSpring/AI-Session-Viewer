use std::path::PathBuf;

use session_core::transfer::{
    self, ExportRequest, ImportOptions, ImportPreview, RelocatePreview, RelocateRequest,
    TransferJob,
};

/// Start exporting to the path the user picked in the save dialog. Returns the job id.
#[tauri::command(async)]
pub fn transfer_start_export(request: ExportRequest, output_path: String) -> Result<String, String> {
    transfer::start_export(request, Some(PathBuf::from(output_path)))
}

/// Read the manifest of an archive and compare it with this machine.
#[tauri::command(async)]
pub fn transfer_inspect(archive_path: String) -> Result<ImportPreview, String> {
    transfer::inspect_archive(&PathBuf::from(archive_path))
}

#[tauri::command(async)]
pub fn transfer_start_import(archive_path: String, options: ImportOptions) -> Result<String, String> {
    transfer::start_import(PathBuf::from(archive_path), options, false)
}

#[tauri::command]
pub fn transfer_job_status(job_id: String) -> Result<TransferJob, String> {
    transfer::job_status(&job_id).ok_or_else(|| "传输任务不存在".to_string())
}

#[tauri::command]
pub fn transfer_cancel(job_id: String) -> Result<(), String> {
    transfer::cancel_job(&job_id)
}

/// What moving a project directory would change on this machine.
#[tauri::command(async)]
pub fn transfer_relocate_preview(request: RelocateRequest) -> Result<RelocatePreview, String> {
    transfer::relocate_preview(&request)
}

#[tauri::command(async)]
pub fn transfer_start_relocate(request: RelocateRequest) -> Result<String, String> {
    transfer::start_relocate(request)
}
