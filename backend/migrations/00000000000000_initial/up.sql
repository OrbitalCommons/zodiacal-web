CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

CREATE TABLE jobs (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    original_filename TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    ra_deg DOUBLE PRECISION,
    dec_deg DOUBLE PRECISION,
    orientation_deg DOUBLE PRECISION,
    pixel_scale_arcsec DOUBLE PRECISION,
    field_width_deg DOUBLE PRECISION,
    field_height_deg DOUBLE PRECISION,
    error_message TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);
