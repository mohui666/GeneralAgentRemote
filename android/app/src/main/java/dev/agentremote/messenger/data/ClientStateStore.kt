package dev.agentremote.messenger.data

import android.annotation.SuppressLint
import android.content.Context
import android.util.Base64
import dev.agentremote.messenger.model.ConnectionTarget
import dev.agentremote.messenger.model.ProviderId
import dev.agentremote.messenger.model.ProjectTreeScope
import dev.agentremote.messenger.model.ServerEvent
import dev.agentremote.messenger.model.Snapshot
import dev.agentremote.messenger.model.StoredCredential
import dev.agentremote.messenger.protocol.WireProtocol
import java.util.UUID
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
    val drafts: Map<DraftScope, String>,
    val pinnedProjects: Set<UUID>,
    val recentProjects: List<UUID>,
    val expandedProjectScopes: Set<ProjectTreeScope>,
    val projectExpansionWasPersisted: Boolean,
)

data class DraftScope(
    val hostId: UUID,
    val provider: ProviderId,
    val projectId: UUID,
    val conversationId: UUID?,
)

internal data class RestoredDrafts(
    val values: Map<DraftScope, String>,
    val selectedDraft: String,
    val migratedLegacyDraft: Boolean,
)

internal data class PersistedDraft(
    val scope: DraftScope,
    val value: String,
)

class ClientStateStore(context: Context) {
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
    private val draftLock = Any()

    fun lastHostId(): UUID? = preferences.getString(LAST_HOST, null)?.let(UUID::fromString)

    fun saveLastHost(hostId: UUID) {
        preferences.edit().putString(LAST_HOST, hostId.toString()).apply()
    }

    fun load(credential: StoredCredential): RestoredClientState {
        val hostId = credential.hostId
        val state = preferences.getString(stateKey(hostId), null)?.let(::JSONObject)
        val selectedConversationId = state.uuidOrNull("conversation")
        val selectedProjectId = state.uuidOrNull("project")
        val selectedProvider = state?.optString("provider")
            ?.takeIf(String::isNotBlank)
            ?.let(ProviderId::fromWire)
        val restoredDrafts = restoreDrafts(
            state = state,
            expectedHostId = hostId,
            selectedProvider = selectedProvider,
            selectedProjectId = selectedProjectId,
            selectedConversationId = selectedConversationId,
        )
        if (state != null && restoredDrafts.migratedLegacyDraft) {
            writeDrafts(state, hostId, restoredDrafts.values, commit = false)
        }
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
            selectedConversationId = selectedConversationId,
            selectedProjectId = selectedProjectId,
            selectedProvider = selectedProvider,
            selectedModel = state.stringOrNull("model"),
            selectedEffort = state.stringOrNull("effort"),
            selectedPermission = state.stringOrNull("permission"),
            draft = restoredDrafts.selectedDraft,
            drafts = restoredDrafts.values,
            pinnedProjects = state.uuidArray("pinned").toSet(),
            recentProjects = state.uuidArray("recent"),
            expandedProjectScopes = state.projectTreeScopes("expanded_projects", hostId),
            projectExpansionWasPersisted = state?.has("expanded_projects") == true,
        )
    }

    fun saveSnapshot(hostId: UUID, encoded: ByteArray) {
        preferences.edit()
            .putString(snapshotKey(hostId), Base64.encodeToString(encoded, Base64.NO_WRAP))
            .apply()
    }

    @SuppressLint("ApplySharedPref")
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
        expandedProjectScopes: Set<ProjectTreeScope>,
        scopedDrafts: Map<DraftScope, String>? = null,
        commit: Boolean = false,
    ) {
        synchronized(draftLock) {
            val existingState = preferences.getString(stateKey(hostId), null)?.let(::JSONObject)
            val drafts = scopedDrafts ?: run {
                val existing = restoreDrafts(
                    state = existingState,
                    expectedHostId = hostId,
                    selectedProvider = existingState?.optString("provider")
                        ?.takeIf(String::isNotBlank)
                        ?.let(ProviderId::fromWire),
                    selectedProjectId = existingState.uuidOrNull("project"),
                    selectedConversationId = existingState.uuidOrNull("conversation"),
                ).values.toMutableMap()
                draftScope(hostId, selectedProvider, selectedProjectId, selectedConversationId)?.let { scope ->
                    if (draft.isEmpty()) existing.remove(scope) else existing[scope] = draft
                }
                existing
            }
            val state = JSONObject()
                .put("conversation", selectedConversationId?.toString())
                .put("project", selectedProjectId?.toString())
                .put("provider", selectedProvider?.wire)
                .put("model", selectedModel)
                .put("effort", selectedEffort)
                .put("permission", selectedPermission)
                .put("drafts", draftEntries(drafts, hostId))
                .put("pinned", JSONArray(pinnedProjects.map(UUID::toString)))
                .put("recent", JSONArray(recentProjects.map(UUID::toString)))
                .put(
                    "expanded_projects",
                    JSONArray(expandedProjectScopes.filter { it.hostId == hostId }.map { scope ->
                        JSONObject()
                            .put("host", scope.hostId.toString())
                            .put("provider", scope.provider.wire)
                            .put("project", scope.projectId.toString())
                    }),
                )
            preferences.edit().putString(stateKey(hostId), state.toString()).also { editor ->
                if (commit) editor.commit() else editor.apply()
            }
        }
    }

    private fun writeDrafts(
        state: JSONObject,
        hostId: UUID,
        drafts: Map<DraftScope, String>,
        commit: Boolean,
    ) {
        state.remove("draft")
        state.put("drafts", draftEntries(drafts, hostId))
        preferences.edit().putString(stateKey(hostId), state.toString()).also { editor ->
            if (commit) editor.commit() else editor.apply()
        }
    }

    fun clear(hostId: UUID) {
        preferences.edit()
            .remove(snapshotKey(hostId))
            .remove(stateKey(hostId))
            .apply()
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

    private fun JSONObject?.projectTreeScopes(name: String, expectedHostId: UUID): Set<ProjectTreeScope> {
        val values = this?.optJSONArray(name) ?: return emptySet()
        return buildSet {
            for (index in 0 until values.length()) {
                val scope = runCatching {
                    val value = values.getJSONObject(index)
                    ProjectTreeScope(
                        hostId = UUID.fromString(value.getString("host")),
                        provider = ProviderId.fromWire(value.getString("provider")),
                        projectId = UUID.fromString(value.getString("project")),
                    )
                }.getOrNull()
                if (scope?.hostId == expectedHostId) add(scope)
            }
        }
    }

    companion object {
        private const val PREFERENCES = "agent_remote_client_state_v2"
        private const val LAST_HOST = "last_host"
    }
}

