# Zodiacal — Plate Solver

Web service for plate-solving arbitrary uploaded astronomy images. Inspired by [astrometry.net](https://astrometry.net).

Upload an image of the night sky and get back the sky coordinates (RA/Dec), orientation, pixel scale, and field of view. Supports FITS, JPEG, PNG, and TIFF.

## Architecture

```
Cargo.toml                        # Workspace root (backend, frontend, shared)
├── shared/src/lib.rs             # Serde types + WS endpoints (AppSocket, SolveSocket)
├── frontend/
│   ├── Trunk.toml                # WASM bundler config
│   ├── index.html                # Trunk entry point
│   └── src/main.rs               # Yew App — upload + live solve progress
├── backend/
│   ├── src/
│   │   ├── main.rs               # Axum server, routes, index loading
│   │   ├── decode.rs             # FITS/JPEG/PNG/TIFF -> ndarray
│   │   ├── db.rs                 # Diesel pool + embedded migrations
│   │   ├── models.rs             # Queryable/Insertable/AsChangeset structs
│   │   ├── schema.rs             # Diesel generated schema
│   │   ├── embedded_assets.rs    # rust-embed SPA serving
│   │   └── handlers/
│   │       ├── health.rs         # GET /api/health
│   │       ├── upload.rs         # POST /api/upload (multipart)
│   │       ├── solve_ws.rs       # WS /ws/solve/:job_id (progress stream)
│   │       └── websocket.rs      # WS /ws (app heartbeat)
│   ├── diesel.toml
│   └── migrations/
├── Dockerfile                    # Single binary deploy
├── docker-compose.yml            # Postgres + backend + index volume
├── scripts/check-migration-names.sh
└── .github/workflows/
    ├── ci.yml                    # lint, audit, fmt, clippy, test
    └── container.yml             # Docker image -> GHCR
```

## Quick Start

```sh
# Prerequisites
rustup target add wasm32-unknown-unknown
cargo install trunk --locked

# Start Postgres
docker compose up db -d

# Copy env
cp .env.example .env

# Build frontend (must happen before backend due to rust-embed)
cd frontend && trunk build && cd ..

# Run
cargo run -p backend -- --dev-mode
# -> http://localhost:3000
```

## Index Files

The solver requires pre-built `.zdcl` index files from the [zodiacal](https://github.com/meawoppl/zodiacal) crate. Without indexes, uploads will be accepted but solving will always fail.

Place index files in the `indexes/` directory (or set `INDEX_DIR` to a custom path):

```sh
# Local development
mkdir -p indexes
cp /path/to/your/*.zdcl indexes/
cargo run -p backend -- --dev-mode
```

### Docker

The container expects indexes mounted at `/app/indexes`:

```sh
# Using docker run
docker run -v /path/to/indexes:/app/indexes:ro \
  -e DATABASE_URL=postgresql://... \
  -p 3000:3000 \
  ghcr.io/orbitalcommons/zodiacal-web:latest

# Using docker-compose (set INDEX_DIR in .env or environment)
INDEX_DIR=/path/to/indexes docker compose up
```

`docker-compose.yml` bind-mounts `${INDEX_DIR:-./indexes}` into the container as a read-only volume.

## Solve Flow

1. **Upload** — `POST /api/upload` with multipart file, returns `{ job_id, status }`
2. **Stream** — connect to `WS /ws/solve/{job_id}` for typed progress messages:
   - `Accepted` — solve starting
   - `Extracting { n_sources }` — star detection phase
   - `Solving { n_verified }` — candidate verification (updates every ~250ms)
   - `Solved { result }` — RA, Dec, scale, orientation, field size
   - `Failed { reason }` — timeout or no match
