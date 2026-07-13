use crate::*;

pub(crate) fn budgeted_report(name: &str, metrics: &Value) -> String {
    format!(
        r#"# Mixmod Default Strategy Report: {name}

## Summary

- Supervisor output tokens: {output_tokens}
- Supervisor input tokens: {input_tokens}
- Total supervisor tokens: {total_tokens}
- Supervisor token usage scope: {token_scope}
- Supervisor token usage comparable: {token_comparable}
- Supervisor model: {supervisor_model}
- Supervisor reasoning effort: {reasoning_effort}
- Supervisor turns: {turns}
- Supervisor control events: {supervisor_control_count}
- Supervisor control interrupts: {supervisor_control_interrupts}
- Supervisor control actions: {supervisor_control_actions}
- Worker brief output tokens: {brief_tokens}
- Worker backend: {worker_backend}
- Worker provider/model: {provider}/{model}
- Local inference verified: {local_verified}
- GPU activity observed: {gpu}
- Final supervisor action: {verdict}

## Conclusion

Default strategy result is `{status}` for this arm. This arm uses the supervisor model to produce a compact executable worker handoff, the configured worker to implement from the original task plus that handoff, and the supervisor model to review compact artifacts.
"#,
        output_tokens = get_u64(metrics, "supervisor_output_tokens").unwrap_or(0),
        input_tokens = get_u64(metrics, "supervisor_input_tokens").unwrap_or(0),
        total_tokens = get_u64(metrics, "supervisor_total_tokens").unwrap_or(0),
        token_scope = get_str(metrics, "supervisor_token_usage_scope").unwrap_or("unknown"),
        token_comparable = get_bool(metrics, "supervisor_token_usage_comparable")
            .map(yes_no)
            .unwrap_or("unknown"),
        supervisor_model = get_str(metrics, "supervisor_model").unwrap_or("unknown"),
        reasoning_effort = get_str(metrics, "supervisor_reasoning_effort").unwrap_or("unknown"),
        turns = get_u64(metrics, "supervision_turn_count").unwrap_or(0),
        supervisor_control_count = get_u64(metrics, "supervisor_control_count").unwrap_or(0),
        supervisor_control_interrupts =
            get_u64(metrics, "supervisor_control_interrupts").unwrap_or(0),
        supervisor_control_actions = display_string_array(metrics, "supervisor_control_actions"),
        brief_tokens = get_u64(metrics, "worker_brief_output_tokens").unwrap_or(0),
        worker_backend = get_str(metrics, "worker_backend").unwrap_or("unknown"),
        provider = get_str(metrics, "opencode_provider").unwrap_or("unknown"),
        model = get_str(metrics, "opencode_model").unwrap_or("unknown"),
        local_verified = get_bool(metrics, "local_inference_verified")
            .map(yes_no)
            .unwrap_or("unknown"),
        gpu = get_bool(metrics, "gpu_activity_observed")
            .map(yes_no)
            .unwrap_or("unknown"),
        verdict = get_str(metrics, "final_codex_action")
            .or_else(|| get_str(metrics, "final_verdict"))
            .unwrap_or("unknown"),
        status = get_str(metrics, "final_status").unwrap_or("unknown"),
    )
}

pub fn experiment_report(root: &Path, name: &str) -> Result<String> {
    ExperimentReportRenderer::load(root, name)?.render()
}

struct ExperimentReportRenderer<'a> {
    root: &'a Path,
    name: &'a str,
    exp_dir: PathBuf,
    inputs: ExperimentReportInputs,
}

impl<'a> ExperimentReportRenderer<'a> {
    fn load(root: &'a Path, name: &'a str) -> Result<Self> {
        Ok(Self {
            root,
            name,
            exp_dir: state_layout(root).experiments().join(name),
            inputs: read_experiment_report_inputs(root, name)?,
        })
    }

