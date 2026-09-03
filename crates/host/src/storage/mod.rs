use std::{
    fs,
    path::{Component, Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use agent_remote_protocol::{
    AttachmentId, AttachmentMetadata, CommandId, Conversation, ConversationId, ConversationState,
    ConversationTitleSource, DeviceId, HostId, ProjectId, ProjectSummary, ProviderId,
    ServerMessage, TimelineItem, TimelineItemId, TimelineItemKind, TimelinePageCursor,
};
use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::Rng;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: ProjectId,
    pub display_name: String,
    pub canonical_path: PathBuf,
    pub enabled_providers: Vec<ProviderId>,
}

impl Project {
    pub fn summary(&self) -> ProjectSummary {
        ProjectSummary {
            id: self.id,
            display_name: self.display_name.clone(),
            short_path: short_project_path(&self.canonical_path),
            enabled_providers: self.enabled_providers.clone(),
            valid: self.canonical_path.is_dir(),
            last_activity_at_ms: None,
            conversation_count: 0,
        }
    }

    pub fn resolve_existing(&self, requested: &Path) -> Result<PathBuf> {
        let candidate = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            self.canonical_path.join(requested)
        };
        let canonical = candidate
            .canonicalize()
            .with_context(|| format!("path does not exist: {}", candidate.display()))?;
        if !canonical.starts_with(&self.canonical_path) {
            bail!("path is outside the authorized project");
        }
        Ok(canonical)
    }

    pub fn resolve_for_write(&self, requested: &Path) -> Result<PathBuf> {
        if requested
            .components()
            .any(|component| component == Component::ParentDir)
        {
            bail!("parent traversal is not allowed");
        }
        let candidate = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            self.canonical_path.join(requested)
        };
        let parent = candidate
            .parent()
            .ok_or_else(|| anyhow!("write path has no parent"))?
            .canonicalize()
            .with_context(|| format!("write parent does not exist: {}", candidate.display()))?;
        if !parent.starts_with(&self.canonical_path) {
            bail!("path is outside the authorized project");
        }
        let file_name = candidate
            .file_name()
            .ok_or_else(|| anyhow!("write path has no file name"))?;
        Ok(parent.join(file_name))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingToken {
    pub token: String,
    pub short_code: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedDevice {
    pub id: DeviceId,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSummary {
    pub id: DeviceId,
    pub name: String,
    pub created_at_ms: i64,
    pub last_seen_at_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct StoredAttachment {
    pub metadata: AttachmentMetadata,
    pub managed_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredCommand {
    Missing,
    Pending,
    Complete(Box<ServerMessage>),
}

pub struct Storage {
    connection: Mutex<Connection>,
}

impl Storage {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create data directory {}", parent.display()))?;
        }
        let connection = Connection::open(path)
            .with_context(|| format!("open SQLite database {}", path.display()))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.execute_batch(MIGRATION_1)?;
        migrate_2(&connection)?;
        migrate_3(&connection)?;
        migrate_4(&connection)?;
        let storage = Self {
            connection: Mutex::new(connection),
        };
        storage.interrupt_orphaned_conversations()?;
        Ok(storage)
    }

    pub fn host_id(&self) -> Result<HostId> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        if let Some(value) = connection
            .query_row(
                "SELECT value FROM host_meta WHERE key = 'host_id'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            return Ok(HostId(parse_uuid(&value)?));
        }
        let id = HostId::new();
        connection.execute(
            "INSERT INTO host_meta(key, value) VALUES('host_id', ?1)",
            [id.to_string()],
        )?;
        Ok(id)
    }

    pub fn add_project(
        &self,
        path: impl AsRef<Path>,
        name: Option<&str>,
        enabled_providers: &[ProviderId],
    ) -> Result<Project> {
        let canonical_path = path
            .as_ref()
            .canonicalize()
            .with_context(|| format!("project path does not exist: {}", path.as_ref().display()))?;
        if !canonical_path.is_dir() {
            bail!("project path is not a directory");
        }
        let display_name = name.map(str::to_owned).unwrap_or_else(|| {
            canonical_path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| canonical_path.display().to_string())
        });
        let providers = if enabled_providers.is_empty() {
            vec![ProviderId::Codex, ProviderId::Grok]
        } else {
            enabled_providers.to_vec()
        };
        let project = Project {
            id: ProjectId::new(),
            display_name,
            canonical_path,
            enabled_providers: providers,
        };
        let providers_json = serde_json::to_string(&project.enabled_providers)?;
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection.execute(
            "INSERT INTO projects(id, display_name, canonical_path, enabled_providers) VALUES(?1, ?2, ?3, ?4)",
            params![
                project.id.to_string(),
                project.display_name,
                project.canonical_path.to_string_lossy(),
                providers_json,
            ],
        )?;
        Ok(project)
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT id, display_name, canonical_path, enabled_providers FROM projects ORDER BY display_name",
        )?;
        let rows = statement.query_map([], |row| {
            let id: String = row.get(0)?;
            let display_name: String = row.get(1)?;
            let canonical_path: String = row.get(2)?;
            let providers: String = row.get(3)?;
            Ok((id, display_name, canonical_path, providers))
        })?;
        rows.map(|row| {
            let (id, display_name, canonical_path, providers) = row?;
            Ok(Project {
                id: ProjectId(parse_uuid(&id)?),
                display_name,
                canonical_path: PathBuf::from(canonical_path),
                enabled_providers: serde_json::from_str(&providers)?,
            })
        })
        .collect()
    }

    pub fn project(&self, id: ProjectId) -> Result<Project> {
        self.list_projects()?
            .into_iter()
            .find(|project| project.id == id)
            .ok_or_else(|| anyhow!("project {id} was not found"))
    }

    pub fn remove_project(&self, id: ProjectId) -> Result<bool> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        Ok(connection.execute("DELETE FROM projects WHERE id = ?1", [id.to_string()])? > 0)
    }

    pub fn create_pairing_token(&self) -> Result<PairingToken> {
        self.create_pairing_token_with_ttl(10 * 60 * 1000)
    }

    fn create_pairing_token_with_ttl(&self, ttl_ms: i64) -> Result<PairingToken> {
        let token = random_token();
        let short_code = token.chars().take(8).collect::<String>().to_uppercase();
        let expires_at_ms = now_ms() + ttl_ms;
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection.execute(
            "INSERT INTO pair_tokens(token_hash, short_code, expires_at_ms) VALUES(?1, ?2, ?3)",
            params![token_hash(&token), short_code, expires_at_ms],
        )?;
        Ok(PairingToken {
            token,
            short_code,
            expires_at_ms,
        })
    }

    pub fn exchange_pairing_token(&self, token: &str, device_name: &str) -> Result<IssuedDevice> {
        let now = now_ms();
        let hash = token_hash(token);
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        let valid = transaction
            .query_row(
                "SELECT 1 FROM pair_tokens WHERE token_hash = ?1 AND used_at_ms IS NULL AND expires_at_ms >= ?2",
                params![hash, now],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !valid {
            bail!("pair token is invalid, expired, or already used");
        }
        transaction.execute(
            "UPDATE pair_tokens SET used_at_ms = ?2 WHERE token_hash = ?1",
            params![hash, now],
        )?;
        let device = IssuedDevice {
            id: DeviceId::new(),
            token: random_token(),
        };
        transaction.execute(
            "INSERT INTO paired_devices(id, display_name, token_hash, created_at_ms) VALUES(?1, ?2, ?3, ?4)",
            params![
                device.id.to_string(),
                device_name,
                token_hash(&device.token),
                now,
            ],
        )?;
        transaction.commit()?;
        Ok(device)
    }

    pub fn authenticate_device(&self, id: DeviceId, token: &str) -> Result<bool> {
        let now = now_ms();
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let matched = connection
            .query_row(
                "SELECT 1 FROM paired_devices WHERE id = ?1 AND token_hash = ?2 AND revoked_at_ms IS NULL",
                params![id.to_string(), token_hash(token)],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if matched {
            connection.execute(
                "UPDATE paired_devices SET last_seen_at_ms = ?2 WHERE id = ?1",
                params![id.to_string(), now],
            )?;
        }
        Ok(matched)
    }

    pub fn list_devices(&self) -> Result<Vec<DeviceSummary>> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT id, display_name, created_at_ms, last_seen_at_ms FROM paired_devices WHERE revoked_at_ms IS NULL ORDER BY created_at_ms",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (id, name, created_at_ms, last_seen_at_ms) = row?;
            Ok(DeviceSummary {
                id: DeviceId(parse_uuid(&id)?),
                name,
                created_at_ms,
                last_seen_at_ms,
            })
        })
        .collect()
    }

    pub fn revoke_device(&self, id: DeviceId) -> Result<bool> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        Ok(connection.execute(
            "UPDATE paired_devices SET revoked_at_ms = ?2 WHERE id = ?1 AND revoked_at_ms IS NULL",
            params![id.to_string(), now_ms()],
        )? > 0)
    }

    pub fn command_state(
        &self,
        device_id: DeviceId,
        command_id: CommandId,
    ) -> Result<StoredCommand> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let result = connection
            .query_row(
                "SELECT result_json FROM used_commands WHERE device_id = ?1 AND command_id = ?2",
                params![device_id.to_string(), command_id.to_string()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?;
        match result {
            None => Ok(StoredCommand::Missing),
            Some(None) => Ok(StoredCommand::Pending),
            Some(Some(value)) => Ok(StoredCommand::Complete(Box::new(
                serde_json::from_str(&value).context("decode stored command result")?,
            ))),
        }
    }

    pub fn begin_command(&self, device_id: DeviceId, command_id: CommandId) -> Result<()> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection.execute(
            "INSERT OR IGNORE INTO used_commands(device_id, command_id, created_at_ms) VALUES(?1, ?2, ?3)",
            params![device_id.to_string(), command_id.to_string(), now_ms()],
        )?;
        Ok(())
    }

    pub fn finish_command(
        &self,
        device_id: DeviceId,
        command_id: CommandId,
        result: &ServerMessage,
    ) -> Result<()> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let result_json = serde_json::to_string(result).context("encode command result")?;
        if connection.execute(
            "UPDATE used_commands SET result_json = ?3 WHERE device_id = ?1 AND command_id = ?2",
            params![device_id.to_string(), command_id.to_string(), result_json],
        )? == 0
        {
            bail!("command was not started");
        }
        Ok(())
    }

    pub fn upsert_conversation(&self, conversation: &Conversation) -> Result<()> {
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO conversations(id, revision, provider, project_id, native_session_id, title, title_source, title_updated_at_ms, selected_model, selected_effort, state, session_options, updated_at_ms)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(id) DO UPDATE SET revision=excluded.revision, native_session_id=excluded.native_session_id, title=excluded.title,
             title_source=excluded.title_source, title_updated_at_ms=excluded.title_updated_at_ms,
             selected_model=excluded.selected_model, selected_effort=excluded.selected_effort, state=excluded.state,
             session_options=excluded.session_options, updated_at_ms=excluded.updated_at_ms",
            params![
                conversation.id.to_string(),
                conversation.revision as i64,
                provider_name(conversation.provider),
                conversation.project_id.to_string(),
                conversation.native_session_id,
                conversation.title,
                title_source_name(conversation.title_source),
                conversation.title_updated_at_ms,
                conversation.selected_model,
                conversation.selected_effort,
                state_name(conversation.state),
                serde_json::to_string(&conversation.session_options)?,
                conversation.updated_at_ms,
            ],
        )?;
        transaction.execute(
            "INSERT INTO provider_session_mappings(conversation_id, provider, project_id, native_session_id)
             VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(conversation_id) DO UPDATE SET provider=excluded.provider, project_id=excluded.project_id, native_session_id=excluded.native_session_id",
            params![
                conversation.id.to_string(),
                provider_name(conversation.provider),
                conversation.project_id.to_string(),
                conversation.native_session_id,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn update_provider_title_if_current(
        &self,
        expected: &Conversation,
        title: &str,
    ) -> Result<Option<Conversation>> {
        let changed = {
            let connection = self.connection.lock().expect("storage mutex poisoned");
            connection.execute(
                "UPDATE conversations
                 SET title = ?7, title_source = 'provider', title_updated_at_ms = ?8,
                     revision = revision + 1
                 WHERE id = ?1 AND provider = ?2 AND project_id = ?3
                   AND native_session_id = ?4 AND title_updated_at_ms = ?5
                   AND title = ?6 AND title_source != 'user' AND title != ?7",
                params![
                    expected.id.to_string(),
                    provider_name(expected.provider),
                    expected.project_id.to_string(),
                    expected.native_session_id,
                    expected.title_updated_at_ms,
                    expected.title,
                    title,
                    now_ms(),
                ],
            )?
        };
        if changed == 0 {
            Ok(None)
        } else {
            Ok(Some(self.conversation(expected.id)?))
        }
    }

    pub fn conversation(&self, id: ConversationId) -> Result<Conversation> {
        self.list_conversations()?
            .into_iter()
            .find(|conversation| conversation.id == id)
            .ok_or_else(|| anyhow!("conversation {id} was not found"))
    }

    pub fn conversation_by_native_session(
        &self,
        provider: ProviderId,
        project_id: ProjectId,
        native_session_id: &str,
    ) -> Result<Option<Conversation>> {
        let conversation_id = {
            let connection = self.connection.lock().expect("storage mutex poisoned");
            connection
                .query_row(
                    "SELECT conversation_id FROM provider_session_mappings WHERE provider = ?1 AND project_id = ?2 AND native_session_id = ?3",
                    params![provider_name(provider), project_id.to_string(), native_session_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
        };
        conversation_id
            .map(|id| self.conversation(ConversationId(parse_uuid(&id)?)))
            .transpose()
    }

    pub fn list_conversations(&self) -> Result<Vec<Conversation>> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT id, revision, provider, project_id, native_session_id, title, title_source, title_updated_at_ms, selected_model, selected_effort, state, session_options, updated_at_ms
             FROM conversations ORDER BY updated_at_ms DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, i64>(12)?,
            ))
        })?;
        rows.map(|row| {
            let (
                id,
                revision,
                provider,
                project_id,
                native_session_id,
                title,
                title_source,
                title_updated_at_ms,
                selected_model,
                selected_effort,
                state,
                session_options,
                updated_at_ms,
            ) = row?;
            Ok(Conversation {
                id: ConversationId(parse_uuid(&id)?),
                revision,
                provider: parse_provider(&provider)?,
                project_id: ProjectId(parse_uuid(&project_id)?),
                native_session_id,
                title,
                title_source: parse_title_source(&title_source)?,
                title_updated_at_ms,
                selected_model,
                selected_effort,
                state: parse_state(&state)?,
                session_options: serde_json::from_str(&session_options)?,
                updated_at_ms,
            })
        })
        .collect()
    }

    pub fn upsert_timeline_item(&self, item: &TimelineItem) -> Result<bool> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let changed = connection.execute(
            "INSERT INTO timeline_items(id, conversation_id, revision, created_at_ms, item_json) VALUES(?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET revision=excluded.revision, created_at_ms=excluded.created_at_ms, item_json=excluded.item_json
             WHERE excluded.revision > timeline_items.revision",
            params![
                item.id.to_string(),
                item.conversation_id.to_string(),
                item.revision as i64,
                item.created_at_ms,
                serde_json::to_string(item)?,
            ],
        )?;
        Ok(changed > 0)
    }

    pub fn list_timeline(&self) -> Result<Vec<TimelineItem>> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let mut statement = connection
            .prepare("SELECT item_json FROM timeline_items ORDER BY created_at_ms, id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    pub fn list_timeline_page(
        &self,
        conversation_id: ConversationId,
        before: Option<TimelinePageCursor>,
        limit: u32,
    ) -> Result<(Vec<TimelineItem>, Option<TimelinePageCursor>)> {
        let limit = limit.clamp(1, 200) as i64;
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let (query, values): (&str, Vec<rusqlite::types::Value>) = match before {
            Some(before) => (
                "SELECT item_json, created_at_ms, id FROM timeline_items
                 WHERE conversation_id = ?1
                   AND (created_at_ms < ?2 OR (created_at_ms = ?2 AND id < ?3))
                 ORDER BY created_at_ms DESC, id DESC LIMIT ?4",
                vec![
                    conversation_id.to_string().into(),
                    before.created_at_ms.into(),
                    before.item_id.to_string().into(),
                    (limit + 1).into(),
                ],
            ),
            None => (
                "SELECT item_json, created_at_ms, id FROM timeline_items
                 WHERE conversation_id = ?1
                 ORDER BY created_at_ms DESC, id DESC LIMIT ?2",
                vec![conversation_id.to_string().into(), (limit + 1).into()],
            ),
        };
        let mut statement = connection.prepare(query)?;
        let rows = statement.query_map(rusqlite::params_from_iter(values), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let has_more = rows.len() > limit as usize;
        if has_more {
            rows.pop();
        }
        let next_before = if has_more {
            rows.last()
                .map(|row| {
                    Ok::<TimelinePageCursor, anyhow::Error>(TimelinePageCursor {
                        created_at_ms: row.1,
                        item_id: TimelineItemId(parse_uuid(&row.2)?),
                    })
                })
                .transpose()?
        } else {
            None
        };
        let mut items = rows
            .into_iter()
            .map(|(json, _, _)| serde_json::from_str(&json).map_err(anyhow::Error::from))
            .collect::<Result<Vec<_>>>()?;
        items.reverse();
        Ok((items, next_before))
    }

    pub fn provider_item_id(
        &self,
        conversation_id: ConversationId,
        provider_item_id: &str,
    ) -> Result<TimelineItemId> {
        let generated = TimelineItemId::new();
        let connection = self.connection.lock().expect("storage mutex poisoned");
        if let Some(value) =
            lookup_provider_item_id(&connection, conversation_id, provider_item_id)?
        {
            return Ok(TimelineItemId(parse_uuid(&value)?));
        }
        connection.execute(
            "INSERT OR IGNORE INTO provider_item_ids(conversation_id, provider_item_id, timeline_item_id)
             VALUES(?1, ?2, ?3)",
            params![
                conversation_id.to_string(),
                provider_item_id,
                generated.to_string(),
            ],
        )?;
        let value = lookup_provider_item_id(&connection, conversation_id, provider_item_id)?
            .ok_or_else(|| anyhow!("provider item id was not stored"))?;
        Ok(TimelineItemId(parse_uuid(&value)?))
    }

    pub fn reconcile_provider_item_alias(
        &self,
        conversation_id: ConversationId,
        provider_item_id: &str,
        kind: &TimelineItemKind,
        canonical_prefix: &str,
    ) -> Result<Option<TimelineItemId>> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        if let Some(value) =
            lookup_provider_item_id(&connection, conversation_id, provider_item_id)?
        {
            return Ok(Some(TimelineItemId(parse_uuid(&value)?)));
        }
        let candidates = {
            let mut statement = connection.prepare(
                "SELECT id, item_json
                 FROM timeline_items
                 WHERE conversation_id = ?1
                   AND EXISTS (
                       SELECT 1 FROM provider_item_ids
                       WHERE provider_item_ids.timeline_item_id = timeline_items.id
                   )
                 ORDER BY created_at_ms, id",
            )?;
            statement
                .query_map(params![conversation_id.to_string()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let canonical_pattern = format!("{canonical_prefix}%");
        for (timeline_item_id, item_json) in candidates {
            let item: TimelineItem = serde_json::from_str(&item_json)?;
            if !timeline_items_are_alias_compatible(&item.kind, kind) {
                continue;
            }
            let already_canonical = connection
                .query_row(
                    "SELECT 1
                     FROM (
                         SELECT provider_item_id, timeline_item_id FROM provider_item_ids
                         UNION ALL
                         SELECT provider_item_id, timeline_item_id FROM provider_item_aliases
                     )
                     WHERE timeline_item_id = ?1 AND provider_item_id LIKE ?2
                     LIMIT 1",
                    params![timeline_item_id, canonical_pattern],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if already_canonical {
                continue;
            }
            connection.execute(
                "INSERT INTO provider_item_aliases(conversation_id, provider_item_id, timeline_item_id)
                 VALUES(?1, ?2, ?3)",
                params![
                    conversation_id.to_string(),
                    provider_item_id,
                    timeline_item_id,
                ],
            )?;
            return Ok(Some(TimelineItemId(parse_uuid(&timeline_item_id)?)));
        }
        Ok(None)
    }

    pub fn alias_provider_item_id(
        &self,
        conversation_id: ConversationId,
        provider_item_id: &str,
        alias_provider_item_id: &str,
    ) -> Result<Option<TimelineItemId>> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let Some(alias_timeline_item_id) =
            lookup_provider_item_id(&connection, conversation_id, alias_provider_item_id)?
        else {
            return Ok(None);
        };
        if let Some(existing) =
            lookup_provider_item_id(&connection, conversation_id, provider_item_id)?
        {
            if existing != alias_timeline_item_id {
                bail!("provider item alias conflicts with an existing timeline item");
            }
            return Ok(Some(TimelineItemId(parse_uuid(&existing)?)));
        }
        connection.execute(
            "INSERT INTO provider_item_aliases(conversation_id, provider_item_id, timeline_item_id)
             VALUES(?1, ?2, ?3)",
            params![
                conversation_id.to_string(),
                provider_item_id,
                alias_timeline_item_id,
            ],
        )?;
        Ok(Some(TimelineItemId(parse_uuid(&alias_timeline_item_id)?)))
    }

    pub fn remote_history_is_stale(
        &self,
        provider: ProviderId,
        project_id: ProjectId,
        native_session_id: &str,
        remote_updated_at_ms: i64,
    ) -> Result<bool> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let stored = connection
            .query_row(
                "SELECT remote_updated_at_ms FROM provider_sync_state WHERE provider = ?1 AND project_id = ?2 AND native_session_id = ?3",
                params![provider_name(provider), project_id.to_string(), native_session_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        Ok(stored.is_none_or(|stored| remote_updated_at_ms > stored))
    }

    pub fn mark_remote_history_synced(
        &self,
        provider: ProviderId,
        project_id: ProjectId,
        native_session_id: &str,
        remote_updated_at_ms: i64,
    ) -> Result<()> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection.execute(
            "INSERT INTO provider_sync_state(provider, project_id, native_session_id, remote_updated_at_ms, synced_at_ms)
             VALUES(?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(provider, project_id, native_session_id) DO UPDATE SET
                 remote_updated_at_ms=MAX(provider_sync_state.remote_updated_at_ms, excluded.remote_updated_at_ms),
                 synced_at_ms=excluded.synced_at_ms",
            params![
                provider_name(provider),
                project_id.to_string(),
                native_session_id,
                remote_updated_at_ms,
                now_ms(),
            ],
        )?;
        Ok(())
    }

    pub fn save_attachment(&self, attachment: &StoredAttachment) -> Result<()> {
        let metadata = &attachment.metadata;
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection.execute(
            "INSERT INTO attachments(id, conversation_id, mime_type, byte_len, width, height, managed_path, created_at_ms)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                metadata.id.to_string(),
                metadata.conversation_id.to_string(),
                metadata.mime_type,
                metadata.byte_len as i64,
                metadata.width,
                metadata.height,
                attachment.managed_path.to_string_lossy(),
                metadata.created_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn attachment(&self, id: AttachmentId) -> Result<StoredAttachment> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let row = connection
            .query_row(
                "SELECT conversation_id, mime_type, byte_len, width, height, managed_path, created_at_ms FROM attachments WHERE id = ?1",
                [id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)? as u64,
                        row.get::<_, Option<u32>>(3)?,
                        row.get::<_, Option<u32>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| anyhow!("attachment {id} was not found"))?;
        Ok(StoredAttachment {
            metadata: AttachmentMetadata {
                id,
                conversation_id: ConversationId(parse_uuid(&row.0)?),
                mime_type: row.1,
                byte_len: row.2,
                width: row.3,
                height: row.4,
                created_at_ms: row.6,
            },
            managed_path: PathBuf::from(row.5),
        })
    }

    fn interrupt_orphaned_conversations(&self) -> Result<()> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection.execute(
            "UPDATE conversations SET state = 'interrupted', revision = revision + 1, updated_at_ms = ?1
             WHERE state IN ('running', 'needs_approval')",
            [now_ms()],
        )?;
        Ok(())
    }
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_millis() as i64
}

fn parse_uuid(value: &str) -> Result<uuid::Uuid> {
    uuid::Uuid::parse_str(value).with_context(|| format!("invalid UUID in database: {value}"))
}

fn lookup_provider_item_id(
    connection: &Connection,
    conversation_id: ConversationId,
    provider_item_id: &str,
) -> Result<Option<String>> {
    Ok(connection
        .query_row(
            "SELECT timeline_item_id
             FROM (
                 SELECT conversation_id, provider_item_id, timeline_item_id FROM provider_item_ids
                 UNION ALL
                 SELECT conversation_id, provider_item_id, timeline_item_id FROM provider_item_aliases
             )
             WHERE conversation_id = ?1 AND provider_item_id = ?2
             LIMIT 1",
            params![conversation_id.to_string(), provider_item_id],
            |row| row.get(0),
        )
        .optional()?)
}

fn timeline_items_are_alias_compatible(
    existing: &TimelineItemKind,
    history: &TimelineItemKind,
) -> bool {
    match (existing, history) {
        (
            TimelineItemKind::AgentMessage {
                phase: existing_phase,
                text: existing_text,
            },
            TimelineItemKind::AgentMessage {
                phase: history_phase,
                text: history_text,
            },
        ) => existing_phase == history_phase && text_prefix_matches(existing_text, history_text),
        (
            TimelineItemKind::Command {
                command: existing_command,
                relative_cwd: existing_cwd,
                ..
            },
            TimelineItemKind::Command {
                command: history_command,
                relative_cwd: history_cwd,
                ..
            },
        ) => existing_command == history_command && existing_cwd == history_cwd,
        (
            TimelineItemKind::FileChange {
                relative_path: existing_path,
                change_kind: existing_kind,
                ..
            },
            TimelineItemKind::FileChange {
                relative_path: history_path,
                change_kind: history_kind,
                ..
            },
        ) => existing_path == history_path && existing_kind == history_kind,
        (
            TimelineItemKind::ToolCall {
                name: existing_name,
                input_summary: existing_input,
                ..
            },
            TimelineItemKind::ToolCall {
                name: history_name,
                input_summary: history_input,
                ..
            },
        ) => {
            existing_name == history_name
                && match (existing_input, history_input) {
                    (Some(existing), Some(history)) => existing == history,
                    _ => true,
                }
        }
        (
            TimelineItemKind::Error {
                code: existing_code,
                message: existing_message,
            },
            TimelineItemKind::Error {
                code: history_code,
                message: history_message,
            },
        ) => {
            existing_code == history_code && text_prefix_matches(existing_message, history_message)
        }
        _ => false,
    }
}

fn text_prefix_matches(left: &str, right: &str) -> bool {
    if left.is_empty() || right.is_empty() {
        left == right
    } else {
        left.starts_with(right) || right.starts_with(left)
    }
}

fn short_project_path(path: &Path) -> String {
    let components = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>();
    components
        .iter()
        .rev()
        .take(2)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("/")
}

fn provider_name(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Codex => "codex",
        ProviderId::Grok => "grok",
    }
}

fn parse_provider(value: &str) -> Result<ProviderId> {
    match value {
        "codex" => Ok(ProviderId::Codex),
        "grok" => Ok(ProviderId::Grok),
        _ => bail!("unknown provider in database: {value}"),
    }
}

fn state_name(state: ConversationState) -> &'static str {
    match state {
        ConversationState::Idle => "idle",
        ConversationState::Running => "running",
        ConversationState::NeedsApproval => "needs_approval",
        ConversationState::Completed => "completed",
        ConversationState::Failed => "failed",
        ConversationState::Interrupted => "interrupted",
        ConversationState::Offline => "offline",
    }
}

fn parse_state(value: &str) -> Result<ConversationState> {
    match value {
        "idle" => Ok(ConversationState::Idle),
        "running" => Ok(ConversationState::Running),
        "needs_approval" => Ok(ConversationState::NeedsApproval),
        "completed" => Ok(ConversationState::Completed),
        "failed" => Ok(ConversationState::Failed),
        "interrupted" => Ok(ConversationState::Interrupted),
        "offline" => Ok(ConversationState::Offline),
        _ => bail!("unknown conversation state in database: {value}"),
    }
}

fn title_source_name(source: ConversationTitleSource) -> &'static str {
    match source {
        ConversationTitleSource::Fallback => "fallback",
        ConversationTitleSource::Generated => "generated",
        ConversationTitleSource::Provider => "provider",
        ConversationTitleSource::User => "user",
    }
}

