package dev.agentremote.messenger.data

import android.content.Context
import android.util.Base64
import dev.agentremote.messenger.model.ConnectionTarget
import dev.agentremote.messenger.model.ProviderId
import dev.agentremote.messenger.model.ServerEvent
import dev.agentremote.messenger.model.Snapshot
import dev.agentremote.messenger.model.StoredCredential
import dev.agentremote.messenger.protocol.WireProtocol
import java.util.UUID
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.ScheduledThreadPoolExecutor
import java.util.concurrent.TimeUnit
import org.json.JSONArray
import org.json.JSONObject

data class RestoredClientState(
    val snapshot: Snapshot?,
    val selectedConversationId: UUID?,
    val selectedProjectId: UUID?,
    val selectedProvider: ProviderId?,
    val selectedModel: String?,
    val selectedEffort: String?,
    val selectedPermission: String?,
    val draft: String,
    val pinnedProjects: Set<UUID>,
    val recentProjects: List<UUID>,
)

private data class PendingSelection(
    val hostId: UUID,
    val selectedConversationId: UUID?,
    val selectedProjectId: UUID?,
    val selectedProvider: ProviderId?,
    val selectedModel: String?,
    val selectedEffort: String?,
    val selectedPermission: String?,
    val draft: String,
    val pinnedProjects: Set<UUID>,
    val recentProjects: List<UUID>,
)

class ClientStateStore(context: Context) {
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
    private val selectionLock = Any()
    private var pendingSelection: PendingSelection? = null
    private var pendingSelectionWrite: ScheduledFuture<*>? = null

    fun lastHostId(): UUID? = preferences.getString(LAST_HOST, null)?.let(UUID::fromString)

    fun saveLastHost(hostId: UUID) {
        preferences.edit().putString(LAST_HOST, hostId.toString()).apply()
    }

    fun load(credential: StoredCredential): RestoredClientState {
        flushSelection(credential.hostId)
        val hostId = credential.hostId
        val state = preferences.getString(stateKey(hostId), null)?.let(::JSONObject)
        val snapshot = preferences.getString(snapshotKey(hostId), null)
            ?.let { Base64.decode(it, Base64.NO_WRAP) }
            ?.let { encoded ->
                runCatching {
                    WireProtocol.decodeServer(
                        encoded,
                        ConnectionTarget(
                            hostId = credential.hostId,
                            origin = credential.origin,
                            relay = credential.relay,
                            credential = credential,
                        ),
                    )
                }.getOrNull()
            }
            ?.let { it as? ServerEvent.SnapshotReceived }
            ?.snapshot
        return RestoredClientState(
            snapshot = snapshot,
            selectedConversationId = state.uuidOrNull("conversation"),
            selectedProjectId = state.uuidOrNull("project"),
            selectedProvider = state?.optString("provider")
                ?.takeIf(String::isNotBlank)
                ?.let(ProviderId::fromWire),
            selectedModel = state.stringOrNull("model"),
            selectedEffort = state.stringOrNull("effort"),
            selectedPermission = state.stringOrNull("permission"),
            draft = state?.optString("draft").orEmpty(),
            pinnedProjects = state.uuidArray("pinned").toSet(),
            recentProjects = state.uuidArray("recent"),
        )
    }

    fun saveSnapshot(hostId: UUID, encoded: ByteArray) {
        preferences.edit()
            .putString(snapshotKey(hostId), Base64.encodeToString(encoded, Base64.NO_WRAP))
            .apply()
    }

    fun saveSelection(
        hostId: UUID,
        selectedConversationId: UUID?,
        selectedProjectId: UUID?,
        selectedProvider: ProviderId?,
        selectedModel: String?,
        selectedEffort: String?,
        selectedPermission: String?,
        draft: String,
        pinnedProjects: Set<UUID>,
        recentProjects: List<UUID>,
    ) {
        val selection = PendingSelection(
            hostId = hostId,
            selectedConversationId = selectedConversationId,
            selectedProjectId = selectedProjectId,
            selectedProvider = selectedProvider,
            selectedModel = selectedModel,
            selectedEffort = selectedEffort,
            selectedPermission = selectedPermission,
            draft = draft,
            pinnedProjects = pinnedProjects.toSet(),
            recentProjects = recentProjects.toList(),
        )
        synchronized(selectionLock) {
            pendingSelection = selection
            pendingSelectionWrite?.cancel(false)
            pendingSelectionWrite = selectionWriter.schedule(
                { flushSelection() },
                SELECTION_WRITE_DELAY_MS,
                TimeUnit.MILLISECONDS,
            )
        }
    }

    fun flush() {
        flushSelection()
    }

    fun clear(hostId: UUID) {
        synchronized(selectionLock) {
            if (pendingSelection?.hostId == hostId) {
                pendingSelection = null
                pendingSelectionWrite?.cancel(false)
                pendingSelectionWrite = null
            }
            preferences.edit()
                .remove(snapshotKey(hostId))
                .remove(stateKey(hostId))
                .apply()
        }
    }

    private fun flushSelection(hostId: UUID? = null) {
        synchronized(selectionLock) {
            val selection = pendingSelection
            if (selection == null || hostId != null && selection.hostId != hostId) return
            pendingSelection = null
            pendingSelectionWrite = null
            writeSelection(selection)
        }
    }

    private fun writeSelection(selection: PendingSelection) {
        val state = JSONObject()
            .put("conversation", selection.selectedConversationId?.toString())
            .put("project", selection.selectedProjectId?.toString())
            .put("provider", selection.selectedProvider?.wire)
            .put("model", selection.selectedModel)
            .put("effort", selection.selectedEffort)
            .put("permission", selection.selectedPermission)
            .put("draft", selection.draft)
            .put("pinned", JSONArray(selection.pinnedProjects.map(UUID::toString)))
            .put("recent", JSONArray(selection.recentProjects.map(UUID::toString)))
        preferences.edit().putString(stateKey(selection.hostId), state.toString()).apply()
    }

    private fun snapshotKey(hostId: UUID) = "snapshot_$hostId"
    private fun stateKey(hostId: UUID) = "state_$hostId"

    private fun JSONObject?.uuidOrNull(name: String): UUID? =
        this?.optString(name)?.takeIf { it.isNotBlank() && it != "null" }?.let(UUID::fromString)

    private fun JSONObject?.stringOrNull(name: String): String? =
        this?.optString(name)?.takeIf { it.isNotBlank() && it != "null" }

    private fun JSONObject?.uuidArray(name: String): List<UUID> {
        val values = this?.optJSONArray(name) ?: return emptyList()
        return buildList {
            for (index in 0 until values.length()) add(UUID.fromString(values.getString(index)))
        }
    }

    companion object {
        private const val PREFERENCES = "agent_remote_client_state_v2"
        private const val LAST_HOST = "last_host"
        private const val SELECTION_WRITE_DELAY_MS = 250L
        private val selectionWriter = ScheduledThreadPoolExecutor(1) { runnable ->
            Thread(runnable, "agent-remote-state-writer").apply { isDaemon = true }
        }.apply {
            removeOnCancelPolicy = true
        }
    }
}
