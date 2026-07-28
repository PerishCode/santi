use keel::atom::{int, string};
use keel::resource;

#[resource]
pub(crate) struct ErrorIncident {
    #[field(string, unique)]
    tag: string,
    #[field(string, unique)]
    incident_key: string,
    #[field(string)]
    code: string,
    #[field(
        string,
        values = (
            "exhausted",
            "internal",
            "invalid",
            "missing",
            "unauthorized",
            "unavailable"
        )
    )]
    category: string,
    #[field(string, values = ("error",))]
    severity: string,
    #[field(string, values = ("changed", "later", "never", "resolved"))]
    retry: string,
    #[field(string)]
    exposure: string,
    #[field(string)]
    scope_kind: string,
    #[field(string)]
    scope_id: string,
    #[field(string)]
    source_component: string,
    #[field(string)]
    source_operation: string,
    #[field(string)]
    latest_source_component: string,
    #[field(string)]
    latest_source_operation: string,
    #[field(string)]
    message: string,
    #[field(string)]
    latest_message: string,
    #[field(string)]
    context: string,
    #[field(string)]
    latest_context: string,
    #[field(int, min = 1)]
    occurrence_count: int,
    #[field(int, min = 1)]
    revision: int,
    #[field(string)]
    first_seen: string,
    #[field(string)]
    last_seen: string,
}

#[resource(frozen)]
pub(crate) struct ResolvedIncident {
    #[field(string, unique)]
    tag: string,
    #[field(string)]
    incident_key: string,
    #[field(string)]
    code: string,
    #[field(
        string,
        values = (
            "exhausted",
            "internal",
            "invalid",
            "missing",
            "unauthorized",
            "unavailable"
        )
    )]
    category: string,
    #[field(string, values = ("error",))]
    severity: string,
    #[field(string, values = ("changed", "later", "never", "resolved"))]
    retry: string,
    #[field(string)]
    exposure: string,
    #[field(string)]
    scope_kind: string,
    #[field(string)]
    scope_id: string,
    #[field(string)]
    source_component: string,
    #[field(string)]
    source_operation: string,
    #[field(string)]
    latest_source_component: string,
    #[field(string)]
    latest_source_operation: string,
    #[field(string)]
    message: string,
    #[field(string)]
    latest_message: string,
    #[field(string)]
    context: string,
    #[field(string)]
    latest_context: string,
    #[field(int, min = 1)]
    occurrence_count: int,
    #[field(int, min = 1)]
    revision: int,
    #[field(string)]
    first_seen: string,
    #[field(string)]
    last_seen: string,
    #[field(string)]
    resolved: string,
    #[field(string, opt)]
    resolved_by: string,
}

#[resource(frozen)]
pub(crate) struct ErrorTransition {
    #[field(string, unique)]
    tag: string,
    #[field(string)]
    incident: string,
    #[field(int, unique = incident, min = 1)]
    revision: int,
    #[field(string, values = ("opened", "resolved"))]
    kind: string,
    #[field(string)]
    payload: string,
    #[field(string)]
    created: string,
}

#[resource(frozen)]
pub(crate) struct ErrorAcknowledgement {
    #[field(string)]
    delivered: string,
    #[relation(ErrorTransition, one2one, root)]
    transition: ErrorTransition,
}
