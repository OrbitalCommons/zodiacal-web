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
pub async fn upload(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<shared::SubmitJobResponse>, StatusCode> {
    // Extract file from multipart form
    let (filename, image_bytes) = loop {
        let field = multipart
            .next_field()
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?;

        let Some(field) = field else {
            return Err(StatusCode::BAD_REQUEST);
        };

        let name = field.file_name().map(|s| s.to_string());
        let bytes = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;

        if !bytes.is_empty() {
            break (name, bytes.to_vec());
        }
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

    // Store bytes for the WS handler to pick up
    state
        .pending_uploads
        .lock()
        .unwrap()
        .insert(job_id, image_bytes);

    tracing::info!(job_id = %job_id, filename = ?filename, "Upload accepted, awaiting WS connection");

    Ok(Json(shared::SubmitJobResponse {
        job_id,
        status: shared::JobStatus::Pending,
    }))
}
