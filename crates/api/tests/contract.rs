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
    render(api::cli::Cli::command(), "santi-api", &mut text);
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        hash,
        "440fac9d30926606b172b8c5c1d36f10363d58a625e8e19206d28eeff4547346"
    );
}
