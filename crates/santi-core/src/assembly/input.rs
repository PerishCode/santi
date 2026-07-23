use santi_provider::Item;

use crate::Store;

pub(crate) fn input(store: &Store, strand: &str) -> Result<Vec<Item>, String> {
    store.assembly(strand)
}
