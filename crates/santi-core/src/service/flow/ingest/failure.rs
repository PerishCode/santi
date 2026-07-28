use crate::service::Service;
use crate::{Fault, Ruled, catalog, engine};

use super::Drive;

impl Service {
    pub(in crate::service) async fn gated(
        &self,
        strand: &str,
        operation: &str,
    ) -> Result<Option<Fault>, String> {
        let key = crate::drive::Error::Failed
            .descriptor()
            .key("strand", strand);
        if self.store.incident(&key).await?.is_none() {
            return Ok(None);
        }
        let pending = self.store.inboxes(strand).await?.len();
        self.store
            .raise(
                drive_failure(
                    strand,
                    Drive {
                        trigger: "admission_guard",
                        inbox: None,
                        operation,
                    },
                    "strand driver recovery is still required",
                    pending,
                ),
                &crate::now(),
            )
            .await
            .map(Some)
    }

    pub(super) async fn stumbled(&self, strand: &str, drive: Drive<'_>, detail: String) -> Fault {
        self.degrade();
        let pending = self
            .store
            .inboxes(strand)
            .await
            .map(|pending| pending.len())
            .unwrap_or_default();
        let error = match self
            .store
            .raise(
                drive_failure(strand, drive, &detail, pending),
                &crate::now(),
            )
            .await
        {
            Ok(error) => error,
            Err(persistence_error) => engine().transient(crate::Signal {
                descriptor: catalog::UNSAVED,
                source: santi_error::Source::new("santi-core", "strand_drive_failure"),
                scope: Some(santi_error::Scope::new("strand", strand)),
                message: "failed to persist strand driver incident".to_string(),
                context: serde_json::json!({
                    "accepted_before_failure": drive.inbox.is_some(),
                    "inbox": drive.inbox,
                    "detail": persistence_error,
                }),
            }),
        };
        eprintln!(
            "santi: strand drive failed code={} incident_id={} strand={} operation={} accepted_before_failure={}",
            error.code,
            error.incident.as_deref().unwrap_or("-"),
            strand,
            drive.operation,
            drive.inbox.is_some(),
        );
        self.dispatched().await;
        error
    }
}

fn drive_failure(
    strand: &str,
    drive: Drive<'_>,
    detail: &str,
    pending: usize,
) -> santi_error::Draft {
    santi_error::Draft {
        key: crate::drive::Error::Failed
            .descriptor()
            .key("strand", strand),
        descriptor: crate::drive::Error::Failed.descriptor(),
        scope: santi_error::Scope::new("strand", strand),
        source: santi_error::Source::new("santi-core", drive.operation),
        message: "strand driver could not start pending work".to_string(),
        context: serde_json::json!({
            "schema": "santi.error.strand_drive.v1",
            "accepted_before_failure": drive.inbox.is_some(),
            "inbox": drive.inbox,
            "pending_count": pending,
            "trigger": drive.trigger,
            "detail": bounded(detail),
            "recovery": {
                "command": format!("santi strand drive {strand}"),
                "resend": false,
            },
        }),
    }
}

fn bounded(detail: &str) -> String {
    const LIMIT: usize = 4096;
    if detail.len() <= LIMIT {
        return detail.to_string();
    }
    let suffix = " [truncated]";
    let mut end = LIMIT.saturating_sub(suffix.len());
    while end > 0 && !detail.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &detail[..end], suffix)
}
