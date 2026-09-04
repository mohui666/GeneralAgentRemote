package dev.agentremote.messenger.ui

import android.app.Application
import android.net.ConnectivityManager
import android.net.Network
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Log
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.agentremote.messenger.data.ClientStateStore
import dev.agentremote.messenger.data.CredentialStore
import dev.agentremote.messenger.data.DraftScope
import dev.agentremote.messenger.data.RemoteClient
import dev.agentremote.messenger.data.draftScope
import dev.agentremote.messenger.model.ConnectionTarget
import dev.agentremote.messenger.model.Conversation
import dev.agentremote.messenger.model.ProjectSummary
import dev.agentremote.messenger.model.ProjectTreeScope
import dev.agentremote.messenger.model.PromptAttachment
import dev.agentremote.messenger.model.ProviderCapability
import dev.agentremote.messenger.model.ProviderId
import dev.agentremote.messenger.model.ServerEvent
import dev.agentremote.messenger.model.Snapshot
import dev.agentremote.messenger.model.StoredCredential
import dev.agentremote.messenger.model.TimelineContent
import dev.agentremote.messenger.model.TimelineItem
import dev.agentremote.messenger.model.TimelinePageCursor
import dev.agentremote.messenger.protocol.WireProtocol
import java.util.UUID
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

enum class SendStatus {
    IDLE,
    SENDING,
    QUEUED,
    FAILED,
}

data class RemoteUiState(
    val phase: String = "未连接",
    val online: Boolean = false,
    val connecting: Boolean = false,
    val retryEnabled: Boolean = true,
    val reconnectAttempt: Int = 0,
    val pairLink: String = "",
    val credentials: List<StoredCredential> = emptyList(),
    val activeHostId: UUID? = null,
    val snapshot: Snapshot? = null,
    val timelineByConversation: Map<UUID, List<TimelineItem>> = emptyMap(),
    val selectedConversationId: UUID? = null,
    val showingNewConversation: Boolean = false,
    val selectedProjectId: UUID? = null,
    val selectedProvider: ProviderId? = null,
    val selectedModel: String? = null,
    val selectedEffort: String? = null,
    val selectedPermission: String? = null,
    val draft: String = "",
    val promptAttachments: List<PromptAttachment> = emptyList(),
    val attachments: Map<UUID, ByteArray> = emptyMap(),
    val pinnedProjects: Set<UUID> = emptySet(),
    val recentProjects: List<UUID> = emptyList(),
    val expandedProjectScopes: Set<ProjectTreeScope> = emptySet(),
    val projectExpansionInitialized: Boolean = false,
    val projectSearch: String = "",
    val historyBefore: Map<UUID, TimelinePageCursor> = emptyMap(),
    val historyExhausted: Set<UUID> = emptySet(),
    val pendingCommands: Set<UUID> = emptySet(),
    val pendingApprovals: Set<UUID> = emptySet(),
    val creatingConversation: Boolean = false,
    val sendStatus: SendStatus = SendStatus.IDLE,
    val sendFailure: String? = null,
    val error: String? = null,
)

internal data class PendingSendContext(
    val commandId: UUID,
    val clientMessageId: String,
    val frame: ByteArray?,
    val conversationId: UUID,
    val projectId: UUID,
    val provider: ProviderId,
    val startsConversation: Boolean,
    val sentDraft: String,
    val sentAttachmentIds: Set<UUID>,
    val attempt: Int? = null,
    val rejectedByHost: Boolean = false,
    val writeFailed: Boolean = false,
    val lastWrittenGeneration: Long? = null,
    val startedAtNanos: Long = System.nanoTime(),
) {
    fun replayed(generation: Long = 0L): PendingSendContext = copy(
        rejectedByHost = false,
        writeFailed = false,
        lastWrittenGeneration = generation,
    )

    fun rejected(): PendingSendContext = copy(rejectedByHost = true, writeFailed = false)

    val retryableFailure: Boolean get() = rejectedByHost || writeFailed
}

internal fun RemoteUiState.removeSentComposer(pending: PendingSendContext): RemoteUiState = copy(
    draft = if (draft == pending.sentDraft) "" else draft,
    promptAttachments = promptAttachments.filterNot { it.id in pending.sentAttachmentIds },
)

internal fun RemoteUiState.completePendingSend(
    pending: PendingSendContext,
    clearComposer: Boolean,
): RemoteUiState = (if (clearComposer) removeSentComposer(pending) else this).copy(
    pendingCommands = pendingCommands - pending.commandId,
    creatingConversation = false,
    selectedConversationId = if (clearComposer && pending.startsConversation) {
        pending.conversationId
    } else {
        selectedConversationId
    },
    showingNewConversation = if (clearComposer && pending.startsConversation) {
        false
    } else {
        showingNewConversation
    },
    sendStatus = SendStatus.IDLE,
    sendFailure = null,
)

internal fun pendingSendAllowsNavigation(pending: PendingSendContext?): Boolean =
    pending == null || pending.retryableFailure

internal fun retryableSendRejection(code: String): Boolean = code == "command_failed"

internal fun projectTreeScope(hostId: UUID, provider: ProviderId, projectId: UUID) =
    ProjectTreeScope(hostId, provider, projectId)

internal fun RemoteUiState.expandSelectedProject(): RemoteUiState {
    val hostId = snapshot?.hostId ?: activeHostId ?: return this
    val provider = selectedProvider ?: return this
    val projectId = selectedProjectId ?: return this
    return copy(
        expandedProjectScopes = expandedProjectScopes + projectTreeScope(hostId, provider, projectId),
        projectExpansionInitialized = true,
    )
}

internal fun RemoteUiState.ensureDefaultProjectExpanded(): RemoteUiState =
    if (projectExpansionInitialized) this else expandSelectedProject()

internal fun RemoteUiState.completeProjectSync(
    commandId: UUID,
    conversationsSynced: Int,
    fullHistoryFallback: Boolean,
): RemoteUiState = copy(
    phase = if (fullHistoryFallback) {
        "已同步 $conversationsSynced 个对话（全量去重）"
    } else {
        "已同步 $conversationsSynced 个对话"
    },
    pendingCommands = pendingCommands - commandId,
)

internal data class ProviderProjectSelection(
    val provider: ProviderId?,
    val projectId: UUID?,
)

private data class ConversationPageScope(
    val hostId: UUID,
    val provider: ProviderId,
    val projectId: UUID,
    val conversationId: UUID,
)

internal fun providerProjectSelection(
    projects: List<ProjectSummary>,
    selectedProvider: ProviderId?,
    selectedProjectId: UUID?,
    recentProjectIds: List<UUID> = emptyList(),
): ProviderProjectSelection {
    val selectedProject = projects.find { it.id == selectedProjectId }
    val provider = selectedProvider
        ?.takeIf { candidate -> projects.any { candidate in it.enabledProviders } }
        ?: selectedProject?.enabledProviders?.firstOrNull()
        ?: projects.firstNotNullOfOrNull { it.enabledProviders.firstOrNull() }
    val projectId = selectedProjectId?.takeIf { selected ->
        projects.any { project ->
            project.id == selected && (provider == null || provider in project.enabledProviders)
        }
    } ?: recentProjectIds.firstOrNull { recent ->
        projects.any { project ->
            project.id == recent && (provider == null || provider in project.enabledProviders)
        }
    } ?: projects.firstOrNull { provider == null || provider in it.enabledProviders }?.id
    return ProviderProjectSelection(provider, projectId)
}

internal fun retainedConversationSelection(
    conversations: List<Conversation>,
    selectedConversationId: UUID?,
    projectId: UUID?,
    provider: ProviderId?,
    preserveMissing: Boolean,
): UUID? {
    val selected = selectedConversationId ?: return null
    val conversation = conversations.find {
        it.id == selected && it.projectId == projectId && it.provider == provider
    }
    return when {
        conversation == null && preserveMissing -> selected
        conversation != null -> selected
        else -> null
    }
}

