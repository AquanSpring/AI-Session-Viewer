//! Session archive export/import. Archives can be gigabytes, so uploads are
//! streamed to disk and downloads are streamed from disk; the work itself
//! runs as a session-core background job that the client polls.

use axum::body::Body;
use axum::extract::{Extension, Path, Query};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use session_core::transfer::{
    self, ExportRequest, ImportOptions, ImportPreview, RelocatePreview, RelocateRequest, TransferJob,
};
use tokio::io::AsyncWriteExt;

use crate::{require_ws_auth, AppToken, WsTicketStore};

type ApiError = (StatusCode, String);

fn bad_request(error: String) -> ApiError {
    (StatusCode::BAD_REQUEST, error)
}

fn internal(error: impl ToString) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobResponse {
    pub job_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadResponse {
    pub upload_id: String,
}

pub async fn start_export(Json(request): Json<ExportRequest>) -> Result<Json<JobResponse>, ApiError> {
    let job_id = tokio::task::spawn_blocking(move || transfer::start_export(request, None))
        .await
        .map_err(internal)?
        .map_err(bad_request)?;
    Ok(Json(JobResponse { job_id }))
}

pub async fn job_status(Path(job_id): Path<String>) -> Result<Json<TransferJob>, ApiError> {
    transfer::job_status(&job_id)
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "传输任务不存在".to_string()))
}

pub async fn cancel_job(Path(job_id): Path<String>) -> Result<Json<()>, ApiError> {
    transfer::cancel_job(&job_id)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e))
}

#[derive(Deserialize)]
pub struct DownloadQuery {
    #[serde(default)]
    pub ticket: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// Keep the suggested file name to a safe ASCII subset.
fn download_name(requested: Option<&str>) -> String {
    let cleaned: String = requested
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(120)
        .collect();
    if cleaned.to_ascii_lowercase().ends_with(".zip") && cleaned.len() > 4 {
        cleaned
    } else {
        "ai-sessions.zip".to_string()
    }
}

/// Plain browser download (no `Authorization` header possible), so it sits
/// outside the auth middleware and accepts a single-use ticket instead.
pub async fn download_export(
    Path(job_id): Path<String>,
    Query(query): Query<DownloadQuery>,
    Extension(app_token): Extension<AppToken>,
    Extension(tickets): Extension<WsTicketStore>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    require_ws_auth(&headers, query.ticket.as_deref(), &app_token, &tickets)
        .map_err(|status| (status, "Authentication required".to_string()))?;
    let (path, _) = transfer::export_file(&job_id).map_err(|e| (StatusCode::NOT_FOUND, e))?;
    let file = tokio::fs::File::open(path).await.map_err(internal)?;
    let length = file.metadata().await.map_err(internal)?.len();
    let name = download_name(query.name.as_deref());
    let body = Body::from_stream(tokio_util::io::ReaderStream::with_capacity(file, 256 * 1024));
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (header::CONTENT_LENGTH, length.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        body,
    )
        .into_response())
}

pub async fn delete_export(Path(job_id): Path<String>) -> Result<Json<()>, ApiError> {
    transfer::delete_export_file(&job_id)
        .map(Json)
        .map_err(bad_request)
}

/// Stream the request body into a fresh upload slot.
pub async fn upload_archive(body: Body) -> Result<Json<UploadResponse>, ApiError> {
    let (upload_id, path) = tokio::task::spawn_blocking(transfer::new_upload)
        .await
        .map_err(internal)?
        .map_err(internal)?;
    let result = async {
        let mut file = tokio::fs::File::create(&path).await?;
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.next().await {
            file.write_all(&chunk.map_err(std::io::Error::other)?).await?;
        }
        file.flush().await?;
        file.sync_all().await
    }
    .await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(path).await;
        return Err(internal(format!("上传归档失败：{error}")));
    }
    Ok(Json(UploadResponse { upload_id }))
}

pub async fn preview_upload(Path(upload_id): Path<String>) -> Result<Json<ImportPreview>, ApiError> {
    tokio::task::spawn_blocking(move || {
        let path = transfer::upload_path(&upload_id)?;
        transfer::inspect_archive(&path)
    })
    .await
    .map_err(internal)?
    .map(Json)
    .map_err(bad_request)
}

pub async fn delete_upload(Path(upload_id): Path<String>) -> Result<Json<()>, ApiError> {
    transfer::delete_upload(&upload_id)
        .map(Json)
        .map_err(bad_request)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportBody {
    pub upload_id: String,
    #[serde(default)]
    pub options: ImportOptions,
}

/// The upload is deleted once the import job ends.
pub async fn start_import(Json(body): Json<ImportBody>) -> Result<Json<JobResponse>, ApiError> {
    let job_id = tokio::task::spawn_blocking(move || {
        let path = transfer::upload_path(&body.upload_id)?;
        transfer::start_import(path, body.options, true)
    })
    .await
    .map_err(internal)?
    .map_err(bad_request)?;
    Ok(Json(JobResponse { job_id }))
}

pub async fn relocate_preview(Json(request): Json<RelocateRequest>) -> Result<Json<RelocatePreview>, ApiError> {
    tokio::task::spawn_blocking(move || transfer::relocate_preview(&request))
        .await
        .map_err(internal)?
        .map(Json)
        .map_err(bad_request)
}

pub async fn start_relocate(Json(request): Json<RelocateRequest>) -> Result<Json<JobResponse>, ApiError> {
    let job_id = tokio::task::spawn_blocking(move || transfer::start_relocate(request))
        .await
        .map_err(internal)?
        .map_err(bad_request)?;
    Ok(Json(JobResponse { job_id }))
}
