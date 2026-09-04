#[cfg(target_arch = "wasm32")]
mod ui;

#[cfg(any(target_arch = "wasm32", test))]
use agent_remote_protocol::{ClientCommand, Conversation, ProjectId, ProviderId};
#[cfg(any(target_arch = "wasm32", test))]
use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd};

#[cfg(any(target_arch = "wasm32", test))]
fn conversation_belongs_to_project(
    conversation: &Conversation,
    project_id: ProjectId,
    provider: ProviderId,
) -> bool {
    conversation.project_id == project_id && conversation.provider == provider
}

#[cfg(any(target_arch = "wasm32", test))]
fn sort_conversations_newest_first(conversations: &mut [&Conversation]) {
    conversations.sort_by_key(|conversation| std::cmp::Reverse(conversation.updated_at_ms));
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
        conversation_belongs_to_project, increment_send_attempt, markdown_to_safe_html,
        retryable_send_rejection, sort_conversations_newest_first,
    };

    fn conversation(
        project_id: ProjectId,
        provider: ProviderId,
        updated_at_ms: i64,
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
            state: ConversationState::Idle,
            session_options: Vec::new(),
            updated_at_ms,
        }
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
