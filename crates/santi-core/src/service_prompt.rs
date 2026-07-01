use santi_provider::{ProviderFunctionTool, ProviderTool};
use serde_json::json;

use crate::{SOUL_WORKSPACE_URI, STRAND_WORKSPACE_URI, soul_memory_uri, strand_memory_uri};

pub(crate) fn provider_tools() -> Vec<ProviderTool> {
    let soul_memory_uri = soul_memory_uri();
    let strand_memory_uri = strand_memory_uri();
    vec![ProviderTool::Function(ProviderFunctionTool {
        name: "shell".to_string(),
        description: format!(
            "Run a shell command. By default commands run in the current execution workspace. Use cwd \"{SOUL_WORKSPACE_URI}\" to work in the current soul workspace, where {soul_memory_uri} is always rendered live in [santi-soul]. Use cwd \"{STRAND_WORKSPACE_URI}\" to work in the current strand workspace, where {strand_memory_uri} is always rendered live in [santi-strand]. Unix-like systems use bash by default; Windows uses pwsh by default."
        ),
        parameters: json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute."
                },
                "cwd": {
                    "type": "string",
                    "description": format!("Optional workspace URI. Supports {SOUL_WORKSPACE_URI}, {SOUL_WORKSPACE_URI}<path>, {STRAND_WORKSPACE_URI}, and {STRAND_WORKSPACE_URI}<path>.")
                }
            },
            "required": ["command"],
            "additionalProperties": false
        }),
    })]
}
