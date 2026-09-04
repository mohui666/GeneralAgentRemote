package dev.agentremote.messenger.data

import dev.agentremote.messenger.model.ProviderId
import java.util.UUID
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ClientStateStoreDraftTest {
    private val host = UUID.fromString("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
    private val otherHost = UUID.fromString("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb")
    private val project = UUID.fromString("cccccccc-cccc-cccc-cccc-cccccccccccc")
    private val conversation = UUID.fromString("dddddddd-dddd-dddd-dddd-dddddddddddd")

    @Test
    fun scopedDraftsRoundTripWithoutCrossingAgentProjectConversationOrHost() {
        val codexProjectDraft = DraftScope(host, ProviderId.CODEX, project, null)
        val codexConversationDraft = DraftScope(host, ProviderId.CODEX, project, conversation)
        val grokProjectDraft = DraftScope(host, ProviderId.GROK, project, null)
        val foreignHostDraft = DraftScope(otherHost, ProviderId.CODEX, project, null)
        val persisted = persistedDrafts(
            mapOf(
                codexProjectDraft to "codex project",
                codexConversationDraft to "codex conversation",
                grokProjectDraft to "grok project",
                foreignHostDraft to "foreign",
            ),
            host,
        )

        val restored = restoreDraftValues(
            persisted = persisted,
            expectedHostId = host,
            selectedProvider = ProviderId.CODEX,
            selectedProjectId = project,
            selectedConversationId = conversation,
            legacyDraft = "",
        )

        assertEquals("codex conversation", restored.selectedDraft)
        assertEquals("codex project", restored.values[codexProjectDraft])
        assertEquals("grok project", restored.values[grokProjectDraft])
        assertFalse(foreignHostDraft in restored.values)
        assertFalse(restored.migratedLegacyDraft)
    }

    @Test
    fun emptyDraftsAreNotSerialized() {
        val scope = DraftScope(host, ProviderId.CODEX, project, null)

        val encoded = persistedDrafts(mapOf(scope to ""), host)

        assertTrue(encoded.isEmpty())
    }

    @Test
    fun legacyDraftMigratesToCurrentSelectionScope() {
        val restored = restoreDraftValues(
            persisted = emptyList(),
            expectedHostId = host,
            selectedProvider = ProviderId.GROK,
            selectedProjectId = project,
            selectedConversationId = conversation,
            legacyDraft = "legacy text",
        )

        val expectedScope = DraftScope(host, ProviderId.GROK, project, conversation)
        assertEquals("legacy text", restored.selectedDraft)
        assertEquals(mapOf(expectedScope to "legacy text"), restored.values)
        assertTrue(restored.migratedLegacyDraft)
    }

    @Test
    fun scopedDraftWinsOverStaleLegacyDraft() {
        val scope = DraftScope(host, ProviderId.CODEX, project, null)
        val restored = restoreDraftValues(
            persisted = listOf(PersistedDraft(scope, "scoped text")),
            expectedHostId = host,
            selectedProvider = ProviderId.CODEX,
            selectedProjectId = project,
            selectedConversationId = null,
            legacyDraft = "stale legacy",
        )

        assertEquals("scoped text", restored.selectedDraft)
        assertEquals(mapOf(scope to "scoped text"), restored.values)
        assertTrue(restored.migratedLegacyDraft)
    }

    @Test
    fun unscopedLegacyDraftIsNotAssignedToAnotherAgent() {
        val restored = restoreDraftValues(
            persisted = emptyList(),
            expectedHostId = host,
            selectedProvider = null,
            selectedProjectId = project,
            selectedConversationId = null,
            legacyDraft = "legacy text",
        )

        assertEquals("", restored.selectedDraft)
        assertTrue(restored.values.isEmpty())
        assertFalse(restored.migratedLegacyDraft)
    }
}
