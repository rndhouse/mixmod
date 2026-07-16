use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::*;

pub(super) const NO_EDIT_PACKET: &str =
    "- none provided; use focused anchor searches in the relevant files.";

pub(super) fn patch_request_edit_packet_from_value(
    work_dir: &Path,
    focus_files: &[String],
    exact_edits: &[String],
    value: &Value,
    packet_keys: &[&str],
) -> String {
    let mut parts = Vec::new();
    append_edit_packet_items(
        &mut parts,
        "Supervisor packet",
        &merged_string_arrays(value, packet_keys),
    );
    parts.extend(source_snippets_for_edit_packet(
        work_dir,
        focus_files,
        exact_edits,
    ));
    finalize_edit_packet(parts)
}

pub(super) fn patch_request_edit_packet_from_decision(
    work_dir: &Path,
    focus_files: &[String],
    exact_edits: &[String],
    decision: &SupervisorFeedbackTurn,
) -> String {
    let mut parts = Vec::new();
    let packet_keys = ["edit_packet", "source_snippets", "anchors", "evidence"];
    append_edit_packet_items(
        &mut parts,
        "Supervisor packet",
        &merged_string_arrays(&decision.feedback, &packet_keys),
    );
    if let Some(nested) = decision.feedback.get("feedback") {
        append_edit_packet_items(
            &mut parts,
            "Supervisor packet",
            &merged_string_arrays(nested, &packet_keys),
        );
        if let Some(control) = nested.get("control") {
            append_edit_packet_items(
                &mut parts,
                "Supervisor packet",
                &merged_string_arrays(control, &packet_keys),
            );
        }
    }
    parts.extend(source_snippets_for_edit_packet(
        work_dir,
        focus_files,
        exact_edits,
    ));
    finalize_edit_packet(parts)
}

fn append_edit_packet_items(parts: &mut Vec<String>, label: &str, items: &[String]) {
    let items = items
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>();
    if !items.is_empty() {
        parts.push(format!("{label}:\n{}", items.join("\n")));
    }
}

fn source_snippets_for_edit_packet(
    work_dir: &Path,
    focus_files: &[String],
    exact_edits: &[String],
) -> Vec<String> {
    let anchors = edit_packet_anchors(exact_edits);
    let mut snippets = Vec::new();
    let mut seen_ranges = Vec::<String>::new();
    for file in focus_files.iter().take(4) {
        let Some(path) = repo_source_file_path(work_dir, file) else {
            continue;
        };
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if metadata.len() > 512_000 {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if anchors.is_empty() {
            if text.len() <= 2600 {
                snippets.push(format!(
                    "Source snippet from {file}:\n```text\n{}\n```",
                    line_numbered_text(&text, 1, text.lines().count())
                ));
            }
            continue;
        }
        let lines = text.lines().collect::<Vec<_>>();
        for anchor in &anchors {
            let Some(line_index) = lines.iter().position(|line| line.contains(anchor)) else {
                continue;
            };
            let start = line_index.saturating_sub(8);
            let end = lines.len().min(line_index + 9);
            let range_key = format!("{file}:{start}:{end}");
            if seen_ranges.contains(&range_key) {
                continue;
            }
            seen_ranges.push(range_key);
            snippets.push(format!(
                "Source snippet from {file} around `{anchor}`:\n```text\n{}\n```",
                lines[start..end]
                    .iter()
                    .enumerate()
                    .map(|(offset, line)| format!("{:>4}: {}", start + offset + 1, line))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
            if snippets.len() >= 4 {
                return snippets;
            }
        }
    }
    snippets
}

fn edit_packet_anchors(exact_edits: &[String]) -> Vec<String> {
    let mut anchors = Vec::new();
    for edit in exact_edits {
        for delimiter in ['"', '`'] {
            for fragment in delimited_fragments(edit, delimiter) {
                if fragment.len() >= 3
                    && fragment.len() <= 160
                    && !fragment.contains('\n')
                    && !anchors.contains(&fragment)
                {
                    anchors.push(fragment);
                }
            }
        }
    }
    anchors
}

fn delimited_fragments(text: &str, delimiter: char) -> Vec<String> {
    let mut fragments = Vec::new();
    let mut start = None;
    for (index, character) in text.char_indices() {
        if character != delimiter {
            continue;
        }
        if let Some(start_index) = start.take() {
            let fragment = text[start_index..index].trim();
            if !fragment.is_empty() {
                fragments.push(fragment.to_string());
            }
        } else {
            start = Some(index + character.len_utf8());
        }
    }
    fragments
}

fn repo_source_file_path(work_dir: &Path, raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim().trim_start_matches("./");
    if trimmed.is_empty() {
        return None;
    }
    let raw_path = Path::new(trimmed);
    let relative = if raw_path.is_absolute() {
        raw_path.strip_prefix(work_dir).ok()?.to_path_buf()
    } else {
        let mut relative = PathBuf::new();
        for component in raw_path.components() {
            match component {
                Component::Normal(part) => relative.push(part),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
            }
        }
        relative
    };
    let path = work_dir.join(relative);
    path.is_file().then_some(path)
}

fn line_numbered_text(text: &str, start_line: usize, line_count: usize) -> String {
    text.lines()
        .take(line_count)
        .enumerate()
        .map(|(index, line)| format!("{:>4}: {}", start_line + index, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn finalize_edit_packet(parts: Vec<String>) -> String {
    if parts.is_empty() {
        NO_EDIT_PACKET.to_string()
    } else {
        truncate_for_report(&parts.join("\n\n"), 4200)
    }
}
