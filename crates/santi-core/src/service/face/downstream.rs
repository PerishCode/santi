use sha2::{Digest as _, Sha256};

use super::{Admission, Service};
use crate::service::flow::ingest::External;
use crate::{downstream, ingest};

impl Service {
    pub fn principal(&self, bearer: &str) -> Result<Option<downstream::Credential>, String> {
        if bearer.is_empty() {
            return Ok(None);
        }
        let digest = hex::encode(Sha256::digest(bearer.as_bytes()));
        self.store.downstreams().map(|downstreams| {
            downstreams
                .into_iter()
                .find(|downstream| same(&downstream.digest, &digest))
        })
    }

    pub fn enroll(
        &self,
        request: crate::downstream::Draft,
    ) -> Result<downstream::Credential, String> {
        let id = request.id.trim();
        let prefix = request.prefix.trim();
        let digest = request.digest.trim().to_ascii_lowercase();
        if id.is_empty() {
            return Err("downstream id must not be empty".to_string());
        }
        if prefix.is_empty() {
            return Err("downstream prefix must not be empty".to_string());
        }
        if !prefix.ends_with(':') {
            return Err("downstream prefix must end with ':'".to_string());
        }
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("downstream digest must be 64 hexadecimal characters".to_string());
        }
        self.store.enroll(id, prefix, &digest)
    }

    pub fn downstreams(&self) -> Result<Vec<downstream::Credential>, String> {
        self.store.downstreams()
    }

    pub fn downstream(
        &self,
        bearer: &str,
        mut request: ingest::Request,
    ) -> Result<Admission, String> {
        if bearer.is_empty() {
            return Ok(Admission::Denied);
        }
        request.request = request.request.trim().to_string();
        if request.request.is_empty() {
            return Err("downstream request must not be empty".to_string());
        }
        if request.request.len() > 256 {
            return Err("downstream request must not exceed 256 bytes".to_string());
        }
        let Some(downstream) = self.principal(bearer)? else {
            return Ok(Admission::Denied);
        };
        if !request.label.starts_with(&downstream.prefix) {
            return Ok(Admission::Forbidden);
        }
        let source = request
            .source
            .clone()
            .map(|reference| ingest::Source::new("downstream").with_ref(reference));
        let digest = hex::encode(Sha256::digest(
            serde_json::to_vec(&request).map_err(|error| error.to_string())?,
        ));
        if let Some(receipt) = self
            .store
            .replayed(&downstream.id, &request.request, &digest)?
        {
            return Ok(Admission::Accepted(crate::ingest::Outcome::Accepted {
                receipt,
            }));
        }
        let outcome = self.external(External {
            soul: &request.soul,
            label: &request.label,
            text: request.text,
            source,
            replay: Some(crate::store::Replay {
                owner: &downstream.id,
                request: &request.request,
                digest: &digest,
            }),
        })?;
        Ok(Admission::Accepted(outcome))
    }
}

fn same(left: &str, right: &str) -> bool {
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
