use clap::CommandFactory;
use sha2::{Digest, Sha256};

fn render(mut command: clap::Command, path: &str, text: &mut String) {
    let children: Vec<_> = command.get_subcommands().cloned().collect();
    text.push_str(path);
    text.push('\n');
    text.push_str(&command.render_long_help().to_string());
    text.push('\n');
    for child in children {
        let name = child.get_name().to_string();
        render(child, &format!("{path} {name}"), text);
    }
}

#[test]
fn cli() {
    let mut text = String::new();
    render(santi::cli::Cli::command(), "santi", &mut text);
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "4ded98a6dfaa9e76315287a0a41df8f09647b7fbf90b6a27e6fb30343439da1e"
    );
}
