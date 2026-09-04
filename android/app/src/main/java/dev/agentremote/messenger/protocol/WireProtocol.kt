package dev.agentremote.messenger.protocol

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.databind.node.ArrayNode
import com.fasterxml.jackson.databind.node.ObjectNode
import com.fasterxml.jackson.dataformat.cbor.CBORFactory
import dev.agentremote.messenger.model.ApprovalOption
import dev.agentremote.messenger.model.AttachmentCapability
import dev.agentremote.messenger.model.AttachmentData
import dev.agentremote.messenger.model.ConnectionTarget
import dev.agentremote.messenger.model.Conversation
import dev.agentremote.messenger.model.EffortOption
import dev.agentremote.messenger.model.ModelOption
import dev.agentremote.messenger.model.PermissionModeOption
import dev.agentremote.messenger.model.PermissionRisk
import dev.agentremote.messenger.model.PlanStep
import dev.agentremote.messenger.model.ProjectSummary
import dev.agentremote.messenger.model.ProviderCapability
import dev.agentremote.messenger.model.ProviderId
import dev.agentremote.messenger.model.PromptAttachment
import dev.agentremote.messenger.model.ServerEvent
import dev.agentremote.messenger.model.SessionOption
import dev.agentremote.messenger.model.SessionOptionValue
import dev.agentremote.messenger.model.SessionSummary
import dev.agentremote.messenger.model.Snapshot
import dev.agentremote.messenger.model.StoredCredential
import dev.agentremote.messenger.model.TimelineContent
import dev.agentremote.messenger.model.TimelineItem
import dev.agentremote.messenger.model.TimelinePageCursor
import java.nio.ByteBuffer
import java.nio.charset.StandardCharsets
import java.util.UUID

object WireProtocol {
    const val VERSION = 5
    const val SUBPROTOCOL = "agent-remote.cbor.v5"
    private val mapper = ObjectMapper(CBORFactory())

    fun pair(target: ConnectionTarget, deviceName: String): ByteArray = command("pair") {
        setUuid("host_id", target.hostId)
        put("pair_token", requireNotNull(target.pairToken))
        put("device_name", deviceName)
    }

    fun authenticate(credential: StoredCredential): ByteArray = command("authenticate") {
        setUuid("host_id", credential.hostId)
        setUuid("device_id", credential.deviceId)
        put("device_token", credential.deviceToken)
    }

    fun getSnapshot(): ByteArray = command("get_snapshot")

    fun encodeSnapshot(snapshot: Snapshot): ByteArray = command("snapshot") {
        set<ObjectNode>("snapshot", snapshotNode(snapshot))
    }

    fun refreshProjects(provider: ProviderId): ByteArray = command("refresh_projects") {
        put("provider", provider.wire)
    }

    fun syncProject(commandId: UUID, projectId: UUID, provider: ProviderId): ByteArray =
        command("sync_project") {
            setUuid("command_id", commandId)
            setUuid("project_id", projectId)
            put("provider", provider.wire)
        }

    fun createConversation(
        commandId: UUID,
        projectId: UUID,
        provider: ProviderId,
        nativeSessionId: String?,
        model: String?,
        effort: String?,
    ): ByteArray = command("create_conversation") {
        setUuid("command_id", commandId)
        setUuid("project_id", projectId)
        put("provider", provider.wire)
        putNullable("native_session_id", nativeSessionId)
        putNullable("model", model)
        putNullable("effort", effort)
    }

    fun startConversation(
        commandId: UUID,
        clientMessageId: String,
        conversationId: UUID,
        projectId: UUID,
        provider: ProviderId,
        model: String?,
        effort: String?,
        permissionMode: String?,
        text: String,
        attachments: List<PromptAttachment>,
        attempt: Int = 0,
    ): ByteArray = command("start_conversation") {
        require(attempt >= 0) { "发送 attempt 不能为负数" }
        setUuid("command_id", commandId)
        put("client_message_id", clientMessageId)
        setUuid("conversation_id", conversationId)
        setUuid("project_id", projectId)
        put("provider", provider.wire)
        putNullable("model", model)
        putNullable("effort", effort)
        putNullable("permission_mode", permissionMode)
        put("text", text)
        putAttachments(attachments)
        put("attempt", attempt)
    }

