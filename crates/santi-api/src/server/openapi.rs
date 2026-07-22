use santi_core::{
    CreateSoulRequest, CreateStrandResponse, CreateWebhookRequest, ForkStrandResponse,
    HealthResponse, MaterialRequest, SantiError, SendStrandAcceptedResponse, SendStrandRequest,
    Soul, Strand, StrandDetail, StrandMaterial, StrandRuntimeSnapshot, WebhookSubscription,
};
use utoipa::{
    OpenApi,
    openapi::{RefOr, schema::Schema},
};

const COMPONENT_DESCRIPTIONS: [(&str, &str); 11] = [
    (
        "Soul",
        "A soul is a cyber-individual, keyed by id alone. It has no name/avatar/desc\ncolumn: identity is the mutable self, and it lives entirely in the soul's\nmemory (rendered live into `[santi-soul]`), not in a profile row. The\ntimestamps are pure provenance.",
    ),
    (
        "CreateSoulRequest",
        "Create a new soul (an individual). Souls are API-managed, never config.\nA soul is id-only; its identity is its memory, so the only thing to supply\nat creation is the initial `[santi-soul]` memory to seed (empty/absent → a\nblank soul that will author its own).",
    ),
    (
        "WebhookSubscription",
        "An API-managed webhook subscription: how an external source reaches a soul.\n`adaptor` selects the boundary normalizer (integration knowledge); `soul_id`\nis who receives the resulting turn; `strand_strategy` picks where the thread\nlives (`per_thread` = one strand per adaptor-derived label, `single` = one\nstrand per subscription); `secret_env` names the env var holding the signing\nsecret (the secret itself is never stored). The `name` is the URL path segment.",
    ),
    (
        "ReceiptState",
        "Current durable responsibility state for one accepted inbox item. A\nmechanically-recovered transition can be immediately followed by `driving`\nin the same transaction; callers inspect `transitions` for that evidence.",
    ),
    (
        "Compact",
        "A compact is a pure projection overlay over a strand's spine. It\nself-describes its coverage by message-id boundaries and carries the\noperator-authored summary while originals remain queryable.",
    ),
    (
        "EffectState",
        "Durable truth for one concrete external-effect attempt. It is deliberately\nnot turn state: one turn may contain several independently settled or\nambiguous effects.",
    ),
    (
        "ImSendRequest",
        "IM inbound: a participant sends content to a soul. The sender's address is\nIM envelope only, carried by the `im:<participant_id>` conversation label.",
    ),
    (
        "ImSendResponse",
        "Durable enqueue confirmation for an IM send. The soul may still be mid-turn.",
    ),
    (
        "ImInboxEntry",
        "One delivered message in a participant's passive inbox.",
    ),
    (
        "ImDelivery",
        "Content-free delivery evidence projected onto an accepted inbox receipt.",
    ),
    (
        "ActorType",
        "No user/account actor: santi is individual-first, not multi-tenant. All\ninbound (a CLI send, a webhook event) arrives as `System` — the sender's\nidentity is metainfo carried in the content, opaque to core, not a distinct\nactor kind. `(actor, message_kind)` is the full marker at the provider\nboundary (see `message_to_provider_item`): Soul→assistant, System+Text→user\n(world-inbound), System+SantiSystem→system (runtime-meta, not user speech).",
    ),
];

const PROPERTY_DESCRIPTIONS: [(&str, &str, &str); 12] = [
    (
        "HealthResponse",
        "active_drive_incidents",
        "Aggregate only: `/health` is public and must never expose strand,\nreceipt, or incident locators.",
    ),
    (
        "Strand",
        "external_label",
        "Opaque external anchor (e.g. a webhook thread key). Unique per soul;\nabsent for strands reached only by id (e.g. CLI-created ones).",
    ),
    (
        "CreateWebhookRequest",
        "strand_strategy",
        "`per_thread` (default) or `single`.",
    ),
    (
        "SendStrandAcceptedResponse",
        "user_message",
        "The content this send just enqueued, once the driver has actually\ncommitted it to the timeline. Absent when this send coalesced into an\nalready-running turn — durably enqueued, but the driver has not drained\nit yet (it will, when that turn completes and re-pokes).",
    ),
    (
        "ReceiptTransition",
        "reconstructed_from",
        "Present only when schema migration reconstructed this evidence from a\ndurable v24 source row. Live transitions leave it unset.",
    ),
    (
        "ReceiptStatus",
        "effects",
        "Per-attempt shell effects reached by any turn carrying this receipt.\nCompletion alone does not imply that any listed external effect applied.",
    ),
    (
        "ReceiptStatus",
        "im_deliveries",
        "Runtime-owned IM replies delivered by any attempt carrying this receipt.\nContents remain in the participant inbox and are intentionally omitted.",
    ),
    (
        "StrandEffect",
        "tool_call_id",
        "Absent only for an imported legacy row whose old schema had no neutral\ntool-call locator.",
    ),
    (
        "EffectTransition",
        "evidence",
        "Human- or runtime-supplied evidence. This is never interpreted as proof\nof idempotency by core.",
    ),
    (
        "EffectStatus",
        "receipt_ids",
        "Obligation roots whose attempts include this effect's turn.",
    ),
    (
        "ImInboxEntry",
        "turn_id",
        "Absent for legacy or operator-authored entries outside a provider turn.",
    ),
    (
        "ImInboxEntry",
        "message_id",
        "Present when automatic delivery used a final assistant message.",
    ),
];

#[derive(OpenApi)]
#[openapi(
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
        super::routes::runtime_snapshot,
        super::routes::turn_events,
        super::im::send_im,
        super::im::poll_im,
        crate::bucket::get_bucket_object
    ),
    components(schemas(
        CreateStrandResponse,
        santi_core::TurnEvent,
        santi_core::TurnEventPage,
        ForkStrandResponse,
        CreateSoulRequest,
        CreateWebhookRequest,
        WebhookSubscription,
        SantiError,
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
        santi_core::ImSendRequest,
        santi_core::ImSendResponse,
        santi_core::ImInboxEntry,
        santi_core::ImDelivery,
        santi_core::ImDeliveryMode,
        santi_core::ImParticipant,
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
        santi_core::ErrorCategory,
        santi_core::ErrorExposure,
        santi_core::ErrorIncident,
        santi_core::ErrorRetry,
        santi_core::ErrorScope,
        santi_core::ErrorSeverity,
        santi_core::ErrorSource,
        santi_core::ErrorTransition,
        santi_core::ErrorTransitionKind,
        santi_core::IncidentStatus,
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
