//! Loads the three local ONNX models into memory once at startup and exposes
//! a small typed API over them. All inference is local — no network calls.

use anyhow::Result;
use ort::session::Session;
use std::path::Path;

pub struct ModelBundle {
    /// CLIP ViT-B/32 visual tower — images/icons -> 512-d vector.
    pub clip: Session,
    /// all-MiniLM-L6-v2 — general text/documents -> 384-d vector.
    pub minilm: Session,
    /// jina-embeddings-v2-base-code — code/scripts -> 384-d vector.
    pub jina_code: Session,
}

impl ModelBundle {
    pub fn load(data_dir: &Path) -> Result<Self> {
        let model_dir = data_dir.join("models");

        let clip = Session::builder()?
            .with_intra_threads(4)?
            .commit_from_file(model_dir.join("clip-vit-b32.onnx"))?;

        let minilm = Session::builder()?
            .with_intra_threads(4)?
            .commit_from_file(model_dir.join("all-MiniLM-L6-v2.onnx"))?;

        let jina_code = Session::builder()?
            .with_intra_threads(4)?
            .commit_from_file(model_dir.join("jina-embeddings-v2-base-code.onnx"))?;

        Ok(Self { clip, minilm, jina_code })
    }

    /// Embed a raster image buffer -> 512-d CLIP vector.
    pub fn embed_image(&self, _rgb_pixels: &[f32], _width: u32, _height: u32) -> Result<Vec<f32>> {
        // Preprocess (resize 224x224, normalize) then session.run(...).
        // Scaffold stub — see docs/ARCHITECTURE.md pipeline table.
        todo!("CLIP preprocessing + ort::Session::run")
    }

    /// Embed free text or a conceptual query -> 384-d MiniLM vector.
    pub fn embed_text(&self, _text: &str) -> Result<Vec<f32>> {
        todo!("tokenize + ort::Session::run against self.minilm")
    }

    /// Embed a code snippet -> 384-d Jina-Code vector.
    pub fn embed_code(&self, _snippet: &str) -> Result<Vec<f32>> {
        todo!("tokenize + ort::Session::run against self.jina_code")
    }
}