fn parse_title_source(value: &str) -> Result<ConversationTitleSource> {
    match value {
        "fallback" => Ok(ConversationTitleSource::Fallback),
        "generated" => Ok(ConversationTitleSource::Generated),
        "provider" => Ok(ConversationTitleSource::Provider),
        "user" => Ok(ConversationTitleSource::User),
        _ => bail!("unknown conversation title source in database: {value}"),
    }
}

fn migrate_2(connection: &Connection) -> Result<()> {
    if !column_exists(connection, "conversations", "title_source")? {
        connection.execute(
            "ALTER TABLE conversations ADD COLUMN title_source TEXT NOT NULL DEFAULT 'provider'",
            [],
        )?;
    }
    if !column_exists(connection, "conversations", "title_updated_at_ms")? {
        connection.execute(
            "ALTER TABLE conversations ADD COLUMN title_updated_at_ms INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    connection.execute_batch(MIGRATION_2)?;
    connection.execute(
        "INSERT INTO host_meta(key, value) VALUES('schema_version', '2')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [],
    )?;
    Ok(())
}

fn migrate_3(connection: &Connection) -> Result<()> {
    if !column_exists(connection, "used_commands", "result_json")? {
        connection.execute("ALTER TABLE used_commands ADD COLUMN result_json TEXT", [])?;
    }
    connection.execute(
        "INSERT INTO host_meta(key, value) VALUES('schema_version', '3')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [],
    )?;
    Ok(())
}

fn migrate_4(connection: &Connection) -> Result<()> {
    connection.execute_batch(MIGRATION_4)?;
    connection.execute(
        "INSERT INTO host_meta(key, value) VALUES('schema_version', '4')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [],
    )?;
    Ok(())
}

fn column_exists(connection: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = statement.query_map([], |row| row.get::<_, String>(1))?;
    for name in names {
        if name? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

const MIGRATION_1: &str = r#"
CREATE TABLE IF NOT EXISTS host_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    canonical_path TEXT NOT NULL UNIQUE,
    enabled_providers TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS pair_tokens (
    token_hash TEXT PRIMARY KEY,
    short_code TEXT NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    used_at_ms INTEGER
);
CREATE TABLE IF NOT EXISTS paired_devices (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    last_seen_at_ms INTEGER,
    revoked_at_ms INTEGER
);
CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL,
    provider TEXT NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    native_session_id TEXT NOT NULL,
    title TEXT NOT NULL,
    selected_model TEXT,
    selected_effort TEXT,
    state TEXT NOT NULL,
    session_options TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS timeline_items (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    item_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS attachments (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    mime_type TEXT NOT NULL,
    byte_len INTEGER NOT NULL,
    width INTEGER,
    height INTEGER,
    managed_path TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS used_commands (
    device_id TEXT NOT NULL REFERENCES paired_devices(id) ON DELETE CASCADE,
    command_id TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(device_id, command_id)
);
CREATE TABLE IF NOT EXISTS provider_session_mappings (
    conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    project_id TEXT NOT NULL,
    native_session_id TEXT NOT NULL,
    UNIQUE(provider, project_id, native_session_id)
);
"#;

const MIGRATION_2: &str = r#"
CREATE TABLE IF NOT EXISTS provider_item_ids (
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    provider_item_id TEXT NOT NULL,
    timeline_item_id TEXT NOT NULL UNIQUE,
    PRIMARY KEY(conversation_id, provider_item_id)
);
CREATE TABLE IF NOT EXISTS provider_sync_state (
    provider TEXT NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    native_session_id TEXT NOT NULL,
    remote_updated_at_ms INTEGER NOT NULL,
    synced_at_ms INTEGER NOT NULL,
    PRIMARY KEY(provider, project_id, native_session_id)
);
CREATE INDEX IF NOT EXISTS timeline_items_conversation_page
    ON timeline_items(conversation_id, created_at_ms DESC, id DESC);
"#;

const MIGRATION_4: &str = r#"
CREATE TABLE IF NOT EXISTS provider_item_aliases (
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    provider_item_id TEXT NOT NULL,
    timeline_item_id TEXT NOT NULL REFERENCES timeline_items(id) ON DELETE CASCADE,
    PRIMARY KEY(conversation_id, provider_item_id)
);
CREATE INDEX IF NOT EXISTS provider_item_aliases_timeline_item
    ON provider_item_aliases(timeline_item_id);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn storage() -> (tempfile::TempDir, Storage) {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage = Storage::open(temp.path().join("state.db")).expect("open storage");
        (temp, storage)
    }

    #[test]
    fn pairing_token_is_single_use_and_device_can_be_revoked() {
        let (_temp, storage) = storage();
        let pair = storage.create_pairing_token().expect("create pair token");
        let device = storage
            .exchange_pairing_token(&pair.token, "phone")
            .expect("exchange token");
        assert!(
            storage
                .authenticate_device(device.id, &device.token)
                .expect("authenticate")
        );
        assert!(
            storage
                .exchange_pairing_token(&pair.token, "other")
                .is_err()
        );
        assert!(storage.revoke_device(device.id).expect("revoke"));
        assert!(
            !storage
                .authenticate_device(device.id, &device.token)
                .expect("authenticate revoked")
        );
    }

    #[test]
    fn expired_pairing_token_is_rejected() {
        let (_temp, storage) = storage();
        let pair = storage
            .create_pairing_token_with_ttl(-1)
            .expect("create expired pair token");
        assert!(
            storage
                .exchange_pairing_token(&pair.token, "phone")
                .is_err()
        );
    }

    #[test]
    fn timeline_cursor_pages_items_with_identical_timestamps_without_skips() {
        let (temp, storage) = storage();
        let project = storage
            .add_project(temp.path(), Some("same-time"), &[ProviderId::Codex])
            .expect("add project");
        let conversation = Conversation {
            id: ConversationId::new(),
            revision: 1,
            provider: ProviderId::Codex,
            project_id: project.id,
            native_session_id: "native-page".to_owned(),
            title: "page".to_owned(),
            title_source: ConversationTitleSource::Provider,
            title_updated_at_ms: 10,
            selected_model: None,
            selected_effort: None,
            state: ConversationState::Idle,
            session_options: Vec::new(),
            updated_at_ms: 10,
        };
        storage
            .upsert_conversation(&conversation)
            .expect("save conversation");
        for index in 0..5 {
            storage
                .upsert_timeline_item(&TimelineItem {
                    id: TimelineItemId::new(),
                    conversation_id: conversation.id,
                    revision: 1,
                    created_at_ms: 42,
                    kind: agent_remote_protocol::TimelineItemKind::UserMessage {
                        text: format!("item {index}"),
                    },
                })
                .expect("save item");
        }

        let mut cursor = None;
        let mut ids = Vec::new();
        loop {
            let (page, next) = storage
                .list_timeline_page(conversation.id, cursor, 2)
                .expect("page");
            ids.extend(page.into_iter().map(|item| item.id));
            match next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 5);
    }

    #[test]
    fn completed_command_result_is_replayed() {
        let (temp, storage) = storage();
        let pair = storage.create_pairing_token().expect("create pair token");
        let device = storage
            .exchange_pairing_token(&pair.token, "phone")
            .expect("exchange token");
        let command = CommandId::new();
        assert_eq!(
            storage
                .command_state(device.id, command)
                .expect("missing result"),
            StoredCommand::Missing
        );
        storage
            .begin_command(device.id, command)
            .expect("begin command");
        let response = ServerMessage::CommandAccepted {
            command_id: command,
        };
        storage
            .finish_command(device.id, command, &response)
            .expect("finish command");
        drop(storage);
        let storage = Storage::open(temp.path().join("state.db")).expect("reopen storage");
        assert_eq!(
            storage
                .command_state(device.id, command)
                .expect("stored result"),
            StoredCommand::Complete(Box::new(response))
        );
    }

    #[test]
    fn pending_command_survives_storage_reopen() {
        let (temp, storage) = storage();
        let pair = storage.create_pairing_token().expect("create pair token");
        let device = storage
            .exchange_pairing_token(&pair.token, "phone")
            .expect("exchange token");
        let command = CommandId::new();
        storage
            .begin_command(device.id, command)
            .expect("begin command");

        drop(storage);
        let storage = Storage::open(temp.path().join("state.db")).expect("reopen storage");
        assert_eq!(
            storage
                .command_state(device.id, command)
                .expect("pending command"),
            StoredCommand::Pending
        );
    }

    #[test]
    fn remote_history_watermark_is_monotonic() {
        let (temp, storage) = storage();
        let project = storage
            .add_project(temp.path(), Some("watermark"), &[ProviderId::Codex])
            .expect("add project");

        assert!(
            storage
                .remote_history_is_stale(ProviderId::Codex, project.id, "native-1", 100)
                .expect("missing watermark")
        );
        storage
            .mark_remote_history_synced(ProviderId::Codex, project.id, "native-1", 120)
            .expect("mark watermark");
        storage
            .mark_remote_history_synced(ProviderId::Codex, project.id, "native-1", 110)
            .expect("mark older watermark");

        assert!(
            !storage
                .remote_history_is_stale(ProviderId::Codex, project.id, "native-1", 120)
                .expect("same watermark")
        );
        assert!(
            !storage
                .remote_history_is_stale(ProviderId::Codex, project.id, "native-1", 119)
                .expect("older remote timestamp")
        );
        assert!(
            storage
                .remote_history_is_stale(ProviderId::Codex, project.id, "native-1", 121)
                .expect("newer remote timestamp")
        );
    }

    #[test]
    fn canonical_provider_item_alias_reuses_a_legacy_timeline_item() {
        let (temp, storage) = storage();
        let project = storage
            .add_project(temp.path(), Some("alias"), &[ProviderId::Codex])
            .expect("add project");
        let conversation = Conversation {
            id: ConversationId::new(),
            revision: 1,
            provider: ProviderId::Codex,
            project_id: project.id,
            native_session_id: "native-alias".to_owned(),
            title: "alias".to_owned(),
            title_source: ConversationTitleSource::Provider,
            title_updated_at_ms: 10,
            selected_model: None,
            selected_effort: None,
            state: ConversationState::Completed,
            session_options: Vec::new(),
            updated_at_ms: 10,
        };
        storage
            .upsert_conversation(&conversation)
            .expect("save conversation");
        let mut legacy_ids = Vec::new();
        for index in 0..2 {
            let provider_item_id = format!("msg_live_{index}");
            let legacy_id = storage
                .provider_item_id(conversation.id, &provider_item_id)
                .expect("legacy provider item id");
            storage
                .upsert_timeline_item(&TimelineItem {
                    id: legacy_id,
                    conversation_id: conversation.id,
                    revision: 1,
                    created_at_ms: 20 + index,
                    kind: TimelineItemKind::AgentMessage {
                        phase: agent_remote_protocol::AgentMessagePhase::Final,
                        text: format!("partial {index}"),
                    },
                })
                .expect("save legacy timeline item");
            legacy_ids.push(legacy_id);
        }

        for (index, legacy_id) in legacy_ids.iter().enumerate() {
            let canonical = format!("codex:v1:turn-1:agent:{index}");
            let final_kind = TimelineItemKind::AgentMessage {
                phase: agent_remote_protocol::AgentMessagePhase::Final,
                text: format!("partial {index} complete"),
            };
            assert_eq!(
                storage
                    .reconcile_provider_item_alias(
                        conversation.id,
                        &canonical,
                        &final_kind,
                        "codex:v1:",
                    )
                    .expect("reconcile alias"),
                Some(*legacy_id)
            );
            assert_eq!(
                storage
                    .provider_item_id(conversation.id, &canonical)
                    .expect("canonical alias"),
                *legacy_id
            );
        }
        assert_ne!(legacy_ids[0], legacy_ids[1]);
    }

    #[test]
    fn canonical_alias_does_not_shift_when_a_legacy_item_is_missing() {
        let (temp, storage) = storage();
        let project = storage
            .add_project(temp.path(), Some("missing"), &[ProviderId::Codex])
            .expect("add project");
        let conversation = Conversation {
            id: ConversationId::new(),
            revision: 1,
            provider: ProviderId::Codex,
            project_id: project.id,
            native_session_id: "native-missing".to_owned(),
            title: "missing".to_owned(),
            title_source: ConversationTitleSource::Provider,
            title_updated_at_ms: 10,
            selected_model: None,
            selected_effort: None,
            state: ConversationState::Completed,
            session_options: Vec::new(),
            updated_at_ms: 10,
        };
        storage
            .upsert_conversation(&conversation)
            .expect("save conversation");
        let mut legacy_ids = Vec::new();
        for (provider_item_id, text, created_at_ms) in
            [("msg_a", "alpha", 20), ("msg_c", "charlie", 22)]
        {
            let item_id = storage
                .provider_item_id(conversation.id, provider_item_id)
                .expect("legacy provider item id");
            storage
                .upsert_timeline_item(&TimelineItem {
                    id: item_id,
                    conversation_id: conversation.id,
                    revision: 1,
                    created_at_ms,
                    kind: TimelineItemKind::AgentMessage {
                        phase: agent_remote_protocol::AgentMessagePhase::Final,
                        text: text.to_owned(),
                    },
                })
                .expect("save legacy item");
            legacy_ids.push(item_id);
        }
        let final_kind = |text: &str| TimelineItemKind::AgentMessage {
            phase: agent_remote_protocol::AgentMessagePhase::Final,
            text: text.to_owned(),
        };

        assert_eq!(
            storage
                .reconcile_provider_item_alias(
                    conversation.id,
                    "codex:v1:turn-1:agent:0",
                    &final_kind("alpha complete"),
                    "codex:v1:",
                )
                .expect("alias A"),
            Some(legacy_ids[0])
        );
        assert_eq!(
            storage
                .reconcile_provider_item_alias(
                    conversation.id,
                    "codex:v1:turn-1:agent:1",
                    &final_kind("bravo complete"),
                    "codex:v1:",
                )
                .expect("missing B"),
            None
        );
        assert_eq!(
            storage
                .reconcile_provider_item_alias(
                    conversation.id,
                    "codex:v1:turn-1:agent:2",
                    &final_kind("charlie complete"),
                    "codex:v1:",
                )
                .expect("alias C"),
            Some(legacy_ids[1])
        );
    }

    #[test]
    fn repeated_compatible_legacy_items_get_distinct_canonical_aliases() {
        let (temp, storage) = storage();
        let project = storage
            .add_project(temp.path(), Some("repeated"), &[ProviderId::Codex])
            .expect("add project");
        let conversation = Conversation {
            id: ConversationId::new(),
            revision: 1,
            provider: ProviderId::Codex,
            project_id: project.id,
            native_session_id: "native-repeated".to_owned(),
            title: "repeated".to_owned(),
            title_source: ConversationTitleSource::Provider,
            title_updated_at_ms: 10,
            selected_model: None,
            selected_effort: None,
            state: ConversationState::Completed,
            session_options: Vec::new(),
            updated_at_ms: 10,
        };
        storage
            .upsert_conversation(&conversation)
            .expect("save conversation");
        let mut legacy_ids = Vec::new();
        for index in 0..2 {
            let item_id = storage
                .provider_item_id(conversation.id, &format!("msg_{index}"))
                .expect("legacy provider item id");
            storage
                .upsert_timeline_item(&TimelineItem {
                    id: item_id,
                    conversation_id: conversation.id,
                    revision: 1,
                    created_at_ms: 20 + index,
                    kind: TimelineItemKind::AgentMessage {
                        phase: agent_remote_protocol::AgentMessagePhase::Final,
                        text: "same partial".to_owned(),
                    },
                })
                .expect("save legacy item");
            legacy_ids.push(item_id);
        }
        let final_kind = TimelineItemKind::AgentMessage {
            phase: agent_remote_protocol::AgentMessagePhase::Final,
            text: "same partial complete".to_owned(),
        };
        let mut reconciled = Vec::new();
        for index in 0..2 {
            reconciled.push(
                storage
                    .reconcile_provider_item_alias(
                        conversation.id,
                        &format!("codex:v1:turn-1:agent:{index}"),
                        &final_kind,
                        "codex:v1:",
                    )
                    .expect("reconcile repeated alias")
                    .expect("matching legacy item"),
            );
        }

        assert_eq!(reconciled, legacy_ids);
        assert_ne!(reconciled[0], reconciled[1]);
    }

    #[test]
    fn migration_four_reconciles_existing_raw_provider_ids() {
        let temp = tempfile::tempdir().expect("temp dir");
        let database = temp.path().join("legacy-v3.db");
        let connection = Connection::open(&database).expect("legacy database");
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .expect("foreign keys");
        connection
            .execute_batch(MIGRATION_1)
            .expect("migration one");
        migrate_2(&connection).expect("migration two");
        migrate_3(&connection).expect("migration three");
        let legacy = Storage {
            connection: Mutex::new(connection),
        };
        let project = legacy
            .add_project(temp.path(), Some("legacy"), &[ProviderId::Codex])
            .expect("add project");
        let conversation = Conversation {
            id: ConversationId::new(),
            revision: 1,
            provider: ProviderId::Codex,
            project_id: project.id,
            native_session_id: "native-legacy".to_owned(),
            title: "legacy".to_owned(),
            title_source: ConversationTitleSource::Provider,
            title_updated_at_ms: 10,
            selected_model: None,
            selected_effort: None,
            state: ConversationState::Completed,
            session_options: Vec::new(),
            updated_at_ms: 10,
        };
        legacy
            .upsert_conversation(&conversation)
            .expect("save conversation");
        let legacy_item_id = TimelineItemId::new();
        legacy
            .upsert_timeline_item(&TimelineItem {
                id: legacy_item_id,
                conversation_id: conversation.id,
                revision: 1,
                created_at_ms: 20,
                kind: TimelineItemKind::AgentMessage {
                    phase: agent_remote_protocol::AgentMessagePhase::Final,
                    text: "partial".to_owned(),
                },
            })
            .expect("save legacy item");
        legacy
            .connection
            .lock()
            .expect("legacy storage mutex")
            .execute(
                "INSERT INTO provider_item_ids(conversation_id, provider_item_id, timeline_item_id)
                 VALUES(?1, ?2, ?3)",
                params![
                    conversation.id.to_string(),
                    "msg_raw",
                    legacy_item_id.to_string(),
                ],
            )
            .expect("raw provider id");
        drop(legacy);

        let migrated = Storage::open(&database).expect("migrate legacy database");
        let canonical_kind = TimelineItemKind::AgentMessage {
            phase: agent_remote_protocol::AgentMessagePhase::Final,
            text: "partial complete".to_owned(),
        };
        assert_eq!(
            migrated
                .reconcile_provider_item_alias(
                    conversation.id,
                    "codex:v1:turn-1:agent:0",
                    &canonical_kind,
                    "codex:v1:",
                )
                .expect("reconcile after migration"),
            Some(legacy_item_id)
        );
        assert_eq!(
            migrated
                .provider_item_id(conversation.id, "codex:v1:turn-1:agent:0")
                .expect("canonical alias"),
            legacy_item_id
        );
    }

    #[test]
    fn project_paths_cannot_escape_the_whitelist() {
        let (temp, storage) = storage();
        let project_root = temp.path().join("project");
        let outside = temp.path().join("outside.txt");
        fs::create_dir(&project_root).expect("project dir");
        fs::write(project_root.join("inside.txt"), "ok").expect("inside file");
        fs::write(&outside, "no").expect("outside file");
        let project = storage
            .add_project(&project_root, None, &[ProviderId::Codex])
            .expect("add project");
        assert!(project.resolve_existing(Path::new("inside.txt")).is_ok());
        assert!(project.resolve_existing(&outside).is_err());
        assert!(
            project
                .resolve_for_write(Path::new("../outside.txt"))
                .is_err()
        );
    }
}
