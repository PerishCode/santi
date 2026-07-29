use std::collections::BTreeMap;

use super::{Origin, Service};
use crate::{environ, environment, service::address::Address};

impl Service {
    pub(super) async fn resolved_environment(
        &self,
        origin: &Origin<'_>,
    ) -> Result<BTreeMap<String, String>, String> {
        let mut declared = self
            .config
            .environment
            .iter()
            .map(|(name, value)| environment::Declaration {
                scope: "global".to_string(),
                name: name.clone(),
                value: value.clone(),
            })
            .collect::<Vec<_>>();
        for (scope, owner) in [
            (environ::Scope::Soul, origin.soul),
            (environ::Scope::Strand, origin.strand),
        ] {
            for variable in self.store.environs(scope, owner).await? {
                environment::legal(&variable.name)?;
                declared.push(environment::Declaration {
                    scope: scope.encode().to_string(),
                    name: variable.name,
                    value: variable.value,
                });
            }
        }
        let resolved = environment::resolve(declared, &|name| std::env::var(name).ok());
        for unresolved in resolved.unresolved {
            self.unresolved(
                Address {
                    strand: origin.strand,
                    turn: origin.turn,
                },
                unresolved,
            );
        }
        Ok(resolved.values)
    }
}