    fun sendMessage(
        commandId: UUID,
        conversationId: UUID,
        clientMessageId: String,
        text: String,
        attachments: List<PromptAttachment>,
        attempt: Int = 0,
    ): ByteArray =
        command("send_message") {
            require(attempt >= 0) { "发送 attempt 不能为负数" }
            setUuid("command_id", commandId)
            setUuid("conversation_id", conversationId)
            put("client_message_id", clientMessageId)
            put("text", text)
            putAttachments(attachments)
            put("attempt", attempt)
        }

    fun withSendAttempt(frame: ByteArray, attempt: Int): ByteArray {
        require(attempt >= 0) { "发送 attempt 不能为负数" }
        val envelope = mapper.readTree(frame) as? ObjectNode ?: error("发送帧不是 CBOR 对象")
        require(envelope.required("protocol_version").asInt() == VERSION) { "发送帧协议版本不匹配" }
        val message = envelope.required("message") as? ObjectNode ?: error("发送消息不是 CBOR 对象")
        require(message.requiredText("type") in setOf("start_conversation", "send_message")) {
            "只有发送消息可以更新 attempt"
        }
        message.put("attempt", attempt)
        return mapper.writeValueAsBytes(envelope)
    }

    fun steer(commandId: UUID, conversationId: UUID, text: String): ByteArray =
        command("steer") {
            setUuid("command_id", commandId)
            setUuid("conversation_id", conversationId)
            put("text", text)
        }

    fun interrupt(commandId: UUID, conversationId: UUID): ByteArray = command("interrupt") {
        setUuid("command_id", commandId)
        setUuid("conversation_id", conversationId)
    }

    fun resolveApproval(commandId: UUID, approvalId: UUID, optionId: String): ByteArray =
        command("resolve_approval") {
            setUuid("command_id", commandId)
            setUuid("approval_id", approvalId)
            put("option_id", optionId)
        }

    fun setSessionOption(
        commandId: UUID,
        conversationId: UUID,
        optionId: String,
        value: String,
    ): ByteArray = command("set_session_option") {
        setUuid("command_id", commandId)
        setUuid("conversation_id", conversationId)
        put("option_id", optionId)
        put("value", value)
    }

    fun getAttachment(attachmentId: UUID): ByteArray = command("get_attachment") {
        setUuid("attachment_id", attachmentId)
    }

    fun renameConversation(commandId: UUID, conversationId: UUID, title: String): ByteArray =
        command("rename_conversation") {
            setUuid("command_id", commandId)
            setUuid("conversation_id", conversationId)
            put("title", title)
        }

    fun getConversationPage(
        conversationId: UUID,
        before: TimelinePageCursor?,
        limit: Int,
    ): ByteArray =
        command("get_conversation_page") {
            setUuid("conversation_id", conversationId)
            if (before == null) {
                putNull("before")
            } else {
                set<ObjectNode>("before", mapper.createObjectNode().apply {
                    put("created_at_ms", before.createdAtMs)
                    setUuid("item_id", before.itemId)
                })
            }
            put("limit", limit)
        }

