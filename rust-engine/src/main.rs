mod config;
mod db;
mod http;
mod models;
mod oshooks;
mod parser_client;
mod router;
mod service;

use config::Mode;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::transport::Server;
use tracing::info;

pub mod pb {
    tonic::include_proto!("deepscan.v1");
}

use pb::index_service_server::IndexServiceServer;
use pb::search_service_server::SearchServiceServer;

/// Shared engine state: loaded ONNX sessions + the open LanceDB connection.
/// `models` is behind a Mutex because `ort::Session::run` takes `&mut self`;
/// the engine handles one embedding call at a time, which is fine at local
/// file-indexing/query volumes (see docs/ARCHITECTURE.md). Only ever
/// constructed in Mode::Local — see `main` below.
pub struct EngineState {
    pub models: Mutex<models::ModelBundle>,
    pub db: db::VectorStore,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = config::Config::load();
    std::env::set_var("RUST_LOG", &cfg.log_level);
    tracing_subscriber::fmt::init();

    let http_addr: SocketAddr = (cfg.http_bind_host, cfg.http_port).into();

    match cfg.mode {
        Mode::Cloud => {
            // Render deployment: no filesystem to index, so no models, no
            // LanceDB, no gRPC server — just the static SPA + a status
            // endpoint that tells visitors to run the engine locally. See
            // docs/ARCHITECTURE.md "Deploying the UI shell to Render".
            info!("DeepScan engine starting in CLOUD mode (UI shell only) on {http_addr}");
            http::serve(None, cfg.mode, http_addr).await?;
        }
        Mode::Local => {
            std::fs::create_dir_all(&cfg.data_dir)?;

            // Defense in depth alongside packaging/macos/launcher.sh's own
            // pre-launch delete: any caller that waits on engine.lock's
            // existence (the Go daemon's start sequence, a dev script) must
            // never observe a lock file left over from a previous process,
            // whose port is almost certainly dead by now.
            let _ = std::fs::remove_file(cfg.data_dir.join("engine.lock"));

            info!("loading ONNX models (CLIP, MiniLM, Jina-Code) from {:?}", cfg.model_dir);
            let models = models::ModelBundle::load(&cfg.model_dir)?;

            let db_path = cfg.data_dir.join("lancedb");
            info!("opening LanceDB at {:?}", db_path);
            let db = db::VectorStore::open(&db_path).await?;

            let state = Arc::new(EngineState { models: Mutex::new(models), db });

            // Bind the gRPC listener — a fixed port in dev
            // (DEEPSCAN_ENGINE_GRPC_PORT), ephemeral in production,
            // published via engine.lock either way so the Go daemon /
            // Java parser bridge can discover this engine.
            let grpc_port = cfg.engine_grpc_port.unwrap_or(0);
            let listener = std::net::TcpListener::bind(("127.0.0.1", grpc_port))?;
            let grpc_addr: SocketAddr = listener.local_addr()?;
            listener.set_nonblocking(true)?;
            write_lockfile(&cfg.data_dir, grpc_addr.port())?;

            info!("DeepScan engine gRPC listening on {grpc_addr}");
            info!("DeepScan engine HTTP service listening on {http_addr}");

            let grpc_server = Server::builder()
                .add_service(SearchServiceServer::new(service::SearchSvc::new(state.clone())))
                .add_service(IndexServiceServer::new(service::IndexSvc::new(state.clone())))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(
                    tokio::net::TcpListener::from_std(listener)?,
                ));

            let http_server = http::serve(Some(state), cfg.mode, http_addr);

            tokio::select! {
                res = grpc_server => res?,
                res = http_server => res?,
            }
        }
    }

    Ok(())
}

fn write_lockfile(data_dir: &std::path::Path, port: u16) -> anyhow::Result<()> {
    let lock = serde_json::json!({ "port": port, "pid": std::process::id() });
    std::fs::write(data_dir.join("engine.lock"), lock.to_string())?;
    Ok(())
}
