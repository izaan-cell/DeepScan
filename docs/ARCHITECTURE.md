# DeepScan — Backend Architecture Specification

DeepScan is a 100% free, fully local, cross-platform (macOS + Windows) universal AI search
application. It indexes and semantically searches every file type on disk — images, code,
documents, audio, and video — using text, image, or code-snippet queries. No Python, no cloud
calls, no telemetry.

The system is a **high-performance monolith with three cooperating processes on one machine**,
talking over loopback gRPC:

| Component | Language | Responsibility |
|---|---|---|
| **Engine** | Rust | Directory scanning, ONNX model inference, LanceDB vector storage/search |
| **Daemon** | Go | Filesystem watching, system tray, native OS hooks (Finder/Explorer), orchestration |
| **Parser** | Java (OpenJDK) | Apache Tika document/enterprise-format extraction, exposed over gRPC to Rust |
| **UI** | HTML/CSS/JS (embeddable in JavaFX `WebView`) | Single-page search interface |

---

## 1. Universal Processing Pipeline — Python Replacement Map

| File Category | Specific Formats | Extractor (no Python) | Vector Model & Runtime |
|---|---|---|---|
| **Images & Icons** | `.png` `.jpg` `.ico` `.svg` | Rust `image` crate (raster decode) + `resvg`/`usvg` (SVG → raster) | CLIP ViT-B/32 (visual tower), ONNX export, run via Rust **`ort`** crate (ONNX Runtime bindings) |
| **Code & Scripts** | `.py` `.js` `.cpp` `.go` `.rs` `.java` | Rust `walkdir` + `ignore` crate for multi-threaded, `.gitignore`-aware traversal; `tree-sitter` for optional symbol/chunk-aware splitting | `jina-embeddings-v2-base-code`, ONNX export, run via `ort` |
| **Documents & PDFs** | `.pdf` `.docx` `.txt` `.md` `.xlsx` | **Java Apache Tika** (`tika-core` + `tika-parsers-standard-package`) running as a small gRPC service; Rust calls it over loopback for extraction | `all-MiniLM-L6-v2`, ONNX export, run via `ort` |
| **Scanned Docs / OCR** | Scanned PDFs, screenshots | **`ocrs`** (pure-Rust OCR, no Python/Tesseract dependency) or Tesseract via Tika's OCR pipeline as a fallback | OCR'd text → MiniLM text embedding; original page also indexed as a CLIP image vector |
| **Audio & Video** | `.mp3` `.wav` `.mp4` `.mov` | **`whisper-rs`** (Rust bindings to `whisper.cpp`) for transcription; `ffmpeg-next` (Rust bindings to native FFmpeg, no shelling out) for frame/audio extraction | Transcript → MiniLM text vector; sampled keyframes → CLIP image vector |

Notes:
- Every extractor above is either a native Rust crate or reached via gRPC — nothing shells out to Python at any point.
- Tika is the one JVM dependency because there is no Rust or Go library that reliably parses legacy Office binary formats (`.doc`, `.xls`), OOXML, RTF, and the long tail of enterprise formats as robustly as Tika does. It's isolated behind a thin gRPC service so it can crash/restart independently of the core engine.

---

## 2. Inter-Process Communication

**Decision: gRPC over `127.0.0.1` loopback TCP, via Protocol Buffers.**

Rationale over shared memory / raw Unix domain sockets:
- Cross-platform parity: Windows named pipes vs. macOS Unix domain sockets would mean two IPC implementations. gRPC-over-loopback-TCP is identical code on both platforms.
- Streaming built in: `.proto` `stream` semantics cover "watch a directory and push events" and "stream large file buffers in chunks" without hand-rolled framing.
- Strong typing across three languages: `protoc` codegen gives Rust (`tonic` + `prost`), Go (`google.golang.org/grpc`), and Java (`grpc-java`) generated stubs from one schema — no serialization drift.
- Loopback-only binding (`127.0.0.1`, random high port negotiated at startup and written to a local lockfile) — never exposed to the network, so gRPC's overhead vs. raw sockets (a few hundred µs/call) is a non-issue at local file-indexing volumes.

