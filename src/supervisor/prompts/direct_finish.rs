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
You may now edit source and test files in the working repo directly. Do not ask the user for approval. Do not commit.
Do not inspect /solution, verifier internals, or unlisted Mixmod state directories. You may inspect the listed artifacts and the repo source.

{direct_finish_policy}

Before approving, run the smallest relevant checks you can. If checks are too expensive or unavailable, record that explicitly.
Return minified JSON only:
{{"action":"approve|stop","summary":"max 60 words","changed_files":[],"checks":["commands run and result"],"risk":"max 30 words"}}
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
