use super::super::*;

#[test]
fn subtracts_unchanged_preexisting_diff_blocks() {
    let before = "diff --git a/task.json b/task.json\nnew file mode 100644\n--- /dev/null\n+++ b/task.json\n@@ -0,0 +1,1 @@\n+{}\n";
    let after = format!(
        "{before}diff --git a/src/generated.rs b/src/generated.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/generated.rs\n@@ -0,0 +1,1 @@\n+pub fn generated() {{}}\n"
    );
    let filtered = diff_without_unchanged_blocks(&after, before);
    assert!(!filtered.contains("task.json"));
    assert!(filtered.contains("src/generated.rs"));
}
