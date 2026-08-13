# CLAUDE.md

## Build & Test Commands

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

# Run backend
cargo run -p backend -- --dev-mode

# Tests
cargo test --workspace

# Formatting
cargo fmt --all --check

# Clippy
cargo clippy --workspace --all-targets

# Docs
cargo doc --no-deps
```

## CI Pre-Push Checklist

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets`
3. `cargo test --workspace`

## Workspace Structure

- `shared/` — Serde types and WebSocket endpoint definition shared by backend and frontend
- `backend/` — Axum server with Diesel/Postgres, embedded frontend assets, WebSocket handler
- `frontend/` — Yew WASM SPA with client-side routing and ws-bridge WebSocket client

## Code Style Guidelines

### Imports
1. std library first
2. External crates (alphabetized)
3. Local modules (alphabetized)
4. NO wildcard imports

### Naming
- Functions/variables: `snake_case`
- Types/traits: `CamelCase`
- Constants: `SCREAMING_SNAKE_CASE`

### Error Handling
- Use `thiserror` for custom error types
- Use `anyhow` for application-level errors
- Use `Result` with `?` operator

### Testing
- NEVER special-case testing in production algorithms
- Write proper unit tests in `#[cfg(test)]` modules
- NO doctests
- Every shared type gets a roundtrip serialization test
- NEVER assert timing/speed in unit tests

## Git Commits

- NO `git add -A`
- NO force push
- NO amend
- NO `--no-verify`
- Merge over rebase
- 10-word max subject line
- Body with bullets

## Migration Naming

- Initial: `00000000000000_description`
- Subsequent: `YYYY-MM-DD-HHMMSS_description` (snake_case)
- Enforced by `scripts/check-migration-names.sh` and CI
