use chrono::NaiveDateTime;
use diesel::prelude::*;
use uuid::Uuid;

use crate::schema::jobs;

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = jobs)]
pub struct Job {
    pub id: Uuid,
    pub original_filename: Option<String>,
    pub status: String,
    pub ra_deg: Option<f64>,
    pub dec_deg: Option<f64>,
    pub orientation_deg: Option<f64>,
    pub pixel_scale_arcsec: Option<f64>,
    pub field_width_deg: Option<f64>,
    pub field_height_deg: Option<f64>,
    pub error_message: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = jobs)]
pub struct NewJob {
    pub original_filename: Option<String>,
}

impl From<Job> for shared::Job {
    fn from(job: Job) -> Self {
        let status = match job.status.as_str() {
            "pending" => shared::JobStatus::Pending,
            "solving" => shared::JobStatus::Solving,
            "solved" => shared::JobStatus::Solved,
            "failed" => shared::JobStatus::Failed,
            _ => shared::JobStatus::Failed,
        };

        let result = if status == shared::JobStatus::Solved {
            match (
                job.ra_deg,
                job.dec_deg,
                job.orientation_deg,
                job.pixel_scale_arcsec,
                job.field_width_deg,
                job.field_height_deg,
            ) {
                (Some(ra), Some(dec), Some(orient), Some(scale), Some(w), Some(h)) => {
                    Some(shared::SolveResult {
                        ra_deg: ra,
                        dec_deg: dec,
                        orientation_deg: orient,
                        pixel_scale_arcsec: scale,
                        field_width_deg: w,
                        field_height_deg: h,
                    })
                }
                _ => None,
            }
        } else {
            None
        };

        shared::Job {
            id: job.id,
            original_filename: job.original_filename,
            status,
            result,
            created_at: job.created_at,
            updated_at: job.updated_at,
        }
    }
}
