package dev.agentremote.messenger.ui

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.agentremote.messenger.data.CredentialStore
import dev.agentremote.messenger.data.RemoteClient
import dev.agentremote.messenger.model.ConnectionTarget
import dev.agentremote.messenger.model.Conversation
import dev.agentremote.messenger.model.ProviderCapability
import dev.agentremote.messenger.model.ProviderId
import dev.agentremote.messenger.model.ServerEvent
import dev.agentremote.messenger.model.Snapshot
import dev.agentremote.messenger.model.StoredCredential
import dev.agentremote.messenger.model.TimelineContent
import dev.agentremote.messenger.model.TimelineItem
import dev.agentremote.messenger.protocol.WireProtocol
import java.util.UUID
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class RemoteUiState(
    val phase: String = "未连接",
    val online: Boolean = false,
    val connecting: Boolean = false,
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
    val selectedNativeSession: String? = null,
    val draft: String = "",
    val attachments: Map<UUID, ByteArray> = emptyMap(),
    val pendingCommands: Set<UUID> = emptySet(),
    val pendingApprovals: Set<UUID> = emptySet(),
    val creatingConversation: Boolean = false,
    val error: String? = null,
)

class RemoteViewModel(application: Application) : AndroidViewModel(application), RemoteClient.Listener {
    private val credentialStore = CredentialStore(application)
    private val client = RemoteClient(this)
    private val mutableState = MutableStateFlow(
        RemoteUiState(credentials = credentialStore.load()),
    )
    val state: StateFlow<RemoteUiState> = mutableState.asStateFlow()
    private var awaitingNewConversation = false
    private var conversationIdsBeforeCreate: Set<UUID> = emptySet()

    init {
        mutableState.value.credentials.firstOrNull()?.let(::connect)
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
        mutableState.update {
            it.copy(
                error = null,
                snapshot = null,
                activeHostId = target.hostId,
                attachments = emptyMap(),
                connecting = true,
            )
        }
        client.connect(target)
    }

    fun connect(credential: StoredCredential) {
        if (state.value.connecting) return
        mutableState.update {
            it.copy(
                error = null,
                snapshot = null,
                activeHostId = credential.hostId,
                attachments = emptyMap(),
                connecting = true,
            )
        }
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
        mutableState.update {
            it.copy(
                phase = "未连接",
                online = false,
                connecting = false,
                activeHostId = null,
                snapshot = null,
                selectedConversationId = null,
                attachments = emptyMap(),
                pendingCommands = emptySet(),
                pendingApprovals = emptySet(),
                creatingConversation = false,
            )
        }
    }

    fun forget(credential: StoredCredential) {
        if (state.value.activeHostId == credential.hostId) disconnect()
        val remaining = state.value.credentials.filterNot { it.hostId == credential.hostId }
        credentialStore.save(remaining)
        mutableState.update { it.copy(credentials = remaining) }
    }

    fun showNewConversation() = mutableState.update {
        defaults(it.copy(showingNewConversation = true, selectedConversationId = null, attachments = emptyMap()))
    }

    fun showConversationList() = mutableState.update {
        it.copy(showingNewConversation = false, selectedConversationId = null, attachments = emptyMap())
    }

    fun selectConversation(id: UUID) {
        mutableState.update {
            it.copy(selectedConversationId = id, showingNewConversation = false, attachments = emptyMap())
        }
        requestConversationImages(id)
    }

    fun selectProject(id: UUID) = mutableState.update { current ->
        defaults(
            current.copy(
                selectedProjectId = id,
                selectedProvider = null,
                selectedModel = null,
                selectedEffort = null,
                selectedNativeSession = null,
            ),
        )
    }

    fun selectProvider(provider: ProviderId) = mutableState.update { current ->
        defaults(
            current.copy(
                selectedProvider = provider,
                selectedModel = null,
                selectedEffort = null,
                selectedNativeSession = null,
            ),
        )
    }

