use super::*;

use santi_core::service::{self, Service};
use santi_core::{ingest, message, strand};

const INPUT_BUDGET_BYTES: usize = 32 * 1024;
const MEMORY_ALLOWANCE_BYTES: usize = INPUT_BUDGET_BYTES / 2;
const OPERATOR_THRESHOLD_BYTES: usize = INPUT_BUDGET_BYTES * 3 / 4;
const MAINTENANCE_LABEL: &str = "santi:memory:maintenance";

#[derive(Clone)]
struct MemoryOrganizingProvider {
    requests: Arc<Mutex<Vec<Request>>>,
    memory_path: std::path::PathBuf,
    first_request_seen: Arc<Notify>,
    release_first_request: Arc<Notify>,
}

impl MemoryOrganizingProvider {
    fn new(memory_path: std::path::PathBuf) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            memory_path,
            first_request_seen: Arc::new(Notify::new()),
            release_first_request: Arc::new(Notify::new()),
        }
    }

    async fn wait_for_maintenance_request(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.first_request_seen.notified())
            .await
            .expect("maintenance provider request observed");
    }

    fn release_maintenance(&self) {
        self.release_first_request.notify_one();
    }
}

#[async_trait]
impl Provider for MemoryOrganizingProvider {
    fn metadata(&self) -> Metadata {
        Metadata {
            provider: Arc::from("memory-organizing-provider"),
            model: "memory-organizing-model".to_string(),
            budget: Some(Cap {
                bytes: INPUT_BUDGET_BYTES,
                source: "test".to_string(),
            }),
        }
    }

    async fn stream(&self, request: Request) -> Result<Streaming, String> {
        let request_index = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len()
        };
        if request_index == 1 {
            self.first_request_seen.notify_one();
            self.release_first_request.notified().await;
            fs::write(
                &self.memory_path,
                "# Organized memory\n\nOnly live facts remain.\n",
            )
            .map_err(|error| error.to_string())?;
        }
        Ok(Box::pin(stream::iter(vec![
            Ok(Event::Text(format!("provider response {request_index}"))),
            Ok(Event::Completed {
                response: Some(format!("memory-response-{request_index}")),
            }),
        ])))
    }
}

