use crate::{Category, Kind, Retry, Severity, Status};

impl Category {
    pub fn db(self) -> &'static str {
        match self {
            Category::Internal => "internal",
            Category::Invalid => "invalid",
            Category::Missing => "missing",
            Category::Exhausted => "exhausted",
            Category::Unauthorized => "unauthorized",
            Category::Unavailable => "unavailable",
        }
    }

    pub fn read(value: &str) -> Self {
        match value {
            "invalid" => Category::Invalid,
            "missing" => Category::Missing,
            "exhausted" => Category::Exhausted,
            "unauthorized" => Category::Unauthorized,
            "unavailable" => Category::Unavailable,
            _ => Category::Internal,
        }
    }
}

impl Severity {
    pub fn db(self) -> &'static str {
        match self {
            Severity::Error => "error",
        }
    }

    pub fn read(_value: &str) -> Self {
        Severity::Error
    }
}

impl Retry {
    pub fn db(self) -> &'static str {
        match self {
            Retry::Never => "never",
            Retry::Later => "later",
            Retry::Changed => "changed",
            Retry::Resolved => "resolved",
        }
    }

    pub fn read(value: &str) -> Self {
        match value {
            "never" => Retry::Never,
            "changed" => Retry::Changed,
            "resolved" => Retry::Resolved,
            _ => Retry::Later,
        }
    }
}

impl Status {
    pub fn db(&self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Resolved => "resolved",
        }
    }

    pub fn read(value: &str) -> Self {
        match value {
            "resolved" => Status::Resolved,
            _ => Status::Active,
        }
    }
}

impl Kind {
    pub fn db(&self) -> &'static str {
        match self {
            Kind::Opened => "opened",
            Kind::Resolved => "resolved",
        }
    }
}
