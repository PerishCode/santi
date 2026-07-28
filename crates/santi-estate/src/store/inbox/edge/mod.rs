use super::{InboxDraft, Store, read, write as inbox};
use keel::{Op, Rank, form};
use santi_model::{downstream, ingest, webhook};

mod types;
pub use types::{Accepted, DownstreamDraft, ReplayDraft, WebhookDraft};

impl Store {
    pub async fn subscribe(
        &self,
        draft: WebhookDraft<'_>,
    ) -> Result<webhook::Subscription, String> {
        self.core
            .batch(async |tx| {
                let soul = read::need(tx, "Soul", "tag", draft.soul).await?;
                if let Some(row) = tx
                    .one(&form("Webhook").when("name", Op::Eq, draft.name))
                    .await?
                {
                    if exact_webhook(&row, &draft, soul) {
                        return Ok(());
                    }
                    return Err(keel::adapt::Error::Adapt(format!(
                        "webhook {} conflicts with an existing subscription",
                        draft.name
                    )));
                }
                tx.put(
                    "Webhook",
                    &[
                        ("name", draft.name),
                        ("adaptor", draft.adaptor),
                        ("strategy", draft.strategy),
                        ("credential", draft.credential),
                        ("created", draft.created),
                        ("updated", draft.created),
                        ("soul", &soul.to_string()),
                    ],
                )
                .await?;
                Ok(())
            })
            .await
            .map_err(read::error)?;
        self.webhook(draft.name)
            .await?
            .ok_or_else(|| "created webhook missing".to_string())
    }

    pub async fn webhook(&self, name: &str) -> Result<Option<webhook::Subscription>, String> {
        let Some(row) = read::one(&self.core, "Webhook", "name", name).await? else {
            return Ok(None);
        };
        Ok(Some(webhook::Subscription {
            name: read::text(&row, "name")?.to_string(),
            adaptor: read::text(&row, "adaptor")?.to_string(),
            soul: read::related(&self.core, "Soul", read::int(&row, "soul")?).await?,
            strategy: read::text(&row, "strategy")?.to_string(),
            credential: read::text(&row, "credential")?.to_string(),
            created: read::text(&row, "created")?.to_string(),
            updated: read::text(&row, "updated")?.to_string(),
        }))
    }

