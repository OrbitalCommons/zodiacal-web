# Zodiacal — Plate Solver

Web service for plate-solving arbitrary uploaded astronomy images. Inspired by [astrometry.net](https://astrometry.net).

Upload an image of the night sky and get back the sky coordinates (RA/Dec), orientation, pixel scale, and field of view.

## Architecture

```
Cargo.toml                        # Workspace root (backend, frontend, shared)
├── shared/src/lib.rs             # Serde types used by both sides
├── frontend/
│   ├── Trunk.toml                # WASM bundler config
│   ├── index.html                # Trunk entry point
│   └── src/main.rs               # Yew App with routing
├── backend/
│   ├── src/
│   │   ├── main.rs               # Axum server, routes, shutdown
│   │   ├── db.rs                 # Diesel pool + embedded migrations
│   │   ├── models.rs             # Queryable/Insertable structs
│   │   ├── schema.rs             # Diesel generated schema
│   │   ├── embedded_assets.rs    # rust-embed SPA serving
│   │   └── handlers/
│   │       ├── health.rs         # GET /api/health
│   │       └── websocket.rs      # ws-bridge typed WebSocket
│   ├── diesel.toml
│   └── migrations/
├── Dockerfile                    # Single binary deploy
├── docker-compose.yml            # Postgres + backend
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
