use keel::adapt::db::Sqlite;
use keel::{Core, bind};
use std::path::Path;
use std::sync::Arc;

mod effect;
mod error;
mod inbox;
mod job;
mod support;
mod thinking;
mod timeline;
mod tool;
mod turn;

pub use effect::{EffectDraft, RedemptionDraft};
pub use inbox::{
    Accepted, Begun, DownstreamDraft, DrainDraft, Inbox, InboxDraft, NoticeDraft, Offer, Opening,
    ReceiptDraft, ReplayDraft, WebhookDraft,
};
pub use job::{
    AttentionDraft, CapabilityDraft, ExpiredJob, JobDraft, JobRecord, Prepared, TransitionDraft,
};
pub use support::{Bootstrap, EnvironDraft, Status, TraceDraft};
use support::{read, write};
pub use thinking::ThinkingDraft;
pub use timeline::{CompactDraft, ForkDraft, MessageDraft};
pub use tool::{CallDraft, ReplyDraft};
pub use turn::{
    ClassifiedFailure, ClassifiedFailureDraft, Completion, CompletionDraft, Interruption,
    InterruptionDraft, OutboxDraft, TurnDraft,
};

#[derive(Clone)]
pub struct Store {
    core: Arc<Core<Sqlite>>,
}

pub struct StrandDraft<'a> {
    pub tag: &'a str,
    pub soul: &'a str,
    pub label: Option<&'a str>,
    pub parent: Option<&'a str>,
    pub fork: Option<i64>,
    pub created: &'a str,
}

impl Store {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let wire = support::wire(path.as_ref()).await?;
        let core = bind(crate::graph(), wire)
            .await
            .map_err(read::error)?
            .share();
        Ok(Self { core })
    }

    pub async fn seed(&self, tag: &str, now: &str) -> Result<santi_model::soul::Soul, String> {
        if let Some(soul) = self.soul(tag).await? {
            return Ok(soul);
        }
        self.core
            .put("Soul", &[("tag", tag), ("created", now), ("updated", now)])
            .await
            .map_err(read::error)?;
        self.soul(tag)
            .await?
            .ok_or_else(|| "created soul missing".to_string())
    }

    pub async fn create_soul(
        &self,
        tag: &str,
        now: &str,
    ) -> Result<santi_model::soul::Soul, String> {
        self.core
            .put("Soul", &[("tag", tag), ("created", now), ("updated", now)])
            .await
            .map_err(read::error)?;
        self.soul(tag)
            .await?
            .ok_or_else(|| "created soul missing".to_string())
    }

    pub async fn soul(&self, tag: &str) -> Result<Option<santi_model::soul::Soul>, String> {
        read::one(&self.core, "Soul", "tag", tag)
            .await?
            .map(|row| read::soul(&row))
            .transpose()
    }

    pub async fn souls(&self) -> Result<Vec<santi_model::soul::Soul>, String> {
        let rows = self
            .core
            .ask(
                &keel::form("Soul")
                    .order("created", keel::Rank::Asc)
                    .order("tag", keel::Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        rows.rows().iter().map(read::soul).collect()
    }

    pub async fn create_strand(
        &self,
        draft: StrandDraft<'_>,
    ) -> Result<santi_model::strand::Strand, String> {
        let tag = draft.tag.to_string();
        self.core
            .batch(async |tx| {
                let soul = read::need(tx, "Soul", "tag", draft.soul).await?;
                let parent = match draft.parent {
                    Some(parent) => Some(read::need(tx, "Strand", "tag", parent).await?),
                    None => None,
                };
                let soul_key = soul.to_string();
                let parent_key = parent.map(|key| key.to_string());
                let fork = draft.fork.map(|at| at.to_string());
                let mut fields = vec![
                    ("tag", draft.tag),
                    ("created", draft.created),
                    ("updated", draft.created),
                    ("soul", soul_key.as_str()),
                ];
                if let Some(label) = draft.label {
                    fields.push(("label", label));
                }
                if let Some(parent) = parent_key.as_deref() {
                    fields.push(("parent", parent));
                }
                if let Some(fork) = fork.as_deref() {
                    fields.push(("fork", fork));
                }
                tx.put("Strand", &fields).await?;
                Ok(())
            })
            .await
            .map_err(read::error)?;
        self.strand(&tag)
            .await?
            .ok_or_else(|| "created strand missing".to_string())
    }

    pub async fn strand(&self, tag: &str) -> Result<Option<santi_model::strand::Strand>, String> {
        let Some(row) = read::one(&self.core, "Strand", "tag", tag).await? else {
            return Ok(None);
        };
        read::strand(&self.core, &row).await.map(Some)
    }

    pub async fn strands(&self) -> Result<Vec<santi_model::strand::Strand>, String> {
        let rows = self
            .core
            .ask(
                &keel::form("Strand")
                    .order("updated", keel::Rank::Desc)
                    .order("tag", keel::Rank::Desc),
            )
            .await
            .map_err(read::error)?;
        let mut strands = Vec::with_capacity(rows.rows().len());
        for row in rows.rows() {
            strands.push(read::strand(&self.core, row).await?);
        }
        Ok(strands)
    }

    pub async fn labeled(
        &self,
        soul: &str,
        label: &str,
        created: &str,
    ) -> Result<santi_model::strand::Strand, String> {
        let tag = santi_model::tag("ss");
        let selected = self
            .core
            .batch(async |tx| {
                let soul = read::need(tx, "Soul", "tag", soul).await?;
                let soul_key = soul.to_string();
                if let Some(row) = tx
                    .one(
                        &keel::form("Strand")
                            .when("soul", keel::Op::Eq, &soul_key)
                            .when("label", keel::Op::Eq, label),
                    )
                    .await?
                {
                    return row
                        .text("tag")
                        .map(str::to_string)
                        .ok_or_else(|| keel::adapt::Error::Adapt("strand tag missing".into()));
                }
                tx.put(
                    "Strand",
                    &[
                        ("tag", tag.as_str()),
                        ("label", label),
                        ("created", created),
                        ("updated", created),
                        ("soul", soul_key.as_str()),
                    ],
                )
                .await?;
                Ok(tag.clone())
            })
            .await
            .map_err(read::error)?;
        self.strand(&selected)
            .await?
            .ok_or_else(|| "selected strand missing".to_string())
    }

    pub async fn selected(
        &self,
        selector: &santi_model::strand::Selector,
        created: &str,
    ) -> Result<santi_model::strand::Strand, String> {
        match selector {
            santi_model::strand::Selector::ById(tag) => self
                .strand(tag)
                .await?
                .ok_or_else(|| "strand not found".to_string()),
            santi_model::strand::Selector::ByLabel { soul, label } => {
                self.labeled(soul, label, created).await
            }
        }
    }

    pub async fn pending_strands(&self) -> Result<Vec<String>, String> {
        let rows = self
            .core
            .ask(
                &keel::form("StrandInbox")
                    .order("created", keel::Rank::Asc)
                    .order("tag", keel::Rank::Asc),
            )
            .await
            .map_err(read::error)?;
        let mut pending = Vec::new();
        for row in rows.rows() {
            let strand = read::related(&self.core, "Strand", read::int(row, "strand")?).await?;
            if !pending.contains(&strand) {
                pending.push(strand);
            }
        }
        Ok(pending)
    }
}
