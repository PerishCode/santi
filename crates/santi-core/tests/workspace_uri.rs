use std::path::PathBuf;

use santi_core::{
    SOUL_WORKSPACE_URI, STRAND_WORKSPACE_URI, WorkspaceRoot, parse_workspace_uri, soul_memory_uri,
    strand_memory_uri, workspace_uri,
};

#[test]
fn builds_memory_uris() {
    assert_eq!(soul_memory_uri(), "soul://MEMORY.md");
    assert_eq!(strand_memory_uri(), "strand://MEMORY.md");
    assert_eq!(
        workspace_uri(SOUL_WORKSPACE_URI, "notes/today.md"),
        "soul://notes/today.md"
    );
    assert_eq!(
        workspace_uri(STRAND_WORKSPACE_URI, "/todo.md"),
        "strand://todo.md"
    );
}

#[test]
fn parses_workspace_roots() {
    let soul = parse_workspace_uri(SOUL_WORKSPACE_URI).expect("soul root");
    assert_eq!(soul.root, WorkspaceRoot::Soul);
    assert_eq!(soul.path, PathBuf::new());

    let strand = parse_workspace_uri("strand://notes/today.md").expect("strand path");
    assert_eq!(strand.root, WorkspaceRoot::Strand);
    assert_eq!(strand.path, PathBuf::from("notes/today.md"));
}

#[test]
fn rejects_old_aliases() {
    assert_eq!(
        parse_workspace_uri("@soul").expect_err("old soul alias"),
        "unsupported workspace alias: @soul; use soul:// or strand://"
    );
    assert_eq!(
        parse_workspace_uri("@strand").expect_err("old strand alias"),
        "unsupported workspace alias: @strand; use soul:// or strand://"
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
        "cwd must use soul:// or strand://"
    );
    assert_eq!(
        parse_workspace_uri("soul://../secret").expect_err("escape"),
        "workspace uri cannot escape soul://"
    );
}
