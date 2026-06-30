use santi_provider::ProviderMessage;

use crate::SantiStore;

pub(crate) fn provider_messages(
    store: &SantiStore,
    soul_session_id: &str,
    tools_through_seq: i64,
) -> Result<Vec<ProviderMessage>, String> {
    store.assembly_input(soul_session_id, tools_through_seq)
}
