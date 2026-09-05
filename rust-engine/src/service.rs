//! Core query/index logic, shared by the gRPC service impls (for the Go
//! daemon and any future native clients) and the JSON-HTTP web service (for
//! the browser-based frontend). Both are thin adapters over `Core`.

use crate::db::FileRow;
use crate::pb::index_service_server::IndexService;
use crate::pb::search_service_server::SearchService;
use crate::pb::*;
use crate::router::{self, ExtractionPlan};
use crate::EngineState;
use futures::{Stream, StreamExt};
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

/// Common shape for both of IndexService's streaming responses.
type ResponseStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

/// App bundles (.app) and Xcode project/workspace packages are
/// directories too, and their insides are exclusively build output /
/// executables / plist metadata — never something the user "input", so
/// these are always excluded regardless of what's inside them.
///
/// This used to also exclude any directory containing a `.git`,
/// `package.json`, `Cargo.toml`, etc. (anything that looked like "a
/// software project") to keep DeepScan's own repo from polluting search
/// with its own source files. That was too broad: it excluded *every*
/// git-tracked project wholesale, including the user's own real code
/// (their own repos on Desktop), which defeats code search entirely — the
/// actual problem was one specific repo (this one), not "any repo
/// anywhere". The skipDirNames list below (node_modules, target, dist,
/// build, vendor, venv, __pycache__, DerivedData) already excludes the
/// actual build/dependency noise regardless of which project it's in.
/// Mirrors go-daemon/watcher.go's skipDirNames — `.gitignore`-respect
/// alone only excludes these if a given project actually lists them,
/// which isn't guaranteed (no `.gitignore` at all, or an incomplete one).
const SKIP_DIR_NAMES: &[&str] =
    &["node_modules", "target", "dist", "build", "vendor", "venv", "__pycache__", "DerivedData"];

/// Directory *extensions* (not names) that mark a macOS package/bundle —
/// a folder the Finder shows and treats as a single opaque file, but that
/// a plain filesystem walk happily descends into. `Photos Library
/// .photoslibrary` is the motivating case: it's a bundle containing
/// thousands of internal cache/thumbnail/database files, none of which
/// are "a file the user input" — walking into it both pollutes search
/// with cache internals and is genuinely slow (thousands of individual
/// files to categorize and embed one at a time).
const BUNDLE_EXTENSIONS: &[&str] =
    &[".app", ".xcodeproj", ".xcworkspace", ".photoslibrary", ".pages", ".key", ".numbers", ".rtfd"];

fn is_software_project_dir(dir: &Path) -> bool {
    let Some(name) = dir.file_name().map(|n| n.to_string_lossy()) else { return false };
    BUNDLE_EXTENSIONS.iter().any(|ext| name.ends_with(ext)) || SKIP_DIR_NAMES.contains(&name.as_ref())
}

pub struct ScoredFile {
    pub path: String,
    pub category: String,
    pub snippet: Option<String>,
    pub score: f32,
}

/// Platform-agnostic core: no gRPC/HTTP types in here, so it's trivially
/// reusable from both transports. Clone is cheap (just an Arc bump) — used
/// to hand a copy into spawned tasks for streaming RPC handlers.
#[derive(Clone)]
pub struct Core {
    state: Arc<EngineState>,
}

impl Core {
    pub fn new(state: Arc<EngineState>) -> Self {
        Self { state }
    }

