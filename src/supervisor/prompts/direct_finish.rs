use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::*;

use super::common::supervisor_artifact_index;

pub(crate) fn supervisor_direct_finish_prompt(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    takeover_decision: &SupervisorFeedbackTurn,
    context_telemetry: &SupervisorContextTelemetry,
    strategy: DefaultStrategyMode,
) -> Result<String> {
    let artifact_index = supervisor_artifact_index(work_dir, artifact_paths);
    let takeover_feedback = serde_json::to_string_pretty(&takeover_decision.feedback)
        .context("failed to serialize takeover feedback")?;
    let direct_plan = if takeover_decision.direct_plan.is_empty() {
        "- no direct_plan provided; infer the smallest finish plan from artifacts".to_string()
    } else {
        takeover_decision
            .direct_plan
            .iter()
            .map(|item| format!("- {}", item.trim()))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let takeover_reason = takeover_decision
        .takeover_reason
        .as_deref()
        .unwrap_or("supervisor takeover selected");
    let direct_finish_policy = supervisor_direct_finish_policy(strategy);
    let context_telemetry = serde_json::to_string_pretty(&context_telemetry.to_prompt_json())
        .context("failed to serialize supervisor context telemetry")?;
    Ok(format!(
        r#"You are the Mixmod supervisor in {strategy_mode} direct finish mode.
You may now make surgical source or test edits in the working repo directly. Do not ask the user for approval. Do not commit.
Do not inspect /solution, verifier internals, or unlisted Mixmod state directories.

Direct-finish contract:
- Direct supervisor edits are for known, bounded cleanup only. The worker owns expensive work.
- Edit only files named in direct_plan, takeover feedback, or the smallest nearby source file needed for the named defect.
- Do not use shell commands, run tests, regenerate artifacts, inspect generated or very large files, or perform broad search unless direct_plan explicitly names the exact command or file and why it is cheap.
- Use listed artifacts and already-known context first. Avoid reading more repo source once the targeted edit is clear.
- If finishing requires broad exploration, broad verification, generated-output synchronization, or discovering where the bug lives, return action=stop and explain that the work should go back to the worker.

{direct_finish_policy}

Before approving, verify only within this surgical contract. Prefer direct code inspection of the edited lines. Record any command only if direct_plan explicitly allowed it and it stayed cheap.
Return minified JSON only:
{{"action":"approve|stop","summary":"max 60 words","changed_files":[],"checks":["commands run and result; empty when none"],"risk":"max 30 words","surgical_contract":{{"why_direct":"max 40 words","target_files":[],"expected_patch_lines":"0|1-20|21-50|over-50","commands_used":false,"command_justification":"max 30 words or empty","broad_work_required":false}}}}
Use action=approve only when the current source state appears to satisfy the original task. Use action=stop if blocked or inconclusive after direct work.

Takeover reason: {takeover_reason}

Direct plan:
{direct_plan}

Supervisor context telemetry:
```json
{context_telemetry}
```

Takeover feedback:
```json
{takeover_feedback}
```

Working repo: {work_dir}

Artifact index:
{artifact_index}
"#,
        work_dir = work_dir.display(),
        strategy_mode = strategy.as_str(),
        direct_finish_policy = direct_finish_policy,
    ))
}
