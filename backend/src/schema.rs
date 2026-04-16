diesel::table! {
    jobs (id) {
        id -> Uuid,
        original_filename -> Nullable<Text>,
        status -> Text,
        ra_deg -> Nullable<Float8>,
        dec_deg -> Nullable<Float8>,
        orientation_deg -> Nullable<Float8>,
        pixel_scale_arcsec -> Nullable<Float8>,
        field_width_deg -> Nullable<Float8>,
        field_height_deg -> Nullable<Float8>,
        error_message -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}
