package dev.agentremote.messenger.debug

import android.os.Bundle
import dev.agentremote.messenger.model.Conversation
import dev.agentremote.messenger.model.ProjectSummary
import dev.agentremote.messenger.model.ProviderId
import dev.agentremote.messenger.model.TimelineItem
import dev.agentremote.messenger.ui.RemoteUiState
import dev.agentremote.messenger.ui.RemoteViewModel
import java.lang.ref.WeakReference
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject

/** Process-local bridge used by the debug-only ADB command receiver. */
object NativeDebugBridge {
    private var viewModelRef: WeakReference<RemoteViewModel>? = null

    @Synchronized
    fun attach(viewModel: RemoteViewModel) {
        viewModelRef = WeakReference(viewModel)
    }

    @Synchronized
    fun detach(viewModel: RemoteViewModel) {
        if (viewModelRef?.get() === viewModel) viewModelRef = null
    }

    fun execute(command: String, arguments: Bundle): JSONObject {
        val viewModel = synchronized(this) { viewModelRef?.get() }
            ?: return failure("app_not_ready", "先启动 Agent Remote，再执行调试命令")
        return runCatching {
            when (command.trim().lowercase().replace('-', '_')) {
                "help" -> help()
                "status" -> success("status", stateJson(viewModel.state.value))
                "projects" -> success("projects", projectsJson(viewModel.state.value))
                "conversations" -> success(
                    "conversations",
                    conversationsJson(viewModel.state.value, arguments.string("project_id")),
                )
                "dump" -> success(
                    "dump",
                    JSONObject()
                        .put("state", stateJson(viewModel.state.value))
                        .put("projects", projectsJson(viewModel.state.value).getJSONArray("projects"))
                        .put(
                            "conversations",
                            conversationsJson(viewModel.state.value, arguments.string("project_id"))
                                .getJSONArray("conversations"),
                        ),
                )
                "select_project" -> {
                    val id = arguments.requiredUuid("id")
                    val project = requireProject(viewModel.state.value, id)
                    val requestedProvider = arguments.string("provider")?.let(ProviderId::fromWire)
                    requestedProvider?.let { provider ->
                        require(provider in project.enabledProviders) {
                            "项目 ${project.displayName} 未启用 ${provider.label}"
                        }
                        viewModel.selectProvider(provider)
                    }
                    viewModel.selectProject(id)
                    require(viewModel.state.value.selectedProjectId == id) {
                        "项目切换被当前待处理发送锁定"
                    }
                    require(
                        requestedProvider == null ||
                            viewModel.state.value.selectedProvider == requestedProvider,
                    ) {
                        "Provider 切换被当前待处理发送锁定"
                    }
                    success("select_project", stateJson(viewModel.state.value))
                }
                "select_conversation" -> {
                    val id = arguments.requiredUuid("id")
                    val conversation = requireConversation(viewModel.state.value, id)
                    if (viewModel.state.value.selectedProvider != conversation.provider) {
                        viewModel.selectProvider(conversation.provider)
                    }
                    if (viewModel.state.value.selectedProjectId != conversation.projectId) {
                        viewModel.selectProject(conversation.projectId)
                    }
                    viewModel.selectConversation(id)
                    require(viewModel.state.value.selectedConversationId == id) {
                        "对话切换被当前待处理发送锁定"
                    }
                    success("select_conversation", stateJson(viewModel.state.value))
                }
                "new_conversation" -> {
                    viewModel.showNewConversation()
                    require(viewModel.state.value.showingNewConversation) {
                        "新建对话被当前待处理发送锁定"
                    }
                    success("new_conversation", stateJson(viewModel.state.value))
                }
                "show_conversations" -> {
                    viewModel.showConversationList()
                    val current = viewModel.state.value
                    require(!current.showingNewConversation && current.selectedConversationId == null) {
                        "返回对话列表被当前待处理发送锁定"
                    }
                    success("show_conversations", stateJson(current))
                }
                "set_draft" -> {
                    val text = arguments.requiredString("text")
                    viewModel.setDraft(text)
                    require(viewModel.state.value.draft == text) { "草稿没有更新" }
                    success("set_draft", stateJson(viewModel.state.value))
                }
                "send" -> {
                    arguments.string("text")?.let(viewModel::setDraft)
                    val before = viewModel.state.value
                    require(before.online) { "Host 当前不在线" }
                    require(before.draft.isNotBlank()) { "草稿为空" }
                    viewModel.sendMessage()
                    require(commandWasQueued(before, viewModel.state.value)) {
                        "消息未进入发送队列；可能已有一条发送等待确认"
                    }
                    success("send", stateJson(viewModel.state.value))
                }
                "steer" -> {
                    arguments.string("text")?.let(viewModel::setDraft)
                    val before = viewModel.state.value
                    require(before.online) { "Host 当前不在线" }
                    require(before.selectedConversationId != null) { "尚未选择对话" }
                    require(before.draft.isNotBlank()) { "草稿为空" }
                    viewModel.steer()
                    require(commandWasQueued(before, viewModel.state.value)) {
                        "追加指令未进入发送队列；可能已有一条发送等待确认"
                    }
                    success("steer", stateJson(viewModel.state.value))
                }
                "interrupt" -> {
                    val before = viewModel.state.value
                    require(before.online) { "Host 当前不在线" }
                    require(before.selectedConversationId != null) { "尚未选择对话" }
                    viewModel.interrupt()
                    require(commandWasQueued(before, viewModel.state.value)) { "停止命令未进入发送队列" }
                    success("interrupt", stateJson(viewModel.state.value))
                }
                "retry" -> {
                    viewModel.retryNow()
                    success("retry", stateJson(viewModel.state.value))
                }
                "disconnect" -> {
                    viewModel.disconnect()
                    require(!viewModel.state.value.online) { "断开命令未生效" }
                    success("disconnect", stateJson(viewModel.state.value))
                }
                "pair" -> {
                    viewModel.setPairLink(arguments.requiredString("text"))
                    viewModel.pair()
                    require(viewModel.state.value.connecting) { "配对没有开始" }
                    success("pair", stateJson(viewModel.state.value))
                }
                "connect_host" -> {
                    val hostId = arguments.requiredUuid("id")
                    val credential = viewModel.state.value.credentials.find { it.hostId == hostId }
                        ?: error("未找到 Host $hostId")
                    viewModel.connect(credential)
                    require(viewModel.state.value.activeHostId == hostId) { "Host 切换没有开始" }
                    success("connect_host", stateJson(viewModel.state.value))
                }
                else -> failure("unknown_command", "未知命令：$command；执行 help 查看命令")
            }
        }.getOrElse { error ->
            failure("command_failed", error.message ?: error::class.java.simpleName)
        }
    }

