use santi_provider::ProviderItem;

use crate::SantiStore;

pub(crate) fn provider_input(
    store: &SantiStore,
    soul_session_id: &str,
) -> Result<Vec<ProviderItem>, String> {
    store.assembly_input(soul_session_id)
}
