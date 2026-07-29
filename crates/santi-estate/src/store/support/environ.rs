use super::super::Store;
use super::read;
use keel::{Op, Rank, form};
use santi_model::environ;

#[derive(Clone, Copy)]
pub struct EnvironDraft<'a> {
    pub scope: environ::Scope,
    pub owner: &'a str,
    pub name: &'a str,
    pub value: &'a str,
    pub occurred: &'a str,
}

impl Store {
    pub async fn set_environ(&self, draft: EnvironDraft<'_>) -> Result<environ::Variable, String> {
        self.need_environ_owner(draft.scope, draft.owner).await?;
        self.core
            .batch(async |tx| {
                let held = tx
                    .one(
                        &form("Environ")
                            .when("scope", Op::Eq, draft.scope.encode())
                            .when("owner", Op::Eq, draft.owner)
                            .when("name", Op::Eq, draft.name),
                    )
                    .await?;
                match held {
                    Some(row) if row.text("value") == Some(draft.value) => {}
                    Some(row) => {
                        tx.set(
                            "Environ",
                            row.key(),
                            &[("value", draft.value), ("updated", draft.occurred)],
                        )
                        .await?;
                    }
                    None => {
                        tx.put(
                            "Environ",
                            &[
                                ("scope", draft.scope.encode()),
                                ("owner", draft.owner),
                                ("name", draft.name),
                                ("value", draft.value),
                                ("created", draft.occurred),
                                ("updated", draft.occurred),
                            ],
                        )
                        .await?;
                    }
                }
                Ok(())
            })
            .await
            .map_err(read::error)?;
        self.environ(draft.scope, draft.owner, draft.name)
            .await?
            .ok_or_else(|| "written environment variable missing".to_string())
    }

    pub async fn environ(
        &self,
        scope: environ::Scope,
        owner: &str,
        name: &str,
    ) -> Result<Option<environ::Variable>, String> {
        let row = self
            .core
            .one(
                &form("Environ")
                    .when("scope", Op::Eq, scope.encode())
                    .when("owner", Op::Eq, owner)
                    .when("name", Op::Eq, name),
            )
            .await
            .map_err(read::error)?;
        row.as_ref().map(variable).transpose()
    }

    pub async fn environs(
        &self,
        scope: environ::Scope,
        owner: &str,
    ) -> Result<Vec<environ::Variable>, String> {
        self.need_environ_owner(scope, owner).await?;
        let rows = self
            .core
            .ask(
                &form("Environ")
                    .when("scope", Op::Eq, scope.encode())
                    .when("owner", Op::Eq, owner)
                    .order("name", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        rows.rows().iter().map(variable).collect()
    }

    pub async fn end_environ(
        &self,
        scope: environ::Scope,
        owner: &str,
        name: &str,
    ) -> Result<bool, String> {
        self.need_environ_owner(scope, owner).await?;
        self.core
            .batch(async |tx| {
                let Some(row) = tx
                    .one(
                        &form("Environ")
                            .when("scope", Op::Eq, scope.encode())
                            .when("owner", Op::Eq, owner)
                            .when("name", Op::Eq, name),
                    )
                    .await?
                else {
                    return Ok(false);
                };
                tx.end("Environ", row.key()).await?;
                Ok(true)
            })
            .await
            .map_err(read::error)
    }

    async fn need_environ_owner(&self, scope: environ::Scope, owner: &str) -> Result<(), String> {
        if read::one(&self.core, scope.unit(), "tag", owner)
            .await?
            .is_some()
        {
            Ok(())
        } else {
            Err(format!("{} not found", scope.encode()))
        }
    }
}

fn variable(row: &keel::Row) -> Result<environ::Variable, String> {
    let scope = match read::text(row, "scope")? {
        "soul" => environ::Scope::Soul,
        "strand" => environ::Scope::Strand,
        value => return Err(format!("unknown environment scope {value}")),
    };
    Ok(environ::Variable {
        scope,
        owner: read::text(row, "owner")?.to_string(),
        name: read::text(row, "name")?.to_string(),
        value: read::text(row, "value")?.to_string(),
        created: read::text(row, "created")?.to_string(),
        updated: read::text(row, "updated")?.to_string(),
    })
}
