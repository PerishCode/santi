use santi_provider::ProviderItem;

use super::{SantiStore, span::Span};

mod render;
use render::*;

impl SantiStore {
    pub fn assembly_input(&self, strand: &str) -> Result<Vec<ProviderItem>, String> {
        let conn = self.conn.lock().unwrap();
        assembly_input_in_conn(&conn, strand)
    }

    pub(crate) fn assembly_input_preview(
        &self,
        strand: &str,
        response: &crate::CompactExecResponse,
        summary: &str,
        metadata: serde_json::Value,
    ) -> Result<Vec<ProviderItem>, String> {
        let conn = self.conn.lock().unwrap();
        let preview = crate::Compact {
            id: response.compact.clone(),
            strand: strand.to_string(),
            summary: summary.to_string(),
            first: response.first.clone(),
            last: response.last.clone(),
            created: None,
            metadata: Some(metadata),
        };
        assembly_input_with_preview(
            &conn,
            strand,
            Some(Preview {
                span: Span {
                    start_seq: response.start_seq,
                    end_seq: response.end_seq,
                },
                absorbed: response.absorbed.as_slice(),
                content: render_compact_for_provider(
                    &preview,
                    Range {
                        span: Span {
                            start_seq: response.start_seq,
                            end_seq: response.end_seq,
                        },
                        collapsed_count: response.collapsed_count,
                    },
                ),
            }),
        )
    }
}

pub(super) fn assembly_input_in_conn(
    conn: &rusqlite::Connection,
    strand: &str,
) -> Result<Vec<ProviderItem>, String> {
    assembly_input_with_preview(conn, strand, None)
}

struct Preview<'a> {
    span: Span,
    absorbed: &'a [String],
    content: String,
}

struct Overlay {
    span: Span,
    content: String,
}

struct Range {
    span: Span,
    collapsed_count: i64,
}
