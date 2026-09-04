package dev.agentremote.messenger.protocol

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.dataformat.cbor.CBORFactory
import dev.agentremote.messenger.model.ApprovalOption
import dev.agentremote.messenger.model.AttachmentCapability
import dev.agentremote.messenger.model.ConnectionTarget
import dev.agentremote.messenger.model.Conversation
import dev.agentremote.messenger.model.EffortOption
import dev.agentremote.messenger.model.ModelOption
import dev.agentremote.messenger.model.PermissionModeOption
import dev.agentremote.messenger.model.PermissionRisk
import dev.agentremote.messenger.model.PlanStep
import dev.agentremote.messenger.model.PromptAttachment
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
import dev.agentremote.messenger.model.TimelinePageCursor
import java.nio.ByteBuffer
import java.util.UUID
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class WireProtocolTest {
    private val mapper = ObjectMapper(CBORFactory())

    @Test
    fun parsesDirectAndRelayPairLinks() {
        val hostId = UUID.fromString("00112233-4455-6677-8899-aabbccddeeff")
        val direct = WireProtocol.parsePairLink(
            "http://192.168.1.25:7437/#host=$hostId&pair=abc_DEF-123",
        )
        assertEquals(hostId, direct.hostId)
        assertEquals("http://192.168.1.25:7437", direct.origin)
        assertEquals("ws://192.168.1.25:7437/ws", direct.webSocketUrl)
        assertFalse(direct.relay)

        val relay = WireProtocol.parsePairLink(
            "https://relay.example.com/#host=$hostId&pair=token&relay=1",
        )
        assertEquals("wss://relay.example.com/client/$hostId", relay.webSocketUrl)
        assertEquals(true, relay.relay)
    }

    @Test
    fun encodesUuidIdsAsSixteenByteCborStrings() {
        val hostId = UUID.fromString("00112233-4455-6677-8899-aabbccddeeff")
        val deviceId = UUID.fromString("ffeeddcc-bbaa-9988-7766-554433221100")
        val bytes = WireProtocol.authenticate(
            StoredCredential(hostId, deviceId, "secret", "https://host", false, "Host"),
        )
        val root: JsonNode = mapper.readTree(bytes)
        assertEquals(2, root["protocol_version"].asInt())
        assertEquals("authenticate", root["message"]["type"].asText())
        assertArrayEquals(uuidBytes(hostId), root["message"]["host_id"].binaryValue())
        assertArrayEquals(uuidBytes(deviceId), root["message"]["device_id"].binaryValue())
    }

    @Test
    fun decodesHostStatusBeforeRelayAuthentication() {
        val hostId = UUID.fromString("00112233-4455-6677-8899-aabbccddeeff")
        val root = mapper.createObjectNode().apply {
            put("protocol_version", 2)
            set<JsonNode>("message", mapper.createObjectNode().apply {
                put("type", "host_status")
                set<JsonNode>("host_id", mapper.nodeFactory.binaryNode(uuidBytes(hostId)))
                put("online", true)
                putNull("message")
            })
        }
        val event = WireProtocol.decodeServer(
            mapper.writeValueAsBytes(root),
            ConnectionTarget(hostId, "https://relay.example.com", relay = true, pairToken = "token"),
        )
        val status = event as ServerEvent.HostStatus
        assertEquals(hostId, status.hostId)
        assertEquals(true, status.online)
    }

    @Test(expected = IllegalArgumentException::class)
    fun rejectsHostScopedEventsFromAnotherHost() {
        val targetHostId = uuid(1)
        val otherHostId = uuid(2)
        val root = mapper.createObjectNode().apply {
            put("protocol_version", 2)
            set<JsonNode>("message", mapper.createObjectNode().apply {
                put("type", "host_status")
                set<JsonNode>("host_id", mapper.nodeFactory.binaryNode(uuidBytes(otherHostId)))
                put("online", true)
                putNull("message")
            })
        }

        WireProtocol.decodeServer(
            mapper.writeValueAsBytes(root),
            ConnectionTarget(targetHostId, "https://relay.example.com", relay = true, pairToken = "token"),
        )
    }

    @Test
    fun decodesStructuredSendTraceWithoutMessageContent() {
        val hostId = uuid(1)
        val commandId = uuid(2)
        val conversationId = uuid(3)
        val root = mapper.createObjectNode().apply {
            put("protocol_version", 2)
            set<JsonNode>("message", mapper.createObjectNode().apply {
                put("type", "send_trace")
                set<JsonNode>("command_id", mapper.nodeFactory.binaryNode(uuidBytes(commandId)))
                put("client_message_id", "client-message-1")
                set<JsonNode>("conversation_id", mapper.nodeFactory.binaryNode(uuidBytes(conversationId)))
                put("stage", "provider_received")
                put("elapsed_ms", 17L)
            })
        }

        val event = WireProtocol.decodeServer(
            mapper.writeValueAsBytes(root),
            ConnectionTarget(hostId, "https://relay.example.com", relay = true, pairToken = "token"),
        ) as ServerEvent.SendTrace

        assertEquals(commandId, event.commandId)
        assertEquals("client-message-1", event.clientMessageId)
        assertEquals(conversationId, event.conversationId)
        assertEquals("provider_received", event.stage)
        assertEquals(17L, event.elapsedMs)
    }

    @Test
    fun encodesLazyStartWithProviderSelectionAndAttachmentBytes() {
        val commandId = UUID.fromString("11111111-2222-3333-4444-555555555555")
        val conversationId = UUID.fromString("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
        val projectId = UUID.fromString("01234567-89ab-cdef-0123-456789abcdef")
        val attachmentId = UUID.fromString("99999999-8888-7777-6666-555555555555")
        val clientMessageId = "client-message-1"
        val content = byteArrayOf(1, 3, 5, 7)
        val root: JsonNode = mapper.readTree(
            WireProtocol.startConversation(
                commandId = commandId,
                clientMessageId = clientMessageId,
                conversationId = conversationId,
                projectId = projectId,
                provider = ProviderId.CODEX,
                model = "gpt-test",
                effort = "high",
                permissionMode = "workspace-write",
                text = "hello",
                attachments = listOf(
                    PromptAttachment(attachmentId, "reference.png", "image/png", content),
                ),
            ),
        )
        val message = root["message"]
        assertEquals("start_conversation", message["type"].asText())
        assertEquals(clientMessageId, message["client_message_id"].asText())
        assertArrayEquals(uuidBytes(conversationId), message["conversation_id"].binaryValue())
        assertArrayEquals(uuidBytes(projectId), message["project_id"].binaryValue())
        assertEquals("codex", message["provider"].asText())
        assertEquals("workspace-write", message["permission_mode"].asText())
        assertEquals(0, message["attempt"].asInt())
        assertArrayEquals(content, message["attachments"][0]["bytes"].binaryValue())
    }

    @Test
    fun userRetryOnlyChangesAttemptAndKeepsSendIdentityAndPayload() {
        val commandId = uuid(21)
        val conversationId = uuid(22)
        val attachmentId = uuid(23)
        val content = byteArrayOf(2, 4, 6)
        val initial = WireProtocol.sendMessage(
            commandId = commandId,
            conversationId = conversationId,
            clientMessageId = "stable-client-message",
            text = "retry payload",
            attachments = listOf(
                PromptAttachment(attachmentId, "retry.png", "image/png", content),
            ),
        )

        val retried = WireProtocol.withSendAttempt(initial, 1)
        val originalMessage = mapper.readTree(initial)["message"]
        val retriedMessage = mapper.readTree(retried)["message"]

        assertEquals(0, originalMessage["attempt"].asInt())
        assertEquals(1, retriedMessage["attempt"].asInt())
        assertArrayEquals(originalMessage["command_id"].binaryValue(), retriedMessage["command_id"].binaryValue())
        assertEquals(originalMessage["client_message_id"].asText(), retriedMessage["client_message_id"].asText())
        assertArrayEquals(originalMessage["conversation_id"].binaryValue(), retriedMessage["conversation_id"].binaryValue())
        assertEquals("retry payload", retriedMessage["text"].asText())
        assertArrayEquals(content, retriedMessage["attachments"][0]["bytes"].binaryValue())
    }

    @Test
    fun encodesStableTimelinePageCursor() {
        val conversationId = UUID.fromString("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
        val itemId = UUID.fromString("00000000-1111-2222-3333-444444444444")
        val root: JsonNode = mapper.readTree(
            WireProtocol.getConversationPage(
                conversationId,
                TimelinePageCursor(createdAtMs = 1234L, itemId = itemId),
                100,
            ),
        )
        val message = root["message"]
        assertEquals("get_conversation_page", message["type"].asText())
        assertEquals(1234L, message["before"]["created_at_ms"].asLong())
        assertArrayEquals(uuidBytes(itemId), message["before"]["item_id"].binaryValue())
        assertEquals(100, message["limit"].asInt())
    }

    @Test
    fun snapshotEncodingRoundTripsEveryFieldAndTimelineVariant() {
        val hostId = uuid(1)
        val projectId = uuid(2)
        val conversationId = uuid(3)
        var itemIndex = 10
        fun item(content: TimelineContent) = TimelineItem(
            id = uuid(itemIndex++),
            conversationId = conversationId,
            revision = itemIndex.toLong(),
            createdAtMs = 1_000L + itemIndex,
            content = content,
        )
        val snapshot = Snapshot(
            hostId = hostId,
            hostName = "Test Host",
            projects = listOf(
                ProjectSummary(
                    id = projectId,
                    displayName = "GeneralAgentRemote",
                    shortPath = "~/GeneralAgentRemote",
                    enabledProviders = listOf(ProviderId.CODEX, ProviderId.GROK),
                    valid = true,
                    lastActivityAtMs = 9_001L,
                    conversationCount = 1,
                ),
                ProjectSummary(
                    id = uuid(4),
                    displayName = "Unavailable",
                    shortPath = "~/Unavailable",
                    enabledProviders = emptyList(),
                    valid = false,
                    lastActivityAtMs = null,
                    conversationCount = 0,
                ),
            ),
            providerCapabilities = listOf(
                ProviderCapability(
                    provider = ProviderId.CODEX,
                    projectId = projectId,
                    state = "ready",
                    version = "1.2.3",
                    detail = "connected",
                    models = listOf(
                        ModelOption(
                            id = "gpt-test",
                            displayName = "GPT Test",
                            effortOptions = listOf(
                                EffortOption("low", "Low"),
                                EffortOption("high", "High"),
                            ),
                            defaultEffort = "high",
                        ),
                        ModelOption(
                            id = "gpt-no-effort",
                            displayName = "GPT No Effort",
                            effortOptions = emptyList(),
                            defaultEffort = null,
                        ),
                    ),
                    supportsSessionList = true,
                    supportsHistory = true,
                    supportsIncrementalSync = true,
                    supportsRename = true,
                    supportsSteer = false,
                    permissionModes = listOf(
                        PermissionModeOption(
                            id = "ask",
                            displayName = "Ask",
                            description = "Ask before elevated actions",
                            risk = PermissionRisk.STANDARD,
                        ),
                        PermissionModeOption(
                            id = "full",
                            displayName = "Full",
                            description = "Allow elevated actions",
                            risk = PermissionRisk.ELEVATED,
                        ),
                    ),
                    defaultPermissionMode = "ask",
                    attachments = AttachmentCapability(
                        allowedMimeTypes = listOf("image/png", "text/plain"),
                        maxCount = 4,
                        maxBytes = 5_000_000,
                        maxTotalBytes = 10_000_000,
                    ),
                    sessions = listOf(SessionSummary("native-1", "Existing", 8_001L)),
                    limitation = "No mid-turn model switch",
                ),
            ),
            conversations = listOf(
                Conversation(
                    id = conversationId,
                    revision = 7,
                    provider = ProviderId.CODEX,
                    projectId = projectId,
                    nativeSessionId = "native-1",
                    title = "Round trip",
                    titleSource = "user",
                    titleUpdatedAtMs = 7_001L,
                    selectedModel = "gpt-test",
                    selectedEffort = "high",
                    state = "needs_approval",
                    sessionOptions = listOf(
                        SessionOption(
                            id = "permission",
                            displayName = "Permission",
                            category = "security",
                            currentValue = "ask",
                            values = listOf(
                                SessionOptionValue("ask", "Ask"),
                                SessionOptionValue("full", "Full"),
                            ),
                        ),
                        SessionOption(
                            id = "mode",
                            displayName = "Mode",
                            category = null,
                            currentValue = "chat",
                            values = emptyList(),
                        ),
                    ),
                    updatedAtMs = 9_001L,
                ),
            ),
            timeline = listOf(
                item(TimelineContent.UserMessage("hello")),
                item(TimelineContent.AgentMessage("reasoning_summary", "summary")),
                item(TimelineContent.Progress("test", "Tests", "completed", null)),
                item(TimelineContent.Progress("custom_progress", "Custom", "running", "details")),
                item(TimelineContent.Plan(listOf(PlanStep("Inspect", "completed")))),
                item(TimelineContent.ToolCall("read_file", "completed", "input", "output")),
                item(TimelineContent.Command("cargo test", "crates/host", "completed", 0, "ok")),
                item(TimelineContent.FileChange("src/main.rs", "modified", "completed")),
                item(
                    TimelineContent.Approval(
                        approvalId = uuid(5),
                        prompt = "Allow?",
                        options = listOf(ApprovalOption("yes", "Allow"), ApprovalOption("no", "Deny")),
                        resolvedOption = "yes",
                    ),
                ),
                item(TimelineContent.Image(uuid(6), "preview")),
                item(TimelineContent.Error("provider_error", "failed")),
            ),
        )

        val encoded = WireProtocol.encodeSnapshot(snapshot)
        val root: JsonNode = mapper.readTree(encoded)
        val decoded = WireProtocol.decodeServer(
            encoded,
            ConnectionTarget(hostId, "https://relay.example.com", relay = true, pairToken = "token"),
        ) as ServerEvent.SnapshotReceived

        assertEquals("snapshot", root["message"]["type"].asText())
        assertEquals("test", root["message"]["snapshot"]["timeline"][2]["kind"]["kind"].asText())
        assertEquals(
            "custom_progress",
            root["message"]["snapshot"]["timeline"][3]["kind"]["kind"]["other"].asText(),
        )
        assertEquals(snapshot, decoded.snapshot)
    }

    private fun uuidBytes(uuid: UUID): ByteArray = ByteBuffer.allocate(16)
        .putLong(uuid.mostSignificantBits)
        .putLong(uuid.leastSignificantBits)
        .array()

    private fun uuid(value: Int): UUID = UUID(0, value.toLong())
}
