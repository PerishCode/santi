use santi_provider::ProviderItem;

use crate::SantiStore;

pub(crate) fn provider_input(
    store: &SantiStore,
    strand_id: &str,
) -> Result<Vec<ProviderItem>, String> {
    store.assembly_input(strand_id)
}
