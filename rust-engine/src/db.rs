//! Single embedded LanceDB table `indexed_files` holding the 512-d CLIP
//! column, the 384-d MiniLM text column, and a separate 768-d Jina-Code
//! column, per docs/ARCHITECTURE.md #3. Text and code get separate columns
//! (not one shared "text_vector") because Jina-Code's native hidden size is
//! 768, not 384 like MiniLM — verified against the actual ONNX export,
//! not assumed uniform.

use anyhow::Result;
use futures::TryStreamExt;
use lancedb::arrow::arrow_array::{
    self, types::Float32Type, FixedSizeListArray, Int64Array, RecordBatch, RecordBatchIterator,
    RecordBatchReader, StringArray,
};
use lancedb::arrow::arrow_schema::{DataType, Field, Fields, Schema};
use lancedb::connection::Connection;
use lancedb::query::{ExecutableQuery, QueryBase};
use std::path::Path;
use std::sync::Arc;

pub struct VectorStore {
    conn: Connection,
}

const TABLE: &str = "indexed_files";
const CLIP_DIM: i32 = 512;
const TEXT_DIM: i32 = 384;
const CODE_DIM: i32 = 768;

/// One row to upsert — a file with whichever vector(s) apply to its category.
pub struct FileRow {
    pub path: String,
    pub category: String,
    pub modified_unix_ms: i64,
    pub snippet: Option<String>,
    pub clip_vector: Option<Vec<f32>>,
    pub text_vector: Option<Vec<f32>>,
    pub code_vector: Option<Vec<f32>>,
}

pub struct ScoredRow {
    pub path: String,
    pub category: String,
    pub snippet: Option<String>,
    pub score: f32,
}

impl VectorStore {
    pub async fn open(path: &Path) -> Result<Self> {
        let conn = lancedb::connect(path.to_str().unwrap()).execute().await?;

        if !conn.table_names().execute().await?.contains(&TABLE.to_string()) {
            let schema = Arc::new(table_schema());
            conn.create_empty_table(TABLE, schema).execute().await?;
        }

        Ok(Self { conn })
    }

    pub async fn total_indexed(&self) -> Result<i64> {
        let table = self.conn.open_table(TABLE).execute().await?;
        Ok(table.count_rows(None).await? as i64)
    }

    /// Appends a batch of rows. LanceDB has no native upsert on a plain
    /// table; the indexer is expected to `DELETE path = ?` before
    /// re-inserting on a `Modified` fs event (see router.rs / service.rs),
    /// so this stays a pure append.
    pub async fn insert_rows(&self, rows: Vec<FileRow>) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let schema = Arc::new(table_schema());
        let batch = rows_to_batch(&schema, &rows)?;
        let table = self.conn.open_table(TABLE).execute().await?;
        let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
        table
            .add(Box::new(reader) as Box<dyn RecordBatchReader + Send>)
            .execute()
            .await?;
        Ok(())
    }

    /// Confirms `path` is a row DeepScan itself indexed, and returns its
    /// category if so. Used to gate the thumbnail/preview endpoint — the
    /// HTTP API has permissive CORS for the Cloud-mode UI shell (see
    /// http.rs), so serving raw file bytes for *any* path a caller names
    /// would turn a local search tool into an arbitrary-file-read primitive
    /// reachable from any webpage the user has open. Requiring the path to
    /// already be a row we indexed keeps it scoped to "files DeepScan
    /// already scanned", not the whole filesystem.
    pub async fn is_indexed_path(&self, path: &str) -> Result<Option<String>> {
        let table = self.conn.open_table(TABLE).execute().await?;
        let escaped = path.replace('\'', "''");
        let mut stream = table.query().only_if(format!("path = '{escaped}'")).limit(1).execute().await?;
        while let Some(batch) = stream.try_next().await? {
            let categories = batch.column_by_name("category").and_then(|c| c.as_any().downcast_ref::<StringArray>());
            if let Some(categories) = categories {
                if batch.num_rows() > 0 {
                    return Ok(Some(categories.value(0).to_string()));
                }
            }
        }
        Ok(None)
    }

    pub async fn delete_path(&self, path: &str) -> Result<()> {
        let table = self.conn.open_table(TABLE).execute().await?;
        let escaped = path.replace('\'', "''");
        table.delete(&format!("path = '{escaped}'")).await?;
        Ok(())
    }

    /// Plain substring match against filename and content — semantic
    /// vector search alone can miss an exact fragment the user actually
    /// typed (a specific variable name, an exact phrase, a filename with
    /// no meaningful embedding), especially since it's the only way
    /// images ever match a text query at all (they have no text/code
    /// vector, only clip_vector, so "type a name and find that image"
    /// otherwise turns up nothing no matter how exact the name is).
    /// Matches get a fixed max score — there's no distance metric here — so
    /// callers should treat them as "definitely relevant", not comparable
    /// on the same scale as a KNN score.
    pub async fn search_literal(&self, query: &str, top_k: usize) -> Result<Vec<ScoredRow>> {
        let table = self.conn.open_table(TABLE).execute().await?;
        let escaped = query.to_lowercase().replace('\'', "''").replace('%', "\\%").replace('_', "\\_");
        let filter = format!("LOWER(path) LIKE '%{escaped}%' OR LOWER(snippet) LIKE '%{escaped}%'");
        let mut stream = table.query().only_if(filter).limit(top_k).execute().await?;

        let mut rows = Vec::new();
        while let Some(batch) = stream.try_next().await? {
            let paths = batch.column_by_name("path").and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let categories = batch.column_by_name("category").and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let snippets = batch.column_by_name("snippet").and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let (Some(paths), Some(categories)) = (paths, categories) else { continue };
            for i in 0..batch.num_rows() {
                rows.push(ScoredRow {
                    path: paths.value(i).to_string(),
                    category: categories.value(i).to_string(),
                    snippet: snippets.map(|s| s.value(i).to_string()).filter(|s| !s.is_empty()),
                    score: 1.0,
                });
            }
        }
        Ok(rows)
    }

    pub async fn search_clip(&self, query: Vec<f32>, top_k: usize) -> Result<Vec<ScoredRow>> {
        self.search_column("clip_vector", query, top_k).await
    }

    pub async fn search_text(&self, query: Vec<f32>, top_k: usize) -> Result<Vec<ScoredRow>> {
        self.search_column("text_vector", query, top_k).await
    }

    pub async fn search_code(&self, query: Vec<f32>, top_k: usize) -> Result<Vec<ScoredRow>> {
        self.search_column("code_vector", query, top_k).await
    }

    async fn search_column(&self, column: &str, query: Vec<f32>, top_k: usize) -> Result<Vec<ScoredRow>> {
        let table = self.conn.open_table(TABLE).execute().await?;
        let mut stream = table
            .query()
            .nearest_to(query)?
            .column(column)
            .limit(top_k)
            .execute()
            .await?;

        let mut rows = Vec::new();
        while let Some(batch) = stream.try_next().await? {
            rows.extend(batch_to_rows(&batch)?);
        }
        Ok(rows)
    }
}