    fun selectModel(model: String?) = mutableState.update { current ->
        val capability = selectedCapability(current)
        val selected = capability?.models?.find { it.id == model }
        current.copy(
            selectedModel = model,
            selectedEffort = selected?.defaultEffort ?: selected?.effortOptions?.firstOrNull()?.id,
        )
    }

    fun selectEffort(effort: String?) = mutableState.update { it.copy(selectedEffort = effort) }

    fun selectNativeSession(sessionId: String?) = mutableState.update {
        it.copy(selectedNativeSession = sessionId)
    }

    fun createConversation() {
        val current = state.value
        if (current.creatingConversation) return
        val projectId = current.selectedProjectId ?: return
        val provider = current.selectedProvider ?: return
        val commandId = UUID.randomUUID()
        val queued = queue(
            commandId,
            WireProtocol.createConversation(
                commandId = commandId,
                projectId = projectId,
                provider = provider,
                nativeSessionId = current.selectedNativeSession,
                model = current.selectedModel,
                effort = current.selectedEffort,
            ),
        )
        if (queued) {
            awaitingNewConversation = true
            conversationIdsBeforeCreate = current.snapshot?.conversations?.map { it.id }?.toSet().orEmpty()
            mutableState.update { it.copy(creatingConversation = true) }
        }
    }

    fun setDraft(value: String) = mutableState.update { it.copy(draft = value) }

