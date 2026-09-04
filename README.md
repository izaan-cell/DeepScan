# DeepScan

A 100% free, fully local, universal AI search application for macOS Finder and Windows
File Explorer. Search every file on disk — images, code, PDFs, video, audio — using a text
query, a code snippet, or a dropped image. Nothing leaves the machine: no cloud calls, no
accounts, no telemetry.

Full backend architecture, the IPC protocol, and the vector schema are documented in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Stack

| Component | Language | Role |
|---|---|---|
| [`rust-engine/`](rust-engine) | Rust | Directory scanning, local ONNX inference (CLIP / MiniLM / Jina-Code), LanceDB vector store |
| [`go-daemon/`](go-daemon) | Go | Filesystem watching, system tray, native Finder/Explorer hooks |
| [`java-parser/`](java-parser) | Java (Apache Tika) | Enterprise document parsing, exposed to Rust over gRPC |
| [`frontend/`](frontend) | HTML/CSS/JS | The search UI (embeds in a JavaFX `WebView`) |
| [`proto/deepscan.proto`](proto/deepscan.proto) | Protocol Buffers | Shared gRPC schema across all three backend components |

## Getting started

```bash
# 1. Rust engine (starts first, owns the gRPC server + LanceDB)
cd rust-engine && cargo run

# 2. Go daemon (filesystem watcher + system tray + OS hooks)
cd go-daemon && go run .

# 3. Java parser bridge (Tika, for documents/PDFs/Office formats)
cd java-parser && mvn compile exec:java -Dexec.mainClass=com.deepscan.parser.TikaServer
```

The frontend in `frontend/` is a static SPA — open `frontend/index.html` directly, or embed
it in a JavaFX `WebView`, once the Rust engine is running (it talks to the engine's local
JSON-HTTP shim on `127.0.0.1`).

## Design

Strict black-and-white, literary-editorial aesthetic — serif typography (Playfair Display),
no monospace/system-UI fonts anywhere user-facing, no color beyond black/white/gray. Design
tokens live at the top of [`frontend/style.css`](frontend/style.css).

No browser storage is used anywhere in the UI (no `localStorage` / `sessionStorage` /
IndexedDB) — every input a user provides is sent to and persisted by the local engine.
