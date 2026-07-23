use santi_core::{
    CreateSoulRequest, CreateStrandResponse, CreateWebhookRequest, Fault, ForkStrandResponse,
    HealthResponse, MaterialRequest, SendStrandAcceptedResponse, SendStrandRequest, Soul, Strand,
    StrandDetail, StrandMaterial, StrandRuntimeSnapshot, WebhookSubscription,
};
use utoipa::{
    Modify, OpenApi,
    openapi::{
        RefOr,
        schema::Schema,
        security::{Http, HttpAuthScheme, SecurityScheme},
    },
};

mod descriptions;
use descriptions::COMPONENT_DESCRIPTIONS;

const PROPERTY_DESCRIPTIONS: [(&str, &str, &str); 12] = [
    (
        "CreateDownstreamRequest",
        "digest",
        "Lowercase or uppercase SHA-256 hex of a high-entropy Bearer token. The digest is stored but never returned.",
    ),
    (
        "IngestRequest",
        "request",
        "Stable idempotency key, unique within the authenticated downstream. Reuse with a changed payload is a conflict.",
    ),
    (
        "TurnEventBatch",
        "cursor",
        "Opaque global high-water mark. Persist it even when events is empty; it reveals aggregate activity but no foreign payload.",
    ),
    (
        "HealthResponse",
        "incidents",
        "Aggregate only: `/health` is public and must never expose strand,\nreceipt, or incident locators.",
    ),
    (
        "Strand",
        "label",
        "Opaque external anchor (e.g. a webhook thread key). Unique per soul;\nabsent for strands reached only by id (e.g. CLI-created ones).",
    ),
    (
        "CreateWebhookRequest",
        "strategy",
        "`per_thread` (default) or `single`.",
    ),
    (
        "SendStrandAcceptedResponse",
        "message",
        "The content this send just enqueued, once the driver has actually\ncommitted it to the timeline. Absent when this send coalesced into an\nalready-running turn — durably enqueued, but the driver has not drained\nit yet (it will, when that turn completes and re-pokes).",
    ),
    (
        "ReceiptTransition",
        "rebuilt",
        "Present only when schema migration reconstructed this evidence from a\ndurable v24 source row. Live transitions leave it unset.",
    ),
    (
        "ReceiptStatus",
        "effects",
        "Per-attempt shell effects reached by any turn carrying this receipt.\nCompletion alone does not imply that any listed external effect applied.",
    ),
    (
        "StrandEffect",
        "call",
        "Absent only for an imported legacy row whose old schema had no neutral\ntool-call locator.",
    ),
    (
        "EffectTransition",
        "evidence",
        "Human- or runtime-supplied evidence. This is never interpreted as proof\nof idempotency by core.",
    ),
    (
        "EffectStatus",
        "receipts",
        "Obligation roots whose attempts include this effect's turn.",
    ),
];

