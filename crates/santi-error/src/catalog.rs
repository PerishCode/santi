use super::{Category, Descriptor, Exposure, Retry, Severity};

pub const UNSAVED: Descriptor = Descriptor {
    code: "runtime.error_engine.persistence_failed",
    category: Category::Internal,
    severity: Severity::Error,
    retry: Retry::Later,
    exposure: Exposure::OPERATOR_ONLY,
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

pub const UNAVAILABLE: Descriptor = Descriptor {
    code: "runtime.unavailable",
    category: Category::Unavailable,
    severity: Severity::Error,
    retry: Retry::Later,
    exposure: Exposure::CALLER_AND_OPERATOR,
};

pub const INTERNAL: Descriptor = Descriptor {
    code: "runtime.internal",
    category: Category::Internal,
    severity: Severity::Error,
    retry: Retry::Later,
    exposure: Exposure::OPERATOR_ONLY,
};
