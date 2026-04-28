use std::sync::Arc;

use axum::extract::{Multipart, State};
use axum::http::StatusCode;
use axum::Json;
use diesel::prelude::*;
use uuid::Uuid;

use crate::models::NewJob;
use crate::schema::jobs;
use crate::{AppState, PendingPayload};

/// POST /api/upload — accept an image file OR a pre-extracted source list,
/// store it for solving, return a job ID.
///
/// The actual solve is triggered when the client connects to `/ws/solve/:job_id`.
///
/// Multipart fields:
/// - `file` (required): EITHER an image (FITS/PNG/JPEG/TIFF) OR a JSON
///   source list in the schema produced by `zodiacal extract`. The JSON
///   path is selected when the filename ends in `.json` or the field's
///   content-type is `application/json`. JSON skips decode + source
///   extraction entirely on the server (~5s win on a 9568×6380 frame).
/// - `scale_min_arcsec`, `scale_max_arcsec` (optional, text): pixel-scale window
/// - `ra_hint_deg`, `dec_hint_deg`, `radius_hint_deg` (optional, text):
///   constrain accepted solutions to a circular sky region
///
/// All hint fields are independent; partial sets are tolerated and ignored
/// at solve time unless the full pair/triple is provided.
///
/// If the JSON source list embeds `ra_deg`/`dec_deg`/`plate_scale_arcsec`,
/// they're auto-promoted to hints unless the multipart fields override them.
pub async fn upload(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<shared::SubmitJobResponse>, StatusCode> {
    let mut filename: Option<String> = None;
    let mut payload: Option<PendingPayload> = None;
    let mut embedded_hints = shared::SolveHints::default();
    let mut hints = shared::SolveHints::default();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    {
        match field.name() {
            Some("file") => {
                let fname = field.file_name().map(|s| s.to_string());
                let ctype = field.content_type().map(|s| s.to_string());
                let is_json = fname
                    .as_deref()
                    .is_some_and(|n| n.to_ascii_lowercase().ends_with(".json"))
                    || ctype.as_deref() == Some("application/json");
                filename = fname;
                let bytes = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
                if bytes.is_empty() {
                    continue;
                }
                if is_json {
                    let parsed = zodiacal::extraction::read_sources_json(bytes.as_ref())
                        .map_err(|_| StatusCode::BAD_REQUEST)?;
                    if parsed.sources.len() < 4 {
                        // Solver requires at least 4 sources to form a quad.
                        return Err(StatusCode::BAD_REQUEST);
                    }
                    // Promote embedded metadata into hints. The multipart
                    // hint fields (handled below) take precedence.
                    if let (Some(ra), Some(dec)) = (parsed.ra_deg, parsed.dec_deg) {
                        embedded_hints.ra_hint_deg = Some(ra);
                        embedded_hints.dec_hint_deg = Some(dec);
                        // Default to a generous 10° radius if RA/Dec given without one.
                        embedded_hints.radius_hint_deg = Some(10.0);
                    }
                    if let Some(scale) = parsed.plate_scale_arcsec {
                        // ±10% bracket around the embedded scale.
                        embedded_hints.scale_min_arcsec = Some(scale * 0.9);
                        embedded_hints.scale_max_arcsec = Some(scale * 1.1);
                    }
                    payload = Some(PendingPayload::Sources {
                        sources: parsed.sources,
                        image_size: (parsed.image_width, parsed.image_height),
                    });
                } else {
                    payload = Some(PendingPayload::Image(bytes.to_vec()));
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

    let Some(payload) = payload else {
        return Err(StatusCode::BAD_REQUEST);
    };

    // Multipart hints win over embedded ones, field by field.
    let hints = shared::SolveHints {
        scale_min_arcsec: hints.scale_min_arcsec.or(embedded_hints.scale_min_arcsec),
        scale_max_arcsec: hints.scale_max_arcsec.or(embedded_hints.scale_max_arcsec),
        ra_hint_deg: hints.ra_hint_deg.or(embedded_hints.ra_hint_deg),
        dec_hint_deg: hints.dec_hint_deg.or(embedded_hints.dec_hint_deg),
        radius_hint_deg: hints.radius_hint_deg.or(embedded_hints.radius_hint_deg),
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

    let payload_kind = match &payload {
        PendingPayload::Image(b) => format!("image ({} bytes)", b.len()),
        PendingPayload::Sources {
            sources,
            image_size,
        } => format!(
            "sources ({} sources, {:.0}x{:.0} px)",
            sources.len(),
            image_size.0,
            image_size.1
        ),
    };
    state
        .pending_uploads
        .lock()
        .unwrap()
        .insert(job_id, (payload, hints.clone()));

    tracing::info!(
        job_id = %job_id,
        filename = ?filename,
        payload = %payload_kind,
        scale = ?(hints.scale_min_arcsec, hints.scale_max_arcsec),
        sky = ?(hints.ra_hint_deg, hints.dec_hint_deg, hints.radius_hint_deg),
        "Upload accepted, awaiting WS connection"
    );

    Ok(Json(shared::SubmitJobResponse {
        job_id,
        status: shared::JobStatus::Pending,
    }))
}