#[derive(OpenApi)]
#[openapi(
    modifiers(&SecurityAddon),
    paths(
        super::routes::health,
        super::routes::create_strand,
        super::routes::list_strands,
        super::routes::create_soul,
        super::routes::list_souls,
        super::routes::get_soul,
        super::routes::create_webhook,
        super::routes::list_webhooks,
        super::ingress::ingest_webhook,
        super::routes::get_strand,
        super::routes::list_messages,
        super::routes::strand_material,
        super::routes::send_strand,
        super::routes::drive_strand,
        super::routes::fork_strand,
        super::routes::compact_exec,
        super::routes::compact_query,
        super::routes::strand_budget,
        super::errors::strand_errors,
        super::errors::errors,
        super::routes::receipt_status,
        super::effects::effect_status,
        super::effects::resolve_effect,
        super::sse::error_events,
        super::sse::turn_event_stream,
        super::routes::runtime_snapshot,
        super::routes::turn_events,
        super::routes::create_downstream,
        super::routes::list_downstreams,
        super::routes::ingest,
        crate::bucket::get_bucket_object
    ),
    components(schemas(
        CreateStrandResponse,
        santi_core::TurnEvent,
        santi_core::TurnEventBatch,
        santi_core::DownstreamCredential,
        santi_core::CreateDownstreamRequest,
        santi_core::IngestRequest,
        ForkStrandResponse,
        CreateSoulRequest,
        CreateWebhookRequest,
        WebhookSubscription,
        Fault,
        HealthResponse,
        MaterialRequest,
        SendStrandRequest,
        SendStrandAcceptedResponse,
        santi_core::DriveStrandResponse,
        santi_core::DriveStrandState,
        santi_core::IngestReceipt,
        santi_core::ReceiptState,
        santi_core::ReceiptStatus,
        santi_core::ReceiptTransition,
        santi_core::EffectState,
        santi_core::EffectTransitionReason,
        santi_core::EffectTransition,
        santi_core::EffectStatus,
        santi_core::EffectResolutionOutcome,
        super::effects::ResolveEffectRequest,
        StrandDetail,
        StrandMaterial,
        StrandRuntimeSnapshot,
        Soul,
        Strand,
        santi_core::ActorType,
        santi_core::Compact,
        santi_core::CompactCapsuleOptions,
        santi_core::CompactExecRequest,
        santi_core::CompactExecResponse,
        santi_core::CompactQueryEntry,
        santi_core::CompactQueryResponse,
        santi_core::ContextBudget,
        santi_core::ContextEstimate,
        santi_core::Category,
        santi_core::Exposure,
        santi_core::Incident,
        santi_core::Retry,
        santi_core::Scope,
        santi_core::Severity,
        santi_core::Source,
        santi_core::Transition,
        santi_core::Kind,
        santi_core::Status,
        santi_core::StrandBudgetSnapshot,
        santi_core::StrandTargetType,
        santi_core::Message,
        santi_core::MessageContent,
        santi_core::MessagePart,
        santi_core::MessageState,
        santi_core::MaterialKind,
        santi_core::MaterialUpdated,
        santi_core::StrandEffect,
        santi_core::StrandMessage,
        santi_core::StrandMessageRef,
        santi_core::ThinkingSpan,
        santi_core::ThinkingSpanState,
        santi_core::ToolCall,
        santi_core::ToolResult,
        santi_core::Turn,
        santi_core::TurnActivity,
        santi_core::TurnActivityState,
        santi_core::TurnStatus,
        santi_core::TurnTriggerType
    ))
)]
struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, document: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = document.components.as_mut() {
            components.add_security_scheme(
                "downstream_bearer",
                SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
            );
        }
    }
}

pub(super) fn document() -> utoipa::openapi::OpenApi {
    let mut document = ApiDoc::openapi();
    for (schema, description) in COMPONENT_DESCRIPTIONS {
        describe_component(&mut document, schema, description);
    }
    for (schema, property, description) in PROPERTY_DESCRIPTIONS {
        describe_property(&mut document, schema, property, description);
    }
    document
}

fn describe_component(
    document: &mut utoipa::openapi::OpenApi,
    schema_name: &str,
    description: &str,
) {
    let Some(schema) = document
        .components
        .as_mut()
        .and_then(|components| components.schemas.get_mut(schema_name))
    else {
        return;
    };
    set_description(schema, description);
}

fn describe_property(
    document: &mut utoipa::openapi::OpenApi,
    schema_name: &str,
    property_name: &str,
    description: &str,
) {
    let Some(RefOr::T(Schema::Object(schema))) = document
        .components
        .as_mut()
        .and_then(|components| components.schemas.get_mut(schema_name))
    else {
        return;
    };
    let Some(property) = schema.properties.get_mut(property_name) else {
        return;
    };
    set_description(property, description);
}

fn set_description(schema: &mut RefOr<Schema>, description: &str) {
    match schema {
        RefOr::Ref(reference) => reference.description = description.into(),
        RefOr::T(Schema::Object(object)) => object.description = Some(description.into()),
        RefOr::T(Schema::Array(array)) => array.description = Some(description.into()),
        RefOr::T(Schema::OneOf(one_of)) => {
            if let Some(item) = one_of
                .items
                .iter_mut()
                .find(|item| matches!(item, RefOr::Ref(_)))
            {
                set_description(item, description);
            }
        }
        RefOr::T(Schema::AllOf(all_of)) => all_of.description = Some(description.into()),
        RefOr::T(Schema::AnyOf(any_of)) => any_of.description = Some(description.into()),
        _ => {}
    }
}
