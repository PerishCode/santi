use santi_provider::Item;

use super::{Store, span::Span};

mod render;
use render::*;

impl Store {
    pub fn assembly(&self, strand: &str) -> Result<Vec<Item>, String> {
        let conn = self.conn.lock().unwrap();
        assembled(&conn, strand)
    }

    pub(crate) fn preview(
        &self,
        strand: &str,
        response: &crate::compact::Report,
        summary: &str,
        metadata: serde_json::Value,
    ) -> Result<Vec<Item>, String> {
        let conn = self.conn.lock().unwrap();
        let preview = crate::compact::Compact {
            id: response.compact.clone(),
            strand: strand.to_string(),
            summary: summary.to_string(),
            first: response.first.clone(),
            last: response.last.clone(),
            created: None,
            metadata: Some(metadata),
        };
        previewed(
            &conn,
            strand,
            Some(Preview {
                span: Span {
                    from: response.from,
                    to: response.to,
                },
                absorbed: response.absorbed.as_slice(),
                content: condensed(
                    &preview,
                    Range {
                        span: Span {
                            from: response.from,
                            to: response.to,
                        },
                        collapsed: response.collapsed,
                    },
                ),
            }),
        )
    }
}

pub(super) fn assembled(conn: &rusqlite::Connection, strand: &str) -> Result<Vec<Item>, String> {
    previewed(conn, strand, None)
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
    collapsed: i64,
}
