package dev.agentremote.messenger.data

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.dataformat.cbor.CBORFactory
import dev.agentremote.messenger.model.ConnectionTarget
import dev.agentremote.messenger.model.ServerEvent
import dev.agentremote.messenger.model.StoredCredential
import java.io.IOException
import java.nio.ByteBuffer
import java.util.UUID
import okhttp3.Protocol
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RemoteClientTest {
    @Test
    fun manualDisconnectStopsAutomaticRetryButAllowsImmediateRetry() {
        val sockets = mutableListOf<FakeWebSocket>()
        val client = RemoteClient(
            listener = NoOpListener,
            socketOpener = WebSocketOpener { request, _ ->
                FakeWebSocket(request).also(sockets::add)
            },
        )

        try {
            client.connect(target())
            assertEquals(1, sockets.size)

            client.disconnect()
            client.networkAvailable()
            assertEquals(1, sockets.size)

            client.retryNow()
            assertEquals(2, sockets.size)
        } finally {
            client.close()
        }
    }

    @Test
    fun staleSocketFailureCannotClearAnImmediateReconnect() {
        val opened = mutableListOf<OpenedSocket>()
        val client = RemoteClient(
            listener = NoOpListener,
            socketOpener = WebSocketOpener { request, listener ->
                val socket = FakeWebSocket(request)
                opened += OpenedSocket(socket, listener)
                socket
            },
        )

        try {
            client.connect(target())
            client.disconnect()
            client.retryNow()
            assertEquals(2, opened.size)

            opened.first().listener.onFailure(
                opened.first().socket,
                IOException("stale connection failed"),
                null,
            )

            authenticate(opened.last(), target())
            opened.last().socket.binarySendCount = 0
            assertTrue(client.send(byteArrayOf(1)))
            assertEquals(1, opened.last().socket.binarySendCount)
        } finally {
            client.close()
        }
    }

    @Test
    fun applicationFramesRequireAuthenticationForCurrentGeneration() {
        val opened = mutableListOf<OpenedSocket>()
        val target = target()
        val client = RemoteClient(
            listener = NoOpListener,
            socketOpener = WebSocketOpener { request, listener ->
                val socket = FakeWebSocket(request)
                opened += OpenedSocket(socket, listener)
                socket
            },
        )

        try {
            client.connect(target)
            assertFalse(client.send(byteArrayOf(1)))

            authenticate(opened.single(), target)
            assertTrue(client.send(byteArrayOf(1)))

            client.retryNow()
            authenticate(opened.first(), target)
            assertFalse(client.send(byteArrayOf(1)))

            authenticate(opened.last(), target)
            assertTrue(client.send(byteArrayOf(1)))
        } finally {
            client.close()
        }
    }

    @Test
    fun websocketSendFalseIsReturnedToCaller() {
        val opened = mutableListOf<OpenedSocket>()
        val target = target()
        val client = RemoteClient(
            listener = NoOpListener,
            socketOpener = WebSocketOpener { request, listener ->
                val socket = FakeWebSocket(request)
                opened += OpenedSocket(socket, listener)
                socket
            },
        )

        try {
            client.connect(target)
            authenticate(opened.single(), target)
            opened.single().socket.sendResult = false

            assertFalse(client.send(byteArrayOf(1)))
            opened.single().socket.sendResult = true
            assertFalse(client.send(byteArrayOf(1)))
        } finally {
            client.close()
        }
    }

    @Test
    fun targetCredentialRemainsRecognizableAcrossReconnectGenerations() {
        val client = RemoteClient(
            listener = NoOpListener,
            socketOpener = WebSocketOpener { request, _ -> FakeWebSocket(request) },
        )
        val target = target()
        val credential = requireNotNull(target.credential)

        try {
            client.connect(target)
            client.retryNow()
            assertTrue(client.targets(credential))

            client.forgetTarget()
            assertEquals(false, client.targets(credential))
        } finally {
            client.close()
        }
    }

    private fun target(): ConnectionTarget {
        val hostId = UUID.fromString("11111111-1111-1111-1111-111111111111")
        return ConnectionTarget(
            hostId = hostId,
            origin = "https://relay.example",
            relay = true,
            credential = StoredCredential(
                hostId = hostId,
                deviceId = UUID.fromString("22222222-2222-2222-2222-222222222222"),
                deviceToken = "test-token",
                origin = "https://relay.example",
                relay = true,
                displayName = "Test Host",
            ),
        )
    }

    private data class OpenedSocket(
        val socket: FakeWebSocket,
        val listener: WebSocketListener,
    )

    private fun authenticate(opened: OpenedSocket, target: ConnectionTarget) {
        opened.listener.onOpen(
            opened.socket,
            Response.Builder()
                .request(opened.socket.request())
                .protocol(Protocol.HTTP_1_1)
                .code(101)
                .message("Switching Protocols")
                .header("Sec-WebSocket-Protocol", "agent-remote.cbor.v4")
                .build(),
        )
        val credential = requireNotNull(target.credential)
        val mapper = ObjectMapper(CBORFactory())
        val envelope = mapper.createObjectNode().apply {
            put("protocol_version", 4)
            set<JsonNode>("message", mapper.createObjectNode().apply {
                put("type", "authenticated")
                set<JsonNode>("host_id", mapper.nodeFactory.binaryNode(uuidBytes(credential.hostId)))
                set<JsonNode>("device_id", mapper.nodeFactory.binaryNode(uuidBytes(credential.deviceId)))
            })
        }
        opened.listener.onMessage(opened.socket, mapper.writeValueAsBytes(envelope).toByteString())
    }

    private fun uuidBytes(uuid: UUID): ByteArray = ByteBuffer.allocate(16)
        .putLong(uuid.mostSignificantBits)
        .putLong(uuid.leastSignificantBits)
        .array()

    private class FakeWebSocket(private val request: Request) : WebSocket {
        var binarySendCount = 0
        var sendResult = true

        override fun request(): Request = request

        override fun queueSize(): Long = 0

        override fun send(text: String): Boolean = true

        override fun send(bytes: ByteString): Boolean {
            binarySendCount += 1
            return sendResult
        }

        override fun close(code: Int, reason: String?): Boolean = true

        override fun cancel() = Unit
    }

    private data object NoOpListener : RemoteClient.Listener {
        override fun onConnecting(target: ConnectionTarget) = Unit

        override fun onConnected(target: ConnectionTarget) = Unit

        override fun onEvent(event: ServerEvent, connectionGeneration: Long) = Unit

        override fun onDisconnected(message: String) = Unit

        override fun onRetryScheduled(attempt: Int, delayMillis: Long) = Unit

        override fun onRetryStopped(message: String) = Unit

        override fun onError(message: String) = Unit
    }
}
