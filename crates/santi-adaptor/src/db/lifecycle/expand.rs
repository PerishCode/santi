use rusqlite::Connection;

use super::table;

struct Expansion {
    table: &'static str,
    column: &'static str,
    sql: &'static str,
}

pub(super) fn expand(conn: &Connection) -> Result<(), String> {
    let changes = [
        Expansion {
            table: "jobs",
            column: "remind_every_seconds",
            sql: r#"ALTER TABLE jobs ADD COLUMN remind_every_seconds INTEGER
                CHECK (remind_every_seconds IS NULL OR remind_every_seconds > 0);"#,
        },
        Expansion {
            table: "jobs",
            column: "started_millis",
            sql: "ALTER TABLE jobs ADD COLUMN started_millis INTEGER;",
        },
        Expansion {
            table: "jobs",
            column: "attention_revision",
            sql: r#"ALTER TABLE jobs ADD COLUMN attention_revision INTEGER NOT NULL DEFAULT 0
                CHECK (attention_revision >= 0);"#,
        },
        Expansion {
            table: "jobs",
            column: "runtime_warned_at",
            sql: "ALTER TABLE jobs ADD COLUMN runtime_warned_at TEXT;",
        },
        Expansion {
            table: "jobs",
            column: "output_warned_at",
            sql: "ALTER TABLE jobs ADD COLUMN output_warned_at TEXT;",
        },
        Expansion {
            table: "jobs",
            column: "last_reminded_at",
            sql: "ALTER TABLE jobs ADD COLUMN last_reminded_at TEXT;",
        },
        Expansion {
            table: "jobs",
            column: "next_reminder_at",
            sql: "ALTER TABLE jobs ADD COLUMN next_reminder_at TEXT;",
        },
        Expansion {
            table: "jobs",
            column: "reminder_tick",
            sql: r#"ALTER TABLE jobs ADD COLUMN reminder_tick INTEGER NOT NULL DEFAULT 0
                CHECK (reminder_tick >= 0);"#,
        },
        Expansion {
            table: "strand_inbox",
            column: "coalesce_key",
            sql: "ALTER TABLE strand_inbox ADD COLUMN coalesce_key TEXT;",
        },
        Expansion {
            table: "strand_inbox",
            column: "coalesce_revision",
            sql: r#"ALTER TABLE strand_inbox ADD COLUMN coalesce_revision INTEGER
                CHECK (coalesce_revision IS NULL OR coalesce_revision > 0);"#,
        },
        Expansion {
            table: "strand_inbox",
            column: "coalesce_causes",
            sql: "ALTER TABLE strand_inbox ADD COLUMN coalesce_causes TEXT;",
        },
    ];

    for change in changes {
        if table(conn, change.table)? && !column(conn, change.table, change.column)? {
            conn.execute_batch(change.sql)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn column(conn: &Connection, table: &str, name: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT COUNT(*) > 0 FROM pragma_table_info(?1) WHERE name = ?2",
        [table, name],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}