    fun decodeServer(bytes: ByteArray, target: ConnectionTarget): ServerEvent {
        val envelope = mapper.readTree(bytes)
        val version = envelope.required("protocol_version").asInt()
        require(version == VERSION) { "Host 协议版本为 $version，Android 客户端仅支持 $VERSION" }
        val message = envelope.required("message")
        return when (message.requiredText("type")) {
            "paired" -> {
                val hostId = message.requiredUuid("host_id")
                require(hostId == target.hostId) { "Host ID 与配对链接不一致" }
                ServerEvent.Paired(
                    StoredCredential(
                        hostId = hostId,
                        deviceId = message.requiredUuid("device_id"),
                        deviceToken = message.requiredText("device_token"),
                        origin = target.origin,
                        relay = target.relay,
                        displayName = target.origin,
                    ),
                )
            }
            "authenticated" -> {
                val hostId = message.requiredUuid("host_id")
                val deviceId = message.requiredUuid("device_id")
                require(hostId == target.hostId) { "认证响应属于其他 Host" }
                target.credential?.let { credential ->
                    require(deviceId == credential.deviceId) { "认证响应属于其他设备" }
                }
                ServerEvent.Authenticated(hostId = hostId, deviceId = deviceId)
            }
            "snapshot" -> {
                val snapshot = parseSnapshot(message.required("snapshot"))
                require(snapshot.hostId == target.hostId) { "快照属于其他 Host" }
                ServerEvent.SnapshotReceived(snapshot, bytes.copyOf())
            }
            "projects_updated" -> ServerEvent.ProjectsUpdated(
                provider = ProviderId.fromWire(message.requiredText("provider")),
                projects = message.requiredArray("projects").map(::parseProject),
                capabilities = message.requiredArray("capabilities").map(::parseCapability),
            )
            "project_sync_completed" -> ServerEvent.ProjectSyncCompleted(
                commandId = message.requiredUuid("command_id"),
                projectId = message.requiredUuid("project_id"),
                provider = ProviderId.fromWire(message.requiredText("provider")),
                conversationsSynced = message.required("conversations_synced").asInt(),
                fullHistoryFallback = message.required("full_history_fallback").asBoolean(),
            )
            "conversation_page" -> ServerEvent.ConversationPage(
                conversationId = message.requiredUuid("conversation_id"),
                items = message.requiredArray("items").map(::parseTimelineItem),
                nextBefore = message.get("next_before")?.takeUnless(JsonNode::isNull)?.let {
                    TimelinePageCursor(
                        createdAtMs = it.required("created_at_ms").asLong(),
                        itemId = it.requiredUuid("item_id"),
                    )
                },
            )
            "provider_changed" -> ServerEvent.ProviderChanged(
                parseCapability(message.required("capability")),
            )
            "conversation_upserted" -> ServerEvent.ConversationUpserted(
                parseConversation(message.required("conversation")),
            )
            "timeline_item_upserted" -> ServerEvent.TimelineUpserted(
                parseTimelineItem(message.required("item")),
            )
            "conversation_removed" -> ServerEvent.ConversationRemoved(
                message.requiredUuid("conversation_id"),
            )
            "attachment_data" -> {
                val metadata = message.required("metadata")
                ServerEvent.AttachmentReceived(
                    AttachmentData(
                        id = metadata.requiredUuid("id"),
                        conversationId = metadata.requiredUuid("conversation_id"),
                        mimeType = metadata.requiredText("mime_type"),
                        bytes = message.required("bytes").binaryValue(),
                    ),
                )
            }
            "host_status" -> {
                val hostId = message.requiredUuid("host_id")
                require(hostId == target.hostId) { "Host 状态属于其他 Host" }
                ServerEvent.HostStatus(
                    hostId = hostId,
                    online = message.required("online").asBoolean(),
                    message = message.textOrNull("message"),
                )
            }
            "send_trace" -> ServerEvent.SendTrace(
                commandId = message.requiredUuid("command_id"),
                clientMessageId = message.requiredText("client_message_id"),
                conversationId = message.requiredUuid("conversation_id"),
                stage = message.requiredText("stage"),
                elapsedMs = message.required("elapsed_ms").asLong(),
            )
            "command_accepted" -> ServerEvent.CommandAccepted(
                message.requiredUuid("command_id"),
            )
            "command_rejected" -> ServerEvent.CommandRejected(
                commandId = message.uuidOrNull("command_id"),
                code = message.requiredText("code"),
                message = message.requiredText("message"),
            )
            "protocol_error" -> ServerEvent.ProtocolError(
                supportedVersion = message.required("supported_version").asInt(),
                message = message.requiredText("message"),
            )
            else -> error("未知 Host 消息：${message.requiredText("type")}")
        }
    }

    fun parsePairLink(value: String): ConnectionTarget {
        val uri = java.net.URI(value.trim())
        require(uri.scheme == "http" || uri.scheme == "https") { "配对链接必须使用 http 或 https" }
        require(!uri.rawAuthority.isNullOrBlank()) { "配对链接缺少 Host 地址" }
        val fragment = uri.rawFragment ?: error("配对链接缺少 #host 和 pair 参数")
        val params = fragment.split('&').associate { part ->
            val pair = part.split('=', limit = 2)
            java.net.URLDecoder.decode(pair[0], StandardCharsets.UTF_8.name()) to
                java.net.URLDecoder.decode(pair.getOrElse(1) { "" }, StandardCharsets.UTF_8.name())
        }
        val hostId = UUID.fromString(params["host"] ?: error("配对链接缺少 host"))
        val pairToken = params["pair"]?.takeIf { it.isNotBlank() }
            ?: error("配对链接缺少 pair token")
        return ConnectionTarget(
            hostId = hostId,
            origin = "${uri.scheme}://${uri.rawAuthority}",
            relay = params["relay"] == "1",
            pairToken = pairToken,
        )
    }

