use anyhow::Result;

use super::Http;
use crate::cli::Environment;

pub(super) async fn run(http: &Http<'_>, base: &str, command: Environment) -> Result<()> {
    match command {
        Environment::List { scope, owner } => {
            http.get(&format!(
                "{base}/api/v1/{}/{owner}/environment",
                scope.path()
            ))
            .await
        }
        Environment::Set {
            scope,
            owner,
            name,
            value,
        } => {
            http.post(
                &format!("{base}/api/v1/{}/{owner}/environment", scope.path()),
                Some(serde_json::json!({
                    "name": name,
                    "value": value,
                })),
            )
            .await
        }
        Environment::End { scope, owner, name } => {
            http.delete(&format!(
                "{base}/api/v1/{}/{owner}/environment/{name}",
                scope.path()
            ))
            .await
        }
    }
}
