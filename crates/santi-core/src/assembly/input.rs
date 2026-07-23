use santi_provider::Item;

use crate::SantiStore;

pub(crate) fn provider_input(store: &SantiStore, strand: &str) -> Result<Vec<Item>, String> {
    store.assembly_input(strand)
}