    private fun command(type: String, populate: ObjectNode.() -> Unit = {}): ByteArray {
        val message = mapper.createObjectNode().apply {
            put("type", type)
            populate()
        }
        val envelope = mapper.createObjectNode().apply {
            put("protocol_version", VERSION)
            set<ObjectNode>("message", message)
        }
        return mapper.writeValueAsBytes(envelope)
    }

    private fun snapshotNode(snapshot: Snapshot) = mapper.createObjectNode().apply {
        setUuid("host_id", snapshot.hostId)
        put("host_name", snapshot.hostName)
        putArray("projects").apply {
            snapshot.projects.forEach { add(projectNode(it)) }
        }
        putArray("provider_capabilities").apply {
            snapshot.providerCapabilities.forEach { add(capabilityNode(it)) }
        }
        putArray("conversations").apply {
            snapshot.conversations.forEach { add(conversationNode(it)) }
        }
        putArray("timeline").apply {
            snapshot.timeline.forEach { add(timelineItemNode(it)) }
        }
    }

    private fun projectNode(project: ProjectSummary) = mapper.createObjectNode().apply {
        setUuid("id", project.id)
        put("display_name", project.displayName)
        put("short_path", project.shortPath)
        putArray("enabled_providers").apply {
            project.enabledProviders.forEach { add(it.wire) }
        }
        put("valid", project.valid)
        if (project.lastActivityAtMs == null) {
            putNull("last_activity_at_ms")
        } else {
            put("last_activity_at_ms", project.lastActivityAtMs)
        }
        put("conversation_count", project.conversationCount)
    }

    private fun capabilityNode(capability: ProviderCapability) = mapper.createObjectNode().apply {
        put("provider", capability.provider.wire)
        setUuid("project_id", capability.projectId)
        set<ObjectNode>("health", mapper.createObjectNode().apply {
            put("provider", capability.provider.wire)
            put("state", capability.state)
            putNullable("version", capability.version)
            putNullable("detail", capability.detail)
        })
        putArray("models").apply {
            capability.models.forEach { model ->
                addObject().apply {
                    put("id", model.id)
                    put("display_name", model.displayName)
                    putArray("effort_options").apply {
                        model.effortOptions.forEach { effort ->
                            addObject().apply {
                                put("id", effort.id)
                                put("display_name", effort.displayName)
                            }
                        }
                    }
                    putNullable("default_effort", model.defaultEffort)
                }
            }
        }
        put("supports_session_list", capability.supportsSessionList)
        put("supports_history", capability.supportsHistory)
        put("supports_incremental_sync", capability.supportsIncrementalSync)
        put("supports_rename", capability.supportsRename)
        put("supports_steer", capability.supportsSteer)
        putArray("permission_modes").apply {
            capability.permissionModes.forEach { permission ->
                addObject().apply {
                    put("id", permission.id)
                    put("display_name", permission.displayName)
                    put("description", permission.description)
                    put("risk", permission.risk.wire)
                }
            }
        }
        putNullable("default_permission_mode", capability.defaultPermissionMode)
        set<ObjectNode>("attachments", mapper.createObjectNode().apply {
            putArray("allowed_mime_types").apply {
                capability.attachments.allowedMimeTypes.forEach(::add)
            }
            put("max_count", capability.attachments.maxCount)
            put("max_bytes", capability.attachments.maxBytes)
            put("max_total_bytes", capability.attachments.maxTotalBytes)
        })
        putArray("sessions").apply {
            capability.sessions.forEach { session ->
                addObject().apply {
                    put("native_session_id", session.nativeSessionId)
                    put("title", session.title)
                    put("updated_at_ms", session.updatedAtMs)
                }
            }
        }
        putNullable("limitation", capability.limitation)
    }

