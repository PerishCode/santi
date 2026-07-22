use crate::assembly::prompt::{SystemPromptRequest, render_system_prompt};
use crate::{
    MaterialKind, MaterialRequest, MaterialUpdated, SantiStreamPayload, Strand, StrandMaterial,
    timestamp_now,
};

use super::Service;

pub(in crate::service) type Key = (String, MaterialKind);

const TEXT_PLAIN_UTF8: &str = "text/plain; charset=utf-8";

impl Service {
    pub fn strand_material(
        &self,
        strand_id: &str,
        request: MaterialRequest,
    ) -> Result<StrandMaterial, String> {
        match request.kind {
            MaterialKind::SystemPrompt => {
                let strand = self
                    .store
                    .strand(strand_id)?
                    .ok_or_else(|| "strand not found".to_string())?;
                self.system_prompt_material(&strand)
            }
        }
    }

    pub(in crate::service) fn system_prompt_text(&self, strand_id: &str) -> Result<String, String> {
        let strand = self
            .store
            .strand(strand_id)?
            .ok_or_else(|| "strand not found".to_string())?;
        Ok(self.system_prompt_material(&strand)?.text)
    }

    fn system_prompt_material(&self, strand: &Strand) -> Result<StrandMaterial, String> {
        let strand_id = strand.id.as_str();
        let text = render_system_prompt(SystemPromptRequest {
            strand_id,
            strand,
            constitution_path: self.constitution_file(),
            soul_memory_path: self.soul_memory_file(&strand.soul_id),
            strand_memory_path: self.strand_memory_file(strand_id),
            soul_memory_allowance_bytes: self.soul_memory_policy().allowance_bytes,
            is_default_soul: strand.soul_id == self.store.default_soul_id(),
        })?;
        let key: Key = (strand_id.to_string(), MaterialKind::SystemPrompt);
        let mut cache = self.material_cache.lock().unwrap();
        if let Some(existing) = cache.get(&key)
            && existing.text == text
        {
            return Ok(existing.clone());
        }

        let updated_at = timestamp_now();
        let material = StrandMaterial {
            strand_id: strand_id.to_string(),
            kind: MaterialKind::SystemPrompt,
            content_type: TEXT_PLAIN_UTF8.to_string(),
            text,
            updated_at: updated_at.clone(),
        };
        cache.insert(key, material.clone());
        drop(cache);

        self.publish_stream(
            strand_id,
            SantiStreamPayload::MaterialUpdated {
                material: MaterialUpdated {
                    strand_id: strand_id.to_string(),
                    kind: MaterialKind::SystemPrompt,
                    updated_at,
                },
            },
        );
        Ok(material)
    }
}
