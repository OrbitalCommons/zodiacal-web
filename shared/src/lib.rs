use serde::{Deserialize, Serialize};
use uuid::Uuid;
use ws_bridge::WsEndpoint;

// ---------------------------------------------------------------------------
// WebSocket endpoint definition — single source of truth for server + client
// ---------------------------------------------------------------------------

/// The main application WebSocket endpoint.
pub struct AppSocket;

impl WsEndpoint for AppSocket {
    const PATH: &'static str = "/ws";
    type ServerMsg = ServerMsg;
    type ClientMsg = ClientMsg;
}

/// Messages sent from the server to the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMsg {
    /// Heartbeat to keep connection alive
    Heartbeat,

    /// Error from server
    Error { message: String },

    /// Server is shutting down
    ServerShutdown {
        reason: String,
        reconnect_delay_ms: u64,
    },

    /// A solve job has been accepted and queued
    JobAccepted { job_id: Uuid },

    /// Progress update for a running solve
    JobProgress { job_id: Uuid, status: JobStatus },

    /// Solve completed with results
    JobCompleted { job_id: Uuid, result: SolveResult },
}

/// Messages sent from the client to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMsg {
    /// Ping — server should respond with Heartbeat
    Ping,

    /// Subscribe to updates for a specific job
    SubscribeJob { job_id: Uuid },
}

// ---------------------------------------------------------------------------
// Per-job solve WebSocket — streams progress during a plate solve
// ---------------------------------------------------------------------------

/// WebSocket endpoint for streaming solve progress on a specific job.
///
/// The actual route is `/ws/solve/:job_id` (the path param is handled by axum).
/// The client connects after uploading via HTTP, and receives progress updates
/// until the solve completes or fails.
pub struct SolveSocket;

impl WsEndpoint for SolveSocket {
    const PATH: &'static str = "/ws/solve";
    type ServerMsg = SolveServerMsg;
    type ClientMsg = SolveClientMsg;
}

/// Server-to-client messages on a solve WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SolveServerMsg {
    /// Connection accepted, solve is starting
    Accepted { job_id: Uuid },

    /// Extracting star sources from the image
    Extracting { n_sources: Option<usize> },

    /// Solver is running; periodic progress update
    Solving { n_verified: usize },

    /// Solve succeeded
    Solved { result: SolveResult },

    /// Solve failed
    Failed { reason: String },
}

/// Client-to-server messages on a solve WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SolveClientMsg {
    /// Request to cancel the solve (best-effort)
    Cancel,
}

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// Status of a plate-solve job.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Job is queued, waiting to be processed
    Pending,
    /// Solver is running
    Solving,
    /// Solve succeeded
    Solved,
    /// Solve failed
    Failed,
}

/// Optional hints supplied with an upload to constrain the solver search.
///
/// All fields are independent and optional. When provided:
/// - `scale_min_arcsec` / `scale_max_arcsec` restrict candidate WCS solutions
///   whose pixel scale falls outside `[min, max]` (arcseconds per pixel).
///   Both must be set together to take effect.
/// - `ra_hint_deg` + `dec_hint_deg` + `radius_hint_deg` restrict accepted
///   solutions to a circular sky region centered on (RA, Dec). All three
///   must be set together to take effect.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SolveHints {
    pub scale_min_arcsec: Option<f64>,
    pub scale_max_arcsec: Option<f64>,
    pub ra_hint_deg: Option<f64>,
    pub dec_hint_deg: Option<f64>,
    pub radius_hint_deg: Option<f64>,
}

/// Result of a successful plate solve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolveResult {
    /// Right ascension of image center (degrees)
    pub ra_deg: f64,
    /// Declination of image center (degrees)
    pub dec_deg: f64,
    /// Image orientation angle (degrees east of north)
    pub orientation_deg: f64,
    /// Pixel scale (arcseconds per pixel)
    pub pixel_scale_arcsec: f64,
    /// Field width (degrees)
    pub field_width_deg: f64,
    /// Field height (degrees)
    pub field_height_deg: f64,
}

// ---------------------------------------------------------------------------
// HTTP API types
// ---------------------------------------------------------------------------

/// Health check response from `/api/health`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
}

/// Response after uploading an image for solving.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitJobResponse {
    pub job_id: Uuid,
    pub status: JobStatus,
}

