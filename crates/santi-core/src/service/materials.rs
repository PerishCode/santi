use crate::assembly::system_prompt::{SystemPromptRequest, render_system_prompt};
use crate::{
    MaterialKind, MaterialRequest, MaterialUpdated, SantiStreamPayload, SessionMaterial,
    SoulProfile, SoulSession, timestamp_now,
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
                let soul_session = self.store.acquire_soul_session(session_id)?.soul_session;
                let snapshot = self
                    .store
                    .runtime_snapshot(session_id)?
                    .ok_or_else(|| "session not found".to_string())?;
                let soul_profile = snapshot
                    .soul_profile
                    .ok_or_else(|| "soul_profile not found".to_string())?;
                self.system_prompt_material(session_id, &soul_session, &soul_profile)
            }
        }
    }

    pub(super) fn system_prompt_text(
        &self,
        session_id: &str,
        soul_session_id: &str,
    ) -> Result<String, String> {
        let snapshot = self
            .store
            .runtime_snapshot(session_id)?
            .ok_or_else(|| "session not found".to_string())?;
        let soul_session = snapshot
            .soul_session
            .ok_or_else(|| "soul_session not found".to_string())?;
        if soul_session.id != soul_session_id {
            return Err("soul_session mismatch".to_string());
        }
        let soul_profile = snapshot
            .soul_profile
            .ok_or_else(|| "soul_profile not found".to_string())?;
        Ok(self
            .system_prompt_material(session_id, &soul_session, &soul_profile)?
            .text)
    }

    fn system_prompt_material(
        &self,
        session_id: &str,
        soul_session: &SoulSession,
        soul_profile: &SoulProfile,
    ) -> Result<SessionMaterial, String> {
        let text = render_system_prompt(SystemPromptRequest {
            session_id,
            soul_session,
            soul_profile,
            soul_memory_path: self.soul_memory_file(),
            session_memory_path: self.session_memory_file(session_id),
        })?;
        let key: MaterialCacheKey = (session_id.to_string(), MaterialKind::SystemPrompt);
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
