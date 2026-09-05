//! Core query/index logic, shared by the gRPC service impls (for the Go
//! daemon and any future native clients) and the JSON-HTTP web service (for
//! the browser-based frontend). Both are thin adapters over `Core`.

use crate::db::FileRow;
use crate::pb::index_service_server::IndexService;
use crate::pb::search_service_server::SearchService;
use crate::pb::*;
use crate::router::{self, ExtractionPlan};
use crate::EngineState;
use std::path::Path;
use std::sync::Arc;
use tonic::{Request, Response, Status, Streaming};

pub struct ScoredFile {
    pub path: String,
    pub category: String,
    pub snippet: Option<String>,
    pub score: f32,
}

/// Platform-agnostic core: no gRPC/HTTP types in here, so it's trivially
/// reusable from both transports.
pub struct Core {
    state: Arc<EngineState>,
}

impl Core {
    pub fn new(state: Arc<EngineState>) -> Self {
        Self { state }
    }

    /// A plain text query searches both the document column (MiniLM) and
    /// the code column (Jina-Code) — matching the "one search bar across
    /// every file type" design — then merges by score. The two models
    /// produce differently-scaled distances, so this is a reasonable
    /// approximation, not a calibrated joint ranking.
    pub async fn search_text(&self, query: &str, top_k: usize) -> anyhow::Result<Vec<ScoredFile>> {
        let (text_vector, code_vector) = {
            let mut models = self.state.models.lock().await;
            (models.embed_text(query)?, models.embed_code(query)?)
        };
        let (text_rows, code_rows) = tokio::try_join!(
            self.state.db.search_text(text_vector, top_k),
            self.state.db.search_code(code_vector, top_k),
        )?;

        let mut combined: Vec<ScoredFile> =
            text_rows.into_iter().chain(code_rows).map(to_scored_file).collect();
        combined.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        combined.truncate(top_k);
        Ok(combined)
    }

    pub async fn search_image(&self, bytes: &[u8], top_k: usize) -> anyhow::Result<Vec<ScoredFile>> {
        let img = image::load_from_memory(bytes)?;
        let vector = {
            let mut models = self.state.models.lock().await;
            models.embed_image(&img)?
        };
        let rows = self.state.db.search_clip(vector, top_k).await?;
        Ok(rows.into_iter().map(to_scored_file).collect())
    }

    pub async fn index_status(&self) -> anyhow::Result<(bool, i64)> {
        let total = self.state.db.total_indexed().await?;
        Ok((true, total))
    }

    /// Indexes (or re-indexes) a single file: categorize, extract, embed,
    /// upsert. Called both from the Go daemon's fs-event stream and from a
    /// one-shot `index_path` directory walk.
    pub async fn index_file(&self, path: &Path) -> anyhow::Result<()> {
        let category = router::categorize(path);
        if category == FileCategory::Unspecified {
            return Ok(());
        }

        self.state.db.delete_path(&path.to_string_lossy()).await?;

        let modified_unix_ms = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let row = match router::plan_for(category) {
            ExtractionPlan::DirectClip => {
                let img = image::open(path)?;
                let vector = {
                    let mut models = self.state.models.lock().await;
                    models.embed_image(&img)?
                };
                FileRow {
                    path: path.to_string_lossy().into_owned(),
                    category: category_label(category).into(),
                    modified_unix_ms,
                    snippet: None,
                    clip_vector: Some(vector),
                    text_vector: None,
                    code_vector: None,
                }
            }
            ExtractionPlan::DirectCode => {
                let text = std::fs::read_to_string(path)?;
                let snippet: String = text.chars().take(280).collect();
                let vector = {
                    let mut models = self.state.models.lock().await;
                    models.embed_code(&text)?
                };
                FileRow {
                    path: path.to_string_lossy().into_owned(),
                    category: category_label(category).into(),
                    modified_unix_ms,
                    snippet: Some(snippet),
                    clip_vector: None,
                    text_vector: None,
                    code_vector: Some(vector),
                }
            }
            ExtractionPlan::ViaTikaThenMiniLm => {
                let text = crate::parser_client::extract_document(&path.to_string_lossy()).await?;
                let snippet: String = text.chars().take(280).collect();
                let vector = {
                    let mut models = self.state.models.lock().await;
                    models.embed_text(&text)?
                };
                FileRow {
                    path: path.to_string_lossy().into_owned(),
                    category: category_label(category).into(),
                    modified_unix_ms,
                    snippet: Some(snippet),
                    clip_vector: None,
                    text_vector: Some(vector),
                    code_vector: None,
                }
            }
            ExtractionPlan::AudioVideoPipeline => {
                // whisper-rs transcription + ffmpeg-next keyframe sampling —
                // left unimplemented at the scaffold boundary; see the
                // pipeline table in docs/ARCHITECTURE.md. Skip rather than
                // fail the whole indexing run.
                return Ok(());
            }
        };

        self.state.db.insert_rows(vec![row]).await
    }

