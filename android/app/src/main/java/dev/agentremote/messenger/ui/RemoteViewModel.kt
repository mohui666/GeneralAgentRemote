package dev.agentremote.messenger.ui

import android.app.Application
import android.net.ConnectivityManager
import android.net.Network
import android.net.Uri
import android.provider.OpenableColumns
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.agentremote.messenger.data.ClientStateStore
import dev.agentremote.messenger.data.CredentialStore
import dev.agentremote.messenger.data.RemoteClient
import dev.agentremote.messenger.model.ConnectionTarget
import dev.agentremote.messenger.model.Conversation
import dev.agentremote.messenger.model.ProjectSummary
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
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

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
    val projectSearch: String = "",
    val historyBefore: Map<UUID, TimelinePageCursor> = emptyMap(),
    val historyExhausted: Set<UUID> = emptySet(),
    val pendingCommands: Set<UUID> = emptySet(),
    val pendingApprovals: Set<UUID> = emptySet(),
    val creatingConversation: Boolean = false,
    val error: String? = null,
)

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
    private val mutableState = MutableStateFlow(
        RemoteUiState(
            phase = if (initialRestored?.snapshot != null) "离线缓存 · 正在恢复连接" else "未连接",
            credentials = initialCredentials,
            activeHostId = initialCredential?.hostId,
            snapshot = initialRestored?.snapshot,
            selectedConversationId = initialRestored?.selectedConversationId,
            selectedProjectId = initialRestored?.selectedProjectId,
            selectedProvider = initialRestored?.selectedProvider,
            selectedModel = initialRestored?.selectedModel,
            selectedEffort = initialRestored?.selectedEffort,
            selectedPermission = initialRestored?.selectedPermission,
            draft = initialRestored?.draft.orEmpty(),
            pinnedProjects = initialRestored?.pinnedProjects.orEmpty(),
            recentProjects = initialRestored?.recentProjects.orEmpty(),
        ),
    )
    val state: StateFlow<RemoteUiState> = mutableState.asStateFlow()

    private var draftConversationId: UUID? = null
    private var pendingSendCommand: UUID? = null
    private var pendingSendFrame: ByteArray? = null
    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) = client.networkAvailable()
    }

    init {
        mutableState.update(::defaults)
        connectivity.registerDefaultNetworkCallback(networkCallback)
        initialCredential?.let(::connect)
    }

    fun setPairLink(value: String) = mutableState.update { it.copy(pairLink = value) }

    fun reportError(message: String) = showError(message)

    fun pair() {
        if (state.value.connecting) return
        val target = runCatching { WireProtocol.parsePairLink(state.value.pairLink) }
            .getOrElse { error ->
                showError(error.message ?: "配对链接无效")
                return
            }
        pendingSendCommand = null
        pendingSendFrame = null
        draftConversationId = null
        mutableState.update {
            it.copy(
                error = null,
                snapshot = null,
                activeHostId = target.hostId,
                selectedConversationId = null,
                attachments = emptyMap(),
                promptAttachments = emptyList(),
                connecting = true,
                retryEnabled = true,
            )
        }
        clientStateStore.saveLastHost(target.hostId)
        client.connect(target)
    }

    fun connect(credential: StoredCredential) {
        if (state.value.connecting && state.value.activeHostId == credential.hostId) return
        if (state.value.activeHostId != credential.hostId) {
            pendingSendCommand = null
            pendingSendFrame = null
            draftConversationId = null
        }
        val restored = clientStateStore.load(credential)
        mutableState.update {
            defaults(
                it.copy(
                    phase = if (restored.snapshot != null) "离线缓存 · 正在恢复连接" else "正在连接",
                    error = null,
                    snapshot = restored.snapshot,
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
                    connecting = true,
                    retryEnabled = true,
                    reconnectAttempt = 0,
                    historyBefore = emptyMap(),
                    historyExhausted = emptySet(),
                    pendingCommands = emptySet(),
                    pendingApprovals = emptySet(),
                ),
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
        pendingSendCommand = null
        pendingSendFrame = null
        mutableState.update {
            it.copy(
                phase = "离线 · 已手动断开",
                online = false,
                connecting = false,
                retryEnabled = false,
                pendingCommands = emptySet(),
                pendingApprovals = emptySet(),
                creatingConversation = false,
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
        if (state.value.activeHostId == credential.hostId) {
            client.disconnect()
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
        draftConversationId = UUID.randomUUID()
        mutableState.update {
            defaults(
                it.copy(
                    showingNewConversation = true,
                    selectedConversationId = null,
                    attachments = emptyMap(),
                    promptAttachments = emptyList(),
                ),
            )
        }
        persistSelection()
    }

    fun showConversationList() {
        draftConversationId = null
        mutableState.update {
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
        draftConversationId = null
        mutableState.update {
            it.copy(
                selectedConversationId = id,
                showingNewConversation = false,
                attachments = emptyMap(),
                promptAttachments = emptyList(),
            )
        }
        persistSelection()
        requestConversationImages(id)
    }

    fun selectProject(id: UUID) {
        mutableState.update { current ->
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
            )
        }
        persistSelection()
        syncSelectedProject()
    }

    fun selectProvider(provider: ProviderId) {
        mutableState.update { current ->
            defaults(
                current.copy(
                    selectedProvider = provider,
                    selectedConversationId = null,
                    selectedModel = null,
                    selectedEffort = null,
                    selectedPermission = null,
                    promptAttachments = emptyList(),
                ),
            )
        }
        persistSelection()
        if (state.value.online) client.send(WireProtocol.refreshProjects(provider))
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

    fun toggleProjectPin(id: UUID) {
        mutableState.update {
            it.copy(
                pinnedProjects = if (id in it.pinnedProjects) it.pinnedProjects - id else it.pinnedProjects + id,
            )
        }
        persistSelection()
    }

    fun setDraft(value: String) {
        mutableState.update { it.copy(draft = value) }
        persistSelection()
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
        if (pendingSendCommand != null) return
        val text = current.draft.trim()
        if (text.isEmpty()) return
        val commandId = UUID.randomUUID()
        val bytes = current.selectedConversationId?.let { conversationId ->
            WireProtocol.sendMessage(
                commandId = commandId,
                conversationId = conversationId,
                clientMessageId = UUID.randomUUID().toString(),
                text = text,
                attachments = current.promptAttachments,
            )
        } ?: run {
            if (!current.showingNewConversation) return
            val conversationId = draftConversationId ?: UUID.randomUUID().also { draftConversationId = it }
            WireProtocol.startConversation(
                commandId = commandId,
                conversationId = conversationId,
                projectId = current.selectedProjectId ?: return,
                provider = current.selectedProvider ?: return,
                model = current.selectedModel,
                effort = current.selectedEffort,
                permissionMode = current.selectedPermission,
                text = text,
                attachments = current.promptAttachments,
            )
        }
        if (queue(commandId, bytes)) {
            pendingSendCommand = commandId
            pendingSendFrame = bytes
            mutableState.update { it.copy(creatingConversation = current.selectedConversationId == null) }
        }
    }

    fun steer() {
        val current = state.value
        val conversationId = current.selectedConversationId ?: return
        val text = current.draft.trim()
        if (text.isEmpty()) return
        val commandId = UUID.randomUUID()
        if (queue(commandId, WireProtocol.steer(commandId, conversationId, text))) {
            mutableState.update { it.copy(draft = "") }
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
        client.send(
            WireProtocol.getConversationPage(
                conversationId,
                before,
                100,
            ),
        )
    }

    fun clearError() = mutableState.update { it.copy(error = null) }

    override fun onConnecting(target: ConnectionTarget) = onMain {
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

    override fun onEvent(event: ServerEvent) = onMain {
        when (event) {
            is ServerEvent.Paired -> {
                upsertCredential(event.credential)
                mutableState.update { it.copy(phase = "配对成功，正在同步", pairLink = "") }
            }
            is ServerEvent.Authenticated -> mutableState.update { it.copy(phase = "认证成功，正在同步") }
            is ServerEvent.SnapshotReceived -> {
                clientStateStore.saveSnapshot(event.snapshot.hostId, event.encoded)
                applySnapshot(event.snapshot)
                state.value.selectedProvider?.let { client.send(WireProtocol.refreshProjects(it)) }
                syncSelectedProject()
                pendingSendFrame?.let(client::send)
            }
            is ServerEvent.ProjectsUpdated -> applyProjects(event)
            is ServerEvent.ProjectSyncCompleted -> mutableState.update {
                it.copy(
                    phase = if (event.fullHistoryFallback) {
                        "已同步 ${event.conversationsSynced} 个对话（全量去重）"
                    } else {
                        "已同步 ${event.conversationsSynced} 个对话"
                    },
                )
            }
            is ServerEvent.ConversationPage -> mutableState.update { current ->
                val snapshot = current.snapshot ?: return@update current
                val timeline = event.items.fold(snapshot.timeline, ::upsertTimeline)
                if (event.nextBefore == null) {
                    current.copy(
                        snapshot = snapshot.copy(timeline = timeline),
                        historyExhausted = current.historyExhausted + event.conversationId,
                    )
                } else {
                    current.copy(
                        snapshot = snapshot.copy(timeline = timeline),
                        historyBefore = current.historyBefore + (event.conversationId to event.nextBefore),
                    )
                }
            }
            is ServerEvent.ProviderChanged -> mutateSnapshot { snapshot ->
                snapshot.copy(
                    providerCapabilities = replaceCapability(snapshot.providerCapabilities, event.capability),
                )
            }
            is ServerEvent.ConversationUpserted -> {
                mutateSnapshot { snapshot ->
                    val conversations = upsertConversation(snapshot.conversations, event.conversation)
                        .sortedByDescending(Conversation::updatedAtMs)
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
                if (event.conversation.id == draftConversationId) {
                    draftConversationId = null
                    mutableState.update {
                        it.copy(
                            selectedConversationId = event.conversation.id,
                            showingNewConversation = false,
                            creatingConversation = false,
                        )
                    }
                }
            }
            is ServerEvent.TimelineUpserted -> {
                mutateSnapshot { snapshot ->
                    snapshot.copy(timeline = upsertTimeline(snapshot.timeline, event.item))
                }
                val approval = event.item.content as? TimelineContent.Approval
                if (approval?.resolvedOption != null) {
                    mutableState.update {
                        it.copy(pendingApprovals = it.pendingApprovals - approval.approvalId)
                    }
                }
                if (event.item.conversationId == state.value.selectedConversationId) requestImage(event.item)
            }
            is ServerEvent.ConversationRemoved -> mutateSnapshot { snapshot ->
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
            is ServerEvent.AttachmentReceived -> mutableState.update { current ->
                if (event.attachment.conversationId != current.selectedConversationId) current else {
                    current.copy(
                        attachments = current.attachments + (event.attachment.id to event.attachment.bytes),
                    )
                }
            }
            is ServerEvent.HostStatus -> mutableState.update {
                it.copy(
                    online = event.online,
                    phase = if (event.online) "Host 在线" else "Host 离线",
                    error = event.message?.takeIf { message -> !event.online && message.isNotBlank() },
                )
            }
            is ServerEvent.CommandAccepted -> {
                if (pendingSendCommand == event.commandId) {
                    pendingSendCommand = null
                    pendingSendFrame = null
                    mutableState.update {
                        it.copy(draft = "", promptAttachments = emptyList(), creatingConversation = false)
                    }
                }
                mutableState.update { it.copy(pendingCommands = it.pendingCommands - event.commandId) }
            }
            is ServerEvent.CommandRejected -> {
                event.commandId?.let { rejected ->
                    mutableState.update { it.copy(pendingCommands = it.pendingCommands - rejected) }
                    if (pendingSendCommand == rejected) {
                        pendingSendCommand = null
                        pendingSendFrame = null
                    }
                }
                mutableState.update {
                    it.copy(pendingApprovals = emptySet(), creatingConversation = false)
                }
                showError("${event.code}: ${event.message}")
                if (event.code == "authentication_failed") {
                    state.value.credentials.find { it.hostId == state.value.activeHostId }?.let(::forget)
                }
            }
            is ServerEvent.ProtocolError -> showError(
                "协议错误：${event.message}（Host 支持 v${event.supportedVersion}）",
            )
        }
        persistSelection()
    }

    override fun onDisconnected(message: String) = onMain {
        mutableState.update {
            it.copy(
                phase = "连接已断开，等待重连",
                online = false,
                connecting = false,
                pendingCommands = pendingSendCommand?.let(::setOf).orEmpty(),
                pendingApprovals = emptySet(),
                creatingConversation = pendingSendCommand != null && draftConversationId != null,
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
        connectivity.unregisterNetworkCallback(networkCallback)
        client.close()
        super.onCleared()
    }

    private fun applySnapshot(snapshot: Snapshot) {
        val active = state.value.credentials.find { it.hostId == snapshot.hostId }
        if (active != null && active.displayName != snapshot.hostName) {
            upsertCredential(active.copy(displayName = snapshot.hostName))
        }
        mutableState.update { current ->
            defaults(
                current.copy(
                    phase = "已连接 ${snapshot.hostName}",
                    online = true,
                    connecting = false,
                    retryEnabled = true,
                    reconnectAttempt = 0,
                    snapshot = snapshot.copy(
                        conversations = snapshot.conversations.sortedByDescending(Conversation::updatedAtMs),
                        timeline = snapshot.timeline.sortedWith(timelineComparator),
                    ),
                    attachments = emptyMap(),
                    historyBefore = emptyMap(),
                    historyExhausted = emptySet(),
                    pendingCommands = emptySet(),
                    pendingApprovals = emptySet(),
                    creatingConversation = false,
                    error = null,
                ),
            )
        }
        state.value.selectedConversationId?.let(::requestConversationImages)
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
        syncSelectedProject()
    }

    private fun defaults(current: RemoteUiState): RemoteUiState {
        val snapshot = current.snapshot ?: return current
        val validProjects = snapshot.projects.filter(ProjectSummary::valid)
        val requestedProvider = current.selectedProvider
            ?: validProjects.firstOrNull()?.enabledProviders?.firstOrNull()
        val projectId = current.selectedProjectId?.takeIf { selected ->
            validProjects.any { project ->
                project.id == selected && (requestedProvider == null || requestedProvider in project.enabledProviders)
            }
        } ?: validProjects.firstOrNull { requestedProvider in it.enabledProviders }?.id
            ?: validProjects.firstOrNull()?.id
        val project = validProjects.find { it.id == projectId }
        val provider = requestedProvider?.takeIf { it in project?.enabledProviders.orEmpty() }
            ?: project?.enabledProviders?.firstOrNull()
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
        val selectedConversation = current.selectedConversationId?.takeIf { id ->
            snapshot.conversations.any {
                it.id == id && it.projectId == projectId && it.provider == provider
            }
        }
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
        val commandId = UUID.randomUUID()
        queue(commandId, WireProtocol.syncProject(commandId, projectId, provider))
    }

    private fun queue(commandId: UUID, bytes: ByteArray): Boolean {
        if (!client.send(bytes)) {
            showError("当前没有可用连接")
            return false
        }
        mutableState.update { it.copy(pendingCommands = it.pendingCommands + commandId) }
        return true
    }

    private fun requestImage(item: TimelineItem) {
        val content = item.content as? TimelineContent.Image ?: return
        if (content.attachmentId !in state.value.attachments) {
            client.send(WireProtocol.getAttachment(content.attachmentId))
        }
    }

    private fun requestConversationImages(conversationId: UUID) {
        state.value.snapshot?.timeline
            ?.asSequence()
            ?.filter { it.conversationId == conversationId }
            ?.forEach(::requestImage)
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
        mutableState.update { current ->
            val snapshot = current.snapshot ?: return@update current
            defaults(current.copy(snapshot = transform(snapshot)))
        }
    }

    private fun persistSelection() {
        val current = state.value
        val hostId = current.activeHostId ?: return
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
        )
    }

    private fun showError(message: String) = mutableState.update { it.copy(error = message) }

    private fun onMain(block: () -> Unit) {
        viewModelScope.launch { block() }
    }

    companion object {
        private val timelineComparator = compareBy<TimelineItem> { it.createdAtMs }.thenBy { it.id }

        private fun replaceCapability(
            current: List<ProviderCapability>,
            incoming: ProviderCapability,
        ): List<ProviderCapability> = current.filterNot {
            it.projectId == incoming.projectId && it.provider == incoming.provider
        } + incoming

        private fun upsertConversation(
            current: List<Conversation>,
            incoming: Conversation,
        ): List<Conversation> {
            val existing = current.find { it.id == incoming.id }
            if (existing != null && existing.revision > incoming.revision) return current
            return current.filterNot { it.id == incoming.id } + incoming
        }

        private fun upsertTimeline(
            current: List<TimelineItem>,
            incoming: TimelineItem,
        ): List<TimelineItem> {
            val existing = current.find { it.id == incoming.id }
            if (existing != null && existing.revision > incoming.revision) return current
            return (current.filterNot { it.id == incoming.id } + incoming).sortedWith(timelineComparator)
        }
    }
}
