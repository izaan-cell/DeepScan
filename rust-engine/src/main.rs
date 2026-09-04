mod db;
mod models;
mod router;
mod service;

use std::net::SocketAddr;
use std::sync::Arc;
use tonic::transport::Server;
use tracing::info;

pub mod pb {
    tonic::include_proto!("deepscan.v1");
}

use pb::index_service_server::IndexServiceServer;
use pb::search_service_server::SearchServiceServer;

/// Shared engine state: loaded ONNX sessions + the open LanceDB connection.
/// Cloned cheaply (Arc) into every gRPC service handler.
pub struct EngineState {
    pub models: models::ModelBundle,
    pub db: db::VectorStore,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let data_dir = dirs_next()?;
    std::fs::create_dir_all(&data_dir)?;

    info!("loading ONNX models (CLIP, MiniLM, Jina-Code)...");
    let models = models::ModelBundle::load(&data_dir)?;

    info!("opening LanceDB at {:?}", data_dir.join("lancedb"));
    let db = db::VectorStore::open(&data_dir.join("lancedb")).await?;

    let state = Arc::new(EngineState { models, db });

    // Bind an ephemeral loopback port and publish it so the Go daemon / Java
    // parser bridge can discover this engine — see docs/ARCHITECTURE.md #4.
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let addr: SocketAddr = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    write_lockfile(&data_dir, addr.port())?;

    info!("DeepScan engine listening on {addr}");

    Server::builder()
        .add_service(SearchServiceServer::new(service::SearchSvc::new(state.clone())))
        .add_service(IndexServiceServer::new(service::IndexSvc::new(state.clone())))
        .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(
            tokio::net::TcpListener::from_std(listener)?,
        ))
        .await?;

    Ok(())
}

fn dirs_next() -> anyhow::Result<std::path::PathBuf> {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))?;
    Ok(std::path::PathBuf::from(home).join(".deepscan"))
}

fn write_lockfile(data_dir: &std::path::Path, port: u16) -> anyhow::Result<()> {
    let lock = serde_json::json!({ "port": port, "pid": std::process::id() });
    std::fs::write(data_dir.join("engine.lock"), lock.to_string())?;
    Ok(())
}
