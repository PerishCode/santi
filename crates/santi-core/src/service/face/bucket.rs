use std::path::PathBuf;

use crate::object;

use super::Service;

impl Service {
    pub fn renderable_ref(&self, value: &str) -> Result<String, String> {
        if !value.starts_with("santi://") {
            return Ok(value.to_string());
        }
        Ok(object::Uri::parse(value)?.http())
    }

    pub fn put_bucket_object(
        &self,
        uri: &object::Uri,
        bytes: &[u8],
    ) -> Result<object::Meta, String> {
        self.ensure_object_bucket(&uri.bucket)?;
        self.object_store().put(uri, bytes)
    }

    pub fn get_bucket_object(
        &self,
        soul: &str,
        strand: &str,
        key: &str,
    ) -> Result<Option<object::Payload>, String> {
        let uri = self.object_uri(soul, strand, key)?;
        self.object_store().get(&uri)
    }

    pub fn head_bucket_object(&self, uri: &object::Uri) -> Result<Option<object::Meta>, String> {
        self.ensure_object_bucket(&uri.bucket)?;
        self.object_store().head(uri)
    }

    pub fn delete_bucket_object(&self, uri: &object::Uri) -> Result<bool, String> {
        self.ensure_object_bucket(&uri.bucket)?;
        self.object_store().delete(uri)
    }

    pub fn list_bucket_objects(
        &self,
        bucket: &object::Bucket,
        prefix: Option<&str>,
    ) -> Result<Vec<object::Meta>, String> {
        self.ensure_object_bucket(bucket)?;
        self.object_store().list(bucket, prefix)
    }

    fn object_store(&self) -> object::Store {
        object::Store::new(PathBuf::from(&self.config.runtime_root))
    }

    fn object_uri(&self, soul: &str, strand: &str, key: &str) -> Result<object::Uri, String> {
        let bucket = object::Bucket::new(soul, strand)?;
        self.ensure_object_bucket(&bucket)?;
        object::Uri::new(bucket, key)
    }

    fn ensure_object_bucket(&self, bucket: &object::Bucket) -> Result<(), String> {
        let strand = self
            .store
            .strand(&bucket.strand)?
            .ok_or_else(|| "strand not found".to_string())?;
        if strand.soul != bucket.soul {
            return Err("soul not found".to_string());
        }
        Ok(())
    }
}