    private fun conversationNode(conversation: Conversation) = mapper.createObjectNode().apply {
        setUuid("id", conversation.id)
        put("revision", conversation.revision)
        put("provider", conversation.provider.wire)
        setUuid("project_id", conversation.projectId)
        put("native_session_id", conversation.nativeSessionId)
        put("title", conversation.title)
        put("title_source", conversation.titleSource)
        put("title_updated_at_ms", conversation.titleUpdatedAtMs)
        putNullable("selected_model", conversation.selectedModel)
        putNullable("selected_effort", conversation.selectedEffort)
        put("state", conversation.state)
        putArray("session_options").apply {
            conversation.sessionOptions.forEach { option ->
                addObject().apply {
                    put("id", option.id)
                    put("display_name", option.displayName)
                    putNullable("category", option.category)
                    put("current_value", option.currentValue)
                    putArray("values").apply {
                        option.values.forEach { value ->
                            addObject().apply {
                                put("value", value.value)
                                put("display_name", value.displayName)
                            }
                        }
                    }
                }
            }
        }
        put("updated_at_ms", conversation.updatedAtMs)
    }

    private fun timelineItemNode(item: TimelineItem) = mapper.createObjectNode().apply {
        setUuid("id", item.id)
        setUuid("conversation_id", item.conversationId)
        put("revision", item.revision)
        put("created_at_ms", item.createdAtMs)
        set<ObjectNode>("kind", timelineContentNode(item.content))
    }

    private fun timelineContentNode(content: TimelineContent): ObjectNode = mapper.createObjectNode().apply {
        when (content) {
            is TimelineContent.UserMessage -> {
                put("type", "user_message")
                put("text", content.text)
            }
            is TimelineContent.AgentMessage -> {
                put("type", "agent_message")
                put("phase", content.phase)
                put("text", content.text)
            }
            is TimelineContent.Progress -> {
                put("type", "progress")
                if (content.kind in STANDARD_PROGRESS_KINDS) {
                    put("kind", content.kind)
                } else {
                    set<ObjectNode>("kind", mapper.createObjectNode().put("other", content.kind))
                }
                put("label", content.label)
                put("status", content.status)
                putNullable("detail", content.detail)
            }
            is TimelineContent.Plan -> {
                put("type", "plan")
                putArray("steps").apply {
                    content.steps.forEach { step ->
                        addObject().apply {
                            put("text", step.text)
                            put("status", step.status)
                        }
                    }
                }
            }
            is TimelineContent.ToolCall -> {
                put("type", "tool_call")
                put("name", content.name)
                put("status", content.status)
                putNullable("input_summary", content.inputSummary)
                putNullable("output_summary", content.outputSummary)
            }
            is TimelineContent.Command -> {
                put("type", "command")
                put("command", content.command)
                putNullable("relative_cwd", content.relativeCwd)
                put("status", content.status)
                if (content.exitCode == null) putNull("exit_code") else put("exit_code", content.exitCode)
                putNullable("output", content.output)
            }
            is TimelineContent.FileChange -> {
                put("type", "file_change")
                put("relative_path", content.relativePath)
                put("change_kind", content.changeKind)
                put("status", content.status)
            }
            is TimelineContent.Approval -> {
                put("type", "approval")
                setUuid("approval_id", content.approvalId)
                put("prompt", content.prompt)
                putArray("options").apply {
                    content.options.forEach { option ->
                        addObject().apply {
                            put("id", option.id)
                            put("label", option.label)
                        }
                    }
                }
                putNullable("resolved_option", content.resolvedOption)
            }
            is TimelineContent.Image -> {
                put("type", "image")
                setUuid("attachment_id", content.attachmentId)
                put("alt", content.alt)
            }
            is TimelineContent.Error -> {
                put("type", "error")
                put("code", content.code)
                put("message", content.message)
            }
        }
    }

    private fun parseSnapshot(node: JsonNode) = Snapshot(
        hostId = node.requiredUuid("host_id"),
        hostName = node.requiredText("host_name"),
        projects = node.requiredArray("projects").map(::parseProject),
        providerCapabilities = node.requiredArray("provider_capabilities").map(::parseCapability),
        conversations = node.requiredArray("conversations").map(::parseConversation),
        timeline = node.requiredArray("timeline").map(::parseTimelineItem),
    )