Port/service discovery: the Rust engine is the server and owns three services (`IndexService`, `SearchService`, `ParserBridgeService`). On startup it binds an ephemeral port and writes `~/.deepscan/engine.lock` (`{"port": 51423, "pid": 1234}`). The Go daemon and Java parser both read this file to connect as clients.

### `.proto` schema

```protobuf
syntax = "proto3";
package deepscan.v1;

// ---------- Shared types ----------

enum FileCategory {
  FILE_CATEGORY_UNSPECIFIED = 0;
  FILE_CATEGORY_IMAGE       = 1;
  FILE_CATEGORY_CODE        = 2;
  FILE_CATEGORY_DOCUMENT    = 3;
  FILE_CATEGORY_AUDIO       = 4;
  FILE_CATEGORY_VIDEO       = 5;
}

enum ChangeKind {
  CHANGE_KIND_UNSPECIFIED = 0;
  CHANGE_KIND_CREATED     = 1;
  CHANGE_KIND_MODIFIED    = 2;
  CHANGE_KIND_DELETED     = 3;
  CHANGE_KIND_RENAMED     = 4;
}

message FileRef {
  string path       = 1;
  int64  size_bytes = 2;
  int64  modified_unix_ms = 3;
  FileCategory category = 4;
}

// ---------- IndexService: Go daemon -> Rust engine ----------
// Go watches the filesystem and streams change events; Rust owns indexing.

service IndexService {
  // Long-lived stream: daemon pushes fs events as they happen.
  rpc WatchEvents(stream FsEvent) returns (stream IndexAck);

  // One-shot: ask the engine to (re)index an explicit path (e.g. user adds a folder).
  rpc IndexPath(IndexPathRequest) returns (stream IndexProgress);

  // Poll-friendly status for the UI's "Index Status Monitor".
  rpc GetIndexStatus(Empty) returns (IndexStatus);
}

message FsEvent {
  ChangeKind kind = 1;
  FileRef    file = 2;
  string     renamed_from = 3; // set only when kind == RENAMED
}

message IndexAck {
  string path = 1;
  bool   accepted = 2;
  string error = 3;
}

message IndexPathRequest {
  string root_path = 1;
  bool   recursive = 2;
}

message IndexProgress {
  int64 files_scanned  = 1;
  int64 files_indexed  = 2;
  int64 files_skipped  = 3;
  string current_path  = 4;
  bool  done = 5;
}

message IndexStatus {
  bool   db_healthy = 1;
  int64  total_indexed_files = 2;
  int64  pending_queue_depth = 3;
  bool   is_scanning = 4;
  string last_error = 5;
}

// ---------- SearchService: UI -> Rust engine ----------

service SearchService {
  rpc Search(SearchRequest) returns (SearchResponse);
  rpc RevealInOs(RevealRequest) returns (RevealResponse); // routed via Go daemon
}

message SearchRequest {
  oneof query {
    string text_query        = 1; // conceptual / code text
    bytes  image_query_bytes = 2; // dropped image/icon, raw bytes
  }
  repeated FileCategory scope = 3; // empty = all
  int32 top_k = 4;
}

message SearchResult {
  FileRef file = 1;
  float   score = 2;
  string  snippet = 3; // text excerpt or thumbnail cache path
}

message SearchResponse {
  repeated SearchResult results = 1;
  int64 query_time_ms = 2;
}

message RevealRequest  { string path = 1; }
message RevealResponse { bool success = 1; string error = 2; }

// ---------- ParserBridgeService: Rust engine -> Java Tika service ----------

service ParserBridgeService {
  // Rust streams raw file bytes (chunked) or a path; Java returns extracted text + metadata.
  rpc ExtractDocument(ExtractRequest) returns (ExtractResponse);
}

message ExtractRequest {
  string path = 1;
  string mime_hint = 2;
}

message ExtractResponse {
  string extracted_text = 1;
  map<string, string> metadata = 2;
  bool ocr_used = 3;
}

message Empty {}
```

---

## 3. Vector Storage — single LanceDB table