    pub async fn delete_file(&self, path: &str) -> anyhow::Result<()> {
        self.state.db.delete_path(path).await
    }
}

fn to_scored_file(row: crate::db::ScoredRow) -> ScoredFile {
    ScoredFile { path: row.path, category: row.category, snippet: row.snippet, score: row.score }
}

fn category_label(c: FileCategory) -> &'static str {
    match c {
        FileCategory::Image => "image",
        FileCategory::Code => "code",
        FileCategory::Document => "document",
        FileCategory::Audio => "audio",
        FileCategory::Video => "video",
        FileCategory::Unspecified => "unspecified",
    }
}

// ---------------------------------------------------------------------
// gRPC adapters
// ---------------------------------------------------------------------

pub struct SearchSvc {
    core: Core,
}

impl SearchSvc {
    pub fn new(state: Arc<EngineState>) -> Self {
        Self { core: Core::new(state) }
    }
}

#[tonic::async_trait]
impl SearchService for SearchSvc {
    async fn search(&self, request: Request<SearchRequest>) -> Result<Response<SearchResponse>, Status> {
        let req = request.into_inner();
        let top_k = if req.top_k > 0 { req.top_k as usize } else { 20 };
        let started = std::time::Instant::now();

        let results = match req.query {
            Some(search_request::Query::TextQuery(text)) => self.core.search_text(&text, top_k).await,
            Some(search_request::Query::ImageQueryBytes(bytes)) => {
                self.core.search_image(&bytes, top_k).await
            }
            None => Ok(vec![]),
        }
        .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(SearchResponse {
            results: results
                .into_iter()
                .map(|r| SearchResult {
                    file: Some(FileRef { path: r.path, size_bytes: 0, modified_unix_ms: 0, category: 0 }),
                    score: r.score,
                    snippet: r.snippet.unwrap_or_default(),
                })
                .collect(),
            query_time_ms: started.elapsed().as_millis() as i64,
        }))
    }

    async fn reveal_in_os(&self, request: Request<RevealRequest>) -> Result<Response<RevealResponse>, Status> {
        match crate::oshooks::reveal(&request.into_inner().path) {
            Ok(()) => Ok(Response::new(RevealResponse { success: true, error: String::new() })),
            Err(e) => Ok(Response::new(RevealResponse { success: false, error: e.to_string() })),
        }
    }
}

pub struct IndexSvc {
    core: Core,
}

impl IndexSvc {
    pub fn new(state: Arc<EngineState>) -> Self {
        Self { core: Core::new(state) }
    }
}

#[tonic::async_trait]
impl IndexService for IndexSvc {
    type WatchEventsStream = tonic::codec::Streaming<IndexAck>;

    async fn watch_events(
        &self,
        _request: Request<Streaming<FsEvent>>,
    ) -> Result<Response<Self::WatchEventsStream>, Status> {
        // The bidirectional-stream signature this generates expects a
        // Streaming<IndexAck> response type from tonic's server codegen,
        // which in practice is produced by tonic::codec::Streaming wrapping
        // an internal channel — wiring that plumbing (spawn a task reading
        // `_request.into_inner()`, calling `self.core.index_file` /
        // `delete_file` per event, replying with IndexAck on an mpsc
        // channel converted via ReceiverStream) is the one piece left as a
        // TODO at this scaffold boundary; `index_path` below shows the same
        // per-file logic in the simpler unary-request/streaming-response
        // shape.
        Err(Status::unimplemented(
            "watch_events: see comment — index_file/delete_file are ready, only the stream plumbing is left",
        ))
    }

    type IndexPathStream = tonic::codec::Streaming<IndexProgress>;

    async fn index_path(
        &self,
        request: Request<IndexPathRequest>,
    ) -> Result<Response<Self::IndexPathStream>, Status> {
        let _ = request;
        Err(Status::unimplemented("index_path: see http::index_path_http for the working equivalent"))
    }

    async fn get_index_status(&self, _request: Request<Empty>) -> Result<Response<IndexStatus>, Status> {
        let (db_healthy, total) = self.core.index_status().await.map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(IndexStatus {
            db_healthy,
            total_indexed_files: total,
            pending_queue_depth: 0,
            is_scanning: false,
            last_error: String::new(),
        }))
    }
}
