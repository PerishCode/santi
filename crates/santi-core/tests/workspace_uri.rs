use std::path::PathBuf;

use santi_core::{
    SESSION_WORKSPACE_URI, SOUL_WORKSPACE_URI, WorkspaceRoot, parse_workspace_uri,
    session_memory_uri, soul_memory_uri, workspace_uri,
};

#[test]
fn builds_memory_uris() {
    assert_eq!(soul_memory_uri(), "soul://MEMORY.md");
    assert_eq!(session_memory_uri(), "session://MEMORY.md");
    assert_eq!(
        workspace_uri(SOUL_WORKSPACE_URI, "notes/today.md"),
        "soul://notes/today.md"
    );
    assert_eq!(
        workspace_uri(SESSION_WORKSPACE_URI, "/todo.md"),
        "session://todo.md"
    );
}

#[test]
fn parses_workspace_roots() {
    let soul = parse_workspace_uri(SOUL_WORKSPACE_URI).expect("soul root");
    assert_eq!(soul.root, WorkspaceRoot::Soul);
    assert_eq!(soul.path, PathBuf::new());

    let session = parse_workspace_uri("session://notes/today.md").expect("session path");
    assert_eq!(session.root, WorkspaceRoot::Session);
    assert_eq!(session.path, PathBuf::from("notes/today.md"));
}

#[test]
fn rejects_old_aliases() {
    assert_eq!(
        parse_workspace_uri("@soul").expect_err("old soul alias"),
        "unsupported workspace alias: @soul; use soul:// or session://"
    );
    assert_eq!(
        parse_workspace_uri("@session").expect_err("old session alias"),
        "unsupported workspace alias: @session; use soul:// or session://"
    );
}

#[test]
fn rejects_invalid_uris() {
    assert_eq!(
        parse_workspace_uri("file://tmp").expect_err("unknown scheme"),
        "unsupported workspace uri: file://tmp"
    );
    assert_eq!(
        parse_workspace_uri("relative/path").expect_err("relative path"),
        "cwd must use soul:// or session://"
    );
    assert_eq!(
        parse_workspace_uri("soul://../secret").expect_err("escape"),
        "workspace uri cannot escape soul://"
    );
}
