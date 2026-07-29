use super::Service;
use crate::{environ, environment};

impl Service {
    pub async fn environs(
        &self,
        scope: environ::Scope,
        owner: &str,
    ) -> Result<Vec<environ::Variable>, String> {
        self.store.environs(scope, owner.trim()).await
    }

    pub async fn set_environ(
        &self,
        scope: environ::Scope,
        owner: &str,
        request: environ::Draft,
    ) -> Result<environ::Variable, String> {
        let owner = owner.trim();
        let name = request.name.trim();
        environment::legal(name)?;
        self.store
            .set_environ(santi_estate::EnvironDraft {
                scope,
                owner,
                name,
                value: &request.value,
                occurred: &crate::now(),
            })
            .await
    }

    pub async fn end_environ(
        &self,
        scope: environ::Scope,
        owner: &str,
        name: &str,
    ) -> Result<bool, String> {
        let name = name.trim();
        environment::legal(name)?;
        self.store.end_environ(scope, owner.trim(), name).await
    }
}
