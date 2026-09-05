#[cfg(target_arch = "wasm32")]
mod ui;

#[cfg(any(target_arch = "wasm32", test))]
use agent_remote_protocol::{
    ClientCommand, Conversation, ConversationId, ConversationState, HostId, ProjectId, ProviderId,
    TimelineItemKind,
};
#[cfg(any(target_arch = "wasm32", test))]
use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd};

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(target_arch = "wasm32", derive(serde::Serialize, serde::Deserialize))]
pub(crate) struct DraftScope {
    pub(crate) host_id: HostId,
    pub(crate) provider: ProviderId,
    pub(crate) project_id: ProjectId,
    /// `None` is the draft for the project's new-conversation composer.
    pub(crate) conversation_id: Option<ConversationId>,
}

#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn draft_scope(
    host_id: HostId,
    provider: ProviderId,
    project_id: Option<ProjectId>,
    conversation_id: Option<ConversationId>,
) -> Option<DraftScope> {
    Some(DraftScope {
        host_id,
        provider,
        project_id: project_id?,
        conversation_id,
    })
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(target_arch = "wasm32", derive(serde::Serialize, serde::Deserialize))]
pub(crate) enum ConversationSortMode {
    // Android's Agent mode reduces to Recent here because the Web tree is already
    // scoped to exactly one selected Provider.
    #[default]
    Recent,
    Active,
}

#[cfg(any(target_arch = "wasm32", test))]
fn conversation_belongs_to_project(
    conversation: &Conversation,
    project_id: ProjectId,
    provider: ProviderId,
) -> bool {
    conversation.project_id == project_id && conversation.provider == provider
}

#[cfg(test)]
fn sort_conversations_newest_first(conversations: &mut [&Conversation]) {
    sort_conversations(conversations, ConversationSortMode::Recent);
}

#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn sort_conversations(conversations: &mut [&Conversation], mode: ConversationSortMode) {
    conversations.sort_by(|left, right| {
        let active_order = match mode {
            ConversationSortMode::Recent => std::cmp::Ordering::Equal,
            ConversationSortMode::Active => {
                conversation_is_active(right).cmp(&conversation_is_active(left))
            }
        };
        active_order
            .then_with(|| right.updated_at_ms.cmp(&left.updated_at_ms))
            .then_with(|| left.id.cmp(&right.id))
    });
}

#[cfg(any(target_arch = "wasm32", test))]
fn conversation_is_active(conversation: &Conversation) -> bool {
    matches!(
        conversation.state,
        ConversationState::Running | ConversationState::NeedsApproval
    )
}

#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn is_collapsible_activity(kind: &TimelineItemKind) -> bool {
    matches!(
        kind,
        TimelineItemKind::Progress { .. }
            | TimelineItemKind::ToolCall { .. }
            | TimelineItemKind::Command { .. }
            | TimelineItemKind::FileChange { .. }
            | TimelineItemKind::Approval {
                resolved_option: Some(_),
                ..
            }
    )
}

#[cfg(any(target_arch = "wasm32", test))]
fn increment_send_attempt(command: &mut ClientCommand) -> Option<u32> {
    let attempt = match command {
        ClientCommand::StartConversation { attempt, .. }
        | ClientCommand::SendMessage { attempt, .. } => attempt,
        _ => return None,
    };
    *attempt += 1;
    Some(*attempt)
}

#[cfg(any(target_arch = "wasm32", test))]
fn retryable_send_rejection(command: &ClientCommand, code: &str) -> bool {
    matches!(
        command,
        ClientCommand::StartConversation { .. } | ClientCommand::SendMessage { .. }
    ) && code == "command_failed"
}

#[cfg(any(target_arch = "wasm32", test))]
fn markdown_to_safe_html(markdown: &str) -> String {
    let options = Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let events = Parser::new_ext(markdown, options).map(|event| match event {
        Event::Html(value) | Event::InlineHtml(value) => Event::Text(value),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: safe_markdown_url(dest_url),
            title,
            id,
        }),
        Event::Start(Tag::Image { .. }) => Event::Text(CowStr::Borrowed("[图片：")),
        Event::End(TagEnd::Image) => Event::Text(CowStr::Borrowed("]")),
        event => event,
    });
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, events);
    html
}

