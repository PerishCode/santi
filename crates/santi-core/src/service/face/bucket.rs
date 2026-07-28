use std::path::PathBuf;

use crate::object;

use super::Service;

impl Service {
    pub fn renderable(&self, value: &str) -> Result<String, String> {
        if !value.starts_with("santi://") {
            return Ok(value.to_string());
        }
        Ok(object::Uri::parse(value)?.http())
    }

    pub async fn stash(&self, uri: &object::Uri, bytes: &[u8]) -> Result<object::Meta, String> {
        self.bucket(&uri.bucket).await?;
        self.objects().put(uri, bytes)
    }

    pub async fn fetch(
        &self,
        soul: &str,
        strand: &str,
        key: &str,
    ) -> Result<Option<object::Payload>, String> {
        let uri = self.uri(soul, strand, key).await?;
        self.objects().get(&uri)
    }

    pub async fn peek(&self, uri: &object::Uri) -> Result<Option<object::Meta>, String> {
        self.bucket(&uri.bucket).await?;
        self.objects().head(uri)
    }

    pub async fn erase(&self, uri: &object::Uri) -> Result<bool, String> {
        self.bucket(&uri.bucket).await?;
        self.objects().delete(uri)
    }

    pub async fn shelve(
        &self,
        bucket: &object::Bucket,
        prefix: Option<&str>,
    ) -> Result<Vec<object::Meta>, String> {
        self.bucket(bucket).await?;
        self.objects().list(bucket, prefix)
    }

    fn objects(&self) -> object::Store {
        object::Store::new(PathBuf::from(&self.config.runtime))
    }

    async fn uri(&self, soul: &str, strand: &str, key: &str) -> Result<object::Uri, String> {
        let bucket = object::Bucket::new(soul, strand)?;
        self.bucket(&bucket).await?;
        object::Uri::new(bucket, key)
    }

    async fn bucket(&self, bucket: &object::Bucket) -> Result<(), String> {
        let strand = self
            .store
            .strand(&bucket.strand)
            .await?
            .ok_or_else(|| "strand not found".to_string())?;
        if strand.soul != bucket.soul {
            return Err("soul not found".to_string());
        }
        Ok(())
    }
}
