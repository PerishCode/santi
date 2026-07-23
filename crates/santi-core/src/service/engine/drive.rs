use crate::message;
use crate::{Fault, turn::Turn};

pub(crate) enum Outcome {
    Started(Turn, Vec<message::Placed>),
    Running(Turn),
    Idle,
    Held(Fault),
    Paused,
    Failed(Fault),
}
