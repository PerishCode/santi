#[derive(Debug, Clone)]
pub(in crate::service) struct Address<T> {
    pub(in crate::service) strand_id: T,
    pub(in crate::service) turn_id: T,
}

impl Address<&str> {
    pub(in crate::service) fn owned(&self) -> Address<String> {
        Address {
            strand_id: self.strand_id.to_owned(),
            turn_id: self.turn_id.to_owned(),
        }
    }
}
