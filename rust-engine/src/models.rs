//! Loads the three local ONNX models into memory once at startup and
//! exposes a small typed embedding API over them. All inference is
//! local — no network calls once the models are on disk.
//!
//! Model files are NOT bundled with this repo (multi-hundred-MB ONNX
//! exports don't belong in git). Place them under
//! `$DEEPSCAN_DATA_DIR/models/` before starting the engine:
//!   - clip-vit-b32.onnx            (+ CLIP's tokenizer.json, if adding text-side CLIP later)
//!   - all-MiniLM-L6-v2.onnx        + all-MiniLM-L6-v2.tokenizer.json
//!   - jina-embeddings-v2-base-code.onnx + jina-code.tokenizer.json

use anyhow::{Context, Result};
use ndarray::{Array4, CowArray};
use ort::{Session, Value};
use std::path::Path;
use tokenizers::Tokenizer;

const CLIP_IMAGE_SIZE: u32 = 224;
// Standard CLIP normalization constants (OpenAI CLIP preprocessing).
const CLIP_MEAN: [f32; 3] = [0.48145466, 0.4578275, 0.40821073];
const CLIP_STD: [f32; 3] = [0.26862954, 0.26130258, 0.27577711];

pub struct ModelBundle {
    pub clip: Session,
    pub minilm: Session,
    pub minilm_tokenizer: Tokenizer,
    pub jina_code: Session,
    pub jina_tokenizer: Tokenizer,
}

impl ModelBundle {
    pub fn load(model_dir: &Path) -> Result<Self> {
        let load_session = |file: &str| -> Result<Session> {
            let path = model_dir.join(file);
            Session::builder()?
                .with_intra_threads(4)?
                .commit_from_file(&path)
                .with_context(|| {
                    format!(
                        "failed to load {file} from {}. See rust-engine/src/models.rs for where \
                         to place ONNX model exports.",
                        path.display()
                    )
                })
        };

        let load_tokenizer = |file: &str| -> Result<Tokenizer> {
            let path = model_dir.join(file);
            Tokenizer::from_file(&path)
                .map_err(|e| anyhow::anyhow!("failed to load tokenizer {file}: {e}"))
        };

        Ok(Self {
            clip: load_session("clip-vit-b32.onnx")?,
            minilm: load_session("all-MiniLM-L6-v2.onnx")?,
            minilm_tokenizer: load_tokenizer("all-MiniLM-L6-v2.tokenizer.json")?,
            jina_code: load_session("jina-embeddings-v2-base-code.onnx")?,
            jina_tokenizer: load_tokenizer("jina-code.tokenizer.json")?,
        })
    }

    /// Embed a decoded RGB8 image (already resized to whatever `image` gave
    /// us) into a 512-d CLIP vector: resize to 224x224, normalize, NCHW.
    pub fn embed_image(&mut self, img: &image::DynamicImage) -> Result<Vec<f32>> {
        let resized = img.resize_exact(
            CLIP_IMAGE_SIZE,
            CLIP_IMAGE_SIZE,
            image::imageops::FilterType::Triangle,
        );
        let rgb = resized.to_rgb8();

        let mut tensor = Array4::<f32>::zeros((1, 3, CLIP_IMAGE_SIZE as usize, CLIP_IMAGE_SIZE as usize));
        for (x, y, pixel) in rgb.enumerate_pixels() {
            for c in 0..3 {
                let normalized = (pixel[c] as f32 / 255.0 - CLIP_MEAN[c]) / CLIP_STD[c];
                tensor[[0, c, y as usize, x as usize]] = normalized;
            }
        }

        let input = Value::from_array(tensor)?;
        let outputs = self.clip.run(ort::inputs!["pixel_values" => input]?)?;
        let embedding = outputs[0].try_extract_tensor::<f32>()?;
        Ok(normalize(embedding.view().iter().copied().collect()))
    }

    /// Embed free text -> 384-d MiniLM vector via mean-pooled last hidden state.
    pub fn embed_text(&mut self, text: &str) -> Result<Vec<f32>> {
        // MiniLM's ONNX graph declares token_type_ids as a required input
        // (unlike Jina-Code's) — verified against the actual exported graph,
        // not assumed. All-zeros is correct here: MiniLM only ever sees a
        // single-segment input, never a sentence-pair, so every token's
        // type id is 0 either way.
        embed_with_tokenizer(&mut self.minilm, &self.minilm_tokenizer, text, true)
    }