    /// A plain text query searches both the document column (MiniLM) and
    /// the code column (Jina-Code) — matching the "one search bar across
    /// every file type" design — plus a plain substring match on filename
    /// and content (db::search_literal), then merges by score. The two
    /// embedding models produce differently-scaled distances, so semantic
    /// ranking between them is a reasonable approximation, not a
    /// calibrated joint score — but a literal match is unambiguous, so
    /// those always outrank a semantic-only hit regardless of its score.
    /// This is also the only way an image ever matches a typed query at
    /// all: images have no text/code vector, only clip_vector, so without
    /// the filename half of this, typing an image's name found nothing.
    pub async fn search_text(&self, query: &str, top_k: usize) -> anyhow::Result<Vec<ScoredFile>> {
        let (text_vector, code_vector) = {
            let mut models = self.state.models.lock().await;
            (models.embed_text(query)?, models.embed_code(query)?)
        };
        let (text_rows, code_rows, literal_rows) = tokio::try_join!(
            self.state.db.search_text(text_vector, top_k),
            self.state.db.search_code(code_vector, top_k),
            self.state.db.search_literal(query, top_k),
        )?;

        let mut by_path: std::collections::HashMap<String, ScoredFile> = std::collections::HashMap::new();
        for row in text_rows.into_iter().chain(code_rows) {
            let file = to_scored_file(row);
            by_path.entry(file.path.clone()).and_modify(|e| e.score = e.score.max(file.score)).or_insert(file);
        }
        // Literal matches always win a tie against a semantic score, since
        // 1.0 is already the ceiling of the KNN scoring in db.rs — insert
        // last so it overwrites rather than being maxed against.
        for row in literal_rows {
            by_path.insert(row.path.clone(), to_scored_file(row));
        }

        let mut combined: Vec<ScoredFile> = by_path.into_values().collect();
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
                // Long enough that a literal substring search (db::search_literal)
                // actually has real content to match against — 280 chars was only
                // ever meant as a UI preview length, but a "find this exact phrase"
                // search needs far more of the file than its first few lines. The
                // frontend still visually clamps its own display to a few lines
                // (see .result-snippet's -webkit-line-clamp), so this doesn't
                // change what's shown, only what's searchable.
                let snippet: String = text.chars().take(8000).collect();
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
                // Long enough that a literal substring search (db::search_literal)
                // actually has real content to match against — 280 chars was only
                // ever meant as a UI preview length, but a "find this exact phrase"
                // search needs far more of the file than its first few lines. The
                // frontend still visually clamps its own display to a few lines
                // (see .result-snippet's -webkit-line-clamp), so this doesn't
                // change what's shown, only what's searchable.
                let snippet: String = text.chars().take(8000).collect();
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

    /// Reads a file's bytes for the result-card preview (thumbnail image,
    /// or the same snippet text search already surfaces) — but only for a
    /// path DeepScan itself indexed. See db::VectorStore::is_indexed_path
    /// for why: this is the one HTTP endpoint that returns raw file
    /// contents, so it must never become "read any path a caller names".
    pub async fn read_indexed_file(&self, path: &str) -> anyhow::Result<Option<Vec<u8>>> {
        if self.state.db.is_indexed_path(path).await?.is_none() {
            return Ok(None);
        }
        Ok(Some(tokio::fs::read(path).await?))
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
    type WatchEventsStream = ResponseStream<IndexAck>;

    /// The Go daemon's long-lived side of this call streams one FsEvent per
    /// filesystem change; each gets embedded + upserted (or deleted) here
    /// and acked back. Runs in a spawned task so the stream can be returned
    /// immediately, per tonic's bidi-streaming pattern.
    async fn watch_events(
        &self,
        request: Request<Streaming<FsEvent>>,
    ) -> Result<Response<Self::WatchEventsStream>, Status> {
        let mut in_stream = request.into_inner();
        let core = self.core.clone();
        let (tx, rx) = mpsc::channel(32);

        tokio::spawn(async move {
            while let Some(event) = in_stream.next().await {
                let event = match event {
                    Ok(e) => e,
                    Err(e) => {
                        let _ = tx.send(Err(Status::internal(e.to_string()))).await;
                        break;
                    }
                };
                let Some(file) = event.file.clone() else { continue };
                let path = std::path::PathBuf::from(&file.path);

                let result = match event.kind() {
                    ChangeKind::Deleted => core.delete_file(&file.path).await,
                    ChangeKind::Renamed => {
                        if !event.renamed_from.is_empty() {
                            let _ = core.delete_file(&event.renamed_from).await;
                        }
                        core.index_file(&path).await
                    }
                    ChangeKind::Created | ChangeKind::Modified => core.index_file(&path).await,
                    ChangeKind::Unspecified => Ok(()),
                };

                let ack = match result {
                    Ok(()) => IndexAck { path: file.path, accepted: true, error: String::new() },
                    Err(e) => IndexAck { path: file.path, accepted: false, error: e.to_string() },
                };
                if tx.send(Ok(ack)).await.is_err() {
                    break; // daemon disconnected
                }
            }
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    type IndexPathStream = ResponseStream<IndexProgress>;

    /// One-shot recursive scan of a directory the user just added (or the
    /// daemon's initial startup scan) — respects .gitignore-style rules via
    /// the `ignore` crate, same as router.rs's traversal design intent.
    async fn index_path(
        &self,
        request: Request<IndexPathRequest>,
    ) -> Result<Response<Self::IndexPathStream>, Status> {
        let req = request.into_inner();
        let core = self.core.clone();
        let (tx, rx) = mpsc::channel(32);

        tokio::spawn(async move {
            let mut walk = ignore::WalkBuilder::new(&req.root_path);
            if !req.recursive {
                walk.max_depth(Some(1));
            }
            // Never prune the root itself (depth 0) — a user who explicitly
            // points DeepScan at a code project still gets it indexed;
            // this only excludes project directories *encountered while
            // walking* a broader root like Desktop or Downloads.
            walk.filter_entry(|entry| {
                entry.depth() == 0
                    || !entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
                    || !is_software_project_dir(entry.path())
            });

            let mut files_scanned = 0i64;
            let mut files_indexed = 0i64;
            let mut files_skipped = 0i64;

            for entry in walk.build() {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    continue;
                }

                files_scanned += 1;
                let path = entry.path().to_path_buf();
                match core.index_file(&path).await {
                    Ok(()) => files_indexed += 1,
                    Err(_) => files_skipped += 1,
                }

                let progress = IndexProgress {
                    files_scanned,
                    files_indexed,
                    files_skipped,
                    current_path: path.to_string_lossy().into_owned(),
                    done: false,
                };
                if tx.send(Ok(progress)).await.is_err() {
                    return;
                }
            }

            let _ = tx
                .send(Ok(IndexProgress {
                    files_scanned,
                    files_indexed,
                    files_skipped,
                    current_path: String::new(),
                    done: true,
                }))
                .await;
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
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
