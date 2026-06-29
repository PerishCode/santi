use santi_provider::ProviderMessage;

use crate::SantiStore;

pub(crate) fn provider_messages(
    store: &SantiStore,
    soul_session_id: &str,
) -> Result<Vec<ProviderMessage>, String> {
    Ok(store
        .assembly_input(soul_session_id)?
        .into_iter()
        .map(|message| ProviderMessage {
            role: message.role,
            content: message.content,
        })
        .collect())
}
