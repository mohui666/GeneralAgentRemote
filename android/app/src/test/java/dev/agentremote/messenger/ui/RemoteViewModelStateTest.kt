package dev.agentremote.messenger.ui

import dev.agentremote.messenger.model.Conversation
import dev.agentremote.messenger.model.ProjectSummary
import dev.agentremote.messenger.model.PromptAttachment
import dev.agentremote.messenger.model.ProviderId
import dev.agentremote.messenger.model.Snapshot
import dev.agentremote.messenger.model.TimelineContent
import dev.agentremote.messenger.model.TimelineItem
import java.util.UUID
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class RemoteViewModelStateTest {
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
    fun commandAcceptanceEndsSendStateAndSelectsStartedConversation() {
        val pending = pending(startsConversation = true, sentDraft = "accepted")
        val completed = RemoteUiState(
            draft = "accepted",
            showingNewConversation = true,
            creatingConversation = true,
            pendingCommands = setOf(pending.commandId),
            sendStatus = SendStatus.QUEUED,
            sendFailure = "stale",
        ).completePendingSend(pending, clearComposer = true)

        assertEquals(SendStatus.IDLE, completed.sendStatus)
        assertEquals(null, completed.sendFailure)
        assertEquals("", completed.draft)
        assertEquals(pending.conversationId, completed.selectedConversationId)
        assertFalse(completed.showingNewConversation)
        assertFalse(completed.creatingConversation)
        assertTrue(completed.pendingCommands.isEmpty())
    }

    @Test
    fun inFlightSendLocksNavigationButRetryableFailureDoesNot() {
        assertEquals(false, pendingSendAllowsNavigation(pending()))
        assertEquals(true, pendingSendAllowsNavigation(pending().rejected()))
        assertEquals(true, pendingSendAllowsNavigation(pending().copy(writeFailed = true)))
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
    fun staleConversationRevisionDoesNotReplaceMergedState() {
        val current = conversation("running", revision = 4)
        val stale = conversation("failed", revision = 3)

        assertEquals(listOf(current), mergeConversation(listOf(current), stale))
    }

    @Test
    fun conversationMergeDoesNotCrossProviderScopeForSameId() {
        val id = UUID.fromString("11111111-1111-1111-1111-111111111111")
        val codex = conversation("running", id = id, provider = ProviderId.CODEX)
        val grok = conversation("completed", id = id, provider = ProviderId.GROK)

        val merged = mergeConversation(listOf(codex), grok)

        assertEquals(2, merged.size)
        assertTrue(codex in merged)
        assertTrue(grok in merged)
    }

    @Test
    fun unavailableProviderFallsBackToSelectedProjectsAuthorizedAgent() {
        val codexProject = project(
            "22222222-2222-2222-2222-222222222222",
            listOf(ProviderId.CODEX),
        )

        assertEquals(
            ProviderProjectSelection(ProviderId.CODEX, codexProject.id),
            providerProjectSelection(listOf(codexProject), ProviderId.GROK, codexProject.id),
        )
    }

    @Test
    fun agentSelectorOnlyOffersProvidersOwnedByValidAuthorizedProjects() {
        val codexProject = project(
            "22222222-2222-2222-2222-222222222222",
            listOf(ProviderId.CODEX),
        )
        val invalidGrokProject = project(
            "33333333-3333-3333-3333-333333333333",
            listOf(ProviderId.GROK),
        ).copy(valid = false)

        assertEquals(listOf(ProviderId.CODEX), availableAgentProviders(listOf(codexProject, invalidGrokProject)))
    }

    @Test
    fun timelineGroupingOnlyRebuildsChangedStreamingTail() {
        val first = timelineItem(
            id = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
            revision = 1,
            content = TimelineContent.UserMessage("question"),
        )
        val activity = timelineItem(
            id = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
            revision = 1,
            content = TimelineContent.Progress("tool", "read", "running", null),
        )
        val streaming = timelineItem(
            id = "cccccccc-cccc-cccc-cccc-cccccccccccc",
            revision = 1,
            content = TimelineContent.AgentMessage("final", "partial"),
        )
        val initial = TimelineGrouping(
            items = listOf(first, activity, streaming),
            blocks = groupTimeline(listOf(first, activity, streaming)),
        )
        val updatedStreaming = streaming.copy(
            revision = 2,
            content = TimelineContent.AgentMessage("final", "complete"),
        )

        val updated = updateTimelineGrouping(initial, listOf(first, activity, updatedStreaming))

        assertSame(initial.blocks[0], updated.blocks[0])
        assertSame(initial.blocks[1], updated.blocks[1])
        assertEquals(updatedStreaming, (updated.blocks[2] as TimelineBlock.Single).item)
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
    fun agentSwitchRestoresItsMostRecentAuthorizedProject() {
        val olderCodex = project(
            "22222222-2222-2222-2222-222222222222",
            listOf(ProviderId.CODEX),
        )
        val grok = project(
            "33333333-3333-3333-3333-333333333333",
            listOf(ProviderId.GROK),
        )
        val recentCodex = project(
            "44444444-4444-4444-4444-444444444444",
            listOf(ProviderId.CODEX),
        )

        assertEquals(
            ProviderProjectSelection(ProviderId.CODEX, recentCodex.id),
            providerProjectSelection(
                projects = listOf(olderCodex, grok, recentCodex),
                selectedProvider = ProviderId.CODEX,
                selectedProjectId = grok.id,
                recentProjectIds = listOf(grok.id, recentCodex.id, olderCodex.id),
            ),
        )
    }

    @Test
    fun historyPageMergesOnceAndKeepsNewestRevision() {
        val current = timelineItem(
            id = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
            revision = 2,
            content = TimelineContent.AgentMessage("final", "current"),
        )
        val stale = current.copy(revision = 1, content = TimelineContent.AgentMessage("final", "stale"))
        val added = timelineItem(
            id = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
            revision = 1,
            content = TimelineContent.UserMessage("older"),
        ).copy(createdAtMs = current.createdAtMs - 1)

        val merged = mergeTimelinePage(listOf(current), listOf(stale, added))

        assertEquals(listOf(added, current), merged)
        val unchanged = listOf(current)
        assertSame(unchanged, mergeTimelinePage(unchanged, listOf(stale)))

        val otherConversation = added.copy(
            conversationId = UUID.fromString("55555555-5555-5555-5555-555555555555"),
        )
        val indexed = indexTimelineByConversation(merged + otherConversation)
        assertEquals(listOf(added, current), indexed[current.conversationId])
        assertEquals(listOf(otherConversation), indexed[otherConversation.conversationId])
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

    @Test
    fun expandedProjectScopeIsIsolatedByHostProviderAndProject() {
        val host = UUID.fromString("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
        val project = UUID.fromString("22222222-2222-2222-2222-222222222222")
        val expanded = RemoteUiState(
            activeHostId = host,
            snapshot = Snapshot(host, "Host", emptyList(), emptyList(), emptyList(), emptyList()),
            selectedProvider = ProviderId.CODEX,
            selectedProjectId = project,
        ).expandSelectedProject()

        assertEquals(
            setOf(projectTreeScope(host, ProviderId.CODEX, project)),
            expanded.expandedProjectScopes,
        )
        assertEquals(
            false,
            projectTreeScope(host, ProviderId.GROK, project) in expanded.expandedProjectScopes,
        )

        val intentionallyCollapsed = expanded.copy(
            expandedProjectScopes = emptySet(),
            projectExpansionInitialized = true,
        ).ensureDefaultProjectExpanded()
        assertTrue(intentionallyCollapsed.expandedProjectScopes.isEmpty())
    }

    @Test
    fun projectTreeGroupingUsesExactScopeAndNewestConversationFirst() {
        val projectId = UUID.fromString("22222222-2222-2222-2222-222222222222")
        val older = conversation(
            state = "completed",
            id = UUID.fromString("11111111-1111-1111-1111-111111111111"),
            updatedAtMs = 10L,
            title = "Older Codex",
        )
        val newer = conversation(
            state = "completed",
            id = UUID.fromString("33333333-3333-3333-3333-333333333333"),
            updatedAtMs = 20L,
            title = "Needle Codex",
        )
        val otherProvider = conversation(
            state = "completed",
            id = UUID.fromString("44444444-4444-4444-4444-444444444444"),
            provider = ProviderId.GROK,
            updatedAtMs = 30L,
            title = "Needle Grok",
        )

        val grouped = conversationsByProjectForProvider(
            listOf(older, otherProvider, newer),
            ProviderId.CODEX,
        )

        assertEquals(listOf(newer, older), grouped[projectId])
        assertTrue(projectTreeMatchesSearch(project(projectId.toString(), listOf(ProviderId.CODEX)), grouped[projectId].orEmpty(), "needle"))
        assertEquals(false, grouped.values.flatten().contains(otherProvider))
    }

    @Test
    fun landscapeDrawerUsesCompactLayoutWithoutChangingPortraitLayout() {
        assertTrue(usesCompactDrawerLayout(360f))
        assertTrue(usesCompactDrawerLayout(480f))
        assertFalse(usesCompactDrawerLayout(481f))
        assertFalse(usesCompactDrawerLayout(800f))
    }

    @Test
    fun homeProjectExpansionUsesExactHostProviderAndProjectScope() {
        val host = UUID.fromString("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
        val otherHost = UUID.fromString("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb")
        val project = UUID.fromString("22222222-2222-2222-2222-222222222222")
        val scopes = setOf(projectTreeScope(host, ProviderId.CODEX, project))

        assertTrue(isHomeProjectExpanded(scopes, host, ProviderId.CODEX, project))
        assertFalse(isHomeProjectExpanded(scopes, otherHost, ProviderId.CODEX, project))
        assertFalse(isHomeProjectExpanded(scopes, host, ProviderId.GROK, project))
        assertFalse(isHomeProjectExpanded(scopes, host, null, project))
    }

    @Test
    fun automaticReplayKeepsExactFrameAttemptAndMessageIds() {
        val original = pending(sentDraft = "retry me").copy(writeFailed = true)
        val replayed = original.replayed(generation = 7L)

        assertEquals(original.commandId, replayed.commandId)
        assertEquals(original.clientMessageId, replayed.clientMessageId)
        assertEquals(original.attempt, replayed.attempt)
        assertSame(original.frame, replayed.frame)
        assertFalse(replayed.retryableFailure)
        assertEquals(7L, replayed.lastWrittenGeneration)
    }

    @Test
    fun hostRejectionRetainsDraftFrameAndIdsForUserRetry() {
        val original = pending(sentDraft = "retry me")
        val rejected = original.rejected()

        assertTrue(rejected.retryableFailure)
        assertEquals(original.commandId, rejected.commandId)
        assertEquals(original.clientMessageId, rejected.clientMessageId)
        assertEquals("retry me", rejected.sentDraft)
        assertArrayEquals(requireNotNull(original.frame), requireNotNull(rejected.frame))
    }

    @Test
    fun onlyDefinitiveProviderFailureAllowsAnotherAttempt() {
        assertTrue(retryableSendRejection("command_failed"))
        assertFalse(retryableSendRejection("command_outcome_unknown"))
        assertFalse(retryableSendRejection("invalid_command_attempt"))
    }

    private fun conversation(
        state: String,
        revision: Long = 1,
        id: UUID = UUID.fromString("11111111-1111-1111-1111-111111111111"),
        provider: ProviderId = ProviderId.CODEX,
        title: String = "Conversation",
        updatedAtMs: Long = 1,
    ) = Conversation(
        id = id,
        revision = revision,
        provider = provider,
        projectId = UUID.fromString("22222222-2222-2222-2222-222222222222"),
        nativeSessionId = "native-session",
        title = title,
        titleSource = "generated",
        titleUpdatedAtMs = 1,
        selectedModel = null,
        selectedEffort = null,
        state = state,
        sessionOptions = emptyList(),
        updatedAtMs = updatedAtMs,
    )

    private fun pending(
        startsConversation: Boolean = true,
        sentDraft: String = "sent text",
        sentAttachmentIds: Set<UUID> = emptySet(),
    ) = PendingSendContext(
        commandId = UUID.fromString("99999999-9999-9999-9999-999999999999"),
        clientMessageId = "client-message",
        frame = byteArrayOf(1),
        conversationId = UUID.fromString("11111111-1111-1111-1111-111111111111"),
        projectId = UUID.fromString("22222222-2222-2222-2222-222222222222"),
        provider = ProviderId.CODEX,
        startsConversation = startsConversation,
        sentDraft = sentDraft,
        sentAttachmentIds = sentAttachmentIds,
        attempt = 0,
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

    private fun timelineItem(id: String, revision: Long, content: TimelineContent) = TimelineItem(
        id = UUID.fromString(id),
        conversationId = UUID.fromString("11111111-1111-1111-1111-111111111111"),
        revision = revision,
        createdAtMs = UUID.fromString(id).mostSignificantBits,
        content = content,
    )
}
