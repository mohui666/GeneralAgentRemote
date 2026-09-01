package dev.agentremote.messenger.protocol

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.dataformat.cbor.CBORFactory
import dev.agentremote.messenger.model.ConnectionTarget
import dev.agentremote.messenger.model.ServerEvent
import dev.agentremote.messenger.model.StoredCredential
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

    private fun uuidBytes(uuid: UUID): ByteArray = ByteBuffer.allocate(16)
        .putLong(uuid.mostSignificantBits)
        .putLong(uuid.leastSignificantBits)
        .array()
}