internal fun mergeConversation(
    current: List<Conversation>,
    incoming: Conversation,
): List<Conversation> {
    val existingIndex = current.indexOfFirst {
        it.id == incoming.id && it.projectId == incoming.projectId && it.provider == incoming.provider
    }
    if (existingIndex >= 0 && current[existingIndex].revision > incoming.revision) return current
    val updated = current.toMutableList()
    if (existingIndex >= 0) updated.removeAt(existingIndex)
    var low = 0
    var high = updated.size
    while (low < high) {
        val middle = (low + high) ushr 1
        if (conversationComparator.compare(updated[middle], incoming) <= 0) {
            low = middle + 1
        } else {
            high = middle
        }
    }
    updated.add(low, incoming)
    return updated
}

private val conversationComparator = compareByDescending<Conversation> { it.updatedAtMs }
    .thenBy { it.projectId }
    .thenBy { it.provider.wire }
    .thenBy { it.id }

private val timelineComparator = compareBy<TimelineItem> { it.createdAtMs }.thenBy { it.id }

internal fun mergeTimelinePage(
    current: List<TimelineItem>,
    incoming: List<TimelineItem>,
): List<TimelineItem> {
    if (incoming.isEmpty()) return current
    val merged = current.toMutableList()
    val positions = current.withIndex().associateTo(HashMap(current.size + incoming.size)) {
        (it.value.conversationId to it.value.id) to it.index
    }
    var changed = false
    incoming.forEach { item ->
        val key = item.conversationId to item.id
        val index = positions[key]
        if (index == null) {
            positions[key] = merged.size
            merged += item
            changed = true
        } else if (
            merged[index].revision < item.revision ||
            (merged[index].revision == item.revision && merged[index] != item)
        ) {
            merged[index] = item
            changed = true
        }
    }
    if (!changed) return current
    merged.sortWith(timelineComparator)
    return merged
}

internal fun indexTimelineByConversation(
    timeline: List<TimelineItem>,
): Map<UUID, List<TimelineItem>> = timeline.groupBy(TimelineItem::conversationId)

class RemoteViewModel(application: Application) : AndroidViewModel(application), RemoteClient.Listener {
    private val credentialStore = CredentialStore(application)
    private val clientStateStore = ClientStateStore(application)
    private val client = RemoteClient(this)
    private val connectivity = application.getSystemService(ConnectivityManager::class.java)
    private val initialCredentials = credentialStore.load()
    private val initialCredential = clientStateStore.lastHostId()
        ?.let { hostId -> initialCredentials.find { it.hostId == hostId } }
        ?: initialCredentials.firstOrNull()
    private val initialRestored = initialCredential?.let(clientStateStore::load)
    private var scopedDrafts = initialRestored?.drafts.orEmpty().toMutableMap()
    private val mutableState = MutableStateFlow(
        RemoteUiState(
            phase = if (initialRestored?.snapshot != null) "离线缓存 · 正在恢复连接" else "未连接",
            credentials = initialCredentials,
            activeHostId = initialCredential?.hostId,
            snapshot = initialRestored?.snapshot,
            timelineByConversation = initialRestored?.snapshot?.timeline
                ?.let(::indexTimelineByConversation)
                .orEmpty(),
            selectedConversationId = initialRestored?.selectedConversationId,
            selectedProjectId = initialRestored?.selectedProjectId,
            selectedProvider = initialRestored?.selectedProvider,
            selectedModel = initialRestored?.selectedModel,
            selectedEffort = initialRestored?.selectedEffort,
            selectedPermission = initialRestored?.selectedPermission,
            draft = initialRestored?.draft.orEmpty(),
            pinnedProjects = initialRestored?.pinnedProjects.orEmpty(),
            recentProjects = initialRestored?.recentProjects.orEmpty(),
            expandedProjectScopes = initialRestored?.expandedProjectScopes.orEmpty(),
            projectExpansionInitialized = initialRestored?.projectExpansionWasPersisted == true,
        ),
    )
    val state: StateFlow<RemoteUiState> = mutableState.asStateFlow()

