//! User-visible titles and provider output sanitization. No session state or I/O.

use std::path::{Component, Path};

use agent_remote_protocol::TimelineItemKind;
use anyhow::{Result, anyhow};

use crate::storage::Project;

pub(super) fn provisional_title(message: &str) -> String {
    let clean = message
        .lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with("```")
                && !line.starts_with("# Files mentioned")
                && !line.starts_with("C:\\")
                && !line.starts_with('/')
        })
        .unwrap_or("新对话")
        .trim_matches(|character: char| {
            character.is_ascii_punctuation() || character.is_whitespace()
        });
    let title = if clean
        .chars()
        .any(|character| ('\u{4e00}'..='\u{9fff}').contains(&character))
    {
        clean.chars().take(20).collect()
    } else {
        clean
            .split_whitespace()
            .take(8)
            .collect::<Vec<_>>()
            .join(" ")
    };
    if title.is_empty() {
        "新对话".to_owned()
    } else {
        title
    }
}

pub(super) fn provider_title(title: &str) -> String {
    let title = title.trim();
    if title.is_empty() {
        "新对话".to_owned()
    } else {
        title.chars().take(80).collect()
    }
}

pub(super) fn safe_relative_display(project: &Project, raw: &str) -> Option<String> {
    let path = Path::new(raw);
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return None;
    }
    if path.is_absolute() {
        return path
            .strip_prefix(&project.canonical_path)
            .ok()
            .map(|relative| relative.to_string_lossy().replace('\\', "/"));
    }
    Some(path.to_string_lossy().replace('\\', "/"))
}

pub(super) fn sanitize_history_kind(project: &Project, kind: TimelineItemKind) -> Result<TimelineItemKind> {
    Ok(match kind {
        TimelineItemKind::Plan { steps } => TimelineItemKind::Plan {
            steps: redact_plan_steps(steps, project),
        },
        TimelineItemKind::Progress {
            kind,
            label,
            status,
            detail,
        } => TimelineItemKind::Progress {
            kind,
            label: redact_project_path(&label, project),
            status,
            detail: detail.map(|value| redact_project_path(&value, project)),
        },
        TimelineItemKind::ToolCall {
            name,
            status,
            input_summary,
            output_summary,
        } => TimelineItemKind::ToolCall {
            name,
            status,
            input_summary: input_summary.map(|value| redact_project_path(&value, project)),
            output_summary: output_summary.map(|value| redact_project_path(&value, project)),
        },
        TimelineItemKind::Command {
            command,
            relative_cwd,
            status,
            exit_code,
            output,
        } => TimelineItemKind::Command {
            command: redact_project_path(&command, project),
            relative_cwd: relative_cwd.and_then(|path| safe_relative_display(project, &path)),
            status,
            exit_code,
            output: output.map(|value| truncate_output(redact_project_path(&value, project))),
        },
        TimelineItemKind::FileChange {
            relative_path,
            change_kind,
            status,
        } => TimelineItemKind::FileChange {
            relative_path: safe_relative_display(project, &relative_path)
                .ok_or_else(|| anyhow!("provider history file change was outside the project"))?,
            change_kind,
            status,
        },
        TimelineItemKind::Approval {
            approval_id,
            prompt,
            options,
            resolved_option,
        } => TimelineItemKind::Approval {
            approval_id,
            prompt: redact_project_path(&prompt, project),
            options,
            resolved_option,
        },
        TimelineItemKind::Error { code, message } => TimelineItemKind::Error {
            code,
            message: redact_remote_error(&message, Some(project)),
        },
        kind => kind,
    })
}

pub(super) fn redact_plan_steps(
    steps: Vec<agent_remote_protocol::PlanStep>,
    project: &Project,
) -> Vec<agent_remote_protocol::PlanStep> {
    steps
        .into_iter()
        .map(|mut step| {
            step.text = redact_project_path(&step.text, project);
            step
        })
        .collect()
}

pub(super) fn redact_project_path(value: &str, project: &Project) -> String {
    let canonical = project.canonical_path.to_string_lossy();
    value.replace(canonical.as_ref(), ".")
}

pub(super) fn redact_remote_error(value: &str, project: Option<&Project>) -> String {
    let value = project.map_or_else(
        || value.to_owned(),
        |project| redact_project_path(value, project),
    );
    value
        .split_inclusive(char::is_whitespace)
        .map(redact_absolute_path_fragment)
        .collect()
}

fn redact_absolute_path_fragment(fragment: &str) -> String {
    let content_len = fragment.trim_end_matches(char::is_whitespace).len();
    let (content, whitespace) = fragment.split_at(content_len);
    let Some(start) = absolute_path_start(content) else {
        return fragment.to_owned();
    };
    let path_end = content
        .trim_end_matches(|character| {
            matches!(character, '"' | '\'' | ')' | ']' | '}' | ',' | ';' | ':')
        })
        .len()
        .max(start);
    format!(
        "{}<host-path>{}{}",
        &content[..start],
        &content[path_end..],
        whitespace
    )
}

fn absolute_path_start(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    for index in 0..bytes.len() {
        let boundary =
            index == 0 || matches!(bytes[index - 1], b'=' | b'(' | b'[' | b'{' | b'"' | b'\'');
        if !boundary {
            continue;
        }
        if bytes[index] == b'/'
            || (bytes[index] == b'\\' && bytes.get(index + 1).is_some_and(|next| *next == b'\\'))
            || (bytes[index].is_ascii_alphabetic()
                && bytes.get(index + 1).is_some_and(|next| *next == b':')
                && bytes
                    .get(index + 2)
                    .is_some_and(|next| matches!(*next, b'/' | b'\\')))
        {
            return Some(index);
        }
    }
    None
}

pub(super) fn truncate_output(mut output: String) -> String {
    const LIMIT: usize = 64 * 1024;
    if output.len() > LIMIT {
        let mut end = LIMIT;
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        output.truncate(end);
        output.push_str("\n[output truncated at 64 KiB]");
    }
    output
}
