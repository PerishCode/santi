use crate::{Fault, StrandMessage, Turn};

pub(crate) enum Outcome {
    Started(Turn, Vec<StrandMessage>),
    Running(Turn),
    Idle,
    Held(Fault),
    Paused,
    Failed(Fault),
}