One embedded LanceDB instance (`~/.deepscan/lancedb/`), one table `indexed_files`, multiple
vector columns:

| Column | Type | Populated for |
|---|---|---|
| `path` | string (key) | all rows |
| `category` | string enum | all rows |
| `modified_unix_ms` | int64 | all rows |
| `clip_vector` | `float32[512]` | images, video keyframes |
| `text_vector` | `float32[384]` | code, documents, OCR text, transcripts |
| `snippet` | string | thumbnail cache path or text excerpt |

LanceDB natively supports nullable vector columns and ANN search (IVF_PQ) per-column, so a
single table serves both modalities — the query path picks `clip_vector` or `text_vector`
depending on whether the query is an image or a text/code string.

---

## 4. Process Lifecycle

1. Rust engine starts first (launched by the Go daemon, or by the OS at login via a
   launchd `plist` / Windows Task Scheduler entry). Binds gRPC server, opens/creates LanceDB,
   loads ONNX models into memory (CLIP + MiniLM + Jina-Code), writes `engine.lock`.
2. Go daemon starts (system tray icon appears), reads `engine.lock`, connects as an
   `IndexService` client, begins recursive filesystem watch on configured roots, streams
   `FsEvent`s.
3. Java Tika bridge starts on demand (or is kept warm as a small always-on JVM), reads
   `engine.lock`'s sibling `parser.lock`, serves `ParserBridgeService`.
4. UI (JavaFX `WebView` hosting the SPA) talks to the Rust engine through a thin local
   JSON-over-HTTP shim served on the same port as gRPC (via `axum`, mounted alongside
   `tonic`) rather than a full grpc-web/Envoy setup — the WebView UI is the one client that
   doesn't need proto codegen, so a plain `fetch()`-friendly `/api/search` and
   `/api/status` endpoint keeps the frontend dependency-free. `RevealInOs` calls go through
   the same shim, which the engine proxies to the Go daemon's native hook.
5. **No browser storage.** The SPA never writes to `localStorage`/`sessionStorage`/IndexedDB.
   Every input the user provides — pasted text, a code snippet, a dropped image's raw bytes —
   is sent straight to the engine over `/api/search` and persisted there (LanceDB row +
   on-disk snippet/thumbnail cache under `~/.deepscan/cache/`), never held client-side only.

## 5. Deploying the UI shell to Vercel

DeepScan's whole premise is local search — a hosted container has no access to a visitor's
Documents folder, so it can never do real indexing/search itself. What a public host *can*
usefully do is serve the frontend shell, so the app has one stable URL instead of everyone
opening a local file. Since that shell is genuinely static (`frontend/index.html` + `.css`
+ `.js`, no build step), it doesn't need a persistent backend process at all — Vercel serves
it directly as static files (see `vercel.json`, `outputDirectory: "frontend"`), which is a
better fit than a free-tier Render web service (0.1 vCPU) that would sit mostly idle running
a Rust binary just to hand back static assets.

The frontend (`frontend/app.js`) already handles having no backend at this origin: it tries
`/api/status` on load, and on a plain static host that 404s (or the fetch fails outright),
so it falls back to `http://127.0.0.1:51424` — a locally-installed engine running on the
visitor's own machine. Chrome will prompt the user to allow the page to reach a
private-network address the first time (Private Network Access) — that prompt is expected,
not a bug.

`rust-engine` still has a `config::Mode::Cloud` code path (skips loading
models/DB/gRPC, serves the same static files + a `/api/status` reporting
`mode: "cloud"`) — not used by the Vercel deploy, but kept because it lets the exact same
binary be self-hosted on any plain server if that's ever preferable to Vercel.

A **Chrome extension or installed PWA cannot replace this.** Browser sandboxing has no API
for recursive background directory watching across arbitrary folders, no persistent
filesystem access without re-prompting, and no way to hook Finder/Explorer context menus —
the File System Access API (`showDirectoryPicker()`) only grants one folder at a time, for
the current session. The Go daemon's `fsnotify` watch + system tray + native OS hooks stay
a real installed background process; nothing in a browser sandbox substitutes for that.
