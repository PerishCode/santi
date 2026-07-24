use super::{Category, Descriptor, Exposure, Retry, Severity};

pub const CONTEXT_BUDGET_EXCEEDED: Descriptor = Descriptor {
    code: "context.budget.exceeded",
    category: Category::Exhausted,
    severity: Severity::Error,
    retry: Retry::Resolved,
    exposure: Exposure::CALLER_AND_OPERATOR,
};

pub const INBOX_CAPACITY_EXCEEDED: Descriptor = Descriptor {
    code: "runtime.inbox.capacity_exceeded",
    category: Category::Exhausted,
    severity: Severity::Error,
    retry: Retry::Later,
    exposure: Exposure::CALLER_AND_OPERATOR,
};

pub const EXECUTION_BUDGET_EXCEEDED: Descriptor = Descriptor {
    code: "runtime.execution_budget.exceeded",
    category: Category::Exhausted,
    severity: Severity::Error,
    retry: Retry::Changed,
    exposure: Exposure::CALLER_AND_OPERATOR,
};

pub const SOUL_MEMORY_INTERVENTION_REQUIRED: Descriptor = Descriptor {
    code: "runtime.soul_memory.intervention_required",
    category: Category::Exhausted,
    severity: Severity::Error,
    retry: Retry::Changed,
    exposure: Exposure::OPERATOR_ONLY,
};

pub const ERROR_ENGINE_PERSISTENCE_FAILED: Descriptor = Descriptor {
    code: "runtime.error_engine.persistence_failed",
    category: Category::Internal,
    severity: Severity::Error,
    retry: Retry::Later,
    exposure: Exposure::OPERATOR_ONLY,
};

pub const PROVIDER_TURN_FAILED: Descriptor = Descriptor {
    code: "provider.turn.failed",
    category: Category::Unavailable,
    severity: Severity::Error,
    retry: Retry::Later,
    exposure: Exposure::CALLER_AND_OPERATOR,
};

pub const RUNTIME_TURN_FAILED: Descriptor = Descriptor {
    code: "runtime.turn.failed",
    category: Category::Internal,
    severity: Severity::Error,
    retry: Retry::Later,
    exposure: Exposure::CALLER_AND_OPERATOR,
};

pub const STRAND_DRIVE_FAILED: Descriptor = Descriptor {
    code: "runtime.strand.drive_failed",
    category: Category::Unavailable,
    severity: Severity::Error,
    retry: Retry::Resolved,
    exposure: Exposure::CALLER_AND_OPERATOR,
};

pub const INVALID_ARGUMENT: Descriptor = Descriptor {
    code: "request.invalid_argument",
    category: Category::Invalid,
    severity: Severity::Error,
    retry: Retry::Changed,
    exposure: Exposure::CALLER_AND_OPERATOR,
};

pub const NOT_FOUND: Descriptor = Descriptor {
    code: "resource.not_found",
    category: Category::Missing,
    severity: Severity::Error,
    retry: Retry::Changed,
    exposure: Exposure::CALLER_AND_OPERATOR,
};

pub const UNAUTHORIZED: Descriptor = Descriptor {
    code: "request.unauthorized",
    category: Category::Unauthorized,
    severity: Severity::Error,
    retry: Retry::Changed,
    exposure: Exposure::CALLER_AND_OPERATOR,
};

pub const INTERNAL: Descriptor = Descriptor {
    code: "runtime.internal",
    category: Category::Internal,
    severity: Severity::Error,
    retry: Retry::Later,
    exposure: Exposure::OPERATOR_ONLY,
};

pub const WINDOW_IDENTITY_MISSING: Descriptor = Descriptor {
    code: "window.identity.missing",
    category: Category::Unauthorized,
    severity: Severity::Error,
    retry: Retry::Resolved,
    exposure: Exposure::CALLER_AND_OPERATOR,
};

pub const WINDOW_CONTENT_INVALID: Descriptor = Descriptor {
    code: "window.content.invalid",
    category: Category::Invalid,
    severity: Severity::Error,
    retry: Retry::Resolved,
    exposure: Exposure::CALLER_AND_OPERATOR,
};

pub const WINDOW_CONTENT_OVERSIZE: Descriptor = Descriptor {
    code: "window.content.oversize",
    category: Category::Invalid,
    severity: Severity::Error,
    retry: Retry::Resolved,
    exposure: Exposure::CALLER_AND_OPERATOR,
};

pub const WINDOW_RATE_LIMITED: Descriptor = Descriptor {
    code: "window.rate.limited",
    category: Category::Exhausted,
    severity: Severity::Error,
    retry: Retry::Later,
    exposure: Exposure::CALLER_AND_OPERATOR,
};

pub const WINDOW_MESSAGE_CONFLICT: Descriptor = Descriptor {
    code: "window.message.conflict",
    category: Category::Invalid,
    severity: Severity::Error,
    retry: Retry::Resolved,
    exposure: Exposure::CALLER_AND_OPERATOR,
};