    fn render(self) -> Result<String> {
        let Self {
            root,
            name,
            exp_dir,
            inputs,
        } = self;
        let ExperimentReportInputs {
            codex_metrics,
            default_metrics,
            default_source,
            default_metrics_path: _,
        } = inputs;

        let codex_status = get_str(&codex_metrics, "final_status").unwrap_or("unknown");
        let default_status = get_str(&default_metrics, "final_status").unwrap_or("unknown");
        let codex_patch_bytes = get_u64(&codex_metrics, "patch_bytes").unwrap_or(0);
        let default_patch_bytes = get_u64(&default_metrics, "patch_bytes").unwrap_or(0);
        let codex_changed_files = get_u64(&codex_metrics, "changed_file_count").unwrap_or(0);
        let default_changed_files = get_u64(&default_metrics, "changed_file_count").unwrap_or(0);
        let codex_changed_lines = get_u64(&codex_metrics, "changed_line_count").unwrap_or(0);
        let default_changed_lines = get_u64(&default_metrics, "changed_line_count").unwrap_or(0);
        let codex_visible = get_u64(&codex_metrics, "codex_visible_bytes")
            .or_else(|| get_u64(&codex_metrics, "approximate_codex_input_bytes"))
            .or_else(|| get_u64(&codex_metrics, "supervisor_input_bytes_fallback"));
        let default_visible = get_u64(&default_metrics, "codex_visible_bytes")
            .or_else(|| get_u64(&default_metrics, "supervisor_input_bytes_fallback"))
            .or_else(|| get_u64(&default_metrics, "approximate_codex_input_bytes"));
        let local_worker_text = get_u64(&default_metrics, "local_worker_text_bytes").unwrap_or(0);
        let mixmod_delegations = get_u64(&default_metrics, "mixmod_delegations").unwrap_or(0);
        let codex_output_tokens = supervisor_output_tokens(&codex_metrics);
        let default_output_tokens = supervisor_output_tokens(&default_metrics);
        let codex_input_tokens = supervisor_input_tokens(&codex_metrics);
        let default_input_tokens = supervisor_input_tokens(&default_metrics);
        let codex_total_tokens = supervisor_total_tokens(&codex_metrics);
        let default_total_tokens = supervisor_total_tokens(&default_metrics);
        let codex_token_usage_comparable = supervisor_token_usage_is_comparable(&codex_metrics);
        let default_token_usage_comparable = supervisor_token_usage_is_comparable(&default_metrics);
        let codex_supervisor_model = get_str(&codex_metrics, "supervisor_model")
            .unwrap_or("unknown")
            .to_string();
        let default_supervisor_model = get_str(&default_metrics, "supervisor_model")
            .unwrap_or("unknown")
            .to_string();
        let codex_reasoning_effort = get_str(&codex_metrics, "supervisor_reasoning_effort")
            .unwrap_or("unknown")
            .to_string();
        let default_reasoning_effort = get_str(&default_metrics, "supervisor_reasoning_effort")
            .unwrap_or("unknown")
            .to_string();
        let default_codex_calls = get_u64(&default_metrics, "codex_calls")
            .or_else(|| get_u64(&default_metrics, "supervision_turn_count"))
            .unwrap_or(0);
        let default_opencode_calls = get_u64(&default_metrics, "opencode_calls").unwrap_or(0);
        let default_worker_backend =
            get_str(&default_metrics, "worker_backend").unwrap_or("unknown");
        let default_local_verified = get_bool(&default_metrics, "local_inference_verified")
            .map(yes_no)
            .unwrap_or("not-run");
        let default_gpu = get_bool(&default_metrics, "gpu_activity_observed")
            .map(yes_no)
            .unwrap_or("not-run");
        let default_timed_out = get_bool(&default_metrics, "opencode_timed_out")
            .map(yes_no)
            .unwrap_or("unknown");
        let default_idle_timed_out = get_bool(&default_metrics, "opencode_idle_timed_out")
            .map(yes_no)
            .unwrap_or("unknown");
        let default_heartbeats = get_u64(&default_metrics, "heartbeat_count").unwrap_or(0);
        let default_supervisor_control_count =
            get_u64(&default_metrics, "supervisor_control_count").unwrap_or(0);
        let default_supervisor_control_interrupts =
            get_u64(&default_metrics, "supervisor_control_interrupts").unwrap_or(0);
        let default_supervisor_control_actions =
            display_string_array(&default_metrics, "supervisor_control_actions");
        let worker_brief_output_tokens = get_u64(&default_metrics, "worker_brief_output_tokens")
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unavailable".to_string());
        let default_provider_model = mixmod_provider_model(&default_metrics);
        let default_qwen = yes_no(
            default_provider_model.contains("qwen3.6")
                || default_provider_model.contains("qwen-3.6"),
        );
        let run_metrics = default_metrics.get("run_metrics").unwrap_or(&Value::Null);
        let opencode_exit_status = get_u64(&default_metrics, "opencode_exit_status")
            .or_else(|| get_u64(run_metrics, "opencode_exit_status"))
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unavailable".to_string());
        let opencode_stdout_bytes = get_u64(&default_metrics, "local_worker_stdout_bytes")
            .or_else(|| get_u64(run_metrics, "stdout_bytes"))
            .unwrap_or(0);
        let opencode_stderr_bytes = get_u64(&default_metrics, "local_worker_stderr_bytes")
            .or_else(|| get_u64(run_metrics, "stderr_bytes"))
            .unwrap_or(0);
        let artifact_sizes = default_metrics
            .get("artifact_byte_sizes")
            .unwrap_or(&Value::Null);
        let mixmod_report_bytes = get_u64(artifact_sizes, REPORT_MD)
            .or_else(|| get_u64(run_metrics, "report_bytes"))
            .unwrap_or(0);
        let mixmod_session_bytes = get_u64(artifact_sizes, SESSION_JSONL)
            .or_else(|| get_u64(run_metrics, "session_bytes"))
            .unwrap_or(0);
        let mixmod_compact_artifact_bytes = std::iter::once(WORKER_BRIEF_JSON)
            .chain(CODEX_REVIEW_ARTIFACTS.iter().copied())
            .filter_map(|name| get_u64(artifact_sizes, name))
            .sum::<u64>();
        let opencode_command = default_metrics
            .get("opencode_command")
            .or_else(|| run_metrics.get("opencode_command"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "unavailable".to_string());

        let token_conclusion = if !codex_token_usage_comparable || !default_token_usage_comparable {
            "Exact token telemetry is not comparable because at least one arm is not marked as cumulative run-level usage; token savings are inconclusive."
                .to_string()
        } else {
            match (
                get_u64(&codex_metrics, "codex_token_usage").or(codex_total_tokens),
                get_u64(&default_metrics, "codex_token_usage").or(default_total_tokens),
                codex_visible,
                default_visible,
            ) {
                (Some(codex), Some(mixmod), _, _) if mixmod < codex => {
                    format!("Mixmod used fewer measured Codex tokens ({mixmod} vs {codex}).")
                }
                (Some(codex), Some(mixmod), _, _) if mixmod >= codex => {
                    format!("Mixmod did not reduce measured Codex tokens ({mixmod} vs {codex}).")
                }
                (_, _, Some(codex), Some(mixmod)) if mixmod < codex => format!(
                    "Exact token telemetry is unavailable; byte proxy favors Mixmod ({mixmod} vs {codex} Codex-visible input bytes)."
                ),
                (_, _, Some(codex), Some(mixmod)) => format!(
                    "Exact token telemetry is unavailable; byte proxy does not favor Mixmod yet ({mixmod} vs {codex} Codex-visible input bytes)."
                ),
                _ => "Exact token telemetry and comparable byte proxies are unavailable; conclusion is inconclusive."
                    .to_string(),
            }
        };

        let full_session = get_bool(&default_metrics, "did_codex_read_full_mixmod_session")
            .map(yes_no)
            .unwrap_or("unknown");
        let artifacts = get_string_array(&default_metrics, "artifact_files_read_by_codex");

        let report = format!(
            r#"# Mixmod Experiment Report: {name}

Generated: {generated}

## Supervisor Output Tokens

| Arm | Output tokens | Delta vs Codex-only |
| --- | ---: | ---: |
| Codex-only | {codex_output_tokens} | 0 |
| Mixmod default strategy | {default_output_tokens} | {default_output_delta} |

Mixmod default strategy beat Codex-only on output tokens: {default_output_win}.

## Summary

- Codex-only final status: {codex_status}
- Mixmod default final status: {default_status}
- Mixmod default metrics source: {default_source}
- Codex-only token telemetry comparable: {codex_token_usage_comparable}
- Mixmod token telemetry comparable: {default_token_usage_comparable}
- Mixmod delegations: {mixmod_delegations}
- Did Codex read the full Mixmod session: {full_session}
- Context-exposure conclusion: {token_conclusion}

## Comparison

| Metric | Codex-only | Mixmod default |
| --- | ---: | ---: |
| Supervisor input tokens | {codex_input_tokens} | {default_input_tokens} |
| Supervisor output tokens | {codex_output_tokens} | {default_output_tokens} |
| Total supervisor tokens | {codex_tokens} | {default_total_tokens} |
| Supervisor model | {codex_supervisor_model} | {default_supervisor_model} |
| Supervisor reasoning effort | {codex_reasoning_effort} | {default_reasoning_effort} |
| Codex-visible bytes | {codex_visible} | {default_visible} |
| Codex calls | 1 | {default_codex_calls} |
| Mixmod worker calls | 0 | {default_opencode_calls} |
| Worker backend | n/a | {default_worker_backend} |
| Worker provider/model | n/a | {default_provider_model} |
| Qwen 3.6 selected | n/a | {default_qwen} |
| Local-worker text bytes | 0 | {local_worker_text} |
| Local inference verified | n/a | {default_local_verified} |
| GPU activity observed | n/a | {default_gpu} |
| Worker timed out | n/a | {default_timed_out} |
| Worker idle timed out | n/a | {default_idle_timed_out} |
| Worker heartbeats | n/a | {default_heartbeats} |
| Supervisor control events | n/a | {default_supervisor_control_count} |
| Supervisor control interrupts | n/a | {default_supervisor_control_interrupts} |
| Supervisor control actions | n/a | {default_supervisor_control_actions} |
| Full session/logs read | n/a | {full_session} |
| Patch bytes | {codex_patch_bytes} | {default_patch_bytes} |
| Files changed | {codex_changed_files} | {default_changed_files} |
| Lines changed | {codex_changed_lines} | {default_changed_lines} |
| Worker brief output tokens | n/a | {worker_brief_output_tokens} |

## Questions

- Did both approaches produce a working patch? Codex-only: {codex_status}; Mixmod default: {default_status}.
- Which evaluator scored the result? Mixmod does not execute project tests directly; use benchmark or official evaluator artifacts for scoring.
- How much Codex-visible text was involved? Current byte proxies are Codex-only `{codex_visible}` and Mixmod default `{default_visible}`.
- How much local-worker text was generated? `{local_worker_text}` bytes were captured for the Mixmod run.
- Did Codex need to read the full Mixmod session? {full_session}.
- Did Mixmod appear to reduce supervisor context exposure? {token_conclusion}

## Mixmod Details

- Worker command: `{opencode_command}`
- Worker exit status: {opencode_exit_status}
- Compact Mixmod artifact bytes: {mixmod_compact_artifact_bytes}
- Worker stdout bytes: {opencode_stdout_bytes}
- Worker stderr bytes: {opencode_stderr_bytes}
- Mixmod report bytes: {mixmod_report_bytes}
- Mixmod session bytes: {mixmod_session_bytes}

## Mixmod Artifacts Read By Codex

{artifact_list}

## Notes

Exact Codex token telemetry is often unavailable through local CLI workflows. This report uses explicit token metrics when present and byte/character proxies otherwise.
"#,
            generated = Utc::now().to_rfc3339(),
            codex_tokens = display_optional_u64(codex_total_tokens),
            default_total_tokens = display_optional_u64(default_total_tokens),
            codex_input_tokens = display_optional_u64(codex_input_tokens),
            default_input_tokens = display_optional_u64(default_input_tokens),
            codex_output_tokens = display_optional_u64(codex_output_tokens),
            default_output_tokens = display_optional_u64(default_output_tokens),
            codex_supervisor_model = codex_supervisor_model,
            default_supervisor_model = default_supervisor_model,
            codex_reasoning_effort = codex_reasoning_effort,
            default_reasoning_effort = default_reasoning_effort,
            default_output_delta = display_delta(default_output_tokens, codex_output_tokens),
            default_output_win = match (default_output_tokens, codex_output_tokens) {
                (Some(mixmod_default), Some(codex)) => yes_no(mixmod_default < codex).to_string(),
                _ => "unknown".to_string(),
            },
            codex_visible = display_optional_u64(codex_visible),
            default_visible = display_optional_u64(default_visible),
            default_status = default_status,
            default_source = default_source,
            codex_token_usage_comparable = yes_no(codex_token_usage_comparable),
            default_token_usage_comparable = yes_no(default_token_usage_comparable),
            default_codex_calls = default_codex_calls,
            default_opencode_calls = default_opencode_calls,
            default_worker_backend = default_worker_backend,
            default_provider_model = default_provider_model,
            default_qwen = default_qwen,
            default_local_verified = default_local_verified,
            default_gpu = default_gpu,
            default_timed_out = default_timed_out,
            default_idle_timed_out = default_idle_timed_out,
            default_heartbeats = default_heartbeats,
            default_supervisor_control_count = default_supervisor_control_count,
            default_supervisor_control_interrupts = default_supervisor_control_interrupts,
            default_supervisor_control_actions = default_supervisor_control_actions,
            default_patch_bytes = default_patch_bytes,
            default_changed_files = default_changed_files,
            default_changed_lines = default_changed_lines,
            worker_brief_output_tokens = worker_brief_output_tokens,
            opencode_command = opencode_command,
            opencode_exit_status = opencode_exit_status,
            mixmod_compact_artifact_bytes = mixmod_compact_artifact_bytes,
            opencode_stdout_bytes = opencode_stdout_bytes,
            opencode_stderr_bytes = opencode_stderr_bytes,
            mixmod_report_bytes = mixmod_report_bytes,
            mixmod_session_bytes = mixmod_session_bytes,
            artifact_list = if artifacts.is_empty() {
                "- unavailable".to_string()
            } else {
                artifacts
                    .iter()
                    .map(|artifact| format!("- `{artifact}`"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
        );

        atomic_write(&exp_dir.join(REPORT_MD), report.as_bytes())?;
        println!("{}", report.trim_end());
        println!("\nwrote {}", display_path(root, &exp_dir.join(REPORT_MD)));
        Ok(report)
    }
}

fn read_experiment_report_inputs(root: &Path, name: &str) -> Result<ExperimentReportInputs> {
    validate_experiment_name(name)?;
    let exp_dir = state_layout(root).experiments().join(name);
    let codex_metrics = read_json_file(&exp_dir.join("codex-only/metrics.json"))
        .unwrap_or_else(|_| placeholder_experiment_metrics("codex-only"));
    let (default_metrics, default_source, default_metrics_path) =
        if let Ok(metrics) = read_json_file(&exp_dir.join("default/metrics.json")) {
            (
                metrics,
                "default".to_string(),
                "default/metrics.json".to_string(),
            )
        } else if let Ok(metrics) = read_json_file(&exp_dir.join("budgeted/metrics.json")) {
            (
                metrics,
                "budgeted (legacy)".to_string(),
                "budgeted/metrics.json".to_string(),
            )
        } else {
            (
                placeholder_experiment_metrics("mixmod-default-strategy"),
                "missing".to_string(),
                "default/metrics.json".to_string(),
            )
        };

    Ok(ExperimentReportInputs {
        codex_metrics,
        default_metrics,
        default_source,
        default_metrics_path,
    })
}
