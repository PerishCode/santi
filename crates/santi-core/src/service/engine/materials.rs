use crate::assembly::prompt::{Prompting, prompted};
use crate::{now, strand::Strand};

use super::Service;
use crate::{material, stream};

pub(in crate::service) type Key = (String, material::Kind);

const PLAIN: &str = "text/plain; charset=utf-8";

impl Service {
    pub fn material(
        &self,
        strand: &str,
        request: material::Request,
    ) -> Result<material::Material, String> {
        match request.kind {
            material::Kind::SystemPrompt => {
                let strand = self
                    .store
                    .strand(strand)?
                    .ok_or_else(|| "strand not found".to_string())?;
                self.prompt(&strand)
            }
        }
    }

    pub(in crate::service) fn wording(&self, strand: &str) -> Result<String, String> {
        let strand = self
            .store
            .strand(strand)?
            .ok_or_else(|| "strand not found".to_string())?;
        Ok(self.prompt(&strand)?.text)
    }

    fn prompt(&self, strand: &Strand) -> Result<material::Material, String> {
        let id = strand.id.as_str();
        let text = prompted(Prompting {
            id,
            strand,
            constitution: self.charter(),
            memoir: self.memoir(&strand.soul),
            journal: self.journal(id),
            allowance: self.regimen().allowance,
            genesis: strand.soul == self.store.genesis(),
        })?;
        let key: Key = (id.to_string(), material::Kind::SystemPrompt);
        let mut cache = self.materials.lock().unwrap();
        if let Some(existing) = cache.get(&key)
            && existing.text == text
        {
            return Ok(existing.clone());
        }

        let updated = now();
        let material = material::Material {
            strand: id.to_string(),
            kind: material::Kind::SystemPrompt,
            content_type: PLAIN.to_string(),
            text,
            updated: updated.clone(),
        };
        cache.insert(key, material.clone());
        drop(cache);

        self.publish(
            id,
            stream::Payload::MaterialUpdated {
                material: material::Updated {
                    strand: id.to_string(),
                    kind: material::Kind::SystemPrompt,
                    updated,
                },
            },
        );
        Ok(material)
    }
}
