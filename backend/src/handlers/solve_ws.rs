use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, State, WebSocketUpgrade};
use axum::response::Response;
use chrono::Utc;
use diesel::prelude::*;
use uuid::Uuid;

use crate::decode::decode_image;
use crate::models::UpdateJob;
use crate::schema::jobs;
use crate::AppState;

/// GET /ws/solve/:job_id — WebSocket endpoint that streams solve progress.
///
/// The client uploads via POST /api/upload first, then connects here.
/// The server sends typed `SolveServerMsg` messages as the solve progresses.
pub async fn handler(
    ws: WebSocketUpgrade,
    Path(job_id): Path<Uuid>,
    State(state): State<Arc<AppState>>,
) -> Response {
    ws.on_upgrade(move |socket| async move {
        let mut conn = ws_bridge::server::into_connection::<shared::SolveSocket>(socket);

        // Retrieve pending upload bytes + hints
        let pending = state.pending_uploads.lock().unwrap().remove(&job_id);
        let Some((image_bytes, hints)) = pending else {
            let _ = conn
                .send(shared::SolveServerMsg::Failed {
                    reason: "No pending upload for this job ID".to_string(),
                })
                .await;
            return;
        };

        let _ = conn.send(shared::SolveServerMsg::Accepted { job_id }).await;

        // Run the solve pipeline, streaming progress
        let (mut tx, _rx) = conn.split();

        if let Err(e) = run_solve_streaming(&state, job_id, &image_bytes, &hints, &mut tx).await {
            tracing::error!(job_id = %job_id, "Solve error: {e:#}");
            let _ = tx
                .send(shared::SolveServerMsg::Failed {
                    reason: format!("{e:#}"),
                })
                .await;
        }
    })
}

/// Run the full solve pipeline, sending progress over the WS sender.
async fn run_solve_streaming(
    state: &AppState,
    job_id: Uuid,
    image_bytes: &[u8],
    hints: &shared::SolveHints,
    tx: &mut ws_bridge::WsSender<shared::SolveServerMsg>,
) -> anyhow::Result<()> {
    // Mark job as solving in DB
    update_job_status(state, job_id, "solving", None)?;

    // Send extracting status
    let _ = tx
        .send(shared::SolveServerMsg::Extracting { n_sources: None })
        .await;

    let image_bytes = image_bytes.to_vec();
    let indexes = state.indexes.clone();
    let hints = hints.clone();

    // Channel to bridge sync callback -> async WS
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<usize>(32);

    // Spawn the blocking solve
    let solve_handle = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let array = decode_image(&image_bytes)?;
        let (h, w) = array.dim();

        let sources = zodiacal::extraction::extract_sources(
            &array,
            &zodiacal::extraction::ExtractionConfig::default(),
        );

        // Report source count (send will fail if receiver dropped — that's fine)
        let _ = progress_tx.blocking_send(sources.len());

        let index_refs: Vec<&zodiacal::index::Index> = indexes.iter().collect();
        let solver_config = build_solver_config(&hints);

        let mut last_sent = Instant::now();
        let (solution, stats) = zodiacal::solver::solve_with_callback(
            &sources,
            &index_refs,
            (w as f64, h as f64),
            &solver_config,
            |stats| {
                // Throttle: send at most every 250ms
                if last_sent.elapsed() > Duration::from_millis(250) {
                    let _ = progress_tx.blocking_send(stats.n_verified);
                    last_sent = Instant::now();
                }
            },
        );

        drop(progress_tx); // Signal completion to the async side
        Ok((solution, stats, w, h, sources.len()))
    });

    // Forward progress to WebSocket while solve runs
    let mut n_sources_sent = false;
    while let Some(n_verified) = progress_rx.recv().await {
        if !n_sources_sent {
            // First message carries source count (n_verified == 0 at that point)
            let _ = tx
                .send(shared::SolveServerMsg::Extracting {
                    n_sources: Some(n_verified),
                })
                .await;
            n_sources_sent = true;
        } else {
            let _ = tx
                .send(shared::SolveServerMsg::Solving { n_verified })
                .await;
        }
    }

    // Solve is done — get the result
    let (solution, stats, width, height, n_sources) = solve_handle.await??;

    // Send extraction count if we didn't get to it yet
    if !n_sources_sent {
        let _ = tx
            .send(shared::SolveServerMsg::Extracting {
                n_sources: Some(n_sources),
            })
            .await;
    }

    match solution {
        Some(sol) => {
            let (ra_rad, dec_rad) = sol.wcs.field_center();
            let ra_deg = ra_rad.to_degrees();
            let dec_deg = dec_rad.to_degrees();
            let pixel_scale_deg = sol.wcs.pixel_scale();
            let pixel_scale_arcsec = pixel_scale_deg * 3600.0;
            // Roll about boresight matching focalplane convention
            // (body +X→+Y about +Z, right-hand rule). zodiacal's TanWcs CD
            // matrix maps pixel offsets to FITS-convention IWC where +x = west
            // (RA-decreasing); using cd[1][0] (the dDec/dx component) over
            // cd[0][0] gives the camera roll directly, with sign matching the
            // trajectory's `roll_deg`.
            let orientation_deg = sol.wcs.cd[1][0].atan2(sol.wcs.cd[0][0]).to_degrees();
            let field_width_deg = pixel_scale_deg * width as f64;
            let field_height_deg = pixel_scale_deg * height as f64;

            let result = shared::SolveResult {
                ra_deg,
                dec_deg,
                orientation_deg,
                pixel_scale_arcsec,
                field_width_deg,
                field_height_deg,
            };

            // Update DB
            let update = UpdateJob {
                status: Some("solved".to_string()),
                ra_deg: Some(ra_deg),
                dec_deg: Some(dec_deg),
                orientation_deg: Some(orientation_deg),
                pixel_scale_arcsec: Some(pixel_scale_arcsec),
                field_width_deg: Some(field_width_deg),
                field_height_deg: Some(field_height_deg),
                error_message: None,
                updated_at: Some(Utc::now().naive_utc()),
            };
            apply_job_update(state, job_id, &update)?;

            let _ = tx.send(shared::SolveServerMsg::Solved { result }).await;

            tracing::info!(
                job_id = %job_id,
                ra = ra_deg,
                dec = dec_deg,
                scale = pixel_scale_arcsec,
                verified = stats.n_verified,
                "Plate solve succeeded"
            );
        }
        None => {
            let reason = if stats.timed_out {
                "Solve timed out".to_string()
            } else {
                format!(
                    "No solution found ({} candidates verified)",
                    stats.n_verified
                )
            };
            update_job_status(state, job_id, "failed", Some(&reason))?;
            let _ = tx
                .send(shared::SolveServerMsg::Failed {
                    reason: reason.clone(),
                })
                .await;
            tracing::warn!(job_id = %job_id, "{reason}");
        }
    }

    Ok(())
}