fn table_schema() -> Schema {
    Schema::new(Fields::from(vec![
        Field::new("path", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("modified_unix_ms", DataType::Int64, false),
        Field::new("snippet", DataType::Utf8, true),
        Field::new(
            "clip_vector",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), CLIP_DIM),
            true,
        ),
        Field::new(
            "text_vector",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), TEXT_DIM),
            true,
        ),
        Field::new(
            "code_vector",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), CODE_DIM),
            true,
        ),
    ]))
}

fn rows_to_batch(schema: &Arc<Schema>, rows: &[FileRow]) -> Result<RecordBatch> {
    let paths = StringArray::from_iter_values(rows.iter().map(|r| r.path.clone()));
    let categories = StringArray::from_iter_values(rows.iter().map(|r| r.category.clone()));
    let modified = Int64Array::from_iter_values(rows.iter().map(|r| r.modified_unix_ms));
    let snippets = StringArray::from(rows.iter().map(|r| r.snippet.clone()).collect::<Vec<_>>());

    let clip_vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        rows.iter()
            .map(|r| r.clip_vector.clone().map(|v| v.into_iter().map(Some))),
        CLIP_DIM,
    );
    let text_vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        rows.iter()
            .map(|r| r.text_vector.clone().map(|v| v.into_iter().map(Some))),
        TEXT_DIM,
    );
    let code_vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        rows.iter()
            .map(|r| r.code_vector.clone().map(|v| v.into_iter().map(Some))),
        CODE_DIM,
    );

    Ok(RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(paths),
            Arc::new(categories),
            Arc::new(modified),
            Arc::new(snippets),
            Arc::new(clip_vectors),
            Arc::new(text_vectors),
            Arc::new(code_vectors),
        ],
    )?)
}

fn batch_to_rows(batch: &RecordBatch) -> Result<Vec<ScoredRow>> {
    let paths = batch
        .column_by_name("path")
        .and_then(|c| c.as_any().downcast_ref::<StringArray>());
    let categories = batch
        .column_by_name("category")
        .and_then(|c| c.as_any().downcast_ref::<StringArray>());
    let snippets = batch
        .column_by_name("snippet")
        .and_then(|c| c.as_any().downcast_ref::<StringArray>());
    let distances = batch
        .column_by_name("_distance")
        .and_then(|c| c.as_any().downcast_ref::<arrow_array::Float32Array>());

    let (Some(paths), Some(categories)) = (paths, categories) else {
        return Ok(vec![]);
    };

    let mut rows = Vec::with_capacity(batch.num_rows());
    for i in 0..batch.num_rows() {
        rows.push(ScoredRow {
            path: paths.value(i).to_string(),
            category: categories.value(i).to_string(),
            snippet: snippets.map(|s| s.value(i).to_string()).filter(|s| !s.is_empty()),
            // LanceDB returns L2 distance for `_distance`; lower is better,
            // so invert to a "higher is more relevant" score for the UI.
            score: distances.map(|d| 1.0 / (1.0 + d.value(i))).unwrap_or(0.0),
        });
    }
    Ok(rows)
}
