package dev.agentremote.messenger.ui

import dev.agentremote.messenger.model.Conversation
import dev.agentremote.messenger.model.ProjectSummary
import dev.agentremote.messenger.model.PromptAttachment
import dev.agentremote.messenger.model.ProviderId
import java.util.UUID
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class RemoteViewModelStateTest {
    @Test
    fun unacknowledgedStartWaitsForExactOutcomeEvenWhenSnapshotIsTerminal() {
        assertEquals(
            PendingStartResolution.UNRESOLVED,
            pendingSendResolution(pending(startsConversation = true), conversation("interrupted")),
        )
        assertEquals(
            PendingStartResolution.UNRESOLVED,
            pendingSendResolution(pending(startsConversation = true), conversation("failed")),
        )
    }

    @Test
    fun startAckThenDisconnectKeepsContextUntilSnapshotIdentifiesConversation() {
        val accepted = pending(startsConversation = true).accepted()

        assertTrue(accepted.trustedAccepted)
        assertEquals(
            PendingStartResolution.UNRESOLVED,
            pendingSendResolution(accepted, null),
        )
        assertEquals(
            PendingStartResolution.LANDED,
            pendingSendResolution(accepted, conversation("running")),
        )
    }

    @Test
    fun replayedStartTrustsHostsPersistedCommandOutcome() {
        val running = conversation("running")
        var replayed = pending(startsConversation = true).replayed()
        assertEquals(
            PendingStartResolution.UNRESOLVED,
            pendingSendResolution(replayed, running),
        )

        replayed = replayed.accepted()

        assertTrue(replayed.trustedAccepted)
        assertEquals(
            PendingStartResolution.LANDED,
            pendingSendResolution(replayed, running),
        )
    }

    @Test
    fun successfulSendRemovesOnlyComposerValuesThatWereActuallySent() {
        val sentAttachment = attachment("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
        val newAttachment = attachment("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb")
        val pending = pending(
            sentDraft = "original draft",
            sentAttachmentIds = setOf(sentAttachment.id),
        )
        val edited = RemoteUiState(
            draft = "new draft while waiting",
            promptAttachments = listOf(sentAttachment, newAttachment),
        )

        val cleared = edited.removeSentComposer(pending)

        assertEquals("new draft while waiting", cleared.draft)
        assertEquals(listOf(newAttachment), cleared.promptAttachments)
    }

    @Test
    fun successfulSendClearsUnchangedSentDraft() {
        val pending = pending(sentDraft = "unchanged")

        assertEquals("", RemoteUiState(draft = "unchanged").removeSentComposer(pending).draft)
    }

    @Test
    fun pendingSendLocksScopeNavigationUntilItFinishes() {
        assertEquals(false, pendingSendAllowsNavigation(pending()))
        assertEquals(true, pendingSendAllowsNavigation(null))
    }

    @Test
    fun projectSyncCompletionReleasesItsPendingCommand() {
        val completed = UUID.fromString("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee")
        val other = UUID.fromString("ffffffff-ffff-ffff-ffff-ffffffffffff")

        val state = RemoteUiState(pendingCommands = setOf(completed, other))
            .completeProjectSync(completed, conversationsSynced = 3, fullHistoryFallback = true)

        assertEquals(setOf(other), state.pendingCommands)
        assertEquals("已同步 3 个对话（全量去重）", state.phase)
    }

    @Test
    fun replayedExistingSendWaitsForExactCommandOutcome() {
        val pending = pending(
            startsConversation = false,
            sentDraft = "same text",
        ).replayed()

        assertEquals(
            PendingStartResolution.UNRESOLVED,
            pendingSendResolution(pending, conversation("completed")),
        )
    }

    @Test
    fun staleConversationRevisionDoesNotReplaceMergedState() {
        val current = conversation("running", revision = 4)
        val stale = conversation("failed", revision = 3)

        assertEquals(listOf(current), mergeConversation(listOf(current), stale))
    }

    @Test
    fun explicitProviderWithoutAuthorizedProjectDoesNotFallBack() {
        val codexProject = project(
            "22222222-2222-2222-2222-222222222222",
            listOf(ProviderId.CODEX),
        )

        assertEquals(
            ProviderProjectSelection(ProviderId.GROK, null),
            providerProjectSelection(listOf(codexProject), ProviderId.GROK, codexProject.id),
        )
    }

    @Test
    fun explicitProviderSelectsAuthorizedProject() {
        val codexProject = project(
            "22222222-2222-2222-2222-222222222222",
            listOf(ProviderId.CODEX),
        )
        val grokProject = project(
            "33333333-3333-3333-3333-333333333333",
            listOf(ProviderId.GROK),
        )

        assertEquals(
            ProviderProjectSelection(ProviderId.GROK, grokProject.id),
            providerProjectSelection(listOf(codexProject, grokProject), ProviderId.GROK, codexProject.id),
        )
    }

    @Test
    fun missingInitialProviderUsesFirstValidProjectDefault() {
        val codexProject = project(
            "22222222-2222-2222-2222-222222222222",
            listOf(ProviderId.CODEX),
        )

        assertEquals(
            ProviderProjectSelection(ProviderId.CODEX, codexProject.id),
            providerProjectSelection(listOf(codexProject), null, null),
        )
    }

    @Test
    fun restoredConversationSurvivesAStaleCachedSnapshotUntilTheFreshSnapshotArrives() {
        val selected = UUID.fromString("11111111-1111-1111-1111-111111111111")
        val project = UUID.fromString("22222222-2222-2222-2222-222222222222")

        assertEquals(
            selected,
            retainedConversationSelection(
                conversations = emptyList(),
                selectedConversationId = selected,
                projectId = project,
                provider = ProviderId.CODEX,
                preserveMissing = true,
            ),
        )
        assertEquals(
            null,
            retainedConversationSelection(
                conversations = emptyList(),
                selectedConversationId = selected,
                projectId = project,
                provider = ProviderId.CODEX,
                preserveMissing = false,
            ),
        )
        assertEquals(
            selected,
            retainedConversationSelection(
                conversations = listOf(conversation("completed")),
                selectedConversationId = selected,
                projectId = project,
                provider = ProviderId.CODEX,
                preserveMissing = false,
            ),
        )
    }

    private fun conversation(state: String, revision: Long = 1) = Conversation(
        id = UUID.fromString("11111111-1111-1111-1111-111111111111"),
        revision = revision,
        provider = ProviderId.CODEX,
        projectId = UUID.fromString("22222222-2222-2222-2222-222222222222"),
        nativeSessionId = "native-session",
        title = "Conversation",
        titleSource = "generated",
        titleUpdatedAtMs = 1,
        selectedModel = null,
        selectedEffort = null,
        state = state,
        sessionOptions = emptyList(),
        updatedAtMs = 1,
    )

    private fun pending(
        startsConversation: Boolean = true,
        sentDraft: String = "sent text",
        sentAttachmentIds: Set<UUID> = emptySet(),
    ) = PendingSendContext(
        commandId = UUID.fromString("99999999-9999-9999-9999-999999999999"),
        frame = byteArrayOf(1),
        conversationId = UUID.fromString("11111111-1111-1111-1111-111111111111"),
        startsConversation = startsConversation,
        sentDraft = sentDraft,
        sentAttachmentIds = sentAttachmentIds,
    )

    private fun attachment(id: String) = PromptAttachment(
        id = UUID.fromString(id),
        fileName = "attachment.png",
        mimeType = "image/png",
        bytes = byteArrayOf(1),
    )

    private fun project(id: String, providers: List<ProviderId>) = ProjectSummary(
        id = UUID.fromString(id),
        displayName = "Project",
        shortPath = "Project",
        enabledProviders = providers,
        valid = true,
        lastActivityAtMs = null,
        conversationCount = 0,
    )
}
