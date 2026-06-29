use std::path::PathBuf;

use crate::{LocalObjectStore, ObjectBucket, ObjectMeta, ObjectPayload, ObjectUri, SantiService};

impl SantiService {
    pub fn renderable_ref(&self, value: &str) -> Result<String, String> {
        if !value.starts_with("santi://") {
            return Ok(value.to_string());
        }
        Ok(ObjectUri::parse(value)?.as_http_path())
    }

    pub fn put_bucket_object(&self, uri: &ObjectUri, bytes: &[u8]) -> Result<ObjectMeta, String> {
        self.ensure_object_bucket(&uri.bucket)?;
        self.object_store().put_object(uri, bytes)
    }

    pub fn get_bucket_object(
        &self,
        soul_id: &str,
        session_id: &str,
        key: &str,
    ) -> Result<Option<ObjectPayload>, String> {
        let uri = self.object_uri(soul_id, session_id, key)?;
        self.object_store().get_object(&uri)
    }

    pub fn head_bucket_object(&self, uri: &ObjectUri) -> Result<Option<ObjectMeta>, String> {
        self.ensure_object_bucket(&uri.bucket)?;
        self.object_store().head_object(uri)
    }

    pub fn delete_bucket_object(&self, uri: &ObjectUri) -> Result<bool, String> {
        self.ensure_object_bucket(&uri.bucket)?;
        self.object_store().delete_object(uri)
    }

    pub fn list_bucket_objects(
        &self,
        bucket: &ObjectBucket,
        prefix: Option<&str>,
    ) -> Result<Vec<ObjectMeta>, String> {
        self.ensure_object_bucket(bucket)?;
        self.object_store().list_objects(bucket, prefix)
    }

    fn object_store(&self) -> LocalObjectStore {
        LocalObjectStore::new(PathBuf::from(&self.config.runtime_root))
    }

    fn object_uri(&self, soul_id: &str, session_id: &str, key: &str) -> Result<ObjectUri, String> {
        let bucket = ObjectBucket::new(soul_id, session_id)?;
        self.ensure_object_bucket(&bucket)?;
        ObjectUri::new(bucket, key)
    }

    fn ensure_object_bucket(&self, bucket: &ObjectBucket) -> Result<(), String> {
        if bucket.soul_id != self.store.default_soul_id() {
            return Err("soul not found".to_string());
        }
        if self.store.session(&bucket.session_id)?.is_none() {
            return Err("session not found".to_string());
        }
        Ok(())
    }
}
