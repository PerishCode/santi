use super::*;

#[tokio::test]
async fn rejects_tool_batch() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (service, provider) = budget_service(
        &temp,
        vec![BudgetProviderStep::Calls {
            count: 2,
            output_bytes: 1,
        }],
    );
    let strand = service.create_strand().expect("create strand").strand;
    service
        .set_strand_execution_budget(&strand.id, execution_budget(4, 1, 100, 50))
        .expect("set budget");
    let response = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "request an oversized batch".to_string(),
                }],
            },
        )
        .await
        .expect("send");

    let runtime = Probe::new(&service)
        .failed_turn(&strand.id, &accepted_turn(&response).id)
        .await;
    assert!(runtime.calls.is_empty());
    assert!(runtime.effects.is_empty());
    let incident = runtime
        .errors
        .iter()
        .find(|incident| incident.code == "runtime.execution_budget.exceeded")
        .expect("execution budget incident");
    assert_eq!(incident.first.context["reason"], "calls");
    assert_eq!(incident.first.context["request"]["calls"], 2);
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn reserves_followup_round() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (service, provider) = budget_service(
        &temp,
        vec![
            BudgetProviderStep::Calls {
                count: 1,
                output_bytes: 1,
            },
            BudgetProviderStep::Calls {
                count: 1,
                output_bytes: 1,
            },
        ],
    );
    let strand = service.create_strand().expect("create strand").strand;
    service
        .set_strand_execution_budget(&strand.id, execution_budget(2, 4, 100, 50))
        .expect("set budget");
    let response = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "keep calling tools".to_string(),
                }],
            },
        )
        .await
        .expect("send");

    let runtime = Probe::new(&service)
        .failed_turn(&strand.id, &accepted_turn(&response).id)
        .await;
    assert_eq!(runtime.calls.len(), 1);
    assert_eq!(runtime.effects.len(), 1);
    let incident = runtime
        .errors
        .iter()
        .find(|incident| incident.code == "runtime.execution_budget.exceeded")
        .expect("execution budget incident");
    assert_eq!(incident.first.context["reason"], "provider_rounds");
    assert_eq!(incident.first.context["request"]["provider_round"], 2);
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn bounds_shell_capture() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (service, provider) = budget_service(
        &temp,
        vec![
            BudgetProviderStep::Calls {
                count: 1,
                output_bytes: 100,
            },
            BudgetProviderStep::Calls {
                count: 1,
                output_bytes: 100,
            },
            BudgetProviderStep::Complete,
        ],
    );
    let strand = service.create_strand().expect("create strand").strand;
    service
        .set_strand_execution_budget(&strand.id, execution_budget(3, 2, 10, 6))
        .expect("set budget");
    let response = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "capture bounded output".to_string(),
                }],
            },
        )
        .await
        .expect("send");

    let runtime = Probe::new(&service)
        .completed_turn(&strand.id, &accepted_turn(&response).id)
        .await;
    assert_eq!(runtime.results.len(), 2);
    let captured_bytes = runtime
        .results
        .iter()
        .map(|result| {
            let output = result.output.as_ref().expect("captured output");
            assert_eq!(output["output_truncated"], true);
            output["stdout"].as_str().map_or(0, str::len)
                + output["stderr"].as_str().map_or(0, str::len)
        })
        .sum::<usize>();
    assert_eq!(captured_bytes, 10);
    assert_eq!(
        runtime.results[0].output.as_ref().unwrap()["output_limit_bytes"],
        6
    );
    assert_eq!(
        runtime.results[1].output.as_ref().unwrap()["output_limit_bytes"],
        4
    );
    assert_eq!(provider.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn preserves_retry_usage() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (service, provider) = budget_service(
        &temp,
        vec![
            BudgetProviderStep::Calls {
                count: 1,
                output_bytes: 1,
            },
            BudgetProviderStep::Fail("temporary provider failure".to_string()),
            BudgetProviderStep::Calls {
                count: 1,
                output_bytes: 1,
            },
        ],
    );
    let strand = service.create_strand().expect("create strand").strand;
    service
        .set_strand_execution_budget(&strand.id, execution_budget(4, 1, 100, 50))
        .expect("set budget");
    let first = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "first attempt".to_string(),
                }],
            },
        )
        .await
        .expect("first send");
    Probe::new(&service)
        .failed_turn(&strand.id, &accepted_turn(&first).id)
        .await;

    let retry = service
        .send_strand(
            &strand.id,
            SendStrandRequest {
                content: vec![MessagePart::Text {
                    text: "retry".to_string(),
                }],
            },
        )
        .await
        .expect("retry send");
    let runtime = Probe::new(&service)
        .failed_turn(&strand.id, &accepted_turn(&retry).id)
        .await;
    assert_eq!(runtime.calls.len(), 1);
    let incident = runtime
        .errors
        .iter()
        .find(|incident| incident.code == "runtime.execution_budget.exceeded")
        .expect("execution budget incident");
    assert_eq!(incident.first.context["reason"], "calls");
    assert_eq!(incident.first.context["usage"]["calls"], 1);
    assert_eq!(provider.requests.lock().unwrap().len(), 3);
}
