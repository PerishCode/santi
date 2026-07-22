use santi_model::{DownstreamCredential, InboxSource, IngestRequest};
use sha2::{Digest as _, Sha256};

use super::{Admission, Service};
use crate::service::flow::ingest::External;

impl Service {
    pub fn principal(&self, bearer: &str) -> Result<Option<DownstreamCredential>, String> {
        if bearer.is_empty() {
            return Ok(None);
        }
        let credential_sha256 = hex::encode(Sha256::digest(bearer.as_bytes()));
        self.store.list_downstreams().map(|downstreams| {
            downstreams
                .into_iter()
                .find(|downstream| same_digest(&downstream.credential_sha256, &credential_sha256))
        })
    }

    pub fn create_downstream(
        &self,
        request: crate::CreateDownstreamRequest,
    ) -> Result<DownstreamCredential, String> {
        let id = request.id.trim();
        let label_prefix = request.label_prefix.trim();
        let credential_sha256 = request.credential_sha256.trim().to_ascii_lowercase();
        if id.is_empty() {
            return Err("downstream id must not be empty".to_string());
        }
        if label_prefix.is_empty() {
            return Err("downstream label_prefix must not be empty".to_string());
        }
        if !label_prefix.ends_with(':') {
            return Err("downstream label_prefix must end with ':'".to_string());
        }
        if credential_sha256.len() != 64
            || !credential_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(
                "downstream credential_sha256 must be 64 hexadecimal characters".to_string(),
            );
        }
        self.store
            .create_downstream(id, label_prefix, &credential_sha256)
    }

    pub fn list_downstreams(&self) -> Result<Vec<DownstreamCredential>, String> {
        self.store.list_downstreams()
    }

    pub fn ingest_downstream(
        &self,
        bearer: &str,
        mut request: IngestRequest,
    ) -> Result<Admission, String> {
        if bearer.is_empty() {
            return Ok(Admission::Denied);
        }
        request.request_id = request.request_id.trim().to_string();
        if request.request_id.is_empty() {
            return Err("downstream request_id must not be empty".to_string());
        }
        if request.request_id.len() > 256 {
            return Err("downstream request_id must not exceed 256 bytes".to_string());
        }
        let Some(downstream) = self.principal(bearer)? else {
            return Ok(Admission::Denied);
        };
        if !request.label.starts_with(&downstream.label_prefix) {
            return Ok(Admission::Forbidden);
        }
        let source = request
            .source_ref
            .clone()
            .map(|reference| InboxSource::new("downstream").with_ref(reference));
        let digest = hex::encode(Sha256::digest(
            serde_json::to_vec(&request).map_err(|error| error.to_string())?,
        ));
        if let Some(receipt) =
            self.store
                .replay_downstream(&downstream.id, &request.request_id, &digest)?
        {
            return Ok(Admission::Accepted(crate::IngestOutcome::Accepted {
                receipt,
            }));
        }
        let outcome = self.ingest_external(External {
            soul: &request.soul_id,
            label: &request.label,
            text: request.text,
            source,
            replay: Some(crate::store::Replay {
                owner: &downstream.id,
                request: &request.request_id,
                digest: &digest,
            }),
        })?;
        Ok(Admission::Accepted(outcome))
    }
}

fn same_digest(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}
