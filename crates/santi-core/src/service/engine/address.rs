#[derive(Debug, Clone)]
pub(in crate::service) struct Address<T> {
    pub(in crate::service) strand: T,
    pub(in crate::service) turn: T,
}

impl Address<&str> {
    pub(in crate::service) fn owned(&self) -> Address<String> {
        Address {
            strand: self.strand.to_owned(),
            turn: self.turn.to_owned(),
        }
    }
}