    private var draftConversationId: UUID? = null
    private var pendingSend: PendingSendContext? = null
    private var awaitingAuthoritativeSnapshot = initialRestored?.snapshot != null
    private var snapshotPersistJob: Job? = null
    private var selectionPersistJob: Job? = null
    private val selectionPersistVersion = AtomicLong()
    private val selectionPersistLock = Any()
    private var latestSavedSelectionVersion = -1L
    private var activeConnectionGeneration: Long? = null
    private val projectSyncGeneration = mutableMapOf<ProjectTreeScope, Long>()
    private val projectSyncCommands = mutableMapOf<UUID, ProjectTreeScope>()
    private val projectRefreshGeneration = mutableMapOf<ProviderId, Long>()
    private val sendTraceStartedAtNanos = linkedMapOf<UUID, Long>()
    private val requestedConversationPages = mutableSetOf<ConversationPageScope>()
    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) = client.networkAvailable()
    }

    init {
        mutableState.update {
            restoreDraft(
                defaults(it, preserveMissingConversation = true).ensureDefaultProjectExpanded(),
            )
        }
        connectivity.registerDefaultNetworkCallback(networkCallback)
        initialCredential?.let(::connect)
    }

    fun setPairLink(value: String) = mutableState.update { it.copy(pairLink = value) }

    fun reportError(message: String) = showError(message)

    fun pair() {
        if (state.value.connecting || !pendingSendAllowsNavigation(pendingSend)) return
        val target = runCatching { WireProtocol.parsePairLink(state.value.pairLink) }
            .getOrElse { error ->
                showError(error.message ?: "配对链接无效")
                return
            }
        rememberDraft(state.value)
        flushState()
        scopedDrafts = mutableMapOf()
        pendingSend = null
        draftConversationId = null
        activeConnectionGeneration = null
        projectSyncGeneration.clear()
        projectSyncCommands.clear()
        projectRefreshGeneration.clear()
        sendTraceStartedAtNanos.clear()
        requestedConversationPages.clear()
        awaitingAuthoritativeSnapshot = false
        snapshotPersistJob?.cancel()
        selectionPersistJob?.cancel()
        mutableState.update {
            it.copy(
                error = null,
                snapshot = null,
                timelineByConversation = emptyMap(),
                activeHostId = target.hostId,
                selectedConversationId = null,
                draft = "",
                attachments = emptyMap(),
                promptAttachments = emptyList(),
                expandedProjectScopes = emptySet(),
                projectExpansionInitialized = false,
                sendStatus = SendStatus.IDLE,
                sendFailure = null,
                connecting = true,
                retryEnabled = true,
            )
        }
        clientStateStore.saveLastHost(target.hostId)
        client.connect(target)
    }

    fun connect(credential: StoredCredential) {
        if (!pendingSendAllowsNavigation(pendingSend)) return
        if (state.value.connecting && state.value.activeHostId == credential.hostId) return
        rememberDraft(state.value)
        flushState()
        if (state.value.activeHostId != credential.hostId) {
            pendingSend = null
            draftConversationId = null
            sendTraceStartedAtNanos.clear()
        }
        activeConnectionGeneration = null
        projectSyncGeneration.clear()
        projectSyncCommands.clear()
        projectRefreshGeneration.clear()
        requestedConversationPages.clear()
        val restored = clientStateStore.load(credential)
        scopedDrafts = restored.drafts.toMutableMap()
        awaitingAuthoritativeSnapshot = restored.snapshot != null
        snapshotPersistJob?.cancel()
        selectionPersistJob?.cancel()
        mutableState.update {
            restoreDraft(
                defaults(
                it.copy(
                    phase = if (restored.snapshot != null) "离线缓存 · 正在恢复连接" else "正在连接",
                    error = null,
                    snapshot = restored.snapshot,
                    timelineByConversation = restored.snapshot?.timeline
                        ?.let(::indexTimelineByConversation)
                        .orEmpty(),
                    activeHostId = credential.hostId,
                    selectedConversationId = restored.selectedConversationId,
                    selectedProjectId = restored.selectedProjectId,
                    selectedProvider = restored.selectedProvider,
                    selectedModel = restored.selectedModel,
                    selectedEffort = restored.selectedEffort,
                    selectedPermission = restored.selectedPermission,
                    draft = restored.draft,
                    attachments = emptyMap(),
                    promptAttachments = emptyList(),
                    pinnedProjects = restored.pinnedProjects,
                    recentProjects = restored.recentProjects,
                    expandedProjectScopes = restored.expandedProjectScopes,
                    projectExpansionInitialized = restored.projectExpansionWasPersisted,
                    connecting = true,
                    retryEnabled = true,
                    reconnectAttempt = 0,
                    historyBefore = emptyMap(),
                    historyExhausted = emptySet(),
                    pendingCommands = emptySet(),
                    pendingApprovals = emptySet(),
                    sendStatus = SendStatus.IDLE,
                    sendFailure = null,
                ),
                    preserveMissingConversation = true,
                ).ensureDefaultProjectExpanded(),
            )
        }
        clientStateStore.saveLastHost(credential.hostId)
        client.connect(
            ConnectionTarget(
                hostId = credential.hostId,
                origin = credential.origin,
                relay = credential.relay,
                credential = credential,
            ),
        )
    }

    fun disconnect() {
        client.disconnect()
        val pending = pendingSend
        mutableState.update {
            it.copy(
                phase = "离线 · 已手动断开",
                online = false,
                connecting = false,
                retryEnabled = false,
                pendingCommands = pending
                    ?.takeUnless(PendingSendContext::retryableFailure)
                    ?.commandId
                    ?.let(::setOf)
                    .orEmpty(),
                pendingApprovals = emptySet(),
                creatingConversation = pending?.startsConversation == true && draftConversationId != null,
            )
        }
        persistSelection()
    }

    fun retryNow() {
        mutableState.update { it.copy(retryEnabled = true, reconnectAttempt = 0, error = null) }
        client.retryNow()
    }

    fun stopRetrying() {
        client.stopRetrying()
        mutableState.update { it.copy(retryEnabled = false) }
    }

    fun forget(credential: StoredCredential) {
        if (
            state.value.activeHostId == credential.hostId &&
            !pendingSendAllowsNavigation(pendingSend)
        ) {
            return
        }
        if (state.value.activeHostId == credential.hostId) {
            client.forgetTarget()
            scopedDrafts = mutableMapOf()
            mutableState.update {
                RemoteUiState(credentials = it.credentials.filterNot { item -> item.hostId == credential.hostId })
            }
        }
        val remaining = state.value.credentials.filterNot { it.hostId == credential.hostId }
        credentialStore.save(remaining)
        clientStateStore.clear(credential.hostId)
        mutableState.update { it.copy(credentials = remaining) }
    }

    fun showNewConversation() {
        if (!pendingSendAllowsNavigation(pendingSend)) return
        abandonRetryableSendForNavigation()
        draftConversationId = UUID.randomUUID()
        updateDraftScope {
            defaults(
                it.copy(
                    showingNewConversation = true,
                    selectedConversationId = null,
                    attachments = emptyMap(),
                    promptAttachments = emptyList(),
                ),
            ).expandSelectedProject()
        }
        persistSelection()
    }

    fun showConversationList() {
        if (!pendingSendAllowsNavigation(pendingSend)) return
        abandonRetryableSendForNavigation()
        draftConversationId = null
        updateDraftScope {
            it.copy(
                showingNewConversation = false,
                selectedConversationId = null,
                attachments = emptyMap(),
                promptAttachments = emptyList(),
            )
        }
        persistSelection()
    }

    fun selectConversation(id: UUID) {
        val current = state.value
        val projectId = current.selectedProjectId ?: return
        val provider = current.selectedProvider ?: return
        selectConversation(projectId, provider, id)
    }

    fun selectConversation(projectId: UUID, provider: ProviderId, id: UUID) {
        if (!pendingSendAllowsNavigation(pendingSend)) return
        val conversation = state.value.snapshot?.conversations?.find {
            it.id == id && it.projectId == projectId && it.provider == provider
        } ?: return
        val project = state.value.snapshot?.projects?.find {
            it.id == projectId && it.valid && provider in it.enabledProviders
        } ?: return
        abandonRetryableSendForNavigation()
        draftConversationId = null
        updateDraftScope { current ->
            defaults(
                current.copy(
                    selectedProjectId = project.id,
                    selectedProvider = provider,
                    selectedConversationId = id,
                    showingNewConversation = false,
                    attachments = emptyMap(),
                    promptAttachments = emptyList(),
                    recentProjects = (listOf(project.id) + current.recentProjects.filterNot { it == project.id }).take(8),
                ),
            ).expandSelectedProject()
        }
        persistSelection()
        requestInitialConversationPage(conversation)
        requestConversationImages(id)
    }

    fun selectProject(id: UUID) {
        if (!pendingSendAllowsNavigation(pendingSend)) return
        val before = state.value
        val provider = before.selectedProvider ?: return
        val projectIsAuthorized = before.snapshot?.projects?.any {
            it.id == id && it.valid && provider in it.enabledProviders
        } == true
        if (!projectIsAuthorized) return
        abandonRetryableSendForNavigation()
        updateDraftScope { current ->
            defaults(
                current.copy(
                    selectedProjectId = id,
                    selectedConversationId = null,
                    selectedModel = null,
                    selectedEffort = null,
                    selectedPermission = null,
                    promptAttachments = emptyList(),
                    recentProjects = (listOf(id) + current.recentProjects.filterNot { it == id }).take(8),
                ),
            ).expandSelectedProject()
        }
        persistSelection()
        if (before.selectedProjectId != id) syncSelectedProject()
    }

    fun selectProvider(provider: ProviderId) {
        if (!pendingSendAllowsNavigation(pendingSend)) return
        val before = state.value
        if (before.selectedProvider == provider) return
        val providerAvailable = before.snapshot?.projects?.any {
            it.valid && provider in it.enabledProviders
        } == true
        if (!providerAvailable) return
        abandonRetryableSendForNavigation()
        updateDraftScope { current ->
            val updated = defaults(
                current.copy(
                    selectedProvider = provider,
                    selectedConversationId = null,
                    selectedModel = null,
                    selectedEffort = null,
                    selectedPermission = null,
                    promptAttachments = emptyList(),
                ),
            ).expandSelectedProject()
            updated.selectedProjectId?.let { selectedProjectId ->
                updated.copy(
                    recentProjects = (
                        listOf(selectedProjectId) + updated.recentProjects.filterNot { it == selectedProjectId }
                        ).take(8),
                )
            } ?: updated
        }
        persistSelection()
        if (state.value.online) {
            refreshProjects(provider)
            syncSelectedProject()
        }
    }

    fun selectModel(model: String?) {
        mutableState.update { current ->
            val selected = selectedCapability(current)?.models?.find { it.id == model }
            current.copy(
                selectedModel = model,
                selectedEffort = selected?.defaultEffort ?: selected?.effortOptions?.firstOrNull()?.id,
            )
        }
        persistSelection()
    }

    fun selectEffort(effort: String?) {
        mutableState.update { it.copy(selectedEffort = effort) }
        persistSelection()
    }

    fun selectPermission(permission: String?) {
        mutableState.update { it.copy(selectedPermission = permission) }
        persistSelection()
    }

    fun setProjectSearch(value: String) = mutableState.update { it.copy(projectSearch = value) }

    fun toggleProjectExpanded(id: UUID) {
        val current = state.value
        val hostId = current.snapshot?.hostId ?: return
        val provider = current.selectedProvider ?: return
        if (current.snapshot.projects.none { it.id == id && it.valid && provider in it.enabledProviders }) return
        val scope = projectTreeScope(hostId, provider, id)
        mutableState.update {
            it.copy(
                expandedProjectScopes = if (scope in it.expandedProjectScopes) {
                    it.expandedProjectScopes - scope
                } else {
                    it.expandedProjectScopes + scope
                },
                projectExpansionInitialized = true,
            )
        }
        persistSelection()
    }

    fun toggleProjectPin(id: UUID) {
        mutableState.update {
            it.copy(
                pinnedProjects = if (id in it.pinnedProjects) it.pinnedProjects - id else it.pinnedProjects + id,
            )
        }
        persistSelection()
    }

    private fun RemoteUiState.activeDraftScope(): DraftScope? = draftScope(
        hostId = snapshot?.hostId ?: activeHostId ?: return null,
        provider = selectedProvider,
        projectId = selectedProjectId,
        conversationId = selectedConversationId.takeUnless { showingNewConversation },
    )

    private fun rememberDraft(current: RemoteUiState) {
        val scope = current.activeDraftScope() ?: return
        if (current.draft.isEmpty()) {
            scopedDrafts.remove(scope)
        } else {
            scopedDrafts[scope] = current.draft
        }
    }

    private fun restoreDraft(current: RemoteUiState): RemoteUiState = current.copy(
        draft = current.activeDraftScope()?.let(scopedDrafts::get).orEmpty(),
    )

    private fun updateDraftScope(transform: (RemoteUiState) -> RemoteUiState) {
        mutableState.update { current ->
            rememberDraft(current)
            val previousScope = current.activeDraftScope()
            val updated = transform(current)
            if (updated.activeDraftScope() == previousScope) updated else restoreDraft(updated)
        }
    }

    fun setDraft(value: String) {
        state.value.activeDraftScope()?.let { scope ->
            if (value.isEmpty()) scopedDrafts.remove(scope) else scopedDrafts[scope] = value
        }
        mutableState.update { it.copy(draft = value) }
        persistSelection(debounce = true)
    }

    fun addPromptAttachments(uris: List<Uri>) {
        val capability = selectedCapability(state.value)?.attachments ?: return
        val available = (capability.maxCount - state.value.promptAttachments.size).coerceAtLeast(0)
        val existingBytes = state.value.promptAttachments.sumOf { it.bytes.size.toLong() }
        viewModelScope.launch {
            val loaded = withContext(Dispatchers.IO) {
                var totalBytes = existingBytes
                uris.take(available).map { uri ->
                    val resolver = getApplication<Application>().contentResolver
                    val mimeType = resolver.getType(uri).orEmpty()
                    val bytes = requireNotNull(resolver.openInputStream(uri)).use { it.readBytes() }
                    require(mimeType in capability.allowedMimeTypes) { "当前 Provider 不支持 $mimeType" }
                    require(bytes.size.toLong() <= capability.maxBytes) {
                        "附件超过 ${capability.maxBytes / 1024 / 1024} MiB 限制"
                    }
                    require(totalBytes + bytes.size <= capability.maxTotalBytes) {
                        "附件总大小超过 ${capability.maxTotalBytes / 1024 / 1024} MiB 限制"
                    }
                    totalBytes += bytes.size
                    PromptAttachment(UUID.randomUUID(), attachmentName(uri), mimeType, bytes)
                }
            }
            mutableState.update { it.copy(promptAttachments = it.promptAttachments + loaded) }
        }.invokeOnCompletion { error ->
            if (error != null) onMain { showError(error.message ?: "读取附件失败") }
        }
    }

    fun removePromptAttachment(id: UUID) = mutableState.update {
        it.copy(promptAttachments = it.promptAttachments.filterNot { attachment -> attachment.id == id })
    }

    fun sendMessage() {
        val current = state.value
        if (pendingSend != null) return
        val text = current.draft.trim()
        if (text.isEmpty()) return
        val projectId = current.selectedProjectId ?: return
        val provider = current.selectedProvider ?: return
        val startsConversation = current.selectedConversationId == null
        val conversationId = current.selectedConversationId ?: run {
            if (!current.showingNewConversation) return
            draftConversationId ?: UUID.randomUUID().also { draftConversationId = it }
        }
        val existingConversationMatchesScope = current.snapshot?.conversations?.any {
            it.id == conversationId && it.projectId == projectId && it.provider == provider
        } == true
        if (!startsConversation && !existingConversationMatchesScope) return
        val commandId = UUID.randomUUID()
        val clientMessageId = UUID.randomUUID().toString()
        val startedAtNanos = System.nanoTime()
        mutableState.update {
            it.copy(sendStatus = SendStatus.SENDING, sendFailure = null, error = null)
        }
        rememberSendTrace(commandId, startedAtNanos)
        logLocalSendTrace(commandId, clientMessageId, conversationId, "click", startedAtNanos)
        pendingSend = PendingSendContext(
            commandId = commandId,
            clientMessageId = clientMessageId,
            frame = null,
            conversationId = conversationId,
            projectId = projectId,
            provider = provider,
            startsConversation = startsConversation,
            sentDraft = current.draft,
            sentAttachmentIds = current.promptAttachments.mapTo(mutableSetOf(), PromptAttachment::id),
            attempt = 0,
            startedAtNanos = startedAtNanos,
        )
        logLocalSendTrace(commandId, clientMessageId, conversationId, "local_pending", startedAtNanos)
        viewModelScope.launch {
            val bytes = withContext(Dispatchers.Default) {
                if (!startsConversation) {
                    WireProtocol.sendMessage(
                        commandId = commandId,
                        conversationId = conversationId,
                        clientMessageId = clientMessageId,
                        text = text,
                        attachments = current.promptAttachments,
                        attempt = 0,
                    )
                } else {
                    WireProtocol.startConversation(
                        commandId = commandId,
                        clientMessageId = clientMessageId,
                        conversationId = conversationId,
                        projectId = projectId,
                        provider = provider,
                        model = current.selectedModel,
                        effort = current.selectedEffort,
                        permissionMode = current.selectedPermission,
                        text = text,
                        attachments = current.promptAttachments,
                        attempt = 0,
                    )
                }
            }
            val pending = pendingSend?.takeIf { it.commandId == commandId } ?: return@launch
            pendingSend = pending.copy(frame = bytes)
            writePendingSend(isReplay = false)
        }
    }

    fun steer() {
        val current = state.value
        val conversationId = current.selectedConversationId ?: return
        val projectId = current.selectedProjectId ?: return
        val provider = current.selectedProvider ?: return
        val conversationMatchesScope = current.snapshot?.conversations?.any {
            it.id == conversationId && it.projectId == projectId && it.provider == provider
        } == true
        if (!conversationMatchesScope) return
        if (pendingSend != null) return
        val text = current.draft.trim()
        if (text.isEmpty()) return
        val commandId = UUID.randomUUID()
        val clientMessageId = commandId.toString()
        val startedAtNanos = System.nanoTime()
        mutableState.update {
            it.copy(sendStatus = SendStatus.SENDING, sendFailure = null, error = null)
        }
        rememberSendTrace(commandId, startedAtNanos)
        logLocalSendTrace(commandId, clientMessageId, conversationId, "click", startedAtNanos)
        val bytes = WireProtocol.steer(commandId, conversationId, text)
        pendingSend = PendingSendContext(
            commandId = commandId,
            clientMessageId = clientMessageId,
            frame = bytes,
            conversationId = conversationId,
            projectId = projectId,
            provider = provider,
            startsConversation = false,
            sentDraft = current.draft,
            sentAttachmentIds = emptySet(),
            startedAtNanos = startedAtNanos,
        )
        logLocalSendTrace(commandId, clientMessageId, conversationId, "local_pending", startedAtNanos)
        writePendingSend(isReplay = false)
    }

    fun retryPendingSend() {
        val existing = pendingSend ?: return
        if (!existing.retryableFailure) return
        val originalFrame = existing.frame ?: return
        val startedAtNanos = System.nanoTime()
        val nextAttempt = existing.attempt?.let { attempt ->
            require(attempt < Int.MAX_VALUE) { "发送重试次数过多" }
            attempt + 1
        }
        val pending = existing.copy(
            frame = if (nextAttempt == null) existing.frame else null,
            attempt = nextAttempt,
            rejectedByHost = false,
            writeFailed = false,
            lastWrittenGeneration = null,
            startedAtNanos = startedAtNanos,
        )
        pendingSend = pending
        mutableState.update {
            it.copy(sendStatus = SendStatus.SENDING, sendFailure = null, error = null)
        }
        rememberSendTrace(pending.commandId, startedAtNanos)
        logLocalSendTrace(
            pending.commandId,
            pending.clientMessageId,
            pending.conversationId,
            "click",
            startedAtNanos,
        )
        logLocalSendTrace(
            pending.commandId,
            pending.clientMessageId,
            pending.conversationId,
            "local_pending",
            startedAtNanos,
        )
        if (nextAttempt == null) {
            writePendingSend(isReplay = true)
            return
        }
        viewModelScope.launch {
            val retriedFrame = withContext(Dispatchers.Default) {
                WireProtocol.withSendAttempt(originalFrame, nextAttempt)
            }
            val active = pendingSend?.takeIf {
                it.commandId == pending.commandId && it.attempt == nextAttempt
            } ?: return@launch
            pendingSend = active.copy(frame = retriedFrame)
            writePendingSend(isReplay = true)
        }
    }

    fun interrupt() {
        val conversationId = state.value.selectedConversationId ?: return
        val commandId = UUID.randomUUID()
        queue(commandId, WireProtocol.interrupt(commandId, conversationId))
    }

    fun resolveApproval(approvalId: UUID, optionId: String) {
        if (approvalId in state.value.pendingApprovals) return
        val commandId = UUID.randomUUID()
        if (queue(commandId, WireProtocol.resolveApproval(commandId, approvalId, optionId))) {
            mutableState.update { it.copy(pendingApprovals = it.pendingApprovals + approvalId) }
        }
    }

    fun setSessionOption(optionId: String, value: String) {
        val conversationId = state.value.selectedConversationId ?: return
        val commandId = UUID.randomUUID()
        queue(commandId, WireProtocol.setSessionOption(commandId, conversationId, optionId, value))
    }

    fun renameConversation(title: String) {
        val conversationId = state.value.selectedConversationId ?: return
        val clean = title.trim()
        if (clean.isEmpty()) return
        val commandId = UUID.randomUUID()
        queue(commandId, WireProtocol.renameConversation(commandId, conversationId, clean))
    }

    fun loadOlder() {
        val current = state.value
        val conversationId = current.selectedConversationId ?: return
        if (conversationId in current.historyExhausted) return
        val before = current.historyBefore[conversationId]
            ?: current.snapshot?.timeline
                ?.filter { it.conversationId == conversationId }
                ?.minWithOrNull(compareBy<TimelineItem> { it.createdAtMs }.thenBy { it.id })
                ?.let { TimelinePageCursor(it.createdAtMs, it.id) }
        if (!client.send(
            WireProtocol.getConversationPage(
                conversationId,
                before,
                100,
            ),
        )) {
            showError("历史记录请求未能写入 WebSocket")
        }
    }

    fun clearError() = mutableState.update { it.copy(error = null) }

    override fun onConnecting(target: ConnectionTarget) = onMain {
        activeConnectionGeneration = null
        mutableState.update {
            it.copy(
                phase = "正在连接 ${target.origin}",
                online = false,
                connecting = true,
                error = null,
            )
        }
    }

    override fun onConnected(target: ConnectionTarget) = onMain {
        mutableState.update {
            it.copy(
                phase = if (target.pairToken != null) "正在配对" else "正在认证",
                online = false,
            )
        }
    }

    override fun onEvent(event: ServerEvent, connectionGeneration: Long) = onMain {
        val belongsToActiveConnection = when (event) {
            is ServerEvent.Paired -> client.targets(event.credential)
            else -> client.isCurrent(connectionGeneration)
        }
        if (!belongsToActiveConnection) return@onMain
        activeConnectionGeneration = connectionGeneration
        var selectionNeedsPersist = false
        when (event) {
            is ServerEvent.Paired -> {
                upsertCredential(event.credential)
                mutableState.update { it.copy(phase = "配对成功，正在同步", pairLink = "") }
            }
            is ServerEvent.Authenticated -> mutableState.update { it.copy(phase = "认证成功，正在同步") }
            is ServerEvent.SnapshotReceived -> {
                awaitingAuthoritativeSnapshot = false
                snapshotPersistJob?.cancel()
                snapshotPersistJob = viewModelScope.launch(Dispatchers.IO) {
                    clientStateStore.saveSnapshot(event.snapshot.hostId, event.encoded)
                }
                if (client.isConnected(connectionGeneration)) {
                    applySnapshot(event.snapshot)
                    reconcilePendingSend(event.snapshot.conversations.findPendingConversation())
                    state.value.selectedProvider?.let(::refreshProjects)
                    replayPendingSend()
                    selectionNeedsPersist = true
                }
            }
            is ServerEvent.ProjectsUpdated -> {
                applyProjects(event)
                scheduleSnapshotPersist()
                selectionNeedsPersist = true
            }
            is ServerEvent.ProjectSyncCompleted -> {
                projectSyncCommands.remove(event.commandId)
                mutableState.update {
                    it.completeProjectSync(
                        event.commandId,
                        event.conversationsSynced,
                        event.fullHistoryFallback,
                    )
                }
            }
            is ServerEvent.ConversationPage -> {
                mutableState.update { current ->
                    val snapshot = current.snapshot ?: return@update current
                    val pageItems = event.items.filter { it.conversationId == event.conversationId }
                    val timeline = mergeTimelinePage(snapshot.timeline, pageItems)
                    val conversationTimeline = mergeTimelinePage(
                        current.timelineByConversation[event.conversationId].orEmpty(),
                        pageItems,
                    )
                    if (event.nextBefore == null) {
                        current.copy(
                            snapshot = snapshot.copy(timeline = timeline),
                            timelineByConversation = current.timelineByConversation +
                                (event.conversationId to conversationTimeline),
                            historyExhausted = current.historyExhausted + event.conversationId,
                        )
                    } else {
                        current.copy(
                            snapshot = snapshot.copy(timeline = timeline),
                            timelineByConversation = current.timelineByConversation +
                                (event.conversationId to conversationTimeline),
                            historyBefore = current.historyBefore + (event.conversationId to event.nextBefore),
                        )
                    }
                }
                scheduleSnapshotPersist()
            }
            is ServerEvent.ProviderChanged -> {
                mutateSnapshot { snapshot ->
                    snapshot.copy(
                        providerCapabilities = replaceCapability(snapshot.providerCapabilities, event.capability),
                    )
                }
                scheduleSnapshotPersist()
                selectionNeedsPersist = true
            }
            is ServerEvent.ConversationUpserted -> {
                val activatesDraft = event.conversation.id == draftConversationId &&
                    event.conversation.projectId == state.value.selectedProjectId &&
                    event.conversation.provider == state.value.selectedProvider
                mutateSnapshot { snapshot ->
                    val conversations = mergeConversation(snapshot.conversations, event.conversation)
                    snapshot.copy(
                        projects = snapshot.projects.map { project ->
                            if (project.id != event.conversation.projectId) {
                                project
                            } else {
                                val projectConversations = conversations.filter {
                                    it.projectId == project.id
                                }
                                project.copy(
                                    conversationCount = projectConversations.size,
                                    lastActivityAtMs = projectConversations.maxOfOrNull(Conversation::updatedAtMs),
                                )
                            }
                        },
                        conversations = conversations,
                    )
                }
                if (activatesDraft) {
                    draftConversationId = null
                    mutableState.update { current ->
                        rememberDraft(current)
                        val previousScope = current.activeDraftScope()
                        val updated = current.copy(
                            selectedConversationId = event.conversation.id,
                            showingNewConversation = false,
                            creatingConversation = false,
                        )
                        if (previousScope != updated.activeDraftScope()) {
                            previousScope?.let(scopedDrafts::remove)
                            rememberDraft(updated)
                        }
                        updated
                    }
                }
                reconcilePendingSend(
                    state.value.snapshot?.conversations?.findPendingConversation(),
                )
                scheduleSnapshotPersist()
                selectionNeedsPersist = activatesDraft
            }
            is ServerEvent.TimelineUpserted -> {
                mutableState.update { current ->
                    val snapshot = current.snapshot ?: return@update current
                    current.copy(
                        snapshot = snapshot.copy(timeline = upsertTimeline(snapshot.timeline, event.item)),
                        timelineByConversation = current.timelineByConversation + (
                            event.item.conversationId to upsertTimeline(
                                current.timelineByConversation[event.item.conversationId].orEmpty(),
                                event.item,
                            )
                            ),
                    )
                }
                val approval = event.item.content as? TimelineContent.Approval
                if (approval?.resolvedOption != null) {
                    mutableState.update {
                        it.copy(pendingApprovals = it.pendingApprovals - approval.approvalId)
                    }
                }
                if (event.item.conversationId == state.value.selectedConversationId) requestImage(event.item)
                scheduleSnapshotPersist()
            }
            is ServerEvent.ConversationRemoved -> {
                val removedSelection = state.value.selectedConversationId == event.conversationId
                mutateSnapshot { snapshot ->
                    val removed = snapshot.conversations.find { it.id == event.conversationId }
                    val conversations = snapshot.conversations.filterNot { it.id == event.conversationId }
                    snapshot.copy(
                        projects = snapshot.projects.map { project ->
                            if (project.id != removed?.projectId) {
                                project
                            } else {
                                val projectConversations = conversations.filter { it.projectId == project.id }
                                project.copy(
                                    conversationCount = projectConversations.size,
                                    lastActivityAtMs = projectConversations.maxOfOrNull(Conversation::updatedAtMs),
                                )
                            }
                        },
                        conversations = conversations,
                        timeline = snapshot.timeline.filterNot { it.conversationId == event.conversationId },
                    )
                }
                mutableState.update {
                    it.copy(timelineByConversation = it.timelineByConversation - event.conversationId)
                }
                scheduleSnapshotPersist()
                selectionNeedsPersist = removedSelection
            }
            is ServerEvent.AttachmentReceived -> mutableState.update { current ->
                if (event.attachment.conversationId != current.selectedConversationId) current else {
                    current.copy(
                        attachments = current.attachments + (event.attachment.id to event.attachment.bytes),
                    )
                }
            }
            is ServerEvent.HostStatus -> if (client.isConnected(connectionGeneration)) {
                mutableState.update {
                    it.copy(
                        online = event.online,
                        phase = if (event.online) "Host 在线" else "Host 离线",
                        error = event.message?.takeIf { message -> !event.online && message.isNotBlank() },
                    )
                }
            }
            is ServerEvent.SendTrace -> logServerSendTrace(event)
            is ServerEvent.CommandAccepted -> {
                val pending = pendingSend
                if (pending?.commandId == event.commandId) {
                    finishPendingSend(clearComposer = true)
                    selectionNeedsPersist = true
                } else {
                    mutableState.update { it.copy(pendingCommands = it.pendingCommands - event.commandId) }
                }
            }
            is ServerEvent.CommandRejected -> {
                event.commandId?.let { rejected ->
                    sendTraceStartedAtNanos.remove(rejected)
                    projectSyncCommands.remove(rejected)?.let { scope ->
                        projectSyncGeneration.remove(scope)
                    }
                    mutableState.update { it.copy(pendingCommands = it.pendingCommands - rejected) }
                    pendingSend?.takeIf { it.commandId == rejected }?.let { pending ->
                        if (pending.attempt != null && retryableSendRejection(event.code)) {
                            pendingSend = pending.rejected()
                            mutableState.update {
                                it.copy(
                                    sendStatus = SendStatus.FAILED,
                                    sendFailure = "Host 拒绝发送，草稿已保留",
                                    creatingConversation = false,
                                )
                            }
                        } else {
                            finishPendingSend(clearComposer = false)
                        }
                    }
                }
                mutableState.update {
                    it.copy(pendingApprovals = emptySet(), creatingConversation = false)
                }
                showError("${event.code}: ${event.message}")
                if (event.code == "authentication_failed") {
                    finishPendingSend(clearComposer = false)
                    val credential = state.value.credentials.find { it.hostId == state.value.activeHostId }
                    if (credential != null) {
                        forget(credential)
                    } else {
                        client.forgetTarget()
                        draftConversationId = null
                        mutableState.update {
                            RemoteUiState(
                                phase = "配对失败",
                                pairLink = it.pairLink,
                                credentials = it.credentials,
                                error = it.error,
                            )
                        }
                    }
                } else if (event.code == "authentication_rate_limited") {
                    finishPendingSend(clearComposer = false)
                    client.disconnect()
                    mutableState.update {
                        it.copy(
                            phase = "认证受限 · 请稍后重试",
                            online = false,
                            connecting = false,
                            retryEnabled = false,
                            pendingCommands = emptySet(),
                            pendingApprovals = emptySet(),
                            creatingConversation = false,
                        )
                    }
                }
            }
            is ServerEvent.ProtocolError -> showError(
                "协议错误：${event.message}（Host 支持 v${event.supportedVersion}）",
            )
        }
        if (selectionNeedsPersist) persistSelection()
    }

    override fun onDisconnected(message: String) = onMain {
        activeConnectionGeneration = null
        projectSyncGeneration.clear()
        projectSyncCommands.clear()
        projectRefreshGeneration.clear()
        requestedConversationPages.clear()
        val pending = pendingSend
        mutableState.update {
            it.copy(
                phase = "连接已断开，等待重连",
                online = false,
                connecting = false,
                pendingCommands = pending
                    ?.takeUnless(PendingSendContext::retryableFailure)
                    ?.commandId
                    ?.let(::setOf)
                    .orEmpty(),
                pendingApprovals = emptySet(),
                creatingConversation = pending?.startsConversation == true && draftConversationId != null,
                sendStatus = when {
                    pending == null -> SendStatus.IDLE
                    pending.retryableFailure -> SendStatus.FAILED
                    else -> SendStatus.QUEUED
                },
                sendFailure = pending?.takeIf(PendingSendContext::retryableFailure)?.let {
                    if (it.rejectedByHost) {
                        "Host 拒绝发送，草稿已保留"
                    } else {
                        "WebSocket 写入失败，草稿和附件已保留"
                    }
                },
                error = message,
            )
        }
    }

    override fun onRetryScheduled(attempt: Int, delayMillis: Long) = onMain {
        mutableState.update {
            it.copy(
                phase = "第 $attempt 次重连 · ${"%.1f".format(delayMillis / 1_000.0)} 秒后",
                retryEnabled = true,
                reconnectAttempt = attempt,
            )
        }
    }

    override fun onRetryStopped(message: String) = onMain {
        mutableState.update { it.copy(phase = "离线 · $message", retryEnabled = false) }
    }

    override fun onError(message: String) = onMain { showError(message) }

    override fun onCleared() {
        flushState()
        connectivity.unregisterNetworkCallback(networkCallback)
        client.close()
        super.onCleared()
    }

    fun flushState() {
        selectionPersistJob?.cancel()
        persistSelectionSnapshot(
            current = state.value,
            drafts = scopedDrafts.toMap(),
            commit = true,
            version = selectionPersistVersion.incrementAndGet(),
        )
    }

    private fun applySnapshot(snapshot: Snapshot) {
        val active = state.value.credentials.find { it.hostId == snapshot.hostId }
        if (active != null && active.displayName != snapshot.hostName) {
            upsertCredential(active.copy(displayName = snapshot.hostName))
        }
        val sortedTimeline = snapshot.timeline.sortedWith(timelineComparator)
        updateDraftScope { current ->
            defaults(
                current.copy(
                    phase = "已连接 ${snapshot.hostName}",
                    online = true,
                    connecting = false,
                    retryEnabled = true,
                    reconnectAttempt = 0,
                    snapshot = snapshot.copy(
                        conversations = snapshot.conversations.sortedWith(conversationComparator),
                        timeline = sortedTimeline,
                    ),
                    timelineByConversation = indexTimelineByConversation(sortedTimeline),
                    attachments = emptyMap(),
                    historyBefore = emptyMap(),
                    historyExhausted = emptySet(),
                    pendingCommands = emptySet(),
                    pendingApprovals = emptySet(),
                    creatingConversation = false,
                    error = null,
                ),
            ).ensureDefaultProjectExpanded()
        }
        val selectedConversation = state.value.snapshot?.conversations?.find {
            it.id == state.value.selectedConversationId &&
                it.projectId == state.value.selectedProjectId &&
                it.provider == state.value.selectedProvider
        }
        selectedConversation?.let(::requestInitialConversationPage)
        selectedConversation?.id?.let(::requestConversationImages)
    }

    private fun applyProjects(event: ServerEvent.ProjectsUpdated) {
        mutateSnapshot { snapshot ->
            val incomingIds = event.projects.map(ProjectSummary::id).toSet()
            val projects = snapshot.projects.map { project ->
                if (event.provider in project.enabledProviders && project.id !in incomingIds) {
                    project.copy(enabledProviders = project.enabledProviders - event.provider)
                } else {
                    project
                }
            }.toMutableList()
            event.projects.forEach { incoming ->
                val index = projects.indexOfFirst { it.id == incoming.id }
                if (index >= 0) projects[index] = incoming else projects += incoming
            }
            snapshot.copy(
                projects = projects.sortedWith(
                    compareByDescending<ProjectSummary> { it.lastActivityAtMs }
                        .thenBy(ProjectSummary::displayName),
                ),
                providerCapabilities = event.capabilities.fold(
                    snapshot.providerCapabilities.filterNot { it.provider == event.provider },
                    ::replaceCapability,
                ),
            )
        }
    }

    private fun defaults(
        current: RemoteUiState,
        preserveMissingConversation: Boolean = false,
    ): RemoteUiState {
        val snapshot = current.snapshot ?: return current
        val validProjects = snapshot.projects.filter(ProjectSummary::valid)
        val selection = providerProjectSelection(
            validProjects,
            current.selectedProvider,
            current.selectedProjectId,
            current.recentProjects,
        )
        val projectId = selection.projectId
        val project = validProjects.find { it.id == projectId }
        val provider = selection.provider
        val capability = snapshot.providerCapabilities.find {
            it.projectId == projectId && it.provider == provider
        }
        val model = current.selectedModel?.takeIf { selected ->
            capability?.models?.any { it.id == selected } == true
        } ?: capability?.models?.firstOrNull()?.id
        val modelOption = capability?.models?.find { it.id == model }
        val effort = current.selectedEffort?.takeIf { selected ->
            modelOption?.effortOptions?.any { it.id == selected } == true
        } ?: modelOption?.defaultEffort ?: modelOption?.effortOptions?.firstOrNull()?.id
        val permission = current.selectedPermission?.takeIf { selected ->
            capability?.permissionModes?.any { it.id == selected } == true
        } ?: capability?.defaultPermissionMode
        val selectedConversation = retainedConversationSelection(
            conversations = snapshot.conversations,
            selectedConversationId = current.selectedConversationId,
            projectId = projectId,
            provider = provider,
            preserveMissing = preserveMissingConversation,
        )
        return current.copy(
            selectedConversationId = selectedConversation,
            selectedProjectId = projectId,
            selectedProvider = provider,
            selectedModel = model,
            selectedEffort = effort,
            selectedPermission = permission,
        )
    }

    private fun selectedCapability(current: RemoteUiState): ProviderCapability? =
        current.snapshot?.providerCapabilities?.find {
            it.projectId == current.selectedProjectId && it.provider == current.selectedProvider
        }

    private fun syncSelectedProject() {
        val current = state.value
        if (!current.online) return
        val projectId = current.selectedProjectId ?: return
        val provider = current.selectedProvider ?: return
        val hostId = current.snapshot?.hostId ?: return
        val generation = activeConnectionGeneration ?: return
        val scope = projectTreeScope(hostId, provider, projectId)
        if (projectSyncGeneration[scope] == generation) return
        val commandId = UUID.randomUUID()
        if (queue(commandId, WireProtocol.syncProject(commandId, projectId, provider))) {
            projectSyncGeneration[scope] = generation
            projectSyncCommands[commandId] = scope
        }
    }

    private fun refreshProjects(provider: ProviderId) {
        val generation = activeConnectionGeneration ?: return
        if (projectRefreshGeneration[provider] == generation) return
        if (client.send(WireProtocol.refreshProjects(provider))) {
            projectRefreshGeneration[provider] = generation
        } else {
            showError("项目刷新请求未能写入 WebSocket")
        }
    }

    private fun queue(commandId: UUID, bytes: ByteArray): Boolean {
        mutableState.update { it.copy(pendingCommands = it.pendingCommands + commandId) }
        if (!client.send(bytes)) {
            mutableState.update { it.copy(pendingCommands = it.pendingCommands - commandId) }
            showError("当前没有可用连接")
            return false
        }
        return true
    }

    private fun writePendingSend(isReplay: Boolean) {
        val pending = pendingSend ?: return
        val frame = pending.frame ?: return
        val generation = activeConnectionGeneration
        mutableState.update {
            it.copy(
                pendingCommands = it.pendingCommands + pending.commandId,
                sendStatus = SendStatus.SENDING,
                sendFailure = null,
                creatingConversation = pending.startsConversation && draftConversationId != null,
            )
        }
        if (generation == null || !client.send(frame)) {
            pendingSend = pending.copy(writeFailed = true)
            mutableState.update {
                it.copy(
                    pendingCommands = it.pendingCommands - pending.commandId,
                    sendStatus = SendStatus.FAILED,
                    sendFailure = "WebSocket 写入失败，草稿和附件已保留",
                    creatingConversation = false,
                )
            }
            showError("WebSocket 写入失败；请在连接恢复后重试")
            return
        }
        pendingSend = if (isReplay) {
            pending.replayed(generation)
        } else {
            pending.copy(
                lastWrittenGeneration = generation,
                rejectedByHost = false,
                writeFailed = false,
            )
        }
        logLocalSendTrace(
            pending.commandId,
            pending.clientMessageId,
            pending.conversationId,
            "websocket_write",
            pending.startedAtNanos,
        )
        mutableState.update {
            it.copy(
                sendStatus = SendStatus.QUEUED,
                sendFailure = null,
                creatingConversation = pending.startsConversation && draftConversationId != null,
            )
        }
    }

    private fun reconcilePendingSend(conversation: Conversation?) {
        val pending = pendingSend ?: return
        if (pending.startsConversation && conversation?.id == pending.conversationId) {
            draftConversationId = null
            mutableState.update { current ->
                rememberDraft(current)
                val previousScope = current.activeDraftScope()
                val updated = current.copy(
                    selectedConversationId = pending.conversationId,
                    showingNewConversation = false,
                    creatingConversation = false,
                )
                if (previousScope != updated.activeDraftScope()) {
                    previousScope?.let(scopedDrafts::remove)
                    rememberDraft(updated)
                }
                updated
            }
        }
    }

    private fun abandonRetryableSendForNavigation() {
        val pending = pendingSend?.takeIf(PendingSendContext::retryableFailure) ?: return
        pendingSend = null
        sendTraceStartedAtNanos.remove(pending.commandId)
        mutableState.update {
            it.copy(
                pendingCommands = it.pendingCommands - pending.commandId,
                creatingConversation = false,
                sendStatus = SendStatus.IDLE,
                sendFailure = null,
            )
        }
    }

    private fun List<Conversation>.findPendingConversation(): Conversation? {
        val pending = pendingSend ?: return null
        return find {
            it.id == pending.conversationId &&
                it.projectId == pending.projectId &&
                it.provider == pending.provider
        }
    }

    private fun replayPendingSend() {
        val pending = pendingSend
            ?.takeUnless(PendingSendContext::rejectedByHost)
            ?: return
        val generation = activeConnectionGeneration ?: return
        if (pending.lastWrittenGeneration == generation) return
        writePendingSend(isReplay = true)
    }

    private fun finishPendingSend(clearComposer: Boolean) {
        val pending = pendingSend ?: return
        pendingSend = null
        mutableState.update { current ->
            rememberDraft(current)
            val previousScope = current.activeDraftScope()
            val completed = current.completePendingSend(pending, clearComposer)
            if (clearComposer && pending.startsConversation && previousScope != completed.activeDraftScope()) {
                previousScope?.let(scopedDrafts::remove)
            }
            rememberDraft(completed)
            completed
        }
    }

    private fun requestImage(item: TimelineItem) {
        val content = item.content as? TimelineContent.Image ?: return
        if (content.attachmentId !in state.value.attachments) {
            client.send(WireProtocol.getAttachment(content.attachmentId))
        }
    }

    private fun requestConversationImages(conversationId: UUID) {
        state.value.timelineByConversation[conversationId]
            ?.asSequence()
            ?.forEach(::requestImage)
    }

    private fun requestInitialConversationPage(conversation: Conversation) {
        val current = state.value
        if (!current.online || activeConnectionGeneration == null) return
        if (
            current.selectedConversationId != conversation.id ||
            current.selectedProjectId != conversation.projectId ||
            current.selectedProvider != conversation.provider
        ) {
            return
        }
        val hostId = current.snapshot?.hostId ?: return
        val scope = ConversationPageScope(
            hostId = hostId,
            provider = conversation.provider,
            projectId = conversation.projectId,
            conversationId = conversation.id,
        )
        if (!requestedConversationPages.add(scope)) return
        if (!client.send(WireProtocol.getConversationPage(conversation.id, before = null, limit = 100))) {
            requestedConversationPages.remove(scope)
            showError("对话历史请求未能写入 WebSocket")
        }
    }

    private fun attachmentName(uri: Uri): String {
        val resolver = getApplication<Application>().contentResolver
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) return cursor.getString(0)
        }
        return uri.lastPathSegment ?: "attachment"
    }

    private fun upsertCredential(credential: StoredCredential) {
        val credentials = state.value.credentials.filterNot { it.hostId == credential.hostId } + credential
        credentialStore.save(credentials)
        clientStateStore.saveLastHost(credential.hostId)
        mutableState.update { it.copy(credentials = credentials, activeHostId = credential.hostId) }
    }

    private fun mutateSnapshot(transform: (Snapshot) -> Snapshot) {
        updateDraftScope { current ->
            val snapshot = current.snapshot ?: return@updateDraftScope current
            defaults(
                current.copy(snapshot = transform(snapshot)),
                preserveMissingConversation = awaitingAuthoritativeSnapshot,
            ).ensureDefaultProjectExpanded()
        }
    }

    private fun scheduleSnapshotPersist() {
        snapshotPersistJob?.cancel()
        snapshotPersistJob = viewModelScope.launch {
            delay(SNAPSHOT_PERSIST_DELAY_MS)
            val snapshot = state.value.snapshot ?: return@launch
            withContext(Dispatchers.IO) {
                val encoded = WireProtocol.encodeSnapshot(snapshot)
                clientStateStore.saveSnapshot(snapshot.hostId, encoded)
            }
        }
    }

    private fun persistSelection(debounce: Boolean = true) {
        selectionPersistJob?.cancel()
        val version = selectionPersistVersion.incrementAndGet()
        selectionPersistJob = viewModelScope.launch {
            if (debounce) delay(SELECTION_PERSIST_DELAY_MS)
            val current = state.value
            val drafts = scopedDrafts.toMap()
            withContext(Dispatchers.IO) {
                persistSelectionSnapshot(current, drafts, commit = false, version = version)
            }
        }
    }

    private fun persistSelectionSnapshot(
        current: RemoteUiState,
        drafts: Map<DraftScope, String>,
        commit: Boolean,
        version: Long,
    ) {
        synchronized(selectionPersistLock) {
            if (version < latestSavedSelectionVersion) return
            val hostId = current.activeHostId
            if (hostId == null) {
                latestSavedSelectionVersion = version
                return
            }
            clientStateStore.saveSelection(
                hostId = hostId,
                selectedConversationId = current.selectedConversationId,
                selectedProjectId = current.selectedProjectId,
                selectedProvider = current.selectedProvider,
                selectedModel = current.selectedModel,
                selectedEffort = current.selectedEffort,
                selectedPermission = current.selectedPermission,
                draft = current.draft,
                pinnedProjects = current.pinnedProjects,
                recentProjects = current.recentProjects,
                expandedProjectScopes = current.expandedProjectScopes,
                scopedDrafts = drafts,
                commit = commit,
            )
            latestSavedSelectionVersion = version
        }
    }

    private fun logLocalSendTrace(
        commandId: UUID,
        clientMessageId: String,
        conversationId: UUID,
        stage: String,
        startedAtNanos: Long,
    ) = logSendTrace(
        commandId = commandId,
        clientMessageId = clientMessageId,
        conversationId = conversationId,
        stage = stage,
        elapsedMs = ((System.nanoTime() - startedAtNanos) / 1_000_000L).coerceAtLeast(0L),
    )

    private fun rememberSendTrace(commandId: UUID, startedAtNanos: Long) {
        sendTraceStartedAtNanos[commandId] = startedAtNanos
        while (sendTraceStartedAtNanos.size > MAX_SEND_TRACE_CONTEXTS) {
            sendTraceStartedAtNanos.remove(sendTraceStartedAtNanos.keys.first())
        }
    }

    private fun logServerSendTrace(event: ServerEvent.SendTrace) {
        val startedAtNanos = sendTraceStartedAtNanos[event.commandId]
        val endToEndElapsedMs = startedAtNanos?.let {
            ((System.nanoTime() - it) / 1_000_000L).coerceAtLeast(0L)
        }
        logSendTrace(
            commandId = event.commandId,
            clientMessageId = event.clientMessageId,
            conversationId = event.conversationId,
            stage = event.stage,
            elapsedMs = endToEndElapsedMs,
            hostElapsedMs = event.elapsedMs,
        )
        if (event.stage == "first_provider_event") {
            sendTraceStartedAtNanos.remove(event.commandId)
        }
    }

    private fun logSendTrace(
        commandId: UUID,
        clientMessageId: String,
        conversationId: UUID,
        stage: String,
        elapsedMs: Long?,
        hostElapsedMs: Long? = null,
    ) {
        val entry = JSONObject()
            .put("commandId", commandId.toString())
            .put("clientMessageId", clientMessageId)
            .put("conversationId", conversationId.toString())
            .put("stage", stage)
            .put("elapsedMs", elapsedMs ?: JSONObject.NULL)
        if (hostElapsedMs != null) entry.put("hostElapsedMs", hostElapsedMs)
        Log.i(
            SEND_TRACE_TAG,
            entry.toString(),
        )
    }

    private fun showError(message: String) = mutableState.update { it.copy(error = message) }

    private fun onMain(block: () -> Unit) {
        viewModelScope.launch { block() }
    }

    companion object {
        private const val SNAPSHOT_PERSIST_DELAY_MS = 200L
        private const val SELECTION_PERSIST_DELAY_MS = 350L
        private const val SEND_TRACE_TAG = "GAR.SendTrace"
        private const val MAX_SEND_TRACE_CONTEXTS = 32
        private fun replaceCapability(
            current: List<ProviderCapability>,
            incoming: ProviderCapability,
        ): List<ProviderCapability> = current.filterNot {
            it.projectId == incoming.projectId && it.provider == incoming.provider
        } + incoming

        private fun upsertTimeline(
            current: List<TimelineItem>,
            incoming: TimelineItem,
        ): List<TimelineItem> {
            val existingIndex = current.indexOfFirst {
                it.conversationId == incoming.conversationId && it.id == incoming.id
            }
            if (existingIndex >= 0) {
                val existing = current[existingIndex]
                if (existing.revision > incoming.revision) return current
                if (existing.createdAtMs == incoming.createdAtMs && existing.id == incoming.id) {
                    return current.toMutableList().apply { set(existingIndex, incoming) }
                }
            }
            val updated = current.toMutableList()
            if (existingIndex >= 0) updated.removeAt(existingIndex)
            var low = 0
            var high = updated.size
            while (low < high) {
                val middle = (low + high) ushr 1
                if (timelineComparator.compare(updated[middle], incoming) <= 0) {
                    low = middle + 1
                } else {
                    high = middle
                }
            }
            updated.add(low, incoming)
            return updated
        }
    }
}
