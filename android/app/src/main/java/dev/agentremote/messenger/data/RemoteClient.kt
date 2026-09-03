package dev.agentremote.messenger.data

import android.os.Build
import dev.agentremote.messenger.model.ConnectionTarget
import dev.agentremote.messenger.model.ServerEvent
import dev.agentremote.messenger.model.StoredCredential
import dev.agentremote.messenger.protocol.WireProtocol
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.TimeUnit
import kotlin.math.min
import kotlin.random.Random
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString

internal class RemoteClient(
    private val listener: Listener,
    private val socketOpener: WebSocketOpener? = null,
) {
    interface Listener {
        fun onConnecting(target: ConnectionTarget)
        fun onConnected(target: ConnectionTarget)
        fun onEvent(event: ServerEvent, connectionGeneration: Long)
        fun onDisconnected(message: String)
        fun onRetryScheduled(attempt: Int, delayMillis: Long)
        fun onRetryStopped(message: String)
        fun onError(message: String)
    }

    private val http = OkHttpClient.Builder()
        .pingInterval(25, TimeUnit.SECONDS)
        .build()
    private val lock = Any()
    private val scheduler = Executors.newSingleThreadScheduledExecutor { runnable ->
        Thread(runnable, "agent-remote-reconnect").apply { isDaemon = true }
    }
    private var socket: WebSocket? = null
    private var target: ConnectionTarget? = null
    private var generation = 0L
    private var readyGeneration: Long? = null
    private var reconnectAttempt = 0
    private var reconnectFuture: ScheduledFuture<*>? = null
    private var retryEnabled = true
    private var closedByUser = false

    fun connect(newTarget: ConnectionTarget) {
        synchronized(lock) {
            closedByUser = false
            retryEnabled = true
            reconnectAttempt = 0
            readyGeneration = null
            cancelReconnectLocked()
            target = newTarget
            generation += 1
            socket?.cancel()
            openLocked(generation, newTarget)
        }
    }

    fun disconnect() {
        disconnect(clearTarget = false)
    }

    fun forgetTarget() {
        disconnect(clearTarget = true)
    }

    private fun disconnect(clearTarget: Boolean) {
        synchronized(lock) {
            closedByUser = true
            retryEnabled = false
            readyGeneration = null
            cancelReconnectLocked()
            generation += 1
            socket?.close(1000, "user disconnected")
            socket = null
            if (clearTarget) target = null
            reconnectAttempt = 0
        }
    }

    fun retryNow() {
        synchronized(lock) {
            val reconnectTarget = target?.takeIf { it.credential != null } ?: return
            retryEnabled = true
            closedByUser = false
            reconnectAttempt = 0
            readyGeneration = null
            cancelReconnectLocked()
            generation += 1
            socket?.cancel()
            socket = null
            openLocked(generation, reconnectTarget)
        }
    }

    fun stopRetrying() {
        synchronized(lock) {
            retryEnabled = false
            cancelReconnectLocked()
        }
        listener.onRetryStopped("已停止自动重连")
    }

    fun close() {
        synchronized(lock) {
            closedByUser = true
            retryEnabled = false
            readyGeneration = null
            cancelReconnectLocked()
            generation += 1
            socket?.cancel()
            socket = null
            target = null
        }
        scheduler.shutdownNow()
        http.dispatcher.executorService.shutdown()
        http.connectionPool.evictAll()
    }

    fun networkAvailable() {
        val shouldRetry = synchronized(lock) {
            retryEnabled && !closedByUser && socket == null && target?.credential != null
        }
        if (shouldRetry) retryNow()
    }

    /**
     * Returns true only after the current socket's onOpen callback validates the subprotocol.
     * OkHttp creates a WebSocket object before onOpen, so socket != null alone is not send-ready.
     */
    fun send(bytes: ByteArray): Boolean = synchronized(lock) {
        val activeSocket = socket?.takeIf { readyGeneration == generation } ?: return@synchronized false
        activeSocket.send(bytes.toByteString())
    }

    fun isCurrent(connectionGeneration: Long): Boolean = current(connectionGeneration)

    fun isConnected(connectionGeneration: Long): Boolean = synchronized(lock) {
        generation == connectionGeneration &&
            readyGeneration == connectionGeneration &&
            !closedByUser &&
            socket != null
    }

    fun targets(credential: StoredCredential): Boolean =
        synchronized(lock) { target?.credential == credential }

    private fun openLocked(connectionGeneration: Long, connectionTarget: ConnectionTarget) {
        readyGeneration = null
        listener.onConnecting(connectionTarget)
        val request = Request.Builder()
            .url(connectionTarget.webSocketUrl)
            .header("Sec-WebSocket-Protocol", WireProtocol.SUBPROTOCOL)
            .build()
        socket = (socketOpener ?: WebSocketOpener(http::newWebSocket)).open(
            request,
            object : WebSocketListener() {
                override fun onOpen(webSocket: WebSocket, response: Response) {
                    if (!current(connectionGeneration)) {
                        webSocket.close(1000, "superseded")
                        return
                    }
                    if (response.header("Sec-WebSocket-Protocol") != WireProtocol.SUBPROTOCOL) {
                        webSocket.close(1002, "subprotocol mismatch")
                        listener.onError("服务端没有接受 ${WireProtocol.SUBPROTOCOL}")
                        return
                    }
                    synchronized(lock) {
                        if (generation == connectionGeneration) readyGeneration = connectionGeneration
                    }
                    listener.onConnected(connectionTarget)
                    val firstFrame = connectionTarget.pairToken?.let {
                        WireProtocol.pair(connectionTarget, "${Build.MANUFACTURER} ${Build.MODEL}")
                    } ?: connectionTarget.credential?.let(WireProtocol::authenticate)
                    if (firstFrame == null) {
                        listener.onError("连接缺少配对 token 或设备凭证")
                        webSocket.close(1008, "missing credentials")
                    } else if (!webSocket.send(firstFrame.toByteString())) {
                        listener.onError("认证请求未能进入 WebSocket 发送队列")
                        webSocket.close(1011, "authentication send failed")
                    }
                }

                override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                    if (!current(connectionGeneration)) return
                    val event = runCatching {
                        WireProtocol.decodeServer(bytes.toByteArray(), currentTarget(connectionTarget))
                    }.getOrElse { error ->
                        listener.onError(error.message ?: "无法解析 Host 消息")
                        webSocket.close(1002, "invalid protocol message")
                        return
                    }
                    if (event is ServerEvent.Paired) {
                        synchronized(lock) {
                            if (generation == connectionGeneration) {
                                reconnectAttempt = 0
                                target = connectionTarget.copy(
                                    pairToken = null,
                                    credential = event.credential,
                                )
                            }
                        }
                        if (!webSocket.send(WireProtocol.getSnapshot().toByteString())) {
                            listener.onError("配对成功，但无法请求 Host 快照")
                        }
                    } else if (event is ServerEvent.Authenticated) {
                        synchronized(lock) {
                            if (generation == connectionGeneration) reconnectAttempt = 0
                        }
                        if (!webSocket.send(WireProtocol.getSnapshot().toByteString())) {
                            listener.onError("认证成功，但无法请求 Host 快照")
                        }
                    }
                    listener.onEvent(event, connectionGeneration)
                }

                override fun onMessage(webSocket: WebSocket, text: String) {
                    if (current(connectionGeneration)) {
                        listener.onError("服务端发送了文本帧；协议要求二进制 CBOR")
                        webSocket.close(1003, "binary CBOR required")
                    }
                }

                override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                    handleDisconnect(connectionGeneration, reason)
                }

                override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                    handleDisconnect(connectionGeneration, t.message ?: "WebSocket 连接失败")
                }
            },
        )
    }

    private fun handleDisconnect(connectionGeneration: Long, message: String) {
        val scheduled = synchronized(lock) {
            if (closedByUser || generation != connectionGeneration) return
            readyGeneration = null
            socket = null
            if (!retryEnabled || reconnectFuture != null) {
                return@synchronized null
            }
            val reconnectTarget = target?.takeIf { it.credential != null } ?: return@synchronized null
            if (reconnectAttempt >= MAX_RECONNECT_ATTEMPTS) {
                retryEnabled = false
                return@synchronized RetrySchedule.Stopped
            }
            val attempt = ++reconnectAttempt
            val delaySeconds = min(30, 1 shl min(5, attempt - 1))
            val delayMillis = delaySeconds * 1_000L + Random.nextLong(0, 800)
            reconnectFuture = scheduler.schedule({
                synchronized(lock) {
                    reconnectFuture = null
                    if (!closedByUser && retryEnabled && generation == connectionGeneration && socket == null) {
                        generation += 1
                        openLocked(generation, reconnectTarget)
                    }
                }
            }, delayMillis, TimeUnit.MILLISECONDS)
            RetrySchedule.Pending(attempt, delayMillis)
        }
        listener.onDisconnected(message.ifBlank { "连接已关闭" })
        when (scheduled) {
            is RetrySchedule.Pending -> listener.onRetryScheduled(scheduled.attempt, scheduled.delayMillis)
            RetrySchedule.Stopped -> listener.onRetryStopped("已达到自动重连上限")
            null -> Unit
        }
    }

    private fun cancelReconnectLocked() {
        reconnectFuture?.cancel(false)
        reconnectFuture = null
    }

    private fun current(expectedGeneration: Long): Boolean = synchronized(lock) {
        generation == expectedGeneration && !closedByUser
    }

    private fun currentTarget(fallback: ConnectionTarget): ConnectionTarget = synchronized(lock) {
        target ?: fallback
    }

    private sealed interface RetrySchedule {
        data class Pending(val attempt: Int, val delayMillis: Long) : RetrySchedule
        data object Stopped : RetrySchedule
    }

    companion object {
        private const val MAX_RECONNECT_ATTEMPTS = 6
    }
}

internal fun interface WebSocketOpener {
    fun open(request: Request, listener: WebSocketListener): WebSocket
}