#[cfg(any(target_arch = "wasm32", test))]
fn safe_markdown_url(url: CowStr<'_>) -> CowStr<'_> {
    let value = url.trim();
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || lower.starts_with('#')
        || (!lower.contains(':') && !lower.starts_with("//"))
    {
        url
    } else {
        CowStr::Borrowed("")
    }
}

pub const fn app_name() -> &'static str {
    "Agent Remote Messenger"
}

#[cfg(target_arch = "wasm32")]
pub fn run() {
    yew::Renderer::<ui::App>::new().render();
}

#[cfg(test)]
mod tests {
    use agent_remote_protocol::{
        ClientCommand, CommandId, Conversation, ConversationId, ConversationState,
        ConversationTitleSource, ProjectId, ProviderId,
    };

    use super::{
        ConversationSortMode, conversation_belongs_to_project, draft_scope, increment_send_attempt,
        markdown_to_safe_html, retryable_send_rejection, sort_conversations,
        sort_conversations_newest_first,
    };

    #[test]
    fn unresolved_approvals_and_errors_remain_visible() {
        use agent_remote_protocol::{ApprovalId, TimelineItemKind};
        let mut approval = TimelineItemKind::Approval {
            approval_id: ApprovalId::new(),
            prompt: "Allow edit?".to_owned(),
            options: Vec::new(),
            resolved_option: None,
        };
        assert!(!super::is_collapsible_activity(&approval));
        if let TimelineItemKind::Approval {
            resolved_option, ..
        } = &mut approval
        {
            *resolved_option = Some("allow".to_owned());
        }
        assert!(super::is_collapsible_activity(&approval));
        assert!(!super::is_collapsible_activity(&TimelineItemKind::Error {
            code: "failed".to_owned(),
            message: "Could not continue".to_owned(),
        }));
    }

    fn conversation(
        project_id: ProjectId,
        provider: ProviderId,
        updated_at_ms: i64,
    ) -> Conversation {
        conversation_in_state(project_id, provider, updated_at_ms, ConversationState::Idle)
    }

    fn conversation_in_state(
        project_id: ProjectId,
        provider: ProviderId,
        updated_at_ms: i64,
        state: ConversationState,
    ) -> Conversation {
        Conversation {
            id: ConversationId::new(),
            revision: 1,
            provider,
            project_id,
            native_session_id: "session".to_owned(),
            title: "Conversation".to_owned(),
            title_source: ConversationTitleSource::Provider,
            title_updated_at_ms: updated_at_ms,
            selected_model: None,
            selected_effort: None,
            state,
            session_options: Vec::new(),
            updated_at_ms,
        }
    }

    #[test]
    fn draft_scope_isolated_by_host_provider_project_and_conversation() {
        let host = agent_remote_protocol::HostId::new();
        let other_host = agent_remote_protocol::HostId::new();
        let project = ProjectId::new();
        let other_project = ProjectId::new();
        let conversation = ConversationId::new();

        let new_conversation = draft_scope(host, ProviderId::Codex, Some(project), None).unwrap();
        assert_eq!(
            new_conversation,
            draft_scope(host, ProviderId::Codex, Some(project), None).unwrap()
        );
        assert_ne!(
            new_conversation,
            draft_scope(host, ProviderId::Codex, Some(project), Some(conversation)).unwrap()
        );
        assert_ne!(
            new_conversation,
            draft_scope(host, ProviderId::Grok, Some(project), None).unwrap()
        );
        assert_ne!(
            new_conversation,
            draft_scope(host, ProviderId::Codex, Some(other_project), None).unwrap()
        );
        assert_ne!(
            new_conversation,
            draft_scope(other_host, ProviderId::Codex, Some(project), None).unwrap()
        );
        assert!(draft_scope(host, ProviderId::Codex, None, None).is_none());
    }

    #[test]
    fn tree_membership_requires_both_project_and_provider() {
        let project = ProjectId::new();
        let other_project = ProjectId::new();
        let item = conversation(project, ProviderId::Codex, 10);

        assert!(conversation_belongs_to_project(
            &item,
            project,
            ProviderId::Codex
        ));
        assert!(!conversation_belongs_to_project(
            &item,
            other_project,
            ProviderId::Codex
        ));
        assert!(!conversation_belongs_to_project(
            &item,
            project,
            ProviderId::Grok
        ));
    }