    private fun commandWasQueued(before: RemoteUiState, after: RemoteUiState): Boolean =
        after.pendingCommands.any { it !in before.pendingCommands }

    private fun help(): JSONObject = success(
        "help",
        JSONObject().put(
            "commands",
            JSONArray(
                listOf(
                    "status",
                    "dump",
                    "projects",
                    "conversations [--es project_id UUID]",
                    "select_project --es id UUID [--es provider codex|grok]",
                    "select_conversation --es id UUID",
                    "new_conversation",
                    "show_conversations",
                    "set_draft --es text TEXT",
                    "send [--es text TEXT]",
                    "steer [--es text TEXT]",
                    "interrupt",
                    "retry",
                    "disconnect",
                    "pair --es text PAIR_URL",
                    "connect_host --es id HOST_UUID",
                ),
            ),
        ),
    )

    private fun stateJson(state: RemoteUiState): JSONObject {
        val snapshot = state.snapshot
        val selectedConversation = snapshot?.conversations?.find { it.id == state.selectedConversationId }
        val selectedTimeline = snapshot?.timeline.orEmpty().filter {
            it.conversationId == state.selectedConversationId
        }
        val lastItem = selectedTimeline.maxWithOrNull(
            compareBy<TimelineItem> { it.createdAtMs }.thenBy { it.id },
        )
        return JSONObject()
            .put("phase", state.phase)
            .put("online", state.online)
            .put("connecting", state.connecting)
            .put("retry_enabled", state.retryEnabled)
            .put("host_id", state.activeHostId?.toString().jsonValue())
            .put("host_name", snapshot?.hostName.jsonValue())
            .put("provider", state.selectedProvider?.wire.jsonValue())
            .put("project_id", state.selectedProjectId?.toString().jsonValue())
            .put("conversation_id", state.selectedConversationId?.toString().jsonValue())
            .put("conversation_state", selectedConversation?.state.jsonValue())
            .put("showing_new_conversation", state.showingNewConversation)
            .put("send_in_flight", state.creatingConversation)
            .put("draft_length", state.draft.length)
            .put("prompt_attachment_count", state.promptAttachments.size)
            .put("pending_command_count", state.pendingCommands.size)
            .put(
                "pending_command_ids",
                JSONArray(state.pendingCommands.map(UUID::toString).sorted()),
            )
            .put("pending_approval_count", state.pendingApprovals.size)
            .put("project_count", snapshot?.projects?.size ?: 0)
            .put("conversation_count", snapshot?.conversations?.size ?: 0)
            .put("selected_timeline_count", selectedTimeline.size)
            .put("last_timeline_revision", lastItem?.revision.jsonValue())
            .put("last_timeline_kind", lastItem?.content?.javaClass?.simpleName.jsonValue())
    }

