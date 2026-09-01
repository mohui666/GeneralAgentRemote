package dev.agentremote.messenger.model

import java.util.UUID

enum class ProviderId(val wire: String, val label: String) {
    CODEX("codex", "Codex"),
    GROK("grok", "Grok");

    companion object {
        fun fromWire(value: String): ProviderId = entries.first { it.wire == value }
    }
}

data class EffortOption(
    val id: String,
    val displayName: String,
)

data class ModelOption(
    val id: String,
    val displayName: String,
    val effortOptions: List<EffortOption>,
    val defaultEffort: String?,
)

data class SessionOptionValue(
    val value: String,
    val displayName: String,
)

data class SessionOption(
    val id: String,
    val displayName: String,
    val category: String?,
    val currentValue: String,
    val values: List<SessionOptionValue>,
)

data class ProjectSummary(
    val id: UUID,
    val displayName: String,
    val enabledProviders: List<ProviderId>,
    val valid: Boolean,
)

data class SessionSummary(
    val nativeSessionId: String,
    val title: String,
    val updatedAtMs: Long,
)

data class ProviderCapability(
    val provider: ProviderId,
    val projectId: UUID,
    val state: String,
    val version: String?,
    val detail: String?,
    val models: List<ModelOption>,
    val supportsSessionList: Boolean,
    val supportsSteer: Boolean,
    val sessions: List<SessionSummary>,
    val limitation: String?,
) {
    val ready: Boolean get() = state == "ready"
}

data class Conversation(
    val id: UUID,
    val revision: Long,
    val provider: ProviderId,
    val projectId: UUID,
    val nativeSessionId: String,
    val title: String,
    val selectedModel: String?,
    val selectedEffort: String?,
    val state: String,
    val sessionOptions: List<SessionOption>,
    val updatedAtMs: Long,
) {
    val running: Boolean get() = state == "running" || state == "needs_approval"
}

data class ApprovalOption(
    val id: String,
    val label: String,
)

data class PlanStep(
    val text: String,
    val status: String,
)

sealed interface TimelineContent {
    data class UserMessage(val text: String) : TimelineContent
    data class AgentMessage(val phase: String, val text: String) : TimelineContent
    data class Progress(
        val kind: String,
        val label: String,
        val status: String,
        val detail: String?,
    ) : TimelineContent
    data class Plan(val steps: List<PlanStep>) : TimelineContent
    data class ToolCall(
        val name: String,
        val status: String,
        val inputSummary: String?,
        val outputSummary: String?,
    ) : TimelineContent
    data class Command(
        val command: String,
        val relativeCwd: String?,
        val status: String,
        val exitCode: Int?,
        val output: String?,
    ) : TimelineContent
    data class FileChange(
        val relativePath: String,
        val changeKind: String,
        val status: String,
    ) : TimelineContent
    data class Approval(
        val approvalId: UUID,
        val prompt: String,
        val options: List<ApprovalOption>,
        val resolvedOption: String?,
    ) : TimelineContent
    data class Image(val attachmentId: UUID, val alt: String) : TimelineContent
    data class Error(val code: String, val message: String) : TimelineContent
}

data class TimelineItem(
    val id: UUID,
    val conversationId: UUID,
    val revision: Long,
    val createdAtMs: Long,
    val content: TimelineContent,
)

data class AttachmentData(
    val id: UUID,
    val conversationId: UUID,
    val mimeType: String,
    val bytes: ByteArray,
)

data class Snapshot(
    val hostId: UUID,
    val hostName: String,
    val projects: List<ProjectSummary>,
    val providerCapabilities: List<ProviderCapability>,
    val conversations: List<Conversation>,
    val timeline: List<TimelineItem>,
)

data class StoredCredential(
    val hostId: UUID,
    val deviceId: UUID,
    val deviceToken: String,
    val origin: String,
    val relay: Boolean,
    val displayName: String,
)

data class ConnectionTarget(
    val hostId: UUID,
    val origin: String,
    val relay: Boolean,
    val pairToken: String? = null,
    val credential: StoredCredential? = null,
) {
    val webSocketUrl: String
        get() {
            val base = origin.trimEnd('/').let {
                when {
                    it.startsWith("https://") -> "wss://${it.removePrefix("https://")}"
                    it.startsWith("http://") -> "ws://${it.removePrefix("http://")}"
                    else -> error("配对地址必须使用 http:// 或 https://")
                }
            }
            return if (relay) "$base/client/$hostId" else "$base/ws"
        }
}

sealed interface ServerEvent {
    data class Paired(val credential: StoredCredential) : ServerEvent
    data class Authenticated(val hostId: UUID, val deviceId: UUID) : ServerEvent
    data class SnapshotReceived(val snapshot: Snapshot) : ServerEvent
    data class ProviderChanged(val capability: ProviderCapability) : ServerEvent
    data class ConversationUpserted(val conversation: Conversation) : ServerEvent
    data class TimelineUpserted(val item: TimelineItem) : ServerEvent
    data class ConversationRemoved(val conversationId: UUID) : ServerEvent
    data class AttachmentReceived(val attachment: AttachmentData) : ServerEvent
    data class HostStatus(val hostId: UUID, val online: Boolean, val message: String?) : ServerEvent
    data class CommandAccepted(val commandId: UUID) : ServerEvent
    data class CommandRejected(
        val commandId: UUID?,
        val code: String,
        val message: String,
    ) : ServerEvent
    data class ProtocolError(val supportedVersion: Int, val message: String) : ServerEvent
}