    pub async fn webhooks(&self) -> Result<Vec<webhook::Subscription>, String> {
        let rows = self
            .core
            .ask(
                &form("Webhook")
                    .order("created", Rank::Asc)
                    .order("name", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        let mut subscriptions = Vec::with_capacity(rows.rows().len());
        for row in rows.rows() {
            let name = read::text(row, "name")?;
            subscriptions.push(
                self.webhook(name)
                    .await?
                    .ok_or_else(|| format!("webhook {name} missing"))?,
            );
        }
        Ok(subscriptions)
    }

    pub async fn enroll(
        &self,
        draft: DownstreamDraft<'_>,
    ) -> Result<downstream::Credential, String> {
        self.core
            .batch(async |tx| {
                let rows = tx.live("Downstream").await?;
                if let Some(row) = rows.iter().find(|row| row.text("tag") == Some(draft.tag)) {
                    if row.text("prefix") == Some(draft.prefix)
                        && row.text("credential_sha256") == Some(draft.digest)
                    {
                        return Ok(());
                    }
                    return Err(keel::adapt::Error::Adapt(
                        "downstream id conflicts with an existing registration".into(),
                    ));
                }
                if rows.iter().any(|row| {
                    row.text("prefix").is_some_and(|prefix| {
                        prefix.starts_with(draft.prefix) || draft.prefix.starts_with(prefix)
                    })
                }) {
                    return Err(keel::adapt::Error::Adapt(
                        "downstream prefix overlaps an existing registration".into(),
                    ));
                }
                tx.put(
                    "Downstream",
                    &[
                        ("tag", draft.tag),
                        ("prefix", draft.prefix),
                        ("credential_sha256", draft.digest),
                        ("created", draft.created),
                        ("updated", draft.created),
                    ],
                )
                .await?;
                Ok(())
            })
            .await
            .map_err(read::error)?;
        self.downstream(draft.tag)
            .await?
            .ok_or_else(|| "created downstream missing".to_string())
    }

    pub async fn downstream(&self, tag: &str) -> Result<Option<downstream::Credential>, String> {
        let Some(row) = read::one(&self.core, "Downstream", "tag", tag).await? else {
            return Ok(None);
        };
        Ok(Some(downstream::Credential {
            id: read::text(&row, "tag")?.to_string(),
            prefix: read::text(&row, "prefix")?.to_string(),
            digest: read::text(&row, "credential_sha256")?.to_string(),
            created: read::text(&row, "created")?.to_string(),
            updated: read::text(&row, "updated")?.to_string(),
        }))
    }

    pub async fn downstreams(&self) -> Result<Vec<downstream::Credential>, String> {
        let rows = self
            .core
            .ask(
                &form("Downstream")
                    .order("created", Rank::Asc)
                    .order("tag", Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        let mut credentials = Vec::with_capacity(rows.rows().len());
        for row in rows.rows() {
            let tag = read::text(row, "tag")?;
            credentials.push(
                self.downstream(tag)
                    .await?
                    .ok_or_else(|| format!("downstream {tag} missing"))?,
            );
        }
        Ok(credentials)
    }

    pub async fn accept_replay(
        &self,
        draft: InboxDraft<'_>,
        replay: ReplayDraft<'_>,
        gate: usize,
    ) -> Result<Accepted, String> {
        let content = serde_json::to_string(draft.content).map_err(|error| error.to_string())?;
        let metadata = draft
            .source
            .and_then(|source| source.metadata.as_ref())
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        let accepted = self
            .core
            .batch(async |tx| {
                let (unit, owner, identity, value, digest) = replay_shape(tx, replay).await?;
                if let Some(row) = tx
                    .one(
                        &form(unit)
                            .when(owner.0, Op::Eq, &owner.1)
                            .when(identity, Op::Eq, value),
                    )
                    .await?
                {
                    if row.text("request_sha256") != Some(digest) {
                        return Err(keel::adapt::Error::Adapt(conflict(replay).into()));
                    }
                    let strand = need_text(&row, "strand")?;
                    let inbox = need_text(&row, "inbox")?;
                    return Ok((strand, inbox, false));
                }
                inbox::accept(tx, &draft, gate, &content, metadata.as_deref()).await?;
                let mut fields = vec![
                    (identity, value),
                    ("request_sha256", digest),
                    ("created", draft.created),
                    ("strand", draft.strand),
                    ("inbox", draft.tag),
                ];
                fields.push((owner.0, owner.1.as_str()));
                tx.put(unit, &fields).await?;
                Ok((draft.strand.to_string(), draft.tag.to_string(), true))
            })
            .await
            .map_err(read::error)?;
        Ok(Accepted {
            receipt: ingest::Receipt {
                strand: accepted.0,
                inbox: accepted.1,
                warning: None,
            },
            inserted: accepted.2,
        })
    }
}

async fn replay_shape<'a>(
    tx: &mut keel::Tx<'_, keel::adapt::db::Sqlite>,
    replay: ReplayDraft<'a>,
) -> Result<
    (
        &'static str,
        (&'static str, String),
        &'static str,
        &'a str,
        &'a str,
    ),
    keel::adapt::Error,
> {
    match replay {
        ReplayDraft::Webhook {
            subscription,
            delivery,
            digest,
        } => Ok((
            "WebhookDelivery",
            (
                "webhook",
                read::need(tx, "Webhook", "name", subscription)
                    .await?
                    .to_string(),
            ),
            "delivery",
            delivery,
            digest,
        )),
        ReplayDraft::Downstream {
            owner,
            request,
            digest,
        } => Ok((
            "DownstreamIngest",
            (
                "downstream",
                read::need(tx, "Downstream", "tag", owner)
                    .await?
                    .to_string(),
            ),
            "request",
            request,
            digest,
        )),
    }
}

fn exact_webhook(row: &keel::Row, draft: &WebhookDraft<'_>, soul: i64) -> bool {
    row.text("adaptor") == Some(draft.adaptor)
        && row.text("strategy") == Some(draft.strategy)
        && row.text("credential") == Some(draft.credential)
        && row.int("soul") == Some(soul)
}

fn conflict(replay: ReplayDraft<'_>) -> &'static str {
    match replay {
        ReplayDraft::Webhook { .. } => "webhook delivery conflicts with an accepted payload",
        ReplayDraft::Downstream { .. } => "downstream request conflicts with an accepted payload",
    }
}

fn need_text(row: &keel::Row, field: &str) -> Result<String, keel::adapt::Error> {
    row.text(field)
        .map(str::to_string)
        .ok_or_else(|| keel::adapt::Error::Adapt(format!("replay {field} missing")))
}
