use std::env;

use anyhow::Result;

use crate::MixmodConfig;
use crate::harness::{AgentBackend, AgentHarness, AgentOutput, AgentRequest};

mod args;
mod config;
mod control;
mod process;
mod verification;

pub(crate) use args::prepare_opencode_args;
#[cfg(test)]
pub(crate) use args::prepare_opencode_control_args;
use args::{redact_opencode_arg, render_opencode_arg};
#[cfg(test)]
pub(crate) use config::{OpenCodeModelSelection, opencode_config_path};
use config::{resolve_opencode_model, resolve_opencode_session_id};
pub(crate) use control::{
    normalize_supervisor_control_action, normalize_supervisor_control_worker_mode, tail_text,
};
#[cfg(test)]
pub(crate) use verification::run_with_local_verification;
#[cfg(not(test))]
use verification::run_with_local_verification;

pub struct ShellOpenCodeRunner {
    config: MixmodConfig,
}

impl ShellOpenCodeRunner {
    pub fn new(config: MixmodConfig) -> Self {
        Self { config }
    }
}

impl AgentHarness for ShellOpenCodeRunner {
    fn run(&self, request: &AgentRequest) -> Result<AgentOutput> {
        let command = env::var("MIXMOD_OPENCODE_COMMAND")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| self.config.opencode.command.clone());
        let selection = resolve_opencode_model(
            &command,
            &request.root,
            &self.config.opencode,
            request.require_local,
        )?;
        let args = env::var("MIXMOD_OPENCODE_ARGS")
            .ok()
            .map(|value| {
                value
                    .split_whitespace()
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .filter(|args| !args.is_empty())
            .unwrap_or_else(|| self.config.opencode.args.clone());

        let rendered_args = args
            .iter()
            .map(|arg| render_opencode_arg(arg, request, &selection))
            .collect::<Vec<_>>();
        let rendered_args =
            prepare_opencode_args(rendered_args, request.resume_session_id.as_deref());

        let mut command_for_metrics = Vec::with_capacity(rendered_args.len() + 1);
        command_for_metrics.push(command.clone());
        let metric_args = args
            .iter()
            .map(|arg| redact_opencode_arg(arg, request, &selection))
            .collect::<Vec<_>>();
        command_for_metrics.extend(prepare_opencode_args(
            metric_args,
            request.resume_session_id.as_deref(),
        ));

        tracing::info!(
            command = %command,
            mode = %request.mode,
            out_dir = %request.out_dir.display(),
                "running OpenCode command"
        );
        let verification = run_with_local_verification(
            &command,
            &rendered_args,
            request,
            &self.config.opencode,
            &selection,
        );

        match verification {
            Ok(mut verification) => {
                let success = verification
                    .exit_status
                    .map(|code| code == 0)
                    .unwrap_or(false);
                let mut actual_session_id = request.resume_session_id.clone();
                if actual_session_id.is_none() {
                    match resolve_opencode_session_id(&command, &request.root, &request.session_id)
                    {
                        Ok(Some(session_id)) => actual_session_id = Some(session_id),
                        Ok(None) => verification.verification_notes.push(format!(
                            "OpenCode session id was not found for title `{}` and directory `{}`.",
                            request.session_id,
                            request.root.display()
                        )),
                        Err(error) => verification
                            .verification_notes
                            .push(format!("OpenCode session id resolution failed: {error}")),
                    }
                }
                Ok(AgentOutput {
                    backend: AgentBackend::OpenCode,
                    command_for_metrics,
                    segments: verification.opencode_segments,
                    exit_status: verification.exit_status,
                    success,
                    stdout: verification.stdout,
                    stderr: verification.stderr,
                    provider: Some(selection.provider),
                    model: Some(selection.model),
                    model_arg: Some(selection.model_arg),
                    session_label: Some(request.session_id.clone()),
                    session_id: actual_session_id,
                    resume_session_id: request.resume_session_id.clone(),
                    session_reused: request.resume_session_id.is_some(),
                    interrupted_by_supervisor: verification.interrupted_by_supervisor,
                    supervisor_control_action: verification.supervisor_control_action,
                    supervisor_control_events: verification.supervisor_control_events,
                    timed_out: verification.timed_out,
                    idle_timed_out: verification.idle_timed_out,
                    heartbeat_count: verification.heartbeat_count,
                    require_local: selection.require_local,
                    local_inference_verified: verification.local_inference_verified,
                    gpu_activity_observed: verification.gpu_activity_observed,
                    backend_activity_observed: verification.backend_activity_observed,
                    verification_notes: verification.verification_notes,
                })
            }
            Err(error) => Ok(AgentOutput {
                backend: AgentBackend::OpenCode,
                command_for_metrics,
                segments: Vec::new(),
                exit_status: None,
                success: false,
                stdout: Vec::new(),
                stderr: format!("failed to run OpenCode command `{command}`: {error}\n")
                    .into_bytes(),
                provider: Some(selection.provider),
                model: Some(selection.model),
                model_arg: Some(selection.model_arg),
                session_label: Some(request.session_id.clone()),
                session_id: None,
                resume_session_id: request.resume_session_id.clone(),
                session_reused: request.resume_session_id.is_some(),
                interrupted_by_supervisor: false,
                supervisor_control_action: None,
                supervisor_control_events: Vec::new(),
                timed_out: false,
                idle_timed_out: false,
                heartbeat_count: 0,
                require_local: selection.require_local,
                local_inference_verified: false,
                gpu_activity_observed: false,
                backend_activity_observed: false,
                verification_notes: vec![format!(
                    "OpenCode execution failed before verification: {error}"
                )],
            }),
        }
    }
}
