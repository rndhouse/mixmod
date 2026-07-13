use super::*;

#[test]
fn experiment_report_handles_missing_telemetry() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    experiment_init(root, "demo", None).unwrap();
    let report = experiment_report(root, "demo").unwrap();
    assert!(report.contains("Exact token telemetry"));
    assert!(
        state_layout(root)
            .experiments()
            .join("demo/report.md")
            .exists()
    );
    assert!(!root.join(".mixmod").exists());
}

#[test]
fn experiment_report_rejects_last_request_token_comparison() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    experiment_init(root, "demo", None).unwrap();
    let exp_dir = state_layout(root).experiments().join("demo");
    fs::create_dir_all(exp_dir.join("codex-only")).unwrap();
    fs::create_dir_all(exp_dir.join("default")).unwrap();
    write_pretty_json(
        &exp_dir.join("codex-only/metrics.json"),
        &json!({
            "final_status": "success",
            "supervisor_input_tokens": 1000,
            "supervisor_output_tokens": 100,
            "supervisor_total_tokens": 1100,
            "codex_token_usage_source": "codex_rollout_total_token_usage"
        }),
        "codex-only metrics",
    )
    .unwrap();
    write_pretty_json(
        &exp_dir.join("default/metrics.json"),
        &json!({
            "final_status": "success",
            "supervisor_input_tokens": 10,
            "supervisor_output_tokens": 1,
            "supervisor_total_tokens": 11,
            "supervisor_token_usage_source": "codex_app_server_last_token_usage",
            "supervisor_token_usage_scope": "last_request",
            "supervisor_token_usage_comparable": false
        }),
        "default metrics",
    )
    .unwrap();

    let report = experiment_report(root, "demo").unwrap();

    assert!(report.contains("Exact token telemetry is not comparable"));
    assert!(!report.contains("Mixmod used fewer measured Codex tokens"));
}
