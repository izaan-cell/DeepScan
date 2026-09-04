//! JSON web service + static frontend hosting. Runs in two modes (see
//! config::Mode):
//!   - Local: full search/status/reveal backed by `service::Core`.
//!   - Cloud (Render): serves the static SPA only; `/api/*` returns a
//!     "install the local engine" response, since a Render box has no
//!     access to the visitor's files. The deployed page's own JS then
//!     talks to a locally-installed engine at 127.0.0.1 instead — see
//!     docs/ARCHITECTURE.md "Deploying the UI shell to Render".

use crate::config::Mode;
use crate::service::Core;
use crate::EngineState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

#[derive(Clone)]
struct AppState {
    core: Option<Arc<Core>>,
}

pub async fn serve(engine_state: Option<Arc<EngineState>>, mode: Mode, addr: SocketAddr) -> anyhow::Result<()> {
    let state = AppState { core: engine_state.map(|s| Arc::new(Core::new(s))) };

    let app = Router::new()
        .route("/api/status", get(status))
        .route("/api/search", post(search))
        .route("/api/reveal", post(reveal))
        .fallback_service(ServeDir::new(frontend_dir()))
        .layer(CorsLayer::permissive()) // loopback-only in Local mode; the Render-hosted
        // shell in Cloud mode legitimately needs to be fetched from a different origin
        // than the local engine it talks to.
        .with_state(state);

    let _ = mode;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn frontend_dir() -> PathBuf {
    let candidate = PathBuf::from("frontend");
    if candidate.exists() {
        return candidate;
    }
    PathBuf::from("../frontend")
}

#[derive(Serialize)]
struct StatusResponse {
    db_healthy: bool,
    total_indexed_files: i64,
    is_scanning: bool,
    mode: &'static str,
    message: Option<&'static str>,
}

async fn status(State(state): State<AppState>) -> impl IntoResponse {
    match &state.core {
        Some(core) => match core.index_status().await {
            Ok((db_healthy, total)) => Json(StatusResponse {
                db_healthy,
                total_indexed_files: total,
                is_scanning: false,
                mode: "local",
                message: None,
            })
            .into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        None => Json(StatusResponse {
            db_healthy: false,
            total_indexed_files: 0,
            is_scanning: false,
            mode: "cloud",
            message: Some(
                "This is the DeepScan UI shell. Install the local engine on your own \
                 machine to index and search your files — see the README.",
            ),
        })
        .into_response(),
    }
}

#[derive(Deserialize)]
struct SearchRequestBody {
    text_query: Option<String>,
    image_query_bytes: Option<Vec<u8>>,
    #[serde(default)]
    #[allow(dead_code)] // scope filtering happens client-side today; kept for API stability
    scope: Vec<String>,
}

#[derive(Serialize)]
struct SearchResultDto {
    path: String,
    category: String,
    snippet: Option<String>,
    score: f32,
}

#[derive(Serialize)]
struct SearchResponseDto {
    results: Vec<SearchResultDto>,
}

async fn search(State(state): State<AppState>, Json(body): Json<SearchRequestBody>) -> impl IntoResponse {
    let Some(core) = state.core.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "This DeepScan deployment has no local engine attached — \
                          search only works against files on the machine actually running the engine."
            })),
        )
            .into_response();
    };

    let result = if let Some(text) = body.text_query.filter(|t| !t.is_empty()) {
        core.search_text(&text, 20).await
    } else if let Some(bytes) = body.image_query_bytes.filter(|b| !b.is_empty()) {
        core.search_image(&bytes, 20).await
    } else {
        return (StatusCode::BAD_REQUEST, "provide text_query or image_query_bytes").into_response();
    };

    match result {
        Ok(results) => Json(SearchResponseDto {
            results: results
                .into_iter()
                .map(|r| SearchResultDto { path: r.path, category: r.category, snippet: r.snippet, score: r.score })
                .collect(),
        })
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct RevealBody {
    path: String,
}

async fn reveal(State(state): State<AppState>, Json(body): Json<RevealBody>) -> impl IntoResponse {
    if state.core.is_none() {
        return (StatusCode::SERVICE_UNAVAILABLE, "no local engine attached").into_response();
    }
    match crate::oshooks::reveal(&body.path) {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
