use super::*;
use santi_api::{end_soul_environ_handler, set_soul_environ_handler, soul_environs_handler};
use santi_core::environ;

#[tokio::test]
async fn roundtrips() {
    let temp = tempfile::tempdir().expect("temp dir");
    let service = Service::open(
        service::Config {
            database: temp.path().join("santi.sqlite").display().to_string(),
            runtime: temp.path().join("runtime").display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
            environment: Default::default(),
        },
        Arc::new(DriverProvider),
    )
    .await
    .expect("open service");

    let Json(written) = okay(
        set_soul_environ_handler(
            State(service.clone()),
            Path(santi_core::GENESIS.to_string()),
            Json(environ::Draft {
                name: "STIM_BASE_URL".to_string(),
                value: "https://stim.example.com".to_string(),
            }),
        )
        .await,
    );
    assert_eq!(written.scope, environ::Scope::Soul);
    assert_eq!(written.name, "STIM_BASE_URL");

    let Json(listed) = okay(
        soul_environs_handler(
            State(service.clone()),
            Path(santi_core::GENESIS.to_string()),
        )
        .await,
    );
    assert_eq!(listed, vec![written]);

    let error = set_soul_environ_handler(
        State(service.clone()),
        Path(santi_core::GENESIS.to_string()),
        Json(environ::Draft {
            name: "SANTI_TURN_ID".to_string(),
            value: "forged".to_string(),
        }),
    )
    .await
    .expect_err("reserved engine name must refuse");
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);

    let status = okay(
        end_soul_environ_handler(
            State(service.clone()),
            Path((santi_core::GENESIS.to_string(), "STIM_BASE_URL".to_string())),
        )
        .await,
    );
    assert_eq!(status, StatusCode::NO_CONTENT);

    let Json(listed) =
        okay(soul_environs_handler(State(service), Path(santi_core::GENESIS.to_string())).await);
    assert!(listed.is_empty());
}

fn okay<T>(result: Result<T, santi_api::ApiError>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!(
            "environment handler failed with {}: {}",
            error.code(),
            error.message()
        ),
    }
}
