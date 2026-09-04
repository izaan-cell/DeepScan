//! Single embedded LanceDB table `indexed_files` holding both the 512-d CLIP
//! column and the 384-d text column, per docs/ARCHITECTURE.md #3.

use anyhow::Result;
use arrow_schema::{DataType, Field, Fields, Schema};
use lancedb::connection::Connection;
use std::path::Path;
use std::sync::Arc;

pub struct VectorStore {
    conn: Connection,
}

const TABLE: &str = "indexed_files";
const CLIP_DIM: i32 = 512;
const TEXT_DIM: i32 = 384;

impl VectorStore {
    pub async fn open(path: &Path) -> Result<Self> {
        let conn = lancedb::connect(path.to_str().unwrap()).execute().await?;

        if !conn.table_names().execute().await?.contains(&TABLE.to_string()) {
            let schema = Arc::new(Schema::new(Fields::from(vec![
                Field::new("path", DataType::Utf8, false),
                Field::new("category", DataType::Utf8, false),
                Field::new("modified_unix_ms", DataType::Int64, false),
                Field::new("snippet", DataType::Utf8, true),
                Field::new(
                    "clip_vector",
                    DataType::FixedSizeList(
                        Arc::new(Field::new("item", DataType::Float32, true)),
                        CLIP_DIM,
                    ),
                    true, // nullable — not every row has an image vector
                ),
                Field::new(
                    "text_vector",
                    DataType::FixedSizeList(
                        Arc::new(Field::new("item", DataType::Float32, true)),
                        TEXT_DIM,
                    ),
                    true, // nullable — not every row has a text vector
                ),
            ])));

            conn.create_empty_table(TABLE, schema).execute().await?;
        }

        Ok(Self { conn })
    }

    pub async fn total_indexed(&self) -> Result<i64> {
        let table = self.conn.open_table(TABLE).execute().await?;
        Ok(table.count_rows(None).await? as i64)
    }

    // upsert_row(...), search_by_clip(query_vec, top_k), search_by_text(query_vec, top_k)
    // follow the same lancedb::Table::{add, query} pattern — omitted here as
    // scaffold boundary; see docs/ARCHITECTURE.md for the SearchService
    // contract these back.
}
