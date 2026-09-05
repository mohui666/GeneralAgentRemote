package dev.agentremote.messenger.model

import java.util.UUID

enum class ProviderId(val wire: String, val label: String) {
    CODEX("codex", "Codex"),
    GROK("grok", "Grok"),
    CLAUDE_CODE("claude_code", "Claude Code"),
    GEMINI_CLI("gemini_cli", "Gemini CLI"),
    COPILOT_CLI("copilot_cli", "GitHub Copilot"),
    OPEN_CODE("open_code", "OpenCode"),
    CURSOR("cursor", "Cursor Agent"),
    CLINE("cline", "Cline"),
    GOOSE("goose", "Goose"),
    JUNIE("junie", "JetBrains Junie"),
    QWEN_CODE("qwen_code", "Qwen Code"),
    KIMI_CLI("kimi_cli", "Kimi CLI"),
    KIRO_CLI("kiro_cli", "Kiro CLI"),
    MISTRAL_VIBE("mistral_vibe", "Mistral Vibe"),
    QODER_CLI("qoder_cli", "Qoder CLI"),
    AUGGIE("auggie", "Augment Auggie"),
    FACTORY_DROID("factory_droid", "Factory Droid"),
    DEVIN("devin", "Devin"),
    CODEBUDDY("codebuddy", "Tencent CodeBuddy"),
    GLM_AGENT("glm_agent", "GLM Agent"),
    KILO_CODE("kilo_code", "Kilo Code"),
    AMP("amp", "Amp");

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

enum class PermissionRisk(val wire: String) {
    STANDARD("standard"),
    ELEVATED("elevated");

    companion object {
        fun fromWire(value: String): PermissionRisk = entries.first { it.wire == value }
    }
}

data class PermissionModeOption(
    val id: String,
    val displayName: String,
    val description: String,
    val risk: PermissionRisk,
)

data class AttachmentCapability(
    val allowedMimeTypes: List<String>,
    val maxCount: Int,
    val maxBytes: Long,
    val maxTotalBytes: Long,
) {
    val supported: Boolean
        get() = maxCount > 0 && maxBytes > 0 && maxTotalBytes > 0 && allowedMimeTypes.isNotEmpty()
}

data class PromptAttachment(
    val id: UUID,
    val fileName: String,
    val mimeType: String,
    val bytes: ByteArray,
)

data class ProjectSummary(
    val id: UUID,
    val displayName: String,
    val shortPath: String,
    val enabledProviders: List<ProviderId>,
    val valid: Boolean,
    val lastActivityAtMs: Long?,
    val conversationCount: Int,
)

data class ProjectTreeScope(
    val hostId: UUID,
    val provider: ProviderId,
    val projectId: UUID,
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
    val supportsHistory: Boolean,
    val supportsIncrementalSync: Boolean,
    val supportsRename: Boolean,
    val supportsSteer: Boolean,
    val permissionModes: List<PermissionModeOption>,
    val defaultPermissionMode: String?,
    val attachments: AttachmentCapability,
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
    val titleSource: String,
    val titleUpdatedAtMs: Long,
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

data class TimelinePageCursor(
    val createdAtMs: Long,
    val itemId: UUID,
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
    data class SnapshotReceived(val snapshot: Snapshot, val encoded: ByteArray) : ServerEvent
    data class ProjectsUpdated(
        val provider: ProviderId,
        val projects: List<ProjectSummary>,
        val capabilities: List<ProviderCapability>,
    ) : ServerEvent
    data class ProjectSyncCompleted(
        val commandId: UUID,
        val projectId: UUID,
        val provider: ProviderId,
        val conversationsSynced: Int,
        val fullHistoryFallback: Boolean,
    ) : ServerEvent
    data class ConversationPage(
        val conversationId: UUID,
        val items: List<TimelineItem>,
        val nextBefore: TimelinePageCursor?,
        val error: String? = null,
    ) : ServerEvent
    data class ProviderChanged(val capability: ProviderCapability) : ServerEvent
    data class ConversationUpserted(val conversation: Conversation) : ServerEvent
    data class TimelineUpserted(val item: TimelineItem) : ServerEvent
    data class ConversationRemoved(val conversationId: UUID) : ServerEvent
    data class AttachmentReceived(val attachment: AttachmentData) : ServerEvent
    data class HostStatus(val hostId: UUID, val online: Boolean, val message: String?) : ServerEvent
    data class SendTrace(
        val commandId: UUID,
        val clientMessageId: String,
        val conversationId: UUID,
        val stage: String,
        val elapsedMs: Long,
    ) : ServerEvent
    data class CommandAccepted(val commandId: UUID) : ServerEvent
    data class CommandRejected(
        val commandId: UUID?,
        val code: String,
        val message: String,
    ) : ServerEvent
    data class ProtocolError(val supportedVersion: Int, val message: String) : ServerEvent
}
