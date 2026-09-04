//! gRPC client to the Java Tika bridge (ParserBridgeService). Connects
//! lazily on first use, reading the port from `parser.lock` the same way
//! go-daemon/lockfile.go reads `engine.lock`.

use crate::pb::parser_bridge_service_client::ParserBridgeServiceClient;
use crate::pb::ExtractRequest;
use anyhow::{Context, Result};
use tonic::transport::Channel;

pub async fn extract_document(path: &str) -> Result<String> {
    let mut client = connect().await?;
    let resp = client
        .extract_document(ExtractRequest { path: path.to_string(), mime_hint: String::new() })
        .await?
        .into_inner();
    Ok(resp.extracted_text)
}

async fn connect() -> Result<ParserBridgeServiceClient<Channel>> {
    let port = read_parser_lock()?;
    let endpoint = format!("http://127.0.0.1:{port}");
    ParserBridgeServiceClient::connect(endpoint)
        .await
        .context("failed to connect to the Java Tika parser bridge — is it running? (java-parser/)")
}

fn read_parser_lock() -> Result<u16> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let raw = std::fs::read_to_string(std::path::Path::new(&home).join(".deepscan/parser.lock"))
        .context("parser.lock not found — start the Java Tika bridge first")?;
    let json: serde_json::Value = serde_json::from_str(&raw)?;
    json["port"]
        .as_u64()
        .map(|p| p as u16)
        .context("parser.lock missing 'port'")
}
