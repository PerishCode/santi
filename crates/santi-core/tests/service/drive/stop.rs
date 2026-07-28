use crate::support::*;
use santi_core::service::{self, Service};
use santi_core::{effect, message, strand, turn};

#[tokio::test]
async fn interrupts() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(GatedFirstProvider::new());
    let service = opened(&temp, provider.clone());
    let strand = service.weave().expect("create strand").strand;
    let first = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "first".to_string(),
                }],
            },
        )
        .await
        .expect("send first");
    let turn = accepted_turn(&first).id.clone();
    provider.wait_for_first_request().await;
    let second = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "second".to_string(),
                }],
            },
        )
        .await
        .expect("queue second");
    assert_eq!(accepted_turn(&second).id, turn);

    let requested = service
        .stop(&turn)
        .expect("request stop")
        .expect("turn exists");
    assert!(requested.accepted);
    assert_eq!(requested.cause, Some(turn::Cause::Operator));
    let repeated = service
        .stop(&turn)
        .expect("repeat stop")
        .expect("turn exists");
    assert_eq!(repeated.requested, requested.requested);

    let runtime = Probe::new(&service).failed_turn(&strand.id, &turn).await;
    assert!(
        runtime.errors.is_empty(),
        "operator stop is not an incident"
    );
    assert!(runtime.messages.iter().any(|message| {
        message.message.kind == message::Kind::SantiSystem
            && message.text.contains("interrupted by operator")
    }));
    let runtime = Probe::new(&service).any_completed(&strand.id).await;
    assert_eq!(
        runtime
            .turns
            .iter()
            .filter(|held| held.status == turn::Status::Completed)
            .count(),
        1
    );
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let input = provider_messages(&requests[1]);
    assert!(input.iter().any(|(_, text)| text.contains("second")));
    assert!(
        input
            .iter()
            .any(|(_, text)| text.contains("interrupted by operator"))
    );
    drop(requests);
    let settled = service
        .stop(&turn)
        .expect("query settled stop")
        .expect("turn exists");
    assert!(settled.settled.is_some());
}

#[tokio::test]
async fn drains() {
    let temp = tempfile::tempdir().expect("temp dir");
    let provider = Arc::new(GatedFirstProvider::new());
    let service = opened(&temp, provider.clone());
    let strand = service.weave().expect("create strand").strand;
    let posted = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "wait".to_string(),
                }],
            },
        )
        .await
        .expect("send");
    let turn = accepted_turn(&posted).id.clone();
    provider.wait_for_first_request().await;

    service.quiesce(Duration::from_millis(100));
    sleep(Duration::from_millis(20)).await;
    let running = service
        .snapshot(&strand.id)
        .expect("snapshot")
        .expect("strand");
    assert_eq!(running.turns[0].status, turn::Status::Running);
    tokio::time::timeout(Duration::from_secs(2), service.drain())
        .await
        .expect("drain completes");

    let runtime = Probe::new(&service).failed_turn(&strand.id, &turn).await;
    let stopped = runtime
        .turns
        .iter()
        .find(|held| held.id == turn)
        .expect("stopped turn");
    assert_eq!(stopped.error.as_deref(), Some("interrupted by shutdown"));
    let projected = service
        .stop(&turn)
        .expect("query stop")
        .expect("turn exists");
    assert_eq!(projected.cause, Some(turn::Cause::Shutdown));
}

#[cfg(unix)]
#[tokio::test]
async fn kills() {
    let temp = tempfile::tempdir().expect("temp dir");
    let pidfile = temp.path().join("child.pid");
    let provider = Arc::new(Sheller {
        pidfile: pidfile.clone(),
        requests: Arc::new(Mutex::new(0)),
    });
    let service = opened(&temp, provider);
    let strand = service.weave().expect("create strand").strand;
    let posted = service
        .send(
            &strand.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "run shell".to_string(),
                }],
            },
        )
        .await
        .expect("send");
    let turn = accepted_turn(&posted).id.clone();
    for _ in 0..100 {
        if pidfile.is_file() {
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    let pid = fs::read_to_string(&pidfile)
        .expect("child pid")
        .parse::<i32>()
        .expect("numeric pid");
    service.stop(&turn).expect("stop shell turn");
    let runtime = Probe::new(&service).failed_turn(&strand.id, &turn).await;
    assert_eq!(runtime.effects.len(), 1);
    assert_eq!(runtime.effects[0].state, effect::State::Unknown);
    for _ in 0..100 {
        if gone(pid) {
            return;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("shell child process {pid} survived turn stop");
}

#[cfg(unix)]
fn gone(pid: i32) -> bool {
    if unsafe { libc::kill(pid, 0) } != 0 {
        return true;
    }
    #[cfg(target_os = "linux")]
    if fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| stat.split_whitespace().nth(2).map(str::to_string))
        .is_some_and(|state| state == "Z")
    {
        return true;
    }
    false
}

fn opened(temp: &tempfile::TempDir, provider: Arc<dyn Provider>) -> Service {
    Service::open(
        service::Config {
            database: temp.path().join("santi.sqlite").display().to_string(),
            runtime: temp.path().join("runtime").display().to_string(),
            execution: temp.path().join("execution").display().to_string(),
            bind: Some("127.0.0.1:0".to_string()),
            constitution: None,
        },
        provider,
    )
    .expect("open service")
}

#[cfg(unix)]
struct Sheller {
    pidfile: std::path::PathBuf,
    requests: Arc<Mutex<usize>>,
}

#[cfg(unix)]
#[async_trait]
impl Provider for Sheller {
    fn metadata(&self) -> Metadata {
        Metadata {
            provider: Arc::from("shell-provider"),
            model: "shell-model".to_string(),
            budget: None,
        }
    }

    async fn stream(&self, _request: Request) -> Result<Streaming, String> {
        let mut requests = self.requests.lock().unwrap();
        *requests += 1;
        if *requests > 1 {
            return Ok(Box::pin(stream::iter(vec![Ok(Event::Completed {
                response: Some("done".to_string()),
            })])));
        }
        let command = format!(
            "sleep 30 & child=$!; printf '%s' \"$child\" > '{}'; wait \"$child\"",
            self.pidfile.display()
        );
        let arguments = json!({"command": command});
        let raw = arguments.to_string();
        Ok(Box::pin(stream::iter(vec![
            Ok(Event::Called(Call {
                response: "resp_shell".to_string(),
                mark: Some("item_shell".to_string()),
                item: json!({
                    "type": "function_call",
                    "id": "item_shell",
                    "call_id": "call_shell_stop",
                    "name": "shell",
                    "arguments": raw,
                }),
                call: "call_shell_stop".to_string(),
                name: "shell".to_string(),
                raw,
                arguments,
            })),
            Ok(Event::Completed {
                response: Some("resp_shell".to_string()),
            }),
        ])))
    }
}