    fun sendMessage() {
        val current = state.value
        val conversationId = current.selectedConversationId ?: return
        val text = current.draft.trim()
        if (text.isEmpty()) return
        val commandId = UUID.randomUUID()
        if (queue(commandId, WireProtocol.sendMessage(commandId, conversationId, text))) {
            mutableState.update { it.copy(draft = "") }
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
        queue(
            commandId,
            WireProtocol.setSessionOption(commandId, conversationId, optionId, value),
        )
    }

    fun clearError() = mutableState.update { it.copy(error = null) }

    override fun onConnecting(target: ConnectionTarget) = onMain {
        mutableState.update {
            it.copy(phase = "正在连接 ${target.origin}", online = false, connecting = true, error = null)
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
            is ServerEvent.Authenticated -> mutableState.update {
                it.copy(phase = "认证成功，正在同步")
            }
            is ServerEvent.SnapshotReceived -> applySnapshot(event.snapshot)
            is ServerEvent.ProviderChanged -> mutateSnapshot { snapshot ->
                snapshot.copy(providerCapabilities = replaceCapability(snapshot.providerCapabilities, event.capability))
            }
            is ServerEvent.ConversationUpserted -> {
                mutateSnapshot { snapshot ->
                    val conversations = upsertConversation(snapshot.conversations, event.conversation)
                    snapshot.copy(conversations = conversations.sortedByDescending(Conversation::updatedAtMs))
                }
                if (awaitingNewConversation && event.conversation.id !in conversationIdsBeforeCreate) {
                    awaitingNewConversation = false
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
                snapshot.copy(
                    conversations = snapshot.conversations.filterNot { it.id == event.conversationId },
                    timeline = snapshot.timeline.filterNot { it.conversationId == event.conversationId },
                )
            }
            is ServerEvent.AttachmentReceived -> mutableState.update { current ->
                if (event.attachment.conversationId != current.selectedConversationId) current else {
                    current.copy(attachments = current.attachments + (event.attachment.id to event.attachment.bytes))
                }
            }
            is ServerEvent.HostStatus -> mutableState.update {
                it.copy(
                    online = event.online,
                    phase = if (event.online) "Host 在线" else "Host 离线",
                    error = event.message?.takeIf { message -> !event.online && message.isNotBlank() },
                )
            }
            is ServerEvent.CommandAccepted -> mutableState.update {
                it.copy(pendingCommands = it.pendingCommands - event.commandId)
            }
            is ServerEvent.CommandRejected -> {
                event.commandId?.let { rejected ->
                    mutableState.update { it.copy(pendingCommands = it.pendingCommands - rejected) }
                }
                awaitingNewConversation = false
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
    }

    override fun onDisconnected(message: String) = onMain {
        awaitingNewConversation = false
        mutableState.update {
            it.copy(
                phase = "连接已断开，等待重连",
                online = false,
                connecting = false,
                pendingCommands = emptySet(),
                pendingApprovals = emptySet(),
                creatingConversation = false,
                error = message,
            )
        }
    }

    override fun onError(message: String) = onMain { showError(message) }

    override fun onCleared() {
        client.disconnect()
        super.onCleared()
    }

    private fun applySnapshot(snapshot: Snapshot) {
        val active = state.value.credentials.find { it.hostId == snapshot.hostId }
        if (active != null && active.displayName != snapshot.hostName) {
            upsertCredential(active.copy(displayName = snapshot.hostName))
        }
        mutableState.update { current ->
            val selectedStillExists = current.selectedConversationId
                ?.takeIf { id -> snapshot.conversations.any { it.id == id } }
            defaults(
                current.copy(
                    phase = "已连接 ${snapshot.hostName}",
                    online = true,
                    connecting = false,
                    snapshot = snapshot.copy(
                        conversations = snapshot.conversations.sortedByDescending(Conversation::updatedAtMs),
                        timeline = snapshot.timeline.sortedWith(timelineComparator),
                    ),
                    selectedConversationId = selectedStillExists,
                    showingNewConversation = snapshot.conversations.isEmpty(),
                    attachments = emptyMap(),
                    pendingCommands = emptySet(),
                    pendingApprovals = emptySet(),
                    creatingConversation = false,
                    error = null,
                ),
            )
        }
        state.value.selectedConversationId?.let(::requestConversationImages)
    }

    private fun defaults(current: RemoteUiState): RemoteUiState {
        val snapshot = current.snapshot ?: return current
        val projectId = current.selectedProjectId?.takeIf { selected ->
            snapshot.projects.any { it.id == selected && it.valid }
        } ?: snapshot.projects.firstOrNull { it.valid }?.id
        val project = snapshot.projects.find { it.id == projectId }
        val provider = current.selectedProvider?.takeIf { it in project?.enabledProviders.orEmpty() }
            ?: project?.enabledProviders?.firstOrNull { candidate ->
                snapshot.providerCapabilities.any {
                    it.projectId == projectId && it.provider == candidate && it.ready
                }
            }
            ?: project?.enabledProviders?.firstOrNull()
        val capability = snapshot.providerCapabilities.find {
            it.projectId == projectId && it.provider == provider
        }
        val model = current.selectedModel?.takeIf { selected -> capability?.models?.any { it.id == selected } == true }
            ?: capability?.models?.firstOrNull()?.id
        val modelOption = capability?.models?.find { it.id == model }
        val effort = current.selectedEffort?.takeIf { selected ->
            modelOption?.effortOptions?.any { it.id == selected } == true
        } ?: modelOption?.defaultEffort ?: modelOption?.effortOptions?.firstOrNull()?.id
        val nativeSession = current.selectedNativeSession?.takeIf { selected ->
            capability?.sessions?.any { it.nativeSessionId == selected } == true
        }
        return current.copy(
            selectedProjectId = projectId,
            selectedProvider = provider,
            selectedModel = model,
            selectedEffort = effort,
            selectedNativeSession = nativeSession,
        )
    }

    private fun selectedCapability(current: RemoteUiState): ProviderCapability? =
        current.snapshot?.providerCapabilities?.find {
            it.projectId == current.selectedProjectId && it.provider == current.selectedProvider
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

    private fun upsertCredential(credential: StoredCredential) {
        val credentials = (state.value.credentials.filterNot { it.hostId == credential.hostId } + credential)
        credentialStore.save(credentials)
        mutableState.update { it.copy(credentials = credentials, activeHostId = credential.hostId) }
    }

    private fun mutateSnapshot(transform: (Snapshot) -> Snapshot) {
        mutableState.update { current ->
            val snapshot = current.snapshot ?: return@update current
            defaults(current.copy(snapshot = transform(snapshot)))
        }
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