    #[test]
    fn project_children_are_sorted_by_latest_activity() {
        let project = ProjectId::new();
        let oldest = conversation(project, ProviderId::Codex, 10);
        let newest = conversation(project, ProviderId::Codex, 30);
        let middle = conversation(project, ProviderId::Codex, 20);
        let mut items = vec![&oldest, &newest, &middle];

        sort_conversations_newest_first(&mut items);

        assert_eq!(
            items
                .into_iter()
                .map(|conversation| conversation.updated_at_ms)
                .collect::<Vec<_>>(),
            vec![30, 20, 10]
        );
    }

    #[test]
    fn active_sort_keeps_running_and_approval_conversations_first() {
        let project = ProjectId::new();
        let newest_idle =
            conversation_in_state(project, ProviderId::Codex, 50, ConversationState::Idle);
        let running =
            conversation_in_state(project, ProviderId::Codex, 20, ConversationState::Running);
        let approval = conversation_in_state(
            project,
            ProviderId::Codex,
            30,
            ConversationState::NeedsApproval,
        );
        let older_idle =
            conversation_in_state(project, ProviderId::Codex, 10, ConversationState::Completed);
        let mut items = vec![&newest_idle, &running, &approval, &older_idle];

        sort_conversations(&mut items, ConversationSortMode::Active);

        assert_eq!(
            items
                .into_iter()
                .map(|conversation| (conversation.state, conversation.updated_at_ms))
                .collect::<Vec<_>>(),
            vec![
                (ConversationState::NeedsApproval, 30),
                (ConversationState::Running, 20),
                (ConversationState::Idle, 50),
                (ConversationState::Completed, 10),
            ]
        );
    }

    #[test]
    fn explicit_retry_increments_attempt_without_changing_message_identity() {
        let command_id = CommandId::new();
        let conversation_id = ConversationId::new();
        let client_message_id = "stable-client-message".to_owned();
        let mut command = ClientCommand::SendMessage {
            command_id,
            attempt: 2,
            conversation_id,
            client_message_id: Some(client_message_id.clone()),
            text: "kept draft".to_owned(),
            attachments: Vec::new(),
        };

        assert_eq!(increment_send_attempt(&mut command), Some(3));
        assert_eq!(command.command_id(), Some(command_id));
        assert_eq!(command.attempt(), 3);
        let ClientCommand::SendMessage {
            conversation_id: actual_conversation_id,
            client_message_id: actual_client_message_id,
            text,
            ..
        } = command
        else {
            panic!("expected send message command");
        };
        assert_eq!(actual_conversation_id, conversation_id);
        assert_eq!(
            actual_client_message_id.as_deref(),
            Some(client_message_id.as_str())
        );
        assert_eq!(text, "kept draft");
    }

    #[test]
    fn only_provider_failures_offer_an_explicit_send_retry() {
        let send = ClientCommand::SendMessage {
            command_id: CommandId::new(),
            attempt: 0,
            conversation_id: ConversationId::new(),
            client_message_id: Some("stable-message".to_owned()),
            text: "hello".to_owned(),
            attachments: Vec::new(),
        };
        assert!(retryable_send_rejection(&send, "command_failed"));
        assert!(!retryable_send_rejection(&send, "command_outcome_unknown"));
        let steer = ClientCommand::Steer {
            command_id: CommandId::new(),
            conversation_id: ConversationId::new(),
            text: "continue".to_owned(),
        };
        assert!(!retryable_send_rejection(&steer, "command_failed"));
    }

    #[test]
    fn markdown_is_formatted_without_trusting_embedded_html_or_active_urls() {
        let rendered = markdown_to_safe_html(
            "## 标题\n\n- **项目**\n\n```rust\nlet value = 1;\n```\n\n<script>alert(1)</script>\n\n[危险](javascript:alert(1))\n\n![跟踪图](https://tracker.invalid/pixel.png)",
        );

        assert!(rendered.contains("<h2>标题</h2>"));
        assert!(rendered.contains("<strong>项目</strong>"));
        assert!(rendered.contains("<code class=\"language-rust\">"));
        assert!(!rendered.contains("<script>"));
        assert!(!rendered.contains("javascript:"));
        assert!(!rendered.contains("<img"));
        assert!(!rendered.contains("tracker.invalid"));
        assert!(rendered.contains("[图片：跟踪图]"));
        assert!(rendered.contains("&lt;script&gt;"));
    }
}