    private fun parseProject(node: JsonNode) = ProjectSummary(
        id = node.requiredUuid("id"),
        displayName = node.requiredText("display_name"),
        shortPath = node.requiredText("short_path"),
        enabledProviders = node.requiredArray("enabled_providers").map { ProviderId.fromWire(it.asText()) },
        valid = node.required("valid").asBoolean(),
        lastActivityAtMs = node.get("last_activity_at_ms")?.takeUnless(JsonNode::isNull)?.asLong(),
        conversationCount = node.required("conversation_count").asInt(),
    )

    private fun parseCapability(node: JsonNode): ProviderCapability {
        val health = node.required("health")
        return ProviderCapability(
            provider = ProviderId.fromWire(node.requiredText("provider")),
            projectId = node.requiredUuid("project_id"),
            state = health.requiredText("state"),
            version = health.textOrNull("version"),
            detail = health.textOrNull("detail"),
            models = node.requiredArray("models").map(::parseModel),
            supportsSessionList = node.required("supports_session_list").asBoolean(),
            supportsHistory = node.required("supports_history").asBoolean(),
            supportsIncrementalSync = node.required("supports_incremental_sync").asBoolean(),
            supportsRename = node.required("supports_rename").asBoolean(),
            supportsSteer = node.required("supports_steer").asBoolean(),
            permissionModes = node.requiredArray("permission_modes").map {
                PermissionModeOption(
                    id = it.requiredText("id"),
                    displayName = it.requiredText("display_name"),
                    description = it.requiredText("description"),
                    risk = PermissionRisk.fromWire(it.requiredText("risk")),
                )
            },
            defaultPermissionMode = node.textOrNull("default_permission_mode"),
            attachments = node.required("attachments").let {
                AttachmentCapability(
                    allowedMimeTypes = it.requiredArray("allowed_mime_types").map(JsonNode::asText),
                    maxCount = it.required("max_count").asInt(),
                    maxBytes = it.required("max_bytes").asLong(),
                    maxTotalBytes = it.required("max_total_bytes").asLong(),
                )
            },
            sessions = node.requiredArray("sessions").map {
                SessionSummary(
                    nativeSessionId = it.requiredText("native_session_id"),
                    title = it.requiredText("title"),
                    updatedAtMs = it.required("updated_at_ms").asLong(),
                )
            },
            limitation = node.textOrNull("limitation"),
        )
    }

    private fun parseModel(node: JsonNode) = ModelOption(
        id = node.requiredText("id"),
        displayName = node.requiredText("display_name"),
        effortOptions = node.requiredArray("effort_options").map {
            EffortOption(it.requiredText("id"), it.requiredText("display_name"))
        },
        defaultEffort = node.textOrNull("default_effort"),
    )

    private fun parseConversation(node: JsonNode) = Conversation(
        id = node.requiredUuid("id"),
        revision = node.required("revision").asLong(),
        provider = ProviderId.fromWire(node.requiredText("provider")),
        projectId = node.requiredUuid("project_id"),
        nativeSessionId = node.requiredText("native_session_id"),
        title = node.requiredText("title"),
        titleSource = node.requiredText("title_source"),
        titleUpdatedAtMs = node.required("title_updated_at_ms").asLong(),
        selectedModel = node.textOrNull("selected_model"),
        selectedEffort = node.textOrNull("selected_effort"),
        state = node.requiredText("state"),
        sessionOptions = node.requiredArray("session_options").map(::parseSessionOption),
        updatedAtMs = node.required("updated_at_ms").asLong(),
    )

    private fun parseSessionOption(node: JsonNode) = SessionOption(
        id = node.requiredText("id"),
        displayName = node.requiredText("display_name"),
        category = node.textOrNull("category"),
        currentValue = node.requiredText("current_value"),
        values = node.requiredArray("values").map {
            SessionOptionValue(it.requiredText("value"), it.requiredText("display_name"))
        },
    )

    private fun parseTimelineItem(node: JsonNode) = TimelineItem(
        id = node.requiredUuid("id"),
        conversationId = node.requiredUuid("conversation_id"),
        revision = node.required("revision").asLong(),
        createdAtMs = node.required("created_at_ms").asLong(),
        content = parseTimelineContent(node.required("kind")),
    )

