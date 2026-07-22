use crate::{SantiError, StrandMessage, Turn};

pub(crate) enum Outcome {
    Started(Turn, Vec<StrandMessage>),
    Running(Turn),
    Idle,
    Held(SantiError),
    Paused,
    Failed(SantiError),
}
