//! Attachment bytes in S3-compatible storage (AWS S3, IONOS, MinIO, ...). The forum never
//! serves the bucket directly: every file goes through /files/<id>, which checks the metadata.

use std::sync::Arc;

use bytes::Bytes;
use object_store::aws::{AmazonS3, AmazonS3Builder};
use object_store::path::Path;
use object_store::{ObjectStoreExt, PutPayload};

#[derive(Clone)]
pub struct Storage {
    store: Arc<AmazonS3>,
}

impl Storage {
    /// Reads the configuration from the environment: FORUM_S3_* for the bucket, and the standard
    /// AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY for the credentials (never written into files).
    pub fn from_env() -> Result<Self, String> {
        let bucket = std::env::var("FORUM_S3_BUCKET").map_err(|_| "FORUM_S3_BUCKET is required")?;
        let region = std::env::var("FORUM_S3_REGION").unwrap_or_else(|_| "us-east-1".to_owned());
        let mut builder = AmazonS3Builder::from_env()
            .with_bucket_name(bucket)
            .with_region(region);
        if let Ok(endpoint) = std::env::var("FORUM_S3_ENDPOINT") {
            // Plain http is only allowed for a local endpoint, e.g. a MinIO container for tests.
            if endpoint.starts_with("http://") {
                builder = builder.with_allow_http(true);
            }
            builder = builder.with_endpoint(endpoint).with_virtual_hosted_style_request(false);
        }
        let store = builder.build().map_err(|error| format!("S3-Konfiguration: {error}"))?;
        Ok(Self { store: Arc::new(store) })
    }

    pub async fn put(&self, id: &str, bytes: Bytes) -> Result<(), object_store::Error> {
        self.store.put(&Path::from(id), PutPayload::from_bytes(bytes)).await.map(|_| ())
    }

    pub async fn get(&self, id: &str) -> Result<Bytes, object_store::Error> {
        self.store.get(&Path::from(id)).await?.bytes().await
    }

    pub async fn delete(&self, id: &str) -> Result<(), object_store::Error> {
        self.store.delete(&Path::from(id)).await
    }
}