    private fun projectsJson(state: RemoteUiState): JSONObject {
        val conversations = state.snapshot?.conversations.orEmpty()
        val projects = state.snapshot?.projects.orEmpty().map { project ->
            JSONObject()
                .put("id", project.id.toString())
                .put("name", project.displayName)
                .put("path", project.shortPath)
                .put("valid", project.valid)
                .put("selected", project.id == state.selectedProjectId)
                .put("providers", JSONArray(project.enabledProviders.map { it.wire }.sorted()))
                .put(
                    "conversation_count",
                    conversations.count { it.projectId == project.id },
                )
                .put("last_activity_at_ms", project.lastActivityAtMs.jsonValue())
        }
        return JSONObject().put("projects", JSONArray(projects))
    }

    private fun conversationsJson(state: RemoteUiState, requestedProjectId: String?): JSONObject {
        val projectId = requestedProjectId?.let(UUID::fromString) ?: state.selectedProjectId
        val conversations = state.snapshot?.conversations.orEmpty()
            .asSequence()
            .filter { projectId == null || it.projectId == projectId }
            .sortedWith(
                compareByDescending<Conversation> { it.updatedAtMs }
                    .thenBy { it.id },
            )
            .map { conversation ->
                JSONObject()
                    .put("id", conversation.id.toString())
                    .put("project_id", conversation.projectId.toString())
                    .put("provider", conversation.provider.wire)
                    .put("title", conversation.title)
                    .put("state", conversation.state)
                    .put("selected", conversation.id == state.selectedConversationId)
                    .put("updated_at_ms", conversation.updatedAtMs)
            }
            .toList()
        return JSONObject()
            .put("project_id", projectId?.toString().jsonValue())
            .put("conversations", JSONArray(conversations))
    }

    private fun requireProject(state: RemoteUiState, id: UUID): ProjectSummary =
        state.snapshot?.projects?.find { it.id == id } ?: error("未找到项目 $id")

    private fun requireConversation(state: RemoteUiState, id: UUID): Conversation =
        state.snapshot?.conversations?.find { it.id == id } ?: error("未找到对话 $id")

    private fun success(command: String, data: JSONObject): JSONObject = JSONObject()
        .put("ok", true)
        .put("command", command)
        .put("data", data)

    private fun failure(code: String, message: String): JSONObject = JSONObject()
        .put("ok", false)
        .put("code", code)
        .put("message", message)

    private fun Bundle.string(name: String): String? = getString(name)?.takeIf { it.isNotBlank() }

    private fun Bundle.requiredString(name: String): String =
        string(name) ?: error("缺少参数：$name")

    private fun Bundle.requiredUuid(name: String): UUID = UUID.fromString(requiredString(name))

    private fun Any?.jsonValue(): Any = this ?: JSONObject.NULL
}
