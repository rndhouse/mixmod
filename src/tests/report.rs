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
