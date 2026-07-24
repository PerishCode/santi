impl crate::effect::State {
    pub fn encode(&self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Dispatching => "dispatching",
            Self::Unknown => "unknown",
            Self::Settled(crate::effect::Outcome::Applied) => "settled_applied",
            Self::Settled(crate::effect::Outcome::NotApplied) => "settled_not_applied",
        }
    }

    pub fn decode(value: &str) -> Self {
        match value {
            "prepared" => Self::Prepared,
            "dispatching" => Self::Dispatching,
            "settled_applied" => Self::Settled(crate::effect::Outcome::Applied),
            "settled_not_applied" => Self::Settled(crate::effect::Outcome::NotApplied),
            _ => Self::Unknown,
        }
    }
}

impl crate::message::Role {
    pub fn encode(&self) -> &'static str {
        match self {
            Self::Soul => "soul",
            Self::System => "system",
        }
    }

    pub fn decode(value: &str) -> Self {
        match value {
            "soul" => Self::Soul,
            _ => Self::System,
        }
    }
}

impl crate::message::State {
    pub fn encode(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Fixed => "fixed",
            Self::Aborted => "aborted",
        }
    }

    pub fn decode(value: &str) -> Self {
        match value {
            "pending" => Self::Pending,
            "fixed" => Self::Fixed,
            "aborted" => Self::Aborted,
            _ => Self::Fixed,
        }
    }
}

impl crate::message::Kind {
    pub fn encode(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::SantiSystem => "santi_system",
        }
    }

    pub fn decode(value: &str) -> Self {
        match value {
            "text" => Self::Text,
            "santi_system" => Self::SantiSystem,
            _ => Self::Text,
        }
    }
}

impl crate::turn::Trigger {
    pub fn decode(value: &str) -> Self {
        match value {
            "strand_send" => Self::StrandSend,
            "system" => Self::System,
            _ => Self::System,
        }
    }
}

impl crate::turn::Status {
    pub fn decode(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => Self::Failed,
        }
    }
}

impl crate::thinking::State {
    pub fn encode(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub fn decode(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => Self::Failed,
        }
    }
}

impl crate::thinking::Reason {
    pub fn encode(&self) -> &'static str {
        match self {
            Self::Spoke => "spoke",
            Self::Called => "called",
            Self::Finished => "finished",
        }
    }

    pub fn decode(value: &str) -> Self {
        match value {
            "spoke" => Self::Spoke,
            "called" => Self::Called,
            "finished" => Self::Finished,
            _ => Self::Finished,
        }
    }
}

impl crate::strand::Target {
    pub fn encode(&self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Compact => "compact",
            Self::Thinking => "thinking",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
        }
    }
}

impl crate::receipt::State {
    pub fn encode(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Recovered => "recovered",
            Self::Driving => "driving",
            Self::Failed => "failed",
            Self::Completed => "completed",
        }
    }

    pub fn decode(value: &str) -> Result<Self, String> {
        match value {
            "accepted" => Ok(Self::Accepted),
            "recovered" => Ok(Self::Recovered),
            "driving" => Ok(Self::Driving),
            "failed" => Ok(Self::Failed),
            "completed" => Ok(Self::Completed),
            other => Err(format!("unknown receipt state {other}")),
        }
    }
}
