use super::{ErrorCategory, ErrorDescriptor, ErrorExposure, ErrorRetry, ErrorSeverity};

pub const CONTEXT_BUDGET_EXCEEDED: ErrorDescriptor = ErrorDescriptor {
    code: "context.budget.exceeded",
    category: ErrorCategory::ResourceExhausted,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::AfterResolution,
    exposure: ErrorExposure::CALLER_AND_OPERATOR,
};

pub const INBOX_CAPACITY_EXCEEDED: ErrorDescriptor = ErrorDescriptor {
    code: "runtime.inbox.capacity_exceeded",
    category: ErrorCategory::ResourceExhausted,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::Later,
    exposure: ErrorExposure::CALLER_AND_OPERATOR,
};

pub const EXECUTION_BUDGET_EXCEEDED: ErrorDescriptor = ErrorDescriptor {
    code: "runtime.execution_budget.exceeded",
    category: ErrorCategory::ResourceExhausted,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::AfterChange,
    exposure: ErrorExposure::CALLER_AND_OPERATOR,
};

pub const SOUL_MEMORY_INTERVENTION_REQUIRED: ErrorDescriptor = ErrorDescriptor {
    code: "runtime.soul_memory.intervention_required",
    category: ErrorCategory::ResourceExhausted,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::AfterChange,
    exposure: ErrorExposure::OPERATOR_ONLY,
};

pub const UPGRADE_FAILED: ErrorDescriptor = ErrorDescriptor {
    code: "runtime.upgrade.failed",
    category: ErrorCategory::Internal,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::AfterChange,
    exposure: ErrorExposure::OPERATOR_ONLY,
};

pub const UPGRADE_HANDOVER_FAILED: ErrorDescriptor = ErrorDescriptor {
    code: "runtime.upgrade.handover_failed",
    category: ErrorCategory::Internal,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::AfterChange,
    exposure: ErrorExposure::OPERATOR_ONLY,
};

pub const ERROR_ENGINE_PERSISTENCE_FAILED: ErrorDescriptor = ErrorDescriptor {
    code: "runtime.error_engine.persistence_failed",
    category: ErrorCategory::Internal,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::Later,
    exposure: ErrorExposure::OPERATOR_ONLY,
};

pub const PROVIDER_TURN_FAILED: ErrorDescriptor = ErrorDescriptor {
    code: "provider.turn.failed",
    category: ErrorCategory::Unavailable,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::Later,
    exposure: ErrorExposure::CALLER_AND_OPERATOR,
};

pub const RUNTIME_TURN_FAILED: ErrorDescriptor = ErrorDescriptor {
    code: "runtime.turn.failed",
    category: ErrorCategory::Internal,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::Later,
    exposure: ErrorExposure::CALLER_AND_OPERATOR,
};

pub const STRAND_DRIVE_FAILED: ErrorDescriptor = ErrorDescriptor {
    code: "runtime.strand.drive_failed",
    category: ErrorCategory::Unavailable,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::AfterResolution,
    exposure: ErrorExposure::CALLER_AND_OPERATOR,
};

pub const INVALID_ARGUMENT: ErrorDescriptor = ErrorDescriptor {
    code: "request.invalid_argument",
    category: ErrorCategory::InvalidInput,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::AfterChange,
    exposure: ErrorExposure::CALLER_AND_OPERATOR,
};

pub const NOT_FOUND: ErrorDescriptor = ErrorDescriptor {
    code: "resource.not_found",
    category: ErrorCategory::NotFound,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::AfterChange,
    exposure: ErrorExposure::CALLER_AND_OPERATOR,
};

pub const UNAUTHORIZED: ErrorDescriptor = ErrorDescriptor {
    code: "request.unauthorized",
    category: ErrorCategory::Unauthorized,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::AfterChange,
    exposure: ErrorExposure::CALLER_AND_OPERATOR,
};

pub const INTERNAL: ErrorDescriptor = ErrorDescriptor {
    code: "runtime.internal",
    category: ErrorCategory::Internal,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::Later,
    exposure: ErrorExposure::OPERATOR_ONLY,
};

pub const WINDOW_IDENTITY_MISSING: ErrorDescriptor = ErrorDescriptor {
    code: "window.identity.missing",
    category: ErrorCategory::Unauthorized,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::AfterResolution,
    exposure: ErrorExposure::CALLER_AND_OPERATOR,
};

pub const WINDOW_CONTENT_INVALID: ErrorDescriptor = ErrorDescriptor {
    code: "window.content.invalid",
    category: ErrorCategory::InvalidInput,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::AfterResolution,
    exposure: ErrorExposure::CALLER_AND_OPERATOR,
};

pub const WINDOW_CONTENT_OVERSIZE: ErrorDescriptor = ErrorDescriptor {
    code: "window.content.oversize",
    category: ErrorCategory::InvalidInput,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::AfterResolution,
    exposure: ErrorExposure::CALLER_AND_OPERATOR,
};

pub const WINDOW_RATE_LIMITED: ErrorDescriptor = ErrorDescriptor {
    code: "window.rate.limited",
    category: ErrorCategory::ResourceExhausted,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::Later,
    exposure: ErrorExposure::CALLER_AND_OPERATOR,
};

pub const WINDOW_MESSAGE_CONFLICT: ErrorDescriptor = ErrorDescriptor {
    code: "window.message.conflict",
    category: ErrorCategory::InvalidInput,
    severity: ErrorSeverity::Error,
    retry: ErrorRetry::AfterResolution,
    exposure: ErrorExposure::CALLER_AND_OPERATOR,
};
