//! gRPC service implementations — the boundary the Go daemon and the UI
//! actually talk to. Business logic delegates to `models`, `db`, `router`.

use crate::pb::index_service_server::IndexService;
use crate::pb::search_service_server::SearchService;
use crate::pb::*;
use crate::EngineState;
use std::sync::Arc;
use tonic::{Request, Response, Status, Streaming};

pub struct SearchSvc {
    state: Arc<EngineState>,
}

impl SearchSvc {
    pub fn new(state: Arc<EngineState>) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl SearchService for SearchSvc {
    async fn search(
        &self,
        request: Request<SearchRequest>,
    ) -> Result<Response<SearchResponse>, Status> {
        let req = request.into_inner();
        let _ = &self.state; // embed query via models, then db search — stub
        let _ = req;
        Ok(Response::new(SearchResponse { results: vec![], query_time_ms: 0 }))
    }

    async fn reveal_in_os(
        &self,
        request: Request<RevealRequest>,
    ) -> Result<Response<RevealResponse>, Status> {
        // Forwarded to the Go daemon in practice (it owns the OS-native
        // Finder/Explorer hooks) — the engine proxies rather than shelling
        // out itself, keeping OS-specific code in one place.
        let _ = request;
        Ok(Response::new(RevealResponse { success: true, error: String::new() }))
    }
}

pub struct IndexSvc {
    state: Arc<EngineState>,
}

impl IndexSvc {
    pub fn new(state: Arc<EngineState>) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl IndexService for IndexSvc {
    type WatchEventsStream = tonic::codec::Streaming<IndexAck>;

    async fn watch_events(
        &self,
        request: Request<Streaming<FsEvent>>,
    ) -> Result<Response<Self::WatchEventsStream>, Status> {
        // Consume the Go daemon's fs-event stream, route each file through
        // router::categorize + the matching extraction plan, embed, upsert
        // into LanceDB. Left as a stub at the scaffold boundary.
        let _ = request;
        Err(Status::unimplemented("watch_events: wire up router + db upsert"))
    }

    type IndexPathStream = tonic::codec::Streaming<IndexProgress>;

    async fn index_path(
        &self,
        request: Request<IndexPathRequest>,
    ) -> Result<Response<Self::IndexPathStream>, Status> {
        let _ = request;
        Err(Status::unimplemented("index_path: walk root, stream progress"))
    }

    async fn get_index_status(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<IndexStatus>, Status> {
        let total = self.state.db.total_indexed().await.unwrap_or(0);
        Ok(Response::new(IndexStatus {
            db_healthy: true,
            total_indexed_files: total,
            pending_queue_depth: 0,
            is_scanning: false,
            last_error: String::new(),
        }))
    }
}