    private fun parseTimelineContent(node: JsonNode): TimelineContent = when (node.requiredText("type")) {
        "user_message" -> TimelineContent.UserMessage(node.requiredText("text"))
        "agent_message" -> TimelineContent.AgentMessage(
            phase = node.requiredText("phase"),
            text = node.requiredText("text"),
        )
        "progress" -> TimelineContent.Progress(
            kind = progressKind(node.required("kind")),
            label = node.requiredText("label"),
            status = node.requiredText("status"),
            detail = node.textOrNull("detail"),
        )
        "plan" -> TimelineContent.Plan(
            node.requiredArray("steps").map {
                PlanStep(it.requiredText("text"), it.requiredText("status"))
            },
        )
        "tool_call" -> TimelineContent.ToolCall(
            name = node.requiredText("name"),
            status = node.requiredText("status"),
            inputSummary = node.textOrNull("input_summary"),
            outputSummary = node.textOrNull("output_summary"),
        )
        "command" -> TimelineContent.Command(
            command = node.requiredText("command"),
            relativeCwd = node.textOrNull("relative_cwd"),
            status = node.requiredText("status"),
            exitCode = node.get("exit_code")?.takeUnless(JsonNode::isNull)?.asInt(),
            output = node.textOrNull("output"),
        )
        "file_change" -> TimelineContent.FileChange(
            relativePath = node.requiredText("relative_path"),
            changeKind = node.requiredText("change_kind"),
            status = node.requiredText("status"),
        )
        "approval" -> TimelineContent.Approval(
            approvalId = node.requiredUuid("approval_id"),
            prompt = node.requiredText("prompt"),
            options = node.requiredArray("options").map {
                ApprovalOption(it.requiredText("id"), it.requiredText("label"))
            },
            resolvedOption = node.textOrNull("resolved_option"),
        )
        "image" -> TimelineContent.Image(
            attachmentId = node.requiredUuid("attachment_id"),
            alt = node.requiredText("alt"),
        )
        "error" -> TimelineContent.Error(
            code = node.requiredText("code"),
            message = node.requiredText("message"),
        )
        else -> error("未知时间线类型：${node.requiredText("type")}")
    }

    private fun progressKind(node: JsonNode): String = when {
        node.isTextual -> node.asText()
        node.has("other") -> node.requiredText("other")
        else -> "progress"
    }

    private fun ObjectNode.setUuid(name: String, value: UUID) {
        val bytes = ByteBuffer.allocate(16).putLong(value.mostSignificantBits).putLong(value.leastSignificantBits).array()
        set<JsonNode>(name, mapper.nodeFactory.binaryNode(bytes))
    }

    private fun ObjectNode.putNullable(name: String, value: String?) {
        if (value == null) putNull(name) else put(name, value)
    }

    private fun ObjectNode.putAttachments(attachments: List<PromptAttachment>) {
        val values = putArray("attachments")
        attachments.forEach { attachment ->
            values.addObject().apply {
                setUuid("id", attachment.id)
                put("file_name", attachment.fileName)
                put("mime_type", attachment.mimeType)
                set<JsonNode>("bytes", mapper.nodeFactory.binaryNode(attachment.bytes))
            }
        }
    }

    private fun JsonNode.required(name: String): JsonNode =
        get(name) ?: error("Host 消息缺少字段 $name")

    private fun JsonNode.requiredText(name: String): String = required(name).asText()

    private fun JsonNode.requiredArray(name: String): ArrayNode =
        required(name) as? ArrayNode ?: error("Host 字段 $name 不是数组")

    private fun JsonNode.textOrNull(name: String): String? =
        get(name)?.takeUnless(JsonNode::isNull)?.asText()

    private fun JsonNode.requiredUuid(name: String): UUID = parseUuid(required(name))

    private fun JsonNode.uuidOrNull(name: String): UUID? =
        get(name)?.takeUnless(JsonNode::isNull)?.let(::parseUuid)

    private fun parseUuid(node: JsonNode): UUID {
        if (node.isTextual) return UUID.fromString(node.asText())
        val bytes = node.binaryValue()
        require(bytes.size == 16) { "UUID 必须是 16-byte CBOR byte string" }
        val buffer = ByteBuffer.wrap(bytes)
        return UUID(buffer.long, buffer.long)
    }

    private val STANDARD_PROGRESS_KINDS = setOf("command", "tool", "web_search", "test", "file")
}
