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
    fn job_status_roundtrip() {
        let status = JobStatus::Solving;
        let json = serde_json::to_string(&status).unwrap();
        let parsed: JobStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, JobStatus::Solving);
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