    /// Embed a code snippet -> 768-d Jina-Code vector. Note: this is a
    /// different dimensionality than embed_text's 384 (Jina-Code's native
    /// hidden size, verified against its actual ONNX export) — stored in
    /// LanceDB's separate `code_vector` column, see db.rs.
    pub fn embed_code(&mut self, snippet: &str) -> Result<Vec<f32>> {
        embed_with_tokenizer(&mut self.jina_code, &self.jina_tokenizer, snippet, false)
    }
}

/// Self-attention memory/compute scales quadratically with sequence
/// length, and neither model's ONNX graph rejects an over-long input —
/// Jina-Code in particular is explicitly a long-context model (trained up
/// to 8192 tokens) and will simply try to run whatever it's given. Every
/// caller is expected to pre-truncate its *input text* to something
/// reasonable already (see service.rs's 8000-char snippet cap), but that
/// alone isn't enough: a search query is raw user input with no cap of
/// its own (someone pasting a huge chunk of code into the search box and
/// hitting Send), and this is the one choke point every embed call — text
/// or code, index-time or query-time — actually passes through. 512
/// tokens comfortably covers everything from a search query to a
/// meaningful chunk of file content, while keeping worst-case attention
/// memory bounded to something sane on an 8GB machine.
const MAX_TOKENS: usize = 512;

/// Shared tokenize -> run -> mean-pool -> L2-normalize path for both
/// text-style models. `needs_token_type_ids` differs per model — verified
/// against each model's actual ONNX graph rather than assumed uniform.
fn embed_with_tokenizer(
    session: &mut Session,
    tokenizer: &Tokenizer,
    text: &str,
    needs_token_type_ids: bool,
) -> Result<Vec<f32>> {
    let encoding = tokenizer
        .encode(text, true)
        .map_err(|e| anyhow::anyhow!("tokenization failed: {e}"))?;

    let mut ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
    let mut mask: Vec<i64> = encoding.get_attention_mask().iter().map(|&m| m as i64).collect();
    ids.truncate(MAX_TOKENS);
    mask.truncate(MAX_TOKENS);
    let seq_len = ids.len();

    let ids_arr = CowArray::from(ndarray::Array2::from_shape_vec((1, seq_len), ids)?).into_dyn();
    let mask_arr = CowArray::from(ndarray::Array2::from_shape_vec((1, seq_len), mask.clone())?).into_dyn();

    let input_ids = Value::from_array(&ids_arr)?;
    let attention_mask = Value::from_array(&mask_arr)?;

    let outputs = if needs_token_type_ids {
        let type_ids = vec![0i64; seq_len];
        let type_arr = CowArray::from(ndarray::Array2::from_shape_vec((1, seq_len), type_ids)?).into_dyn();
        let token_type_ids = Value::from_array(&type_arr)?;
        session.run(ort::inputs![
            "input_ids" => input_ids,
            "attention_mask" => attention_mask,
            "token_type_ids" => token_type_ids,
        ]?)?
    } else {
        session.run(ort::inputs![
            "input_ids" => input_ids,
            "attention_mask" => attention_mask,
        ]?)?
    };

    // last_hidden_state: [1, seq_len, hidden_dim] -> mean-pool over
    // non-padding tokens, matching sentence-transformers' pooling.
    let hidden = outputs[0].try_extract_tensor::<f32>()?;
    let shape = hidden.shape();
    let hidden_dim = shape[2];

    let mut pooled = vec![0f32; hidden_dim];
    let mut valid_tokens = 0f32;
    for (t, &m) in mask.iter().enumerate() {
        if m == 0 {
            continue;
        }
        valid_tokens += 1.0;
        for d in 0..hidden_dim {
            pooled[d] += hidden[[0, t, d]];
        }
    }
    if valid_tokens > 0.0 {
        for v in pooled.iter_mut() {
            *v /= valid_tokens;
        }
    }

    Ok(normalize(pooled))
}

fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
    v
}
