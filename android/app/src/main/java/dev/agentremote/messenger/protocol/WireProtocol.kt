package dev.agentremote.messenger.protocol

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.databind.node.ArrayNode
import com.fasterxml.jackson.databind.node.ObjectNode
import com.fasterxml.jackson.dataformat.cbor.CBORFactory
import dev.agentremote.messenger.model.ApprovalOption
import dev.agentremote.messenger.model.AttachmentData
import dev.agentremote.messenger.model.ConnectionTarget
import dev.agentremote.messenger.model.Conversation
import dev.agentremote.messenger.model.EffortOption
import dev.agentremote.messenger.model.ModelOption
import dev.agentremote.messenger.model.PlanStep
import dev.agentremote.messenger.model.ProjectSummary
import dev.agentremote.messenger.model.ProviderCapability
import dev.agentremote.messenger.model.ProviderId
import dev.agentremote.messenger.model.ServerEvent
import dev.agentremote.messenger.model.SessionOption
import dev.agentremote.messenger.model.SessionOptionValue
import dev.agentremote.messenger.model.SessionSummary
import dev.agentremote.messenger.model.Snapshot
import dev.agentremote.messenger.model.StoredCredential
import dev.agentremote.messenger.model.TimelineContent
import dev.agentremote.messenger.model.TimelineItem
import java.nio.ByteBuffer
import java.nio.charset.StandardCharsets
import java.util.UUID

object WireProtocol {
    const val VERSION = 1
    const val SUBPROTOCOL = "agent-remote.cbor.v1"
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

    fun sendMessage(commandId: UUID, conversationId: UUID, text: String): ByteArray =
        command("send_message") {
            setUuid("command_id", commandId)
            setUuid("conversation_id", conversationId)
            put("text", text)
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
            "authenticated" -> ServerEvent.Authenticated(
                hostId = message.requiredUuid("host_id"),
                deviceId = message.requiredUuid("device_id"),
            )
            "snapshot" -> ServerEvent.SnapshotReceived(parseSnapshot(message.required("snapshot")))
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
            "host_status" -> ServerEvent.HostStatus(
                hostId = message.requiredUuid("host_id"),
                online = message.required("online").asBoolean(),
                message = message.textOrNull("message"),
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
        enabledProviders = node.requiredArray("enabled_providers").map { ProviderId.fromWire(it.asText()) },
        valid = node.required("valid").asBoolean(),
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
            supportsSteer = node.required("supports_steer").asBoolean(),
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
        else -> node.toString()
    }

    private fun ObjectNode.setUuid(name: String, value: UUID) {
        val bytes = ByteBuffer.allocate(16).putLong(value.mostSignificantBits).putLong(value.leastSignificantBits).array()
        set<JsonNode>(name, mapper.nodeFactory.binaryNode(bytes))
    }

    private fun ObjectNode.putNullable(name: String, value: String?) {
        if (value == null) putNull(name) else put(name, value)
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
}
