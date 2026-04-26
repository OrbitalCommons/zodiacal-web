mod db;
mod decode;
mod embedded_assets;
mod handlers;
mod models;
mod schema;

use crate::db::DbPool;
use axum::{
    routing::{get, post},
    Router,
};
use clap::Parser;
use std::{collections::HashMap, env, path::PathBuf, sync::Arc};
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use ws_bridge::WsEndpoint;
use zodiacal::index::Index;

#[derive(Parser, Debug, Clone)]
#[command(name = "zodiacal-web")]
#[command(about = "Astronomy plate-solving web service")]
struct Args {
    /// Enable development mode (relaxed config requirements)
    #[arg(long)]
    dev_mode: bool,
}

/// In-memory map of jobs awaiting their solve WebSocket connection.
/// Keyed by job_id, value is (image bytes, optional solver hints).
pub type PendingUploads = Arc<std::sync::Mutex<HashMap<uuid::Uuid, (Vec<u8>, shared::SolveHints)>>>;

#[derive(Clone)]
pub struct AppState {
    pub dev_mode: bool,
    pub db_pool: DbPool,
    pub indexes: Arc<Vec<Index>>,
    pub pending_uploads: PendingUploads,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    if args.dev_mode {
        tracing::warn!("DEV MODE ENABLED");
    }

    // Load .env file if present
    dotenvy::dotenv().ok();

    // Create database pool and run migrations
    let pool = db::create_pool()?;

    tracing::info!("Running database migrations...");
    match db::run_migrations(&pool) {
        Ok(applied) => {
            if applied.is_empty() {
                tracing::info!("Database is up to date");
            } else {
                for m in &applied {
                    tracing::info!("Applied migration: {}", m);
                }
            }
        }
        Err(e) => {
            tracing::error!("Failed to run migrations: {}", e);
            return Err(e);
        }
    }

    // Load plate-solving indexes
    let index_dir = env::var("INDEX_DIR").unwrap_or_else(|_| "indexes".to_string());
    let indexes = load_indexes(&PathBuf::from(&index_dir))?;
    tracing::info!(
        "Loaded {} plate-solving index(es) from {}",
        indexes.len(),
        index_dir
    );

    let app_state = Arc::new(AppState {
        dev_mode: args.dev_mode,
        db_pool: pool,
        indexes: Arc::new(indexes),
        pending_uploads: Arc::new(std::sync::Mutex::new(HashMap::new())),
    });

    // CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Router
    let app = Router::new()
        .route("/api/health", get(handlers::health::health))
        .route("/api/upload", post(handlers::upload::upload))
        .route("/ws/solve/:job_id", get(handlers::solve_ws::handler))
        .with_state(app_state)
        .route(shared::AppSocket::PATH, handlers::websocket::handler())
        .fallback(axum::routing::get(embedded_assets::serve_embedded_frontend))
        .layer(axum::extract::DefaultBodyLimit::max(512 * 1024 * 1024)) // 512 MB
        .layer(cors);

    // Bind and serve
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("{}:{}", host, port);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {}", listener.local_addr()?);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Load all .zdcl index files from a directory.
fn load_indexes(dir: &std::path::Path) -> anyhow::Result<Vec<Index>> {
    let mut indexes = Vec::new();

    if !dir.exists() {
        tracing::warn!("Index directory does not exist: {}", dir.display());
        return Ok(indexes);
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "zdcl") {
            match Index::load(&path) {
                Ok(idx) => {
                    tracing::info!("Loaded index: {}", path.display());
                    indexes.push(idx);
                }
                Err(e) => {
                    tracing::error!("Failed to load index {}: {}", path.display(), e);
                }
            }
        }
    }

    Ok(indexes)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("Received Ctrl+C, shutting down..."),
        _ = terminate => tracing::info!("Received SIGTERM, shutting down..."),
    }
}
