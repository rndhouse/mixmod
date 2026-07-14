use super::*;
use clap::Parser;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::time::Duration;
use tempfile::TempDir;

struct FakeRunner;

impl AgentHarness for FakeRunner {
    fn run(&self, request: &AgentRequest) -> Result<AgentOutput> {
        fs::create_dir_all(request.root.join("src")).with_context(|| {
            format!(
                "failed to create fake source dir under {}",
                request.root.display()
            )
        })?;
        atomic_write(
            &request.root.join("src/generated.rs"),
            b"pub fn generated() -> &'static str {\n    \"ok\"\n}\n",
        )?;
        Ok(fake_successful_opencode_output(
            request,
            b"Summary: generated a file\nTests: not run\n".to_vec(),
            request.session_id.clone(),
            Vec::new(),
            request.resume_session_id.clone(),
        ))
    }
}

struct EmptyPatchThenPatchRunner {
    calls: AtomicUsize,
}

impl EmptyPatchThenPatchRunner {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl AgentHarness for EmptyPatchThenPatchRunner {
    fn run(&self, request: &AgentRequest) -> Result<AgentOutput> {
        let call = self.calls.fetch_add(1, AtomicOrdering::SeqCst);
        let (stdout, resume_session_id) = if call == 0 {
            assert!(request.resume_session_id.is_none());
            (
                b"Summary: I found the edit but did not modify files.\n".to_vec(),
                None,
            )
        } else {
            assert_eq!(
                request.resume_session_id.as_deref(),
                Some("ses_empty_patch")
            );
            assert!(request.instruction.contains("Empty-Patch Follow-Up"));
            fs::create_dir_all(request.root.join("src"))?;
            atomic_write(
                &request.root.join("src/generated.rs"),
                b"pub fn generated() -> &'static str {\n    \"followup\"\n}\n",
            )?;
            (
                b"Summary: made the intended edit after empty-patch follow-up.\n".to_vec(),
                Some("ses_empty_patch".to_string()),
            )
        };
        Ok(fake_successful_opencode_output(
            request,
            stdout,
            "ses_empty_patch".to_string(),
            vec![json!({"call": call})],
            resume_session_id,
        ))
    }
}

struct RevisionNoopThenPatchRunner {
    calls: AtomicUsize,
}

impl RevisionNoopThenPatchRunner {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl AgentHarness for RevisionNoopThenPatchRunner {
    fn run(&self, request: &AgentRequest) -> Result<AgentOutput> {
        let call = self.calls.fetch_add(1, AtomicOrdering::SeqCst);
        let stdout = if call == 0 {
            assert_eq!(request.resume_session_id.as_deref(), Some("ses_revision"));
            b"Summary: inspected files but made no revision delta.\n".to_vec()
        } else {
            assert_eq!(request.resume_session_id.as_deref(), Some("ses_revision"));
            assert!(request.instruction.contains("Revision No-Op Follow-Up"));
            assert!(
                request
                    .instruction
                    .contains("Your previous revision turn made no repository changes")
            );
            assert!(
                request
                    .instruction
                    .contains("Apply the exact requested revision now.")
            );
            assert!(request.instruction.contains("BLOCKED"));
            assert!(request.instruction.contains("Do not only inspect files"));
            fs::create_dir_all(request.root.join("src"))?;
            atomic_write(
                &request.root.join("src/revised.rs"),
                b"pub fn revised() -> &'static str {\n    \"done\"\n}\n",
            )?;
            b"Summary: applied the requested revision delta.\n".to_vec()
        };

        Ok(fake_successful_opencode_output(
            request,
            stdout,
            "ses_revision".to_string(),
            vec![json!({"call": call})],
            request.resume_session_id.clone(),
        ))
    }
}

struct PatchThenSelfReviewRunner {
    calls: AtomicUsize,
}

impl PatchThenSelfReviewRunner {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl AgentHarness for PatchThenSelfReviewRunner {
    fn run(&self, request: &AgentRequest) -> Result<AgentOutput> {
        let call = self.calls.fetch_add(1, AtomicOrdering::SeqCst);
        let stdout = if call == 0 {
            assert!(request.resume_session_id.is_none());
            fs::create_dir_all(request.root.join("src"))?;
            atomic_write(
                &request.root.join("src/generated.rs"),
                b"pub fn generated() -> &'static str {\n    \"ok\" // temporary debug marker\n}\n",
            )?;
            b"Changed files: src/generated.rs\n".to_vec()
        } else {
            assert_eq!(
                request.resume_session_id.as_deref(),
                Some("ses_self_review")
            );
            assert!(request.instruction.contains("Worker Self-Review Cleanup"));
            assert!(
                request
                    .instruction
                    .contains("not a new implementation slice")
            );
            assert!(request.instruction.contains("worker-session.patch"));
            assert!(request.instruction.contains("src/generated.rs"));
            assert!(!request.instruction.contains("README.md"));
            let review_patch =
                fs::read_to_string(request.out_dir.join("worker-session.patch")).unwrap();
            assert!(review_patch.contains("src/generated.rs"));
            assert!(!review_patch.contains("README.md"));
            atomic_write(
                &request.root.join("src/generated.rs"),
                b"pub fn generated() -> &'static str {\n    \"ok\"\n}\n",
            )?;
            b"Cleanup changed files: src/generated.rs\nConcerns: none\n".to_vec()
        };

        Ok(fake_successful_opencode_output(
            request,
            stdout,
            "ses_self_review".to_string(),
            vec![json!({"call": call})],
            request.resume_session_id.clone(),
        ))
    }
}

fn init_git(root: &Path) {
    Command::new("git")
        .arg("init")
        .current_dir(root)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(root)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(root)
        .output()
        .unwrap();
}

fn minimal_opencode_output() -> AgentOutput {
    AgentOutput {
        backend: AgentBackend::OpenCode,
        command_for_metrics: Vec::new(),
        segments: Vec::new(),
        exit_status: None,
        success: false,
        stdout: Vec::new(),
        stderr: Vec::new(),
        provider: None,
        model: None,
        model_arg: None,
        session_label: None,
        session_id: None,
        resume_session_id: None,
        session_reused: false,
        interrupted_by_supervisor: false,
        supervisor_control_action: None,
        supervisor_control_events: Vec::new(),
        timed_out: false,
        idle_timed_out: false,
        heartbeat_count: 0,
        require_local: false,
        local_inference_verified: false,
        gpu_activity_observed: false,
        backend_activity_observed: false,
        verification_notes: Vec::new(),
    }
}

fn fake_successful_opencode_output(
    request: &AgentRequest,
    stdout: Vec<u8>,
    session_id: String,
    segments: Vec<Value>,
    resume_session_id: Option<String>,
) -> AgentOutput {
    let mut output = minimal_opencode_output();
    output.command_for_metrics = vec!["fake-opencode".to_string()];
    output.segments = segments;
    output.exit_status = Some(0);
    output.success = true;
    output.stdout = stdout;
    output.provider = Some("fake-local".to_string());
    output.model = Some(DEFAULT_OPENCODE_LOCAL_MODEL.to_string());
    output.model_arg = Some(format!("fake-local/{DEFAULT_OPENCODE_LOCAL_MODEL}"));
    output.session_label = Some(request.session_id.clone());
    output.session_id = Some(session_id);
    output.resume_session_id = resume_session_id;
    output.session_reused = request.resume_session_id.is_some();
    output
}

mod core;
mod experiment;
mod install;
mod opencode;
mod report;
mod run;
mod supervisor;
mod tasks;
