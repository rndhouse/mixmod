use super::*;

#[test]
fn fixture_task_metadata_stays_out_of_worker_workdirs() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("pkg")).unwrap();
    atomic_write(
        &root.join(TASK_JSON),
        br#"{
  "title": "Fixture task",
  "instructions": "Use this as Mixmod experiment metadata.",
  "files": [],
  "tests": []
}"#,
    )
    .unwrap();
    atomic_write(&root.join(TASK_MD), b"# Fixture task\n").unwrap();
    atomic_write(&root.join("README.md"), b"fixture repo\n").unwrap();
    atomic_write(&root.join("pkg/task.json"), b"{\"project\":true}\n").unwrap();

    experiment_init(root, "fixture-meta", Some(root)).unwrap();

    let exp_dir = state_layout(root).experiments().join("fixture-meta");
    let task = read_json_file(&exp_dir.join(TASK_JSON)).unwrap();
    assert_eq!(get_str(&task, "title"), Some("Fixture task"));

    for workdir in ["mixmod", "default"] {
        let workdir = exp_dir.join("work").join(workdir);
        assert!(!workdir.join(TASK_JSON).exists());
        assert!(!workdir.join(TASK_MD).exists());
        assert!(workdir.join("README.md").exists());
        assert!(workdir.join("pkg/task.json").exists());
    }
}
