use santi_provider::{ProviderFunctionTool, ProviderTool};
use serde_json::json;

use crate::{SESSION_WORKSPACE_URI, SOUL_WORKSPACE_URI, session_memory_uri, soul_memory_uri};

pub(crate) fn provider_tools() -> Vec<ProviderTool> {
    let soul_memory_uri = soul_memory_uri();
    let session_memory_uri = session_memory_uri();
    vec![ProviderTool::Function(ProviderFunctionTool {
        name: "shell".to_string(),
        description: format!(
            "Run a shell command. By default commands run in the current execution workspace. Use cwd \"{SOUL_WORKSPACE_URI}\" to work in the current soul workspace, where {soul_memory_uri} is always rendered live in [santi-soul]. Use cwd \"{SESSION_WORKSPACE_URI}\" to work in the current session workspace, where {session_memory_uri} is always rendered live in [santi-session]. Unix-like systems use bash by default; Windows uses pwsh by default."
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
                    "description": format!("Optional workspace URI. Supports {SOUL_WORKSPACE_URI}, {SOUL_WORKSPACE_URI}<path>, {SESSION_WORKSPACE_URI}, and {SESSION_WORKSPACE_URI}<path>.")
                }
            },
            "required": ["command"],
            "additionalProperties": false
        }),
    })]
}