#[tokio::test]
async fn pressure_lifecycle() {
    let temp = tempfile::tempdir().expect("temp dir");
    let runtime_root = temp.path().join("runtime");
    let memory_path = runtime_root
        .join("souls")
        .join(santi_core::DEFAULT_SOUL_ID)
        .join("memory")
        .join("MEMORY.md");
    fs::create_dir_all(memory_path.parent().expect("memory parent")).expect("create memory parent");
    let oversized_memory = format!(
        "# Oversized memory\n{}\nSOURCE_TAIL_MUST_NOT_APPEAR",
        "x".repeat(OPERATOR_THRESHOLD_BYTES + 256)
    );
    fs::write(&memory_path, &oversized_memory).expect("write oversized memory");

    let provider = Arc::new(MemoryOrganizingProvider::new(memory_path.clone()));
    let service = Service::open(
        service::Config {
            database_path: temp.path().join("santi.sqlite").display().to_string(),
            runtime_root: runtime_root.display().to_string(),
            execution_root: temp.path().join("execution").display().to_string(),
            bind_addr: Some("127.0.0.1:0".to_string()),
            constitution_path: None,
        },
        provider.clone(),
    )
    .expect("open service");
    let first = service.create_strand().expect("first strand").strand;
    let second = service.create_strand().expect("second strand").strand;

    let first_send = service
        .send_strand(
            &first.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "first queued request".to_string(),
                }],
            },
        )
        .await
        .expect("pressure still accepts first request");
    assert!(first_send.turn.is_none(), "ordinary strand must be paused");
    provider.wait_for_maintenance_request().await;

    let second_send = service
        .send_strand(
            &second.id,
            strand::Post {
                content: vec![message::Part::Text {
                    text: "second queued request".to_string(),
                }],
            },
        )
        .await
        .expect("pressure still accepts second request");
    assert!(
        second_send.turn.is_none(),
        "all ordinary strands must pause"
    );
    assert_eq!(
        fs::read_to_string(&memory_path).expect("memory remains readable"),
        oversized_memory,
        "runtime must not mutate the source memory"
    );

    let strands = service.list_strands().expect("list strands");
    let maintenance = strands
        .iter()
        .find(|strand| strand.label.as_deref() == Some(MAINTENANCE_LABEL))
        .expect("dedicated maintenance strand");
    assert_eq!(
        strands
            .iter()
            .filter(|strand| strand.label.as_deref() == Some(MAINTENANCE_LABEL))
            .count(),
        1,
        "one soul gets exactly one maintenance strand"
    );
    let maintenance_runtime = service
        .runtime_snapshot(&maintenance.id)
        .expect("maintenance runtime")
        .expect("maintenance strand");
    assert_eq!(
        maintenance_runtime
            .messages
            .iter()
            .filter(|message| message.text.contains("kind: soul_memory_maintenance"))
            .count(),
        1,
        "the same source revision must not enqueue duplicate metaprompts"
    );
    assert!(
        maintenance_runtime.messages.iter().any(|message| {
            message
                .text
                .contains("Do not echo the whole file into provider context")
        }),
        "metaprompt must teach bounded inspection"
    );

    {
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 1, "only maintenance may run under pressure");
        let maintenance_request = &requests[0];
        let instructions = maintenance_request
            .instructions
            .as_deref()
            .expect("maintenance instructions");
        assert!(instructions.contains("kind: soul_memory_projection"));
        assert!(instructions.contains(&format!("allowance_bytes: {MEMORY_ALLOWANCE_BYTES}")));
        assert!(!instructions.contains("SOURCE_TAIL_MUST_NOT_APPEAR"));
        assert!(
            provider_messages(maintenance_request)
                .iter()
                .any(|(_, content)| content.contains("kind: soul_memory_maintenance"))
        );
    }

    let active = service
        .errors(
            &santi_core::Scope::new("soul", santi_core::DEFAULT_SOUL_ID),
            10,
        )
        .expect("soul incidents");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].code, "runtime.soul_memory.intervention_required");
    assert_eq!(active[0].status, santi_core::Status::Active);
    assert!(!active[0].exposure.caller);
    assert!(active[0].exposure.operator);

    provider.release_maintenance();
    Probe::new(&service).any_completed(&first.id).await;
    Probe::new(&service).any_completed(&second.id).await;

    {
        let requests = provider.requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            3,
            "maintenance relief resumes both queued strands"
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| provider_messages(request)
                    .iter()
                    .any(|(_, content)| content.contains("kind: soul_memory_maintenance")))
                .count(),
            1,
            "maintenance is not duplicated"
        );
    }

    for (strand, expected) in [
        (&first, "first queued request"),
        (&second, "second queued request"),
    ] {
        let runtime = service
            .runtime_snapshot(&strand.id)
            .expect("ordinary runtime")
            .expect("ordinary strand");
        assert!(
            runtime
                .messages
                .iter()
                .any(|message| message.text == expected)
        );
        assert_eq!(
            runtime
                .turns
                .iter()
                .filter(|turn| turn.status == santi_core::turn::Status::Completed)
                .count(),
            1
        );
    }

    let resolved = service
        .errors(
            &santi_core::Scope::new("soul", santi_core::DEFAULT_SOUL_ID),
            10,
        )
        .expect("resolved soul incidents");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].status, santi_core::Status::Resolved);

    let after_relief = service
        .ingest(
            santi_core::strand::Selector::ById(first.id.clone()),
            message::Content::text("normal ingest after relief"),
            message::Kind::Text,
            "strand_send",
        )
        .expect("normal ingest");
    let ingest::Outcome::Accepted { receipt } = after_relief else {
        panic!("normal ingest was rejected after relief");
    };
    assert!(
        receipt.warning.is_none(),
        "normal ingest drive failed after relief: {:?}",
        receipt.warning
    );
    Probe::new(&service).completed_count(&first.id, 2).await;
}