/// Full job record returned by the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub original_filename: Option<String>,
    pub status: JobStatus,
    pub result: Option<SolveResult>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_msg_heartbeat_roundtrip() {
        let msg = ServerMsg::Heartbeat;
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ServerMsg = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, ServerMsg::Heartbeat));
    }

    #[test]
    fn server_msg_error_roundtrip() {
        let msg = ServerMsg::Error {
            message: "something broke".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ServerMsg = serde_json::from_str(&json).unwrap();
        match parsed {
            ServerMsg::Error { message } => assert_eq!(message, "something broke"),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn server_msg_shutdown_roundtrip() {
        let msg = ServerMsg::ServerShutdown {
            reason: "restarting".to_string(),
            reconnect_delay_ms: 1000,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ServerMsg = serde_json::from_str(&json).unwrap();
        match parsed {
            ServerMsg::ServerShutdown {
                reason,
                reconnect_delay_ms,
            } => {
                assert_eq!(reason, "restarting");
                assert_eq!(reconnect_delay_ms, 1000);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn server_msg_job_accepted_roundtrip() {
        let id = Uuid::new_v4();
        let msg = ServerMsg::JobAccepted { job_id: id };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ServerMsg = serde_json::from_str(&json).unwrap();
        match parsed {
            ServerMsg::JobAccepted { job_id } => assert_eq!(job_id, id),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn server_msg_job_completed_roundtrip() {
        let id = Uuid::new_v4();
        let result = SolveResult {
            ra_deg: 180.0,
            dec_deg: 45.0,
            orientation_deg: 12.5,
            pixel_scale_arcsec: 1.2,
            field_width_deg: 2.0,
            field_height_deg: 1.5,
        };
        let msg = ServerMsg::JobCompleted {
            job_id: id,
            result: result.clone(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ServerMsg = serde_json::from_str(&json).unwrap();
        match parsed {
            ServerMsg::JobCompleted { job_id, result: r } => {
                assert_eq!(job_id, id);
                assert_eq!(r.ra_deg, 180.0);
                assert_eq!(r.dec_deg, 45.0);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn client_msg_ping_roundtrip() {
        let msg = ClientMsg::Ping;
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ClientMsg = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, ClientMsg::Ping));
    }

    #[test]
    fn client_msg_subscribe_roundtrip() {
        let id = Uuid::new_v4();
        let msg = ClientMsg::SubscribeJob { job_id: id };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ClientMsg = serde_json::from_str(&json).unwrap();
        match parsed {
            ClientMsg::SubscribeJob { job_id } => assert_eq!(job_id, id),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn solve_server_msg_accepted_roundtrip() {
        let id = Uuid::new_v4();
        let msg = SolveServerMsg::Accepted { job_id: id };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: SolveServerMsg = serde_json::from_str(&json).unwrap();
        match parsed {
            SolveServerMsg::Accepted { job_id } => assert_eq!(job_id, id),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn solve_server_msg_solving_roundtrip() {
        let msg = SolveServerMsg::Solving { n_verified: 42 };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: SolveServerMsg = serde_json::from_str(&json).unwrap();
        match parsed {
            SolveServerMsg::Solving { n_verified } => assert_eq!(n_verified, 42),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn solve_server_msg_solved_roundtrip() {
        let result = SolveResult {
            ra_deg: 180.0,
            dec_deg: -45.0,
            orientation_deg: 10.0,
            pixel_scale_arcsec: 1.5,
            field_width_deg: 2.0,
            field_height_deg: 1.5,
        };
        let msg = SolveServerMsg::Solved { result };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: SolveServerMsg = serde_json::from_str(&json).unwrap();
        match parsed {
            SolveServerMsg::Solved { result: r } => assert_eq!(r.ra_deg, 180.0),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn solve_server_msg_failed_roundtrip() {
        let msg = SolveServerMsg::Failed {
            reason: "timed out".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: SolveServerMsg = serde_json::from_str(&json).unwrap();
        match parsed {
            SolveServerMsg::Failed { reason } => assert_eq!(reason, "timed out"),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn solve_client_msg_cancel_roundtrip() {
        let msg = SolveClientMsg::Cancel;
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: SolveClientMsg = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SolveClientMsg::Cancel));
    }

    #[test]
    fn job_status_roundtrip() {
        let status = JobStatus::Solving;
        let json = serde_json::to_string(&status).unwrap();
        let parsed: JobStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, JobStatus::Solving);
    }

    #[test]
    fn solve_hints_default_is_all_none() {
        let hints = SolveHints::default();
        assert_eq!(hints.scale_min_arcsec, None);
        assert_eq!(hints.scale_max_arcsec, None);
        assert_eq!(hints.ra_hint_deg, None);
        assert_eq!(hints.dec_hint_deg, None);
        assert_eq!(hints.radius_hint_deg, None);
    }

    #[test]
    fn solve_hints_roundtrip() {
        let hints = SolveHints {
            scale_min_arcsec: Some(0.1),
            scale_max_arcsec: Some(0.2),
            ra_hint_deg: Some(213.5),
            dec_hint_deg: Some(-55.8),
            radius_hint_deg: Some(1.0),
        };
        let json = serde_json::to_string(&hints).unwrap();
        let parsed: SolveHints = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, hints);
    }

    #[test]
    fn solve_hints_partial_roundtrip() {
        let hints = SolveHints {
            scale_min_arcsec: Some(0.1),
            scale_max_arcsec: Some(0.2),
            ..Default::default()
        };
        let json = serde_json::to_string(&hints).unwrap();
        let parsed: SolveHints = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.scale_min_arcsec, Some(0.1));
        assert_eq!(parsed.ra_hint_deg, None);
    }

    #[test]
    fn solve_result_roundtrip() {
        let result = SolveResult {
            ra_deg: 123.456,
            dec_deg: -67.89,
            orientation_deg: 45.0,
            pixel_scale_arcsec: 0.5,
            field_width_deg: 1.0,
            field_height_deg: 0.75,
        };
        let json = serde_json::to_string(&result).unwrap();
        let parsed: SolveResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.ra_deg, 123.456);
        assert_eq!(parsed.dec_deg, -67.89);
    }

    #[test]
    fn job_roundtrip() {
        let job = Job {
            id: Uuid::new_v4(),
            original_filename: Some("andromeda.fits".to_string()),
            status: JobStatus::Pending,
            result: None,
            created_at: chrono::Utc::now().naive_utc(),
            updated_at: chrono::Utc::now().naive_utc(),
        };
        let json = serde_json::to_string(&job).unwrap();
        let parsed: Job = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, job.id);
        assert_eq!(parsed.original_filename, Some("andromeda.fits".to_string()));
    }
}
