use santi_core::{Fault, Health, environ, job, soul::Soul, strand::Strand, webhook};
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
use santi_core::{material, soul, strand, stream};

const PROPERTY_DESCRIPTIONS: [(&str, &str, &str); 13] = [
    (
        "downstream::Draft",
        "digest",
        "Lowercase or uppercase SHA-256 hex of a high-entropy Bearer token. The digest is stored but never returned.",
    ),
    (
        "ingest::Request",
        "request",
        "Stable idempotency key, unique within the authenticated downstream. Reuse with a changed payload is a conflict.",
    ),
    (
        "event::Batch",
        "cursor",
        "Opaque global high-water mark. Persist it even when events is empty; it reveals aggregate activity but no foreign payload.",
    ),
    (
        "Health",
        "incidents",
        "Aggregate only: `/health` is public and must never expose strand,\nreceipt, or incident locators.",
    ),
    (
        "Strand",
        "label",
        "Opaque external anchor (e.g. a webhook thread key). Unique per soul;\nabsent for strands reached only by id (e.g. CLI-created ones).",
    ),
    (
        "webhook::Draft",
        "strategy",
        "`per_thread` (default) or `single`.",
    ),
    (
        "strand::Posted",
        "message",
        "The content this send just enqueued, once the driver has actually\ncommitted it to the timeline. Absent when this send coalesced into an\nalready-running turn — durably enqueued, but the driver has not drained\nit yet (it will, when that turn completes and re-pokes).",
    ),
    (
        "receipt::Transition",
        "rebuilt",
        "Present only when schema migration reconstructed this evidence from a\ndurable v24 source row. Live transitions leave it unset.",
    ),
    (
        "receipt::Status",
        "effects",
        "Per-attempt shell effects reached by any turn carrying this receipt.\nCompletion alone does not imply that any listed external effect applied.",
    ),
    (
        "effect::Effect",
        "call",
        "Absent only for an imported legacy row whose old schema had no neutral\ntool-call locator.",
    ),
    (
        "effect::Status",
        "receipts",
        "Obligation roots whose attempts include this effect's turn.",
    ),
    (
        "job::Accepted",
        "job",
        "The accepted resource. Its current state may advance quickly, but create\nsuccess promises that the detached sidecar durably claimed execution responsibility.",
    ),
    (
        "job::Log",
        "next",
        "Opaque monotonic byte cursor for the next read of this stream.",
    ),
];

#[derive(OpenApi)]
#[openapi(
    modifiers(&SecurityAddon),
    paths(
        super::routes::health,
        super::routes::weave,
        super::routes::strands,
        super::routes::awaken,
        super::routes::souls,
        super::routes::get_soul,
        super::routes::soul_environs,
        super::routes::set_soul_environ,
        super::routes::end_soul_environ,
        super::routes::subscribe,
        super::routes::webhooks,
        super::jobs::create,
        super::jobs::list,
        super::jobs::get,
        super::jobs::cancel,
        super::jobs::logs,
        super::jobs::acknowledge,
        super::ingress::ingest_webhook,
        super::routes::get_strand,
        super::routes::strand_environs,
        super::routes::set_strand_environ,
        super::routes::end_strand_environ,
        super::routes::list_messages,
        super::routes::material,
        super::routes::send,
        super::routes::drive,
        super::routes::fork,
        super::routes::exec,
        super::routes::page,
        super::routes::audit,
        super::errors::stranded,
        super::errors::errors,
        super::routes::receipt,
        super::effects::effect,
        super::effects::trail,
        super::effects::settle,
        super::sse::transitions,
        super::sse::turn_event_stream,
        super::routes::snapshot,
        super::routes::turn_events,
        super::routes::stop,
        super::routes::enroll,
        super::routes::downstreams,
        super::routes::ingest,
        crate::bucket::fetch
    ),
    components(schemas(
        strand::Created,
        santi_core::event::Event,
        santi_core::event::Batch,
        santi_core::downstream::Credential,
        santi_core::downstream::Draft,
        santi_core::ingest::Request,
        strand::Forked,
        soul::Draft,
        environ::Scope,
        environ::Variable,
        environ::Draft,
        webhook::Draft,
        webhook::Subscription,
        super::jobs::CreateJobRequest,
        job::Accepted,
        job::Job,
        job::Origin,
        job::State,
        job::Stream,
        job::Log,
        Fault,
        Health,
        material::Request,
        strand::Post,
        strand::Posted,
        santi_core::drive::Response,
        santi_core::drive::State,
        santi_core::ingest::Receipt,
        santi_core::receipt::State,
        santi_core::receipt::Status,
        santi_core::receipt::Transition,
        santi_core::effect::State,
        santi_core::effect::Status,
        santi_core::effect::Outcome,
        santi_core::trace::Record,
        santi_core::trace::Tag,
        super::effects::ResolveEffectRequest,
        strand::Detail,
        material::Material,
        stream::Snapshot,
        Soul,
        Strand,
        santi_core::message::Role,
        santi_core::compact::Compact,
        santi_core::compact::Capsule,
        santi_core::compact::Exec,
        santi_core::compact::Report,
        santi_core::compact::Entry,
        santi_core::compact::Page,
        santi_core::budget::Cap,
        santi_core::budget::Estimate,
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
        santi_core::budget::Snapshot,
        santi_core::strand::Target,
        santi_core::message::Message,
        santi_core::message::Content,
        santi_core::message::Part,
        santi_core::message::State,
        santi_core::material::Kind,
        santi_core::material::Updated,
        santi_core::effect::Effect,
        santi_core::message::Placed,
        santi_core::message::Relation,
        santi_core::thinking::Span,
        santi_core::thinking::State,
        santi_core::tool::Call,
        santi_core::tool::Reply,
        santi_core::turn::Turn,
        santi_core::turn::Activity,
        santi_core::turn::Motion,
        santi_core::turn::Status,
        santi_core::turn::Trigger,
        santi_core::turn::Cause,
        santi_core::turn::Stop
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
