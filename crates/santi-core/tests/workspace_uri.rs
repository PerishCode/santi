use std::path::PathBuf;

use santi_core::{SOULSPACE, STRANDSPACE, housed, parsed, soulward, strandward};

#[test]
fn builds_memory_uris() {
    assert_eq!(soulward(), "soul://MEMORY.md");
    assert_eq!(strandward(), "strand://MEMORY.md");
    assert_eq!(housed(SOULSPACE, "notes/today.md"), "soul://notes/today.md");
    assert_eq!(housed(STRANDSPACE, "/todo.md"), "strand://todo.md");
}

#[test]
fn parses_workspace_roots() {
    let soul = parsed(SOULSPACE).expect("soul root");
    assert_eq!(soul.root, santi_core::workspace::Root::Soul);
    assert_eq!(soul.path, PathBuf::new());

    let strand = parsed("strand://notes/today.md").expect("strand path");
    assert_eq!(strand.root, santi_core::workspace::Root::Strand);
    assert_eq!(strand.path, PathBuf::from("notes/today.md"));
}

#[test]
fn rejects_old_aliases() {
    assert_eq!(
        parsed("@soul").expect_err("old soul alias"),
        "unsupported workspace alias: @soul; use soul:// or strand://"
    );
    assert_eq!(
        parsed("@strand").expect_err("old strand alias"),
        "unsupported workspace alias: @strand; use soul:// or strand://"
    );
}

#[test]
fn rejects_invalid_uris() {
    assert_eq!(
        parsed("file://tmp").expect_err("unknown scheme"),
        "unsupported workspace uri: file://tmp"
    );
    assert_eq!(
        parsed("relative/path").expect_err("relative path"),
        "cwd must use soul:// or strand://"
    );
    assert_eq!(
        parsed("soul://../secret").expect_err("escape"),
        "workspace uri cannot escape soul://"
    );
}
