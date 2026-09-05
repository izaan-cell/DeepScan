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
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
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

    // Permissive CORS is only actually needed in Cloud mode, where the
    // Render-hosted UI shell (a different origin) fetches this engine. In
    // Local mode the frontend is served from this same origin, and one of
    // these routes (/api/thumbnail) returns raw file bytes — permissive
    // CORS there would let *any* webpage the user has open probe/read
    // files DeepScan has indexed just by the user having the app running.
    let cors = match mode {
        Mode::Cloud => CorsLayer::permissive(),
        Mode::Local => CorsLayer::new(),
    };

    let app = Router::new()
        .route("/api/status", get(status))
        .route("/api/search", post(search))
        .route("/api/reveal", post(reveal))
        .route("/api/thumbnail", get(thumbnail))
        .fallback_service(ServeDir::new(frontend_dir()))
        .layer(cors)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn frontend_dir() -> PathBuf {
    // Packaged app: the binary lives at Contents/Resources/bin/deepscan-engine
    // (macOS) or stage/bin/deepscan-engine.exe (Windows), with frontend/ as a
    // sibling of bin/ either way — resolve relative to the executable's own
    // location, not the current working directory. A GUI-launched app's CWD
    // is unrelated to its bundle location (typically "/" or the user's home
    // dir on macOS), so a CWD-relative path only ever worked under `cargo
    // run`, never for an actually-launched packaged app.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin_dir) = exe.parent() {
            let bundled = bin_dir.join("../frontend");
            if bundled.exists() {
                return bundled;
            }
        }
    }

    // Dev fallback: relative to CWD (cargo run from the repo root or from
    // inside rust-engine/).
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
    // The frontend sends a single scope string (e.g. "all", "code"), not
    // an array — this was previously typed as Vec<String>, which made
    // every real search from the actual UI fail JSON deserialization
    // (422 Unprocessable Entity) despite curl tests without a `scope`
    // field always succeeding and masking the bug.
    #[serde(default)]
    #[allow(dead_code)] // scope filtering happens client-side today; kept for API stability
    scope: Option<String>,
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

#[derive(Deserialize)]
struct ThumbnailQuery {
    path: String,
}

// A result card's image preview needs actual bytes, not just metadata —
// this is the one endpoint that serves raw file contents, so it's the one
// endpoint that matters most for CORS/path-scoping (see the `cors` setup
// in `serve` and `Core::read_indexed_file`'s own doc comment).
const MAX_PREVIEW_BYTES: u64 = 25 * 1024 * 1024;

async fn thumbnail(State(state): State<AppState>, Query(q): Query<ThumbnailQuery>) -> impl IntoResponse {
    let Some(core) = state.core.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no local engine attached").into_response();
    };

    match tokio::fs::metadata(&q.path).await {
        Ok(meta) if meta.len() > MAX_PREVIEW_BYTES => {
            return (StatusCode::PAYLOAD_TOO_LARGE, "file too large to preview").into_response();
        }
        _ => {}
    }

    match core.read_indexed_file(&q.path).await {
        Ok(Some(bytes)) => ([(header::CONTENT_TYPE, mime_for_path(&q.path))], bytes).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "not an indexed file").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn mime_for_path(path: &str) -> &'static str {
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        "heic" | "heif" => "image/heic",
        _ => "application/octet-stream",
    }
}