internal fun draftScope(
    hostId: UUID,
    provider: ProviderId?,
    projectId: UUID?,
    conversationId: UUID?,
): DraftScope? = if (provider == null || projectId == null) {
    null
} else {
    DraftScope(hostId, provider, projectId, conversationId)
}

internal fun restoreDrafts(
    state: JSONObject?,
    expectedHostId: UUID,
    selectedProvider: ProviderId?,
    selectedProjectId: UUID?,
    selectedConversationId: UUID?,
): RestoredDrafts {
    val persisted = mutableListOf<PersistedDraft>()
    val entries = state?.optJSONArray("drafts")
    if (entries != null) {
        for (index in 0 until entries.length()) {
            val entry = entries.getJSONObject(index)
            persisted += PersistedDraft(
                scope = DraftScope(
                    hostId = UUID.fromString(entry.getString("host")),
                    provider = ProviderId.fromWire(entry.getString("provider")),
                    projectId = UUID.fromString(entry.getString("project")),
                    conversationId = entry.optString("conversation")
                        .takeIf { it.isNotBlank() && it != "null" }
                        ?.let(UUID::fromString),
                ),
                value = entry.getString("value"),
            )
        }
    }
    val legacyDraft = state?.optString("draft")
        ?.takeIf { it.isNotEmpty() && it != "null" }
        .orEmpty()
    return restoreDraftValues(
        persisted = persisted,
        expectedHostId = expectedHostId,
        selectedProvider = selectedProvider,
        selectedProjectId = selectedProjectId,
        selectedConversationId = selectedConversationId,
        legacyDraft = legacyDraft,
    )
}

internal fun restoreDraftValues(
    persisted: List<PersistedDraft>,
    expectedHostId: UUID,
    selectedProvider: ProviderId?,
    selectedProjectId: UUID?,
    selectedConversationId: UUID?,
    legacyDraft: String,
): RestoredDrafts {
    val values = persisted
        .asSequence()
        .filter { it.scope.hostId == expectedHostId && it.value.isNotEmpty() }
        .associateTo(linkedMapOf()) { it.scope to it.value }
    val selectedScope = draftScope(expectedHostId, selectedProvider, selectedProjectId, selectedConversationId)
    val migrated = legacyDraft.isNotEmpty() && selectedScope != null
    if (migrated && selectedScope !in values) values[selectedScope] = legacyDraft
    return RestoredDrafts(
        values = values,
        selectedDraft = selectedScope?.let { values[it] }.orEmpty(),
        migratedLegacyDraft = migrated,
    )
}

internal fun persistedDrafts(
    drafts: Map<DraftScope, String>,
    expectedHostId: UUID,
): List<PersistedDraft> = drafts.entries
    .asSequence()
    .filter { (scope, draft) -> scope.hostId == expectedHostId && draft.isNotEmpty() }
    .sortedWith(
        compareBy<Map.Entry<DraftScope, String>> { it.key.provider.wire }
            .thenBy { it.key.projectId.toString() }
            .thenBy { it.key.conversationId?.toString().orEmpty() },
    )
    .map { PersistedDraft(it.key, it.value) }
    .toList()

internal fun draftEntries(drafts: Map<DraftScope, String>, expectedHostId: UUID): JSONArray = JSONArray().apply {
    persistedDrafts(drafts, expectedHostId)
        .asSequence()
        .forEach { persisted ->
            put(
                JSONObject()
                    .put("host", persisted.scope.hostId.toString())
                    .put("provider", persisted.scope.provider.wire)
                    .put("project", persisted.scope.projectId.toString())
                    .apply { persisted.scope.conversationId?.let { put("conversation", it.toString()) } }
                    .put("value", persisted.value),
            )
        }
}
