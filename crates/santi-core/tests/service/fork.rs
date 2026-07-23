use super::support::*;
use santi_core::service::{self, Service};
use santi_core::{material, message};

#[test]
fn fork_syncs_workspace() {
    let temp = tempfile::tempdir().expect("temp dir");
    let runtime = temp.path().join("runtime");
    let service = Service::open(
        service::Config {
            database: temp.path().join("santi.sqlite").display().to_string(),
            runtime: runtime.display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service");
    let parent = service.weave().expect("create parent").strand;
    let base = runtime.join("strands").join(&parent.id).join("memory");
    fs::create_dir_all(base.join("notes")).expect("create parent workspace");
    fs::write(base.join("MEMORY.md"), "# Parent strand memory\n").expect("write memory");
    fs::write(base.join("notes/plan.md"), "plan v1\n").expect("write note");

    let child = service.fork(&parent.id).expect("fork").strand;
    let dir = runtime.join("strands").join(&child.id).join("memory");
    assert_eq!(
        fs::read_to_string(dir.join("MEMORY.md")).expect("child memory"),
        "# Parent strand memory\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("notes/plan.md")).expect("child note"),
        "plan v1\n"
    );

    fs::write(base.join("notes/plan.md"), "parent changed\n").expect("rewrite parent");
    assert_eq!(
        fs::read_to_string(dir.join("notes/plan.md")).expect("child note unchanged"),
        "plan v1\n"
    );
}

#[cfg(unix)]
#[test]
fn fork_rejects_symlink() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = temp.path().join("santi.sqlite");
    let runtime = temp.path().join("runtime");
    let service = Service::open(
        service::Config {
            database: db.display().to_string(),
            runtime: runtime.display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service");
    let parent = service.weave().expect("create parent").strand;
    {
        let store = Store::open(&db).expect("open store directly");
        let first = store
            .append_message(Draft {
                strand: &parent.id,
                actor: message::Role::System,
                id: store.system(),
                content: message::Content::text("first"),
                state: message::State::Fixed,
                intake: message::Intake::Record,
            })
            .expect("append first")
            .strand_message;
        let second = store
            .append_message(Draft {
                strand: &parent.id,
                actor: message::Role::System,
                id: store.system(),
                content: message::Content::text("second"),
                state: message::State::Fixed,
                intake: message::Intake::Record,
            })
            .expect("append second")
            .strand_message;
        store
            .compact(
                &parent.id,
                &first.message.id,
                &second.message.id,
                "parent compact",
            )
            .expect("parent compact");
    }
    let base = runtime.join("strands").join(&parent.id).join("memory");
    fs::create_dir_all(&base).expect("create parent workspace");
    fs::write(base.join("real.md"), "real\n").expect("write target");
    std::os::unix::fs::symlink("real.md", base.join("link.md")).expect("create symlink");

    let err = service
        .fork(&parent.id)
        .expect_err("symlink workspace should fail fork");
    assert!(
        err.contains("cannot copy symlink"),
        "unexpected error: {err}"
    );

    let strands = service.strands().expect("list strands");
    assert_eq!(strands.len(), 1, "fork child strand should be rolled back");
    assert_eq!(strands[0].id, parent.id);
    let runtime = runtime.join("strands");
    let child_dirs = fs::read_dir(&runtime)
        .expect("runtime strands dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name() != parent.id.as_str())
        .collect::<Vec<_>>();
    assert!(
        child_dirs.is_empty(),
        "failed fork should not leave child workspace dirs: {child_dirs:?}"
    );

    let conn = Connection::open(&db).expect("open sqlite");
    let child_strands: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM strands WHERE parent_strand_id = ?1",
            params![parent.id],
            |row| row.get(0),
        )
        .expect("count child strands");
    assert_eq!(child_strands, 0, "child strand row should be rolled back");
    let child_entries: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM r_strand_entries WHERE strand_id != ?1",
            params![parent.id],
            |row| row.get(0),
        )
        .expect("count child entries");
    assert_eq!(
        child_entries, 0,
        "child r_strand_entries should be rolled back"
    );
    let child_compacts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM compacts WHERE strand_id != ?1",
            params![parent.id],
            |row| row.get(0),
        )
        .expect("count child compacts");
    assert_eq!(child_compacts, 0, "child compacts should be rolled back");
}

#[test]
fn fork_prompt_topology() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = Service::open(
        service::Config {
            database: temp.path().join("santi.sqlite").display().to_string(),
            runtime: temp.path().join("runtime").display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service");
    let parent = service.weave().expect("create parent").strand;
    let store = Store::open(temp.path().join("santi.sqlite")).expect("open store directly");
    store
        .append_santi_system_message(
            &parent.id,
            message::Content::text("<system_message>\nkind: seed\n</system_message>"),
            message::Intake::Record,
        )
        .expect("append parent entry");
    drop(store);
    let child = service.fork(&parent.id).expect("fork").strand;

    let text = service
        .strand_material(
            &child.id,
            material::Request {
                kind: material::Kind::SystemPrompt,
            },
        )
        .expect("system prompt")
        .text;

    assert!(text.contains("[santi-fork]"));
    assert!(text.contains(&format!("parent: {}", parent.id)));
    assert!(text.contains("fork: 1"));
    assert!(!text.contains("merge"));
    assert!(!text.contains("sandbox"));
    assert!(!text.contains("recommend"));
}
