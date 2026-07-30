use super::support::*;
use santi_core::service::{self, Service};
use santi_core::{environ, environment, message, strand};
use santi_provider::Call;
use serde_json::json;

#[derive(Clone, Default)]
struct ShellProvider {
    requests: Arc<Mutex<Vec<Request>>>,
}

fn declaration(scope: &str, name: &str, value: &str) -> environment::Declaration {
    environment::Declaration {
        scope: scope.to_string(),
        name: name.to_string(),
        value: value.to_string(),
    }
}

#[test]
fn cascade_and_references() {
    let held = environment::resolve(
        [
            declaration("global", "A", "global"),
            declaration("soul", "A", "soul"),
            declaration("strand", "B", "env:// KEY "),
        ],
        &|name| (name == "KEY").then(|| "resolved".to_string()),
    );
    assert_eq!(held.values.get("A").map(String::as_str), Some("soul"));
    assert_eq!(held.values.get("B").map(String::as_str), Some("resolved"));
    assert!(held.unresolved.is_empty());
}

#[test]
fn passthrough() {
    let held = environment::resolve([declaration("strand", "A", "env://MISSING")], &|_| None);
    assert_eq!(
        held.values.get("A").map(String::as_str),
        Some("env://MISSING")
    );
    assert_eq!(held.unresolved.len(), 1);
    let one = &held.unresolved[0];
    let two = environment::Unresolved {
        reference: "OTHER".to_string(),
        ..one.clone()
    };
    assert_ne!(one.dedupe("ss_1"), two.dedupe("ss_1"));
    assert_ne!(one.dedupe("ss_1"), one.dedupe("ss_2"));
}

#[test]
fn precedence() {
    let held = environment::resolve(
        [
            declaration("soul", "A", ""),
            declaration("soul", "SANTI_TURN_ID", "forged"),
        ],
        &|_| None,
    );
    assert_eq!(held.values.get("A").map(String::as_str), Some(""));
    assert!(!held.values.contains_key("SANTI_TURN_ID"));
}

#[test]
fn portability() {
    assert!(environment::legal("STIM_BASE_URL").is_ok());
    assert!(environment::legal("_STIM_1").is_ok());
    assert!(environment::legal("").is_err());
    assert!(environment::legal("NOT-PORTABLE").is_err());
    assert!(environment::legal("SANTI_TURN_ID").is_err());
}

#[test]
fn allowlist() {
    let allowed = environment::allowed();
    assert!(allowed.contains("PATH"));
    assert!(!allowed.contains("OPENAI_API_KEY"));
    assert!(
        !allowed
            .iter()
            .any(|name| name.starts_with(environment::RESERVED))
    );
}

#[async_trait]
impl Provider for ShellProvider {
    fn metadata(&self) -> Metadata {
        Metadata {
            provider: Arc::from("shell-provider"),
            model: "shell-model".to_string(),
            budget: None,
        }
    }

    async fn stream(&self, request: Request) -> Result<Streaming, String> {
        let round = {
            let mut requests = self.requests.lock().unwrap();
            let round = requests.len();
            requests.push(request);
            round
        };
        if round == 0 {
            let arguments = json!({
                "command": "printf '%s|%s|%s|%s|%s|%s' \"$SHARED\" \"$STRAND_ONLY\" \"$GLOBAL_ONLY\" \"$BROKEN\" \"${SANTI_RUNTIME_CAPABILITY%%.*}\" \"${SANTI_CAPABILITY_PRIVATE_KEY-unset}\""
            });
            Ok(Box::pin(stream::iter(vec![
                Ok(Event::Called(Call {
                    response: "response_env".to_string(),
                    mark: None,
                    item: json!({"type": "function_call"}),
                    call: "call_env".to_string(),
                    name: "shell".to_string(),
                    raw: arguments.to_string(),
                    arguments,
                })),
                Ok(Event::Completed {
                    response: Some("response_env".to_string()),
                }),
            ])))
        } else {
            Ok(Box::pin(stream::iter(vec![
                Ok(Event::Text("environment checked".to_string())),
                Ok(Event::Completed {
                    response: Some("response_done".to_string()),
                }),
            ])))
        }
    }
}

#[tokio::test]
async fn cascade() {
    let temp = tempfile::tempdir().expect("temp dir");
    bootstrap(&temp).await;
    let provider = Arc::new(ShellProvider::default());
    let service = Service::open(
        service::Config {
            database: temp.path().join("santi.sqlite").display().to_string(),
            runtime: temp.path().join("runtime").display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
            environment: [
                ("SHARED".to_string(), "global".to_string()),
                ("GLOBAL_ONLY".to_string(), "global-only".to_string()),
            ]
            .into_iter()
            .collect(),
        },
        provider.clone(),
    )
    .await
    .expect("open service")
    .authorized(Some(
        santi_core::capability::Issuer::new(
            "santi.example",
            "stim.reply",
            santi_core::capability::Key {
                id: "test-2026",
                private: "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc",
            },
            120,
        )
        .expect("test issuer"),
    ));
    service
        .set_environ(
            environ::Scope::Soul,
            santi_core::GENESIS,
            environ::Draft {
                name: "SHARED".to_string(),
                value: "soul".to_string(),
            },
        )
        .await
        .expect("set soul override");
    service
        .set_environ(
            environ::Scope::Soul,
            santi_core::GENESIS,
            environ::Draft {
                name: "BROKEN".to_string(),
                value: "env://SANTI_TEST_REFERENCE_THAT_DOES_NOT_EXIST".to_string(),
            },
        )
        .await
        .expect("set unresolved reference");

    let strand = service.weave().await.expect("create strand").strand;
    service
        .set_environ(
            environ::Scope::Strand,
            &strand.id,
            environ::Draft {
                name: "SHARED".to_string(),
                value: "strand".to_string(),
            },
        )
        .await
        .expect("set strand override");
    service
        .set_environ(
            environ::Scope::Strand,
            &strand.id,
            environ::Draft {
                name: "STRAND_ONLY".to_string(),
                value: "strand-only".to_string(),
            },
        )
        .await
        .expect("set strand value");

    let response = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "inspect the environment".to_string(),
                }],
            },
        )
        .await
        .expect("send strand");
    Probe::new(&service)
        .completed_turn(&strand.id, &accepted_turn(&response).id)
        .await;

    let requests = provider.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].input.iter().any(|item| {
        matches!(
            item,
            Item::Output { output, .. }
                if output.contains(
                    "strand|strand-only|global-only|env://SANTI_TEST_REFERENCE_THAT_DOES_NOT_EXIST|santi1|unset"
                )
        )
    }));

    let mut detail = service
        .strand(&strand.id)
        .await
        .expect("read strand")
        .expect("strand");
    for _ in 0..50 {
        if detail.messages.iter().any(|placed| {
            placed.message.kind == message::Kind::SantiSystem
                && placed.text.contains("environment_unresolved")
        }) {
            return;
        }
        sleep(Duration::from_millis(20)).await;
        detail = service
            .strand(&strand.id)
            .await
            .expect("read strand")
            .expect("strand");
    }
    panic!("unresolved environment system message did not land");
}
