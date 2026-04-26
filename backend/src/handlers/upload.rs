use std::sync::Arc;

use axum::extract::{Multipart, State};
use axum::http::StatusCode;
use axum::Json;
use diesel::prelude::*;
use uuid::Uuid;

use crate::models::NewJob;
use crate::schema::jobs;
use crate::AppState;

/// POST /api/upload — accept an image file, store it for solving, return a job ID.
///
/// The actual solve is triggered when the client connects to `/ws/solve/:job_id`.
///
/// Multipart fields:
/// - `file` (required): the image bytes (FITS/PNG/JPEG/TIFF)
/// - `scale_min_arcsec`, `scale_max_arcsec` (optional, text): pixel-scale window
/// - `ra_hint_deg`, `dec_hint_deg`, `radius_hint_deg` (optional, text):
///   constrain accepted solutions to a circular sky region
///
/// All hint fields are independent; partial sets are tolerated and ignored
/// at solve time unless the full pair/triple is provided.
pub async fn upload(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<shared::SubmitJobResponse>, StatusCode> {
    let mut filename: Option<String> = None;
    let mut image_bytes: Option<Vec<u8>> = None;
    let mut hints = shared::SolveHints::default();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    {
        match field.name() {
            Some("file") => {
                filename = field.file_name().map(|s| s.to_string());
                let bytes = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
                if !bytes.is_empty() {
                    image_bytes = Some(bytes.to_vec());
                }
            }
            Some(
                name @ ("scale_min_arcsec" | "scale_max_arcsec" | "ra_hint_deg" | "dec_hint_deg"
                | "radius_hint_deg"),
            ) => {
                let key = name.to_string();
                let raw = field.text().await.map_err(|_| StatusCode::BAD_REQUEST)?;
                let raw = raw.trim();
                if raw.is_empty() {
                    continue;
                }
                let v = raw.parse::<f64>().map_err(|_| StatusCode::BAD_REQUEST)?;
                if !v.is_finite() {
                    return Err(StatusCode::BAD_REQUEST);
                }
                match key.as_str() {
                    "scale_min_arcsec" => hints.scale_min_arcsec = Some(v),
                    "scale_max_arcsec" => hints.scale_max_arcsec = Some(v),
                    "ra_hint_deg" => hints.ra_hint_deg = Some(v),
                    "dec_hint_deg" => hints.dec_hint_deg = Some(v),
                    "radius_hint_deg" => hints.radius_hint_deg = Some(v),
                    _ => unreachable!(),
                }
            }
            // Ignore unknown fields rather than rejecting — clients might
            // send extras (e.g. CSRF tokens) without breaking the upload.
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let Some(image_bytes) = image_bytes else {
        return Err(StatusCode::BAD_REQUEST);
    };

    // Insert pending job in DB
    let job_id = {
        let mut conn = state
            .db_pool
            .get()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let new_job = NewJob {
            original_filename: filename.clone(),
        };
        diesel::insert_into(jobs::table)
            .values(&new_job)
            .returning(jobs::id)
            .get_result::<Uuid>(&mut conn)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };

    state
        .pending_uploads
        .lock()
        .unwrap()
        .insert(job_id, (image_bytes, hints.clone()));

    tracing::info!(
        job_id = %job_id,
        filename = ?filename,
        scale = ?(hints.scale_min_arcsec, hints.scale_max_arcsec),
        sky = ?(hints.ra_hint_deg, hints.dec_hint_deg, hints.radius_hint_deg),
        "Upload accepted, awaiting WS connection"
    );

    Ok(Json(shared::SubmitJobResponse {
        job_id,
        status: shared::JobStatus::Pending,
    }))
}
