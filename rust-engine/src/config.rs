//! Env-driven configuration. Loads `.env.dev` then `.env` in development
//! (see .env.example at the repo root) — dotenvy never overwrites a
//! variable already set in the process environment, so the more specific
//! file must load first. Mirrors go-daemon/config.go's precedence exactly
//! so both processes agree on ports/paths without a second source of truth.

use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// The real product: loads ONNX models, opens LanceDB, runs the gRPC
    /// server for the Go daemon, binds HTTP to 127.0.0.1 only. What a user
    /// runs on their own machine.
    Local,
    /// What's deployed on Render: serves the static frontend + a minimal
    /// status API, binds 0.0.0.0:$PORT, does NOT load models or open a
    /// database (no local files to search from a cloud box — see
    /// docs/ARCHITECTURE.md "Deploying the UI shell"). The frontend served
    /// this way talks to a *locally installed* engine at 127.0.0.1 for
    /// actual search.
    Cloud,
}

pub struct Config {
    pub mode: Mode,
    pub data_dir: PathBuf,
    /// Where the ONNX models + tokenizers live. Defaults to
    /// `data_dir/models`, but a packaged app overrides this to point
    /// directly at its bundled, read-only Resources/models — avoiding a
    /// several-hundred-MB copy into the user's writable data dir on first
    /// launch (see packaging/macos/launcher.sh, packaging/windows/launcher.ps1).
    pub model_dir: PathBuf,
    pub engine_grpc_port: Option<u16>, // None => ephemeral (production default)
    pub http_bind_host: [u8; 4],
    pub http_port: u16,
    pub log_level: String,
}

impl Config {
    pub fn load() -> Self {
        let env = std::env::var("DEEPSCAN_ENV").unwrap_or_else(|_| "development".into());
        if env == "development" {
            let _ = dotenvy::from_filename(repo_relative(".env.dev"));
        }
        let _ = dotenvy::from_filename(repo_relative(".env"));

        // Render (and most PaaS providers) inject $PORT and expect the
        // process to bind 0.0.0.0 to it — treat that as authoritative
        // evidence we're in cloud mode even if DEEPSCAN_MODE wasn't set
        // explicitly.
        let render_port = std::env::var("PORT").ok().and_then(|p| p.parse::<u16>().ok());
        let mode = match std::env::var("DEEPSCAN_MODE").as_deref() {
            Ok("cloud") => Mode::Cloud,
            Ok("local") => Mode::Local,
            _ if render_port.is_some() => Mode::Cloud,
            _ => Mode::Local,
        };

        let data_dir = expand_home(
            &std::env::var("DEEPSCAN_DATA_DIR").unwrap_or_else(|_| "~/.deepscan".into()),
        );

        let model_dir = std::env::var("DEEPSCAN_MODEL_DIR")
            .map(|p| expand_home(&p))
            .unwrap_or_else(|_| data_dir.join("models"));

        let http_port = render_port
            .or_else(|| std::env::var("DEEPSCAN_ENGINE_HTTP_PORT").ok().and_then(|p| p.parse().ok()))
            .unwrap_or(51424);

        Self {
            mode,
            data_dir,
            model_dir,
            engine_grpc_port: std::env::var("DEEPSCAN_ENGINE_GRPC_PORT")
                .ok()
                .and_then(|p| p.parse().ok()),
            http_bind_host: if mode == Mode::Cloud { [0, 0, 0, 0] } else { [127, 0, 0, 1] },
            http_port,
            log_level: std::env::var("DEEPSCAN_LOG_LEVEL").unwrap_or_else(|_| "info".into()),
        }
    }
}

/// Resolves a dev-only relative path (e.g. ".env.dev") against the repo
/// root so `cargo run` works whether invoked from the repo root or from
/// inside rust-engine/.
fn repo_relative(name: &str) -> PathBuf {
    if PathBuf::from(name).exists() {
        return PathBuf::from(name);
    }
    PathBuf::from("..").join(name)
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix('~') {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest.trim_start_matches('/'));
        }
    }
    PathBuf::from(path)
}
