use crate::assembly::system_prompt::{SystemPromptRequest, render_system_prompt};
use crate::{
    MaterialKind, MaterialRequest, MaterialUpdated, SantiStreamPayload, SessionMaterial,
    SoulProfile, Strand, timestamp_now,
};

use super::{MaterialCacheKey, SantiService};

const TEXT_PLAIN_UTF8: &str = "text/plain; charset=utf-8";

impl SantiService {
    pub fn session_material(
        &self,
        session_id: &str,
        request: MaterialRequest,
    ) -> Result<SessionMaterial, String> {
        match request.kind {
            MaterialKind::SystemPrompt => {
                // Operator-facing material view: the default soul's prompt.
                let strand = self
                    .store
                    .acquire_strand(self.store.default_soul_id(), session_id)?
                    .strand;
                let soul_profile = self
                    .store
                    .soul_profile(&strand.soul_id)?
                    .ok_or_else(|| "soul_profile not found".to_string())?;
                self.system_prompt_material(session_id, &strand, &soul_profile)
            }
        }
    }

    pub(super) fn system_prompt_text(
        &self,
        session_id: &str,
        strand_id: &str,
    ) -> Result<String, String> {
        // Load identity + memory from THIS strand's soul (not a hardcoded
        // default) so every soul speaks as itself.
        let strand = self
            .store
            .strand(strand_id)?
            .ok_or_else(|| "strand not found".to_string())?;
        let soul_profile = self
            .store
            .soul_profile(&strand.soul_id)?
            .ok_or_else(|| "soul_profile not found".to_string())?;
        Ok(self
            .system_prompt_material(session_id, &strand, &soul_profile)?
            .text)
    }

    fn system_prompt_material(
        &self,
        session_id: &str,
        strand: &Strand,
        soul_profile: &SoulProfile,
    ) -> Result<SessionMaterial, String> {
        let text = render_system_prompt(SystemPromptRequest {
            session_id,
            strand,
            soul_profile,
            soul_memory_path: self.soul_memory_file(&soul_profile.soul_id),
            session_memory_path: self.session_memory_file(session_id),
        })?;
        // Cache per (session, soul): two souls on one session must not collide.
        let key: MaterialCacheKey = (
            format!("{session_id}:{}", soul_profile.soul_id),
            MaterialKind::SystemPrompt,
        );
        let mut cache = self.material_cache.lock().unwrap();
        if let Some(existing) = cache.get(&key)
            && existing.text == text
        {
            return Ok(existing.clone());
        }

        let updated_at = timestamp_now();
        let material = SessionMaterial {
            session_id: session_id.to_string(),
            kind: MaterialKind::SystemPrompt,
            content_type: TEXT_PLAIN_UTF8.to_string(),
            text,
            updated_at: updated_at.clone(),
        };
        cache.insert(key, material.clone());
        drop(cache);

        self.publish_stream(
            session_id,
            SantiStreamPayload::MaterialUpdated {
                material: MaterialUpdated {
                    session_id: session_id.to_string(),
                    kind: MaterialKind::SystemPrompt,
                    updated_at,
                },
            },
        );
        Ok(material)
    }
}
