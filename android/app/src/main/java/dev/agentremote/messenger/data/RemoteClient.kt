package dev.agentremote.messenger.data

import android.os.Build
import dev.agentremote.messenger.model.ConnectionTarget
import dev.agentremote.messenger.model.ServerEvent
import dev.agentremote.messenger.protocol.WireProtocol
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.TimeUnit
import kotlin.random.Random
import kotlin.math.min
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString

class RemoteClient(
    private val listener: Listener,
) {
    interface Listener {
        fun onConnecting(target: ConnectionTarget)
        fun onConnected(target: ConnectionTarget)
        fun onEvent(event: ServerEvent)
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
    private var reconnectAttempt = 0
    private var reconnectFuture: ScheduledFuture<*>? = null
    private var retryEnabled = true
    private var closedByUser = false

    fun connect(newTarget: ConnectionTarget) {
        synchronized(lock) {
            closedByUser = false
            retryEnabled = true
            reconnectAttempt = 0
            cancelReconnectLocked()
            target = newTarget
            generation += 1
            socket?.cancel()
            openLocked(generation, newTarget)
        }
    }

    fun disconnect() {
        synchronized(lock) {
            closedByUser = true
            retryEnabled = false
            cancelReconnectLocked()
            generation += 1
            socket?.close(1000, "user disconnected")
            socket = null
            target = null
            reconnectAttempt = 0
        }
    }

    fun retryNow() {
        synchronized(lock) {
            val reconnectTarget = target?.takeIf { it.credential != null } ?: return
            retryEnabled = true
            closedByUser = false
            reconnectAttempt = 0
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

    fun send(bytes: ByteArray): Boolean = synchronized(lock) {
        socket?.send(bytes.toByteString()) ?: false
    }

    private fun openLocked(connectionGeneration: Long, connectionTarget: ConnectionTarget) {
        listener.onConnecting(connectionTarget)
        val request = Request.Builder()
            .url(connectionTarget.webSocketUrl)
            .header("Sec-WebSocket-Protocol", WireProtocol.SUBPROTOCOL)
            .build()
        socket = http.newWebSocket(
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
                    listener.onConnected(connectionTarget)
                    val firstFrame = connectionTarget.pairToken?.let {
                        WireProtocol.pair(connectionTarget, "${Build.MANUFACTURER} ${Build.MODEL}")
                    } ?: connectionTarget.credential?.let(WireProtocol::authenticate)
                    if (firstFrame == null) {
                        listener.onError("连接缺少配对 token 或设备凭证")
                        webSocket.close(1008, "missing credentials")
                    } else {
                        webSocket.send(firstFrame.toByteString())
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
                        webSocket.send(WireProtocol.getSnapshot().toByteString())
                    } else if (event is ServerEvent.Authenticated) {
                        synchronized(lock) {
                            if (generation == connectionGeneration) reconnectAttempt = 0
                        }
                        webSocket.send(WireProtocol.getSnapshot().toByteString())
                    }
                    listener.onEvent(event)
                }

                override fun onMessage(webSocket: WebSocket, text: String) {
                    if (current(connectionGeneration)) {
                        listener.onError("服务端发送了文本帧；协议要求二进制 CBOR")
                        webSocket.close(1003, "binary CBOR required")
                    }
                }

                override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                    if (current(connectionGeneration)) handleDisconnect(connectionGeneration, reason)
                }

                override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                    if (current(connectionGeneration)) {
                        handleDisconnect(connectionGeneration, t.message ?: "WebSocket 连接失败")
                    }
                }
            },
        )
    }

    private fun handleDisconnect(connectionGeneration: Long, message: String) {
        listener.onDisconnected(message.ifBlank { "连接已关闭" })
        val scheduled = synchronized(lock) {
            socket = null
            if (closedByUser || !retryEnabled || generation != connectionGeneration || reconnectFuture != null) {
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
