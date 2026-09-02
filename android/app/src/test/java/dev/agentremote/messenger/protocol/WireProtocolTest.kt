package dev.agentremote.messenger.protocol

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.dataformat.cbor.CBORFactory
import dev.agentremote.messenger.model.ConnectionTarget
import dev.agentremote.messenger.model.PromptAttachment
import dev.agentremote.messenger.model.ProviderId
import dev.agentremote.messenger.model.ServerEvent
import dev.agentremote.messenger.model.StoredCredential
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
        assertEquals(1, root["protocol_version"].asInt())
        assertEquals("authenticate", root["message"]["type"].asText())
        assertArrayEquals(uuidBytes(hostId), root["message"]["host_id"].binaryValue())
        assertArrayEquals(uuidBytes(deviceId), root["message"]["device_id"].binaryValue())
    }

    @Test
    fun decodesHostStatusBeforeRelayAuthentication() {
        val hostId = UUID.fromString("00112233-4455-6677-8899-aabbccddeeff")
        val root = mapper.createObjectNode().apply {
            put("protocol_version", 1)
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

    @Test
    fun encodesLazyStartWithProviderSelectionAndAttachmentBytes() {
        val commandId = UUID.fromString("11111111-2222-3333-4444-555555555555")
        val conversationId = UUID.fromString("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
        val projectId = UUID.fromString("01234567-89ab-cdef-0123-456789abcdef")
        val attachmentId = UUID.fromString("99999999-8888-7777-6666-555555555555")
        val content = byteArrayOf(1, 3, 5, 7)
        val root: JsonNode = mapper.readTree(
            WireProtocol.startConversation(
                commandId = commandId,
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
        assertArrayEquals(uuidBytes(conversationId), message["conversation_id"].binaryValue())
        assertArrayEquals(uuidBytes(projectId), message["project_id"].binaryValue())
        assertEquals("codex", message["provider"].asText())
        assertEquals("workspace-write", message["permission_mode"].asText())
        assertArrayEquals(content, message["attachments"][0]["bytes"].binaryValue())
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

    private fun uuidBytes(uuid: UUID): ByteArray = ByteBuffer.allocate(16)
        .putLong(uuid.mostSignificantBits)
        .putLong(uuid.leastSignificantBits)
        .array()
}