fn update_job_status(
    state: &AppState,
    job_id: Uuid,
    status: &str,
    error_message: Option<&str>,
) -> anyhow::Result<()> {
    let mut conn = state.db_pool.get()?;
    diesel::update(jobs::table.filter(jobs::id.eq(job_id)))
        .set((
            jobs::status.eq(status),
            jobs::error_message.eq(error_message),
            jobs::updated_at.eq(Utc::now().naive_utc()),
        ))
        .execute(&mut conn)?;
    Ok(())
}

fn apply_job_update(state: &AppState, job_id: Uuid, update: &UpdateJob) -> anyhow::Result<()> {
    let mut conn = state.db_pool.get()?;
    diesel::update(jobs::table.filter(jobs::id.eq(job_id)))
        .set(update)
        .execute(&mut conn)?;
    Ok(())
}

/// Translate optional client-supplied hints into a [`zodiacal::solver::SolverConfig`].
///
/// Hints are independent and only take effect when the full pair/triple is
/// provided (scale needs both min/max; sky region needs RA, Dec, radius).
/// The 60s solve timeout is always applied.
fn build_solver_config(hints: &shared::SolveHints) -> zodiacal::solver::SolverConfig {
    let scale_range = match (hints.scale_min_arcsec, hints.scale_max_arcsec) {
        (Some(lo), Some(hi)) if lo > 0.0 && hi >= lo => Some((lo, hi)),
        _ => None,
    };

    let within = match (hints.ra_hint_deg, hints.dec_hint_deg, hints.radius_hint_deg) {
        (Some(ra), Some(dec), Some(radius)) if radius > 0.0 => {
            let center = starfield::Equatorial::from_degrees(ra, dec);
            Some(zodiacal::solver::SkyRegion::from_degrees(center, radius))
        }
        _ => None,
    };

    zodiacal::solver::SolverConfig {
        timeout: Some(Duration::from_secs(60)),
        scale_range,
        within,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_solver_config_no_hints() {
        let cfg = build_solver_config(&shared::SolveHints::default());
        assert_eq!(cfg.scale_range, None);
        assert!(cfg.within.is_none());
        assert_eq!(cfg.timeout, Some(Duration::from_secs(60)));
    }

    #[test]
    fn build_solver_config_scale_pair() {
        let hints = shared::SolveHints {
            scale_min_arcsec: Some(0.1),
            scale_max_arcsec: Some(0.2),
            ..Default::default()
        };
        let cfg = build_solver_config(&hints);
        assert_eq!(cfg.scale_range, Some((0.1, 0.2)));
        assert!(cfg.within.is_none());
    }

    #[test]
    fn build_solver_config_partial_scale_ignored() {
        // Only one of min/max → ignored
        let hints = shared::SolveHints {
            scale_min_arcsec: Some(0.1),
            ..Default::default()
        };
        assert_eq!(build_solver_config(&hints).scale_range, None);
    }

    #[test]
    fn build_solver_config_invalid_scale_ignored() {
        let hints = shared::SolveHints {
            scale_min_arcsec: Some(0.5),
            scale_max_arcsec: Some(0.1), // backwards
            ..Default::default()
        };
        assert_eq!(build_solver_config(&hints).scale_range, None);
    }

    #[test]
    fn build_solver_config_sky_region() {
        let hints = shared::SolveHints {
            ra_hint_deg: Some(213.5),
            dec_hint_deg: Some(-55.8),
            radius_hint_deg: Some(1.0),
            ..Default::default()
        };
        let cfg = build_solver_config(&hints);
        assert!(cfg.within.is_some());
        let region = cfg.within.unwrap();
        // 1 deg in radians
        assert!((region.radius_rad - 1.0_f64.to_radians()).abs() < 1e-12);
    }

    #[test]
    fn build_solver_config_partial_sky_ignored() {
        // Missing radius → no region
        let hints = shared::SolveHints {
            ra_hint_deg: Some(213.5),
            dec_hint_deg: Some(-55.8),
            ..Default::default()
        };
        assert!(build_solver_config(&hints).within.is_none());
    }
}
