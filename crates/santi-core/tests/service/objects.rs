use super::support::*;
use santi_core::service::{self, Service};

#[tokio::test]
async fn bucket_objects_are_scoped() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = Service::open(
        service::Config {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
            constitution_path: None,
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service");
    let strand = service.create_strand().expect("create strand").strand;
    let bucket = ObjectBucket::new("soul_default", strand.id.as_str()).expect("bucket");
    let uri = ObjectUri::new(bucket.clone(), "avatars/santi.svg").expect("uri");

    let meta = service
        .put_bucket_object(&uri, b"<svg>avatar</svg>")
        .expect("put object");
    assert_eq!(meta.uri.as_santi_uri(), uri.as_santi_uri());
    assert_eq!(meta.len, 17);
    assert_eq!(
        service
            .renderable_ref(&uri.as_santi_uri())
            .expect("renderable ref"),
        format!(
            "/api/v1/bucket/soul_default/{}/avatars/santi.svg",
            strand.id
        )
    );

    let object = service
        .get_bucket_object("soul_default", &strand.id, "avatars/santi.svg")
        .expect("get object")
        .expect("object exists");
    assert_eq!(object.bytes, b"<svg>avatar</svg>");
    let objects = service
        .list_bucket_objects(&bucket, Some("avatars"))
        .expect("list objects");
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].uri.key, "avatars/santi.svg");
    let objects = service
        .list_bucket_objects(&bucket, Some("avatars/santi"))
        .expect("list object prefix");
    assert_eq!(objects.len(), 1);
    assert!(service.delete_bucket_object(&uri).expect("delete object"));
    assert!(
        service
            .get_bucket_object("soul_default", &strand.id, "avatars/santi.svg")
            .expect("get deleted object")
            .is_none()
    );
}

#[tokio::test]
async fn bucket_rejects_unsafe_keys() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = Service::open(
        service::Config {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: temp.path().join("runtime").display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
            constitution_path: None,
        },
        Arc::new(FakeProvider::default()),
    )
    .expect("open service");
    let strand = service.create_strand().expect("create strand").strand;

    assert!(
        service
            .get_bucket_object("soul_default", &strand.id, "../escape.txt")
            .expect_err("unsafe key")
            .contains("object key")
    );
    assert!(
        service
            .get_bucket_object("soul_default", &strand.id, "bad//key.txt")
            .expect_err("empty segment")
            .contains("object key")
    );
    assert!(
        service
            .get_bucket_object("unknown_soul", &strand.id, "safe.txt")
            .expect_err("unknown soul")
            .contains("soul not found")
    );
}
