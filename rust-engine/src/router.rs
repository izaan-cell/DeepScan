//! Maps a file path to a `FileCategory` and to the extraction path it needs
//! before embedding — see the pipeline table in docs/ARCHITECTURE.md.

use crate::pb::FileCategory;
use std::path::Path;

pub fn categorize(path: &Path) -> FileCategory {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "ico" | "svg" | "webp" | "gif" => FileCategory::Image,
        "py" | "js" | "ts" | "jsx" | "tsx" | "cpp" | "c" | "h" | "go" | "rs" | "java" | "rb"
        | "swift" | "kt" => FileCategory::Code,
        "pdf" | "docx" | "doc" | "txt" | "md" | "xlsx" | "xls" | "pptx" | "rtf" => {
            FileCategory::Document
        }
        "mp3" | "wav" | "flac" | "m4a" | "aac" => FileCategory::Audio,
        "mp4" | "mov" | "mkv" | "avi" | "webm" => FileCategory::Video,
        _ => FileCategory::Unspecified,
    }
}

/// What an indexer worker should do with a file, given its category.
pub enum ExtractionPlan {
    /// Rasterize + run through CLIP directly (Rust `image`/`resvg`).
    DirectClip,
    /// Read as UTF-8 text, chunk if large, run through Jina-Code (Rust-only).
    DirectCode,
    /// Hand off to the Java Tika bridge over gRPC, then embed the returned
    /// text with MiniLM. Falls back to `ocrs` locally if Tika reports a
    /// scanned/no-text-layer PDF.
    ViaTikaThenMiniLm,
    /// Extract audio with `ffmpeg-next`, transcribe with `whisper-rs`, embed
    /// the transcript with MiniLM; sample keyframes and embed with CLIP.
    AudioVideoPipeline,
}

pub fn plan_for(category: FileCategory) -> ExtractionPlan {
    match category {
        FileCategory::Image => ExtractionPlan::DirectClip,
        FileCategory::Code => ExtractionPlan::DirectCode,
        FileCategory::Document => ExtractionPlan::ViaTikaThenMiniLm,
        FileCategory::Audio | FileCategory::Video => ExtractionPlan::AudioVideoPipeline,
        FileCategory::Unspecified => ExtractionPlan::DirectCode, // best-effort text read
    }
}
