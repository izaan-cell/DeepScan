# DeepScan

A 100% free, fully local, cross-platform AI file search app for macOS and Windows.
Search your own files by *meaning* — type a phrase, paste a code snippet, or drop in an
image — instead of remembering exact filenames or keywords. Nothing leaves the machine:
no cloud calls, no accounts, no telemetry.

Download the latest build (`DeepScan.dmg` for macOS, `DeepScan.msi` for Windows) from
[Releases](https://github.com/izaan-cell/DeepScan/releases/latest). Full backend
architecture, the IPC protocol, and the vector schema are documented in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## What it's built with

| Component | Language | Role |
|---|---|---|
| [`rust-engine/`](rust-engine) | Rust | Local ONNX inference (CLIP / MiniLM / Jina-Code), the LanceDB vector store, and the HTTP API the frontend talks to |
| [`go-daemon/`](go-daemon) | Go | Filesystem watching (`fsnotify`) and native Finder/Explorer reveal hooks |
| [`java-parser/`](java-parser) | Java (Apache Tika) | Document text extraction (PDF, DOCX, etc.), exposed to Rust over gRPC |
| [`frontend/`](frontend) | HTML/CSS/JS | The search UI |
| [`packaging/macos/DeepScanWindow.swift`](packaging/macos/DeepScanWindow.swift) | Swift/Cocoa | The native macOS window (real window chrome, Dock icon, menu bar) wrapping the frontend in a `WKWebView` |
| [`packaging/windows/`](packaging/windows) | WiX | Windows MSI installer |
| [`proto/deepscan.proto`](proto/deepscan.proto) | Protocol Buffers | Shared gRPC schema between the Rust engine and Go daemon |

Three ONNX models run fully offline: CLIP (image search), MiniLM (text/document search),
and Jina-Code (code search).

## How it works

1. On launch, the Go daemon walks your Desktop, Documents, Downloads, Pictures, Movies,
   Music, and Applications folders once, then watches them live for changes. It skips
   build/dependency noise (`node_modules`, `.git`, `target`, `dist`, ...) and opaque
   bundles (`.app`, `.photoslibrary`, Xcode projects).
2. Each file is categorized (image / code / document / audio / video) and routed to the
   matching model: images through CLIP, code through Jina-Code, documents (extracted via
   Tika) through MiniLM.
3. The resulting vector, plus a text snippet, is stored in a local LanceDB table under
   `~/.deepscan/`.
4. A search embeds your query the same way and runs a nearest-neighbor vector search,
   merged with a plain substring match against filenames and content so an exact name or
   phrase always surfaces regardless of semantic similarity.
5. Results show an image thumbnail or code/document snippet inline, and can be revealed
   directly in Finder/Explorer with one click.

## Running from source

```bash
# 1. Rust engine (starts first, owns the HTTP API + LanceDB)
cd rust-engine && cargo run

# 2. Go daemon (filesystem watcher + reveal-in-Finder hooks)
cd go-daemon && go run .

# 3. Java parser bridge (Tika, for documents/PDFs/Office formats — optional)
cd java-parser && mvn compile exec:java -Dexec.mainClass=com.deepscan.parser.TikaServer
```

`packaging/macos/build-dmg.sh` builds the actual packaged `.app`/`.dmg` (compiles all
three components, bundles the ONNX models, and produces a signed native window); CI runs
the equivalent for both platforms on every push to `main` and publishes a GitHub Release.

## Design

Strict black-and-white, literary-editorial aesthetic — serif typography (Playfair
Display), no monospace/system-UI fonts anywhere user-facing except inline code snippets,
no color beyond black/white/gray. Design tokens live at the top of
[`frontend/style.css`](frontend/style.css).

No browser storage is used anywhere in the UI (no `localStorage` / `sessionStorage` /
IndexedDB) — every input a user provides is sent to and persisted by the local engine.
