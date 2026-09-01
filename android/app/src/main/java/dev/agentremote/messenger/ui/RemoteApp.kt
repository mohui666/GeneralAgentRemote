package dev.agentremote.messenger.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.NavigationDrawerItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedCard
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.agentremote.messenger.model.Conversation
import dev.agentremote.messenger.model.ProviderCapability
import dev.agentremote.messenger.model.ProviderId
import dev.agentremote.messenger.model.SessionOption
import dev.agentremote.messenger.model.StoredCredential
import dev.agentremote.messenger.model.TimelineContent
import dev.agentremote.messenger.model.TimelineItem
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import java.util.UUID
import kotlinx.coroutines.launch

private val AgentRemoteColors = lightColorScheme(
    primary = Color(0xFF24483E),
    onPrimary = Color.White,
    primaryContainer = Color(0xFFD8E8E1),
    onPrimaryContainer = Color(0xFF102B24),
    secondary = Color(0xFF8C542F),
    secondaryContainer = Color(0xFFF3E0D2),
    background = Color(0xFFF7F7F2),
    surface = Color(0xFFFFFEF9),
    surfaceVariant = Color(0xFFE8EAE4),
    error = Color(0xFFB3261E),
)

@Composable
fun AgentRemoteTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = AgentRemoteColors, typography = MaterialTheme.typography, content = content)
}

@Composable
fun RemoteApp(viewModel: RemoteViewModel) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    LaunchedEffect(state.error) {
        state.error?.let {
            snackbar.showSnackbar(it)
            viewModel.clearError()
        }
    }

    Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        if (state.snapshot == null) {
            ConnectionScreen(state, viewModel)
        } else {
            ConversationShell(state, viewModel)
        }
        SnackbarHost(
            hostState = snackbar,
            modifier = Modifier.align(Alignment.BottomCenter).navigationBarsPadding().imePadding(),
        )
    }
}

@Composable
private fun ConnectionScreen(state: RemoteUiState, viewModel: RemoteViewModel) {
    val clipboard = LocalClipboardManager.current
    val context = LocalContext.current
    val scanner = remember(context) {
        val options = GmsBarcodeScannerOptions.Builder()
            .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
            .enableAutoZoom()
            .build()
        GmsBarcodeScanning.getClient(context, options)
    }
    Column(
        Modifier
            .fillMaxSize()
            .statusBarsPadding()
            .navigationBarsPadding()
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        Spacer(Modifier.height(24.dp))
        Text("Agent Remote", style = MaterialTheme.typography.displaySmall, fontWeight = FontWeight.Bold)
        Text(
            "在手机上继续电脑里的 Codex 与 Grok 会话。项目文件和 Agent 都留在 Host。",
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        StatusCard(state.phase, state.online)
        OutlinedCard(Modifier.fillMaxWidth()) {
            Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("配对新 Host", style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
                Text(
                    "在电脑运行 agent-remote-host pair，然后粘贴完整链接。也可以从浏览器把链接分享给本应用。",
                    style = MaterialTheme.typography.bodyMedium,
                )
                OutlinedTextField(
                    value = state.pairLink,
                    onValueChange = viewModel::setPairLink,
                    modifier = Modifier.fillMaxWidth(),
                    label = { Text("配对链接") },
                    placeholder = { Text("http://host/#host=…&pair=…") },
                    minLines = 3,
                )
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    OutlinedButton(
                        onClick = {
                            scanner.startScan()
                                .addOnSuccessListener { barcode ->
                                    barcode.rawValue?.let(viewModel::setPairLink)
                                        ?: viewModel.reportError("二维码里没有可读取的配对链接")
                                }
                                .addOnCanceledListener { }
                                .addOnFailureListener { error ->
                                    viewModel.reportError(error.message ?: "无法启动扫码")
                                }
                        },
                        modifier = Modifier.weight(1f),
                    ) { Text("扫码") }
                    OutlinedButton(
                        onClick = { clipboard.getText()?.text?.let(viewModel::setPairLink) },
                        modifier = Modifier.weight(1f),
                    ) { Text("粘贴") }
                }
                Button(
                    onClick = viewModel::pair,
                    enabled = state.pairLink.isNotBlank() && !state.connecting,
                    modifier = Modifier.fillMaxWidth(),
                ) { Text(if (state.connecting) "正在连接…" else "连接并配对") }
            }
        }
        if (state.credentials.isNotEmpty()) {
            Text("已保存 Host", style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
            state.credentials.forEach { credential ->
                SavedHostCard(
                    credential = credential,
                    connecting = state.connecting && state.activeHostId == credential.hostId,
                    onConnect = { viewModel.connect(credential) },
                    onForget = { viewModel.forget(credential) },
                )
            }
        }
    }
}

@Composable
private fun StatusCard(text: String, online: Boolean) {
    Card(
        colors = CardDefaults.cardColors(
            containerColor = if (online) Color(0xFFDDEFE5) else MaterialTheme.colorScheme.surfaceVariant,
        ),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            Modifier.padding(horizontal = 16.dp, vertical = 13.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                Modifier
                    .size(10.dp)
                    .background(if (online) Color(0xFF21834F) else Color(0xFF8A8F89), RoundedCornerShape(99.dp)),
            )
            Spacer(Modifier.width(10.dp))
            Text(text, fontWeight = FontWeight.Medium)
        }
    }
}

@Composable
private fun SavedHostCard(
    credential: StoredCredential,
    connecting: Boolean,
    onConnect: () -> Unit,
    onForget: () -> Unit,
) {
    OutlinedCard(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(5.dp)) {
            Text(credential.displayName, fontWeight = FontWeight.Bold, style = MaterialTheme.typography.titleMedium)
            Text(credential.origin, style = MaterialTheme.typography.bodySmall)
            Text(
                if (credential.relay) "公开 Relay" else "直接连接",
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                style = MaterialTheme.typography.labelMedium,
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                TextButton(onClick = onForget) { Text("删除") }
                Button(onClick = onConnect, enabled = !connecting) { Text(if (connecting) "连接中" else "连接") }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ConversationShell(state: RemoteUiState, viewModel: RemoteViewModel) {
    val snapshot = requireNotNull(state.snapshot)
    val drawerState = androidx.compose.material3.rememberDrawerState(androidx.compose.material3.DrawerValue.Closed)
    val scope = rememberCoroutineScope()
    ModalNavigationDrawer(
        drawerState = drawerState,
        drawerContent = {
            ModalDrawerSheet(Modifier.fillMaxHeight().widthIn(max = 340.dp)) {
                Column(Modifier.statusBarsPadding().padding(16.dp)) {
                    Text(snapshot.hostName, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold)
                    Text(state.phase, style = MaterialTheme.typography.bodySmall)
                    Spacer(Modifier.height(14.dp))
                    Button(
                        onClick = {
                            viewModel.showNewConversation()
                            scope.launch { drawerState.close() }
                        },
                        modifier = Modifier.fillMaxWidth(),
                    ) { Text("新建会话") }
                }
                HorizontalDivider()
                LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(8.dp)) {
                    items(snapshot.conversations, key = { it.id }) { conversation ->
                        NavigationDrawerItem(
                            label = {
                                Column {
                                    Text(conversation.title, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                    Text(
                                        "${conversation.provider.label} · ${stateLabel(conversation.state)}",
                                        style = MaterialTheme.typography.labelSmall,
                                    )
                                }
                            },
                            selected = state.selectedConversationId == conversation.id,
                            onClick = {
                                viewModel.selectConversation(conversation.id)
                                scope.launch { drawerState.close() }
                            },
                        )
                    }
                }
                HorizontalDivider()
                TextButton(
                    onClick = viewModel::disconnect,
                    modifier = Modifier.padding(10.dp).fillMaxWidth(),
                ) { Text("断开 Host") }
                Spacer(Modifier.navigationBarsPadding())
            }
        },
    ) {
        Scaffold(
            topBar = {
                CenterAlignedTopAppBar(
                    title = {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            Text(snapshot.hostName, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(state.phase, style = MaterialTheme.typography.labelSmall)
                        }
                    },
                    navigationIcon = {
                        TextButton(onClick = { scope.launch { drawerState.open() } }) { Text("会话") }
                    },
                    actions = {
                        AssistChip(
                            onClick = {},
                            label = { Text(if (state.online) "在线" else "离线") },
                            enabled = false,
                        )
                        Spacer(Modifier.width(8.dp))
                    },
                    colors = TopAppBarDefaults.topAppBarColors(
                        containerColor = MaterialTheme.colorScheme.background,
                    ),
                    modifier = Modifier.statusBarsPadding(),
                )
            },
        ) { padding ->
            Box(Modifier.fillMaxSize().padding(padding)) {
                val conversation = snapshot.conversations.find { it.id == state.selectedConversationId }
                if (state.showingNewConversation || conversation == null) {
                    NewConversationScreen(state, viewModel)
                } else {
                    ConversationScreen(state, conversation, viewModel)
                }
            }
        }
    }
    BackHandler(enabled = !state.showingNewConversation) { viewModel.showNewConversation() }
}

@Composable
private fun NewConversationScreen(state: RemoteUiState, viewModel: RemoteViewModel) {
    val snapshot = requireNotNull(state.snapshot)
    val projects = snapshot.projects.filter { it.valid }
    val project = projects.find { it.id == state.selectedProjectId }
    val capability = snapshot.providerCapabilities.find {
        it.projectId == state.selectedProjectId && it.provider == state.selectedProvider
    }
    val model = capability?.models?.find { it.id == state.selectedModel }
    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .navigationBarsPadding()
            .padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Text("开始新会话", style = MaterialTheme.typography.headlineMedium, fontWeight = FontWeight.Bold)
        Text("模型和 effort 来自当前 Provider，不在手机端写死。", color = MaterialTheme.colorScheme.onSurfaceVariant)
        Picker(
            label = "项目",
            entries = projects.map { PickerEntry(it.id, it.displayName) },
            selected = state.selectedProjectId,
            onSelect = viewModel::selectProject,
        )
        Picker(
            label = "Agent",
            entries = project?.enabledProviders.orEmpty().map { PickerEntry(it, it.label) },
            selected = state.selectedProvider,
            onSelect = viewModel::selectProvider,
        )
        if (capability != null) {
            ProviderStatus(capability)
        }
        Picker(
            label = "模型",
            entries = capability?.models.orEmpty().map { PickerEntry(it.id, it.displayName) },
            selected = state.selectedModel,
            onSelect = viewModel::selectModel,
            emptyLabel = "Provider 默认模型",
        )
        Picker(
            label = "Reasoning effort",
            entries = model?.effortOptions.orEmpty().map { PickerEntry(it.id, it.displayName) },
            selected = state.selectedEffort,
            onSelect = viewModel::selectEffort,
            emptyLabel = "Provider 默认 effort",
        )
        if (capability?.supportsSessionList == true) {
            Picker(
                label = "已有 Agent 会话（可选）",
                entries = listOf(PickerEntry<String?>(null, "新会话")) + capability.sessions.map {
                    PickerEntry(it.nativeSessionId, it.title)
                },
                selected = state.selectedNativeSession,
                onSelect = viewModel::selectNativeSession,
            )
        }
        Button(
            onClick = viewModel::createConversation,
            enabled = state.online && !state.creatingConversation && state.selectedProjectId != null && state.selectedProvider != null && capability?.ready == true,
            modifier = Modifier.fillMaxWidth().height(52.dp),
        ) { Text(if (state.creatingConversation) "正在创建…" else "创建会话") }
        if (projects.isEmpty()) {
            Text("Host 没有有效授权项目。请先在电脑使用 project add。", color = MaterialTheme.colorScheme.error)
        }
    }
}

@Composable
private fun ProviderStatus(capability: ProviderCapability) {
    val color = if (capability.ready) Color(0xFFDDEFE5) else MaterialTheme.colorScheme.secondaryContainer
    Card(colors = CardDefaults.cardColors(containerColor = color), modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(13.dp), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text("${capability.provider.label}: ${providerStateLabel(capability.state)}", fontWeight = FontWeight.Bold)
            capability.version?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
            capability.detail?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
            capability.limitation?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
        }
    }
}

@Composable
private fun ConversationScreen(
    state: RemoteUiState,
    conversation: Conversation,
    viewModel: RemoteViewModel,
) {
    val snapshot = requireNotNull(state.snapshot)
    val capability = snapshot.providerCapabilities.find {
        it.projectId == conversation.projectId && it.provider == conversation.provider
    }
    val timeline = snapshot.timeline.filter { it.conversationId == conversation.id }
    val listState = rememberLazyListState()
    LaunchedEffect(timeline.size, timeline.lastOrNull()?.revision) {
        if (timeline.isNotEmpty()) listState.animateScrollToItem(timeline.lastIndex)
    }
    Column(Modifier.fillMaxSize().navigationBarsPadding().imePadding()) {
        ConversationHeader(conversation, viewModel)
        if (timeline.isEmpty()) {
            Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                Text("发送第一条消息开始", color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        } else {
            LazyColumn(
                state = listState,
                modifier = Modifier.weight(1f).fillMaxWidth(),
                contentPadding = PaddingValues(14.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                items(timeline, key = { it.id }) { item ->
                    TimelineCard(
                        item = item,
                        attachment = (item.content as? TimelineContent.Image)?.let {
                            state.attachments[it.attachmentId]
                        },
                        approvalPending = (item.content as? TimelineContent.Approval)?.approvalId in state.pendingApprovals,
                        onApproval = viewModel::resolveApproval,
                    )
                }
            }
        }
        Composer(
            draft = state.draft,
            running = conversation.running,
            online = state.online,
            supportsSteer = capability?.supportsSteer == true,
            onDraft = viewModel::setDraft,
            onSend = viewModel::sendMessage,
            onSteer = viewModel::steer,
            onInterrupt = viewModel::interrupt,
        )
    }
}

@Composable
private fun ConversationHeader(conversation: Conversation, viewModel: RemoteViewModel) {
    Column(
        Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surface).padding(14.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(conversation.title, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(
                    "${conversation.provider.label} · ${conversation.selectedModel ?: "默认模型"}",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            StatePill(conversation.state)
        }
        conversation.sessionOptions.forEach { option ->
            SessionOptionPicker(option, enabled = !conversation.running) { value ->
                viewModel.setSessionOption(option.id, value)
            }
        }
    }
}

@Composable
private fun StatePill(state: String) {
    val color = when (state) {
        "running" -> Color(0xFFDCE8FF)
        "needs_approval" -> Color(0xFFFFE7C2)
        "completed" -> Color(0xFFDDEFE5)
        "failed" -> Color(0xFFFFDAD6)
        "interrupted" -> Color(0xFFE8E1F4)
        "offline" -> Color(0xFFE1E3E0)
        else -> MaterialTheme.colorScheme.surfaceVariant
    }
    Surface(color = color, shape = RoundedCornerShape(999.dp)) {
        Text(stateLabel(state), Modifier.padding(horizontal = 10.dp, vertical = 5.dp), style = MaterialTheme.typography.labelMedium)
    }
}

@Composable
private fun SessionOptionPicker(option: SessionOption, enabled: Boolean, onSelect: (String) -> Unit) {
    Picker(
        label = option.displayName,
        entries = option.values.map { PickerEntry(it.value, it.displayName) },
        selected = option.currentValue,
        onSelect = onSelect,
        enabled = enabled,
    )
}

@Composable
private fun Composer(
    draft: String,
    running: Boolean,
    online: Boolean,
    supportsSteer: Boolean,
    onDraft: (String) -> Unit,
    onSend: () -> Unit,
    onSteer: () -> Unit,
    onInterrupt: () -> Unit,
) {
    Surface(shadowElevation = 10.dp) {
        Column(Modifier.fillMaxWidth().padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(
                value = draft,
                onValueChange = onDraft,
                modifier = Modifier.fillMaxWidth(),
                placeholder = { Text(if (running) "输入追加指令…" else "给 Agent 发消息…") },
                minLines = 2,
                maxLines = 6,
                enabled = online,
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (running) {
                    if (supportsSteer) {
                        Button(
                            onClick = onSteer,
                            enabled = online && draft.isNotBlank(),
                            modifier = Modifier.weight(1f),
                        ) { Text("追加指令") }
                    }
                    OutlinedButton(
                        onClick = onInterrupt,
                        enabled = online,
                        modifier = Modifier.weight(1f),
                        colors = ButtonDefaults.outlinedButtonColors(contentColor = MaterialTheme.colorScheme.error),
                    ) { Text("停止") }
                } else {
                    Button(
                        onClick = onSend,
                        enabled = online && draft.isNotBlank(),
                        modifier = Modifier.fillMaxWidth(),
                    ) { Text("发送") }
                }
            }
        }
    }
}

@Composable
private fun TimelineCard(
    item: TimelineItem,
    attachment: ByteArray?,
    approvalPending: Boolean,
    onApproval: (UUID, String) -> Unit,
) {
    when (val content = item.content) {
        is TimelineContent.UserMessage -> MessageBubble(content.text, user = true)
        is TimelineContent.AgentMessage -> MessageBubble(
            text = content.text,
            user = false,
            label = when (content.phase) {
                "final" -> "Agent 最终回复"
                "reasoning_summary" -> "推理摘要"
                else -> "Agent"
            },
        )
        is TimelineContent.Progress -> GenericTimelineCard(
            title = "${content.kind} · ${content.label}",
            status = content.status,
            body = content.detail,
        )
        is TimelineContent.Plan -> GenericTimelineCard(
            title = "计划",
            body = content.steps.joinToString("\n") { "${statusMark(it.status)} ${it.text}" },
        )
        is TimelineContent.ToolCall -> GenericTimelineCard(
            title = "工具 · ${content.name}",
            status = content.status,
            body = listOfNotNull(content.inputSummary, content.outputSummary).joinToString("\n\n").ifBlank { null },
        )
        is TimelineContent.Command -> CodeTimelineCard(content)
        is TimelineContent.FileChange -> GenericTimelineCard(
            title = "文件 · ${content.changeKind}",
            status = content.status,
            body = content.relativePath,
        )
        is TimelineContent.Approval -> ApprovalCard(content, approvalPending, onApproval)
        is TimelineContent.Image -> ImageCard(content.alt, attachment)
        is TimelineContent.Error -> GenericTimelineCard(
            title = "错误 · ${content.code}",
            body = content.message,
            error = true,
        )
    }
}

@Composable
private fun MessageBubble(text: String, user: Boolean, label: String = "你") {
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = if (user) Arrangement.End else Arrangement.Start,
    ) {
        Card(
            modifier = Modifier.fillMaxWidth(if (user) 0.86f else 0.96f),
            colors = CardDefaults.cardColors(
                containerColor = if (user) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surface,
            ),
        ) {
            Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(5.dp)) {
                Text(label, style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.Bold)
                Text(text, style = MaterialTheme.typography.bodyLarge)
            }
        }
    }
}

@Composable
private fun GenericTimelineCard(
    title: String,
    status: String? = null,
    body: String? = null,
    error: Boolean = false,
) {
    OutlinedCard(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.outlinedCardColors(
            containerColor = if (error) MaterialTheme.colorScheme.errorContainer else MaterialTheme.colorScheme.surface,
        ),
    ) {
        Column(Modifier.padding(13.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row {
                Text(title, fontWeight = FontWeight.Bold, modifier = Modifier.weight(1f))
                status?.let { Text(stateLabel(it), style = MaterialTheme.typography.labelMedium) }
            }
            body?.let { Text(it) }
        }
    }
}

@Composable
private fun CodeTimelineCard(content: TimelineContent.Command) {
    OutlinedCard(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(13.dp), verticalArrangement = Arrangement.spacedBy(7.dp)) {
            Text("命令 · ${stateLabel(content.status)}", fontWeight = FontWeight.Bold)
            val scroll = rememberScrollState()
            Text(
                content.command,
                fontFamily = FontFamily.Monospace,
                modifier = Modifier.fillMaxWidth().horizontalScroll(scroll),
            )
            content.relativeCwd?.let { Text("cwd: $it", style = MaterialTheme.typography.bodySmall) }
            content.output?.let {
                Text(it, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall)
            }
            content.exitCode?.let { Text("exit $it", style = MaterialTheme.typography.labelSmall) }
        }
    }
}

@Composable
private fun ApprovalCard(
    content: TimelineContent.Approval,
    pending: Boolean,
    onApproval: (UUID, String) -> Unit,
) {
    Card(
        colors = CardDefaults.cardColors(containerColor = Color(0xFFFFE7C2)),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(Modifier.padding(15.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text("需要你的许可", fontWeight = FontWeight.Bold, style = MaterialTheme.typography.titleMedium)
            Text(content.prompt)
            if (content.resolvedOption != null) {
                Text("已选择：${content.resolvedOption}", fontWeight = FontWeight.SemiBold)
            } else {
                content.options.forEach { option ->
                    Button(
                        onClick = { onApproval(content.approvalId, option.id) },
                        enabled = !pending,
                        modifier = Modifier.fillMaxWidth(),
                    ) { Text(if (pending) "正在提交…" else option.label) }
                }
            }
        }
    }
}

@Composable
private fun ImageCard(alt: String, bytes: ByteArray?) {
    var fullscreen by remember { mutableStateOf(false) }
    val bitmap = remember(bytes) { bytes?.let(::decodePreview) }
    OutlinedCard(
        Modifier.fillMaxWidth().clickable(enabled = bitmap != null) { fullscreen = true },
    ) {
        Column(Modifier.padding(10.dp), verticalArrangement = Arrangement.spacedBy(7.dp)) {
            if (bitmap == null) {
                Box(Modifier.fillMaxWidth().height(180.dp), contentAlignment = Alignment.Center) {
                    Text(if (bytes == null) "正在读取图片…" else "无法解码图片")
                }
            } else {
                Image(
                    bitmap = bitmap.asImageBitmap(),
                    contentDescription = alt,
                    contentScale = ContentScale.Fit,
                    modifier = Modifier.fillMaxWidth().height(240.dp),
                )
            }
            Text(alt, style = MaterialTheme.typography.bodySmall)
        }
    }
    if (fullscreen && bitmap != null) {
        Dialog(
            onDismissRequest = { fullscreen = false },
            properties = DialogProperties(usePlatformDefaultWidth = false),
        ) {
            Box(
                Modifier.fillMaxSize().background(Color.Black).clickable { fullscreen = false },
                contentAlignment = Alignment.Center,
            ) {
                Image(
                    bitmap = bitmap.asImageBitmap(),
                    contentDescription = alt,
                    contentScale = ContentScale.Fit,
                    modifier = Modifier.fillMaxSize().padding(16.dp),
                )
                TextButton(
                    onClick = { fullscreen = false },
                    modifier = Modifier.align(Alignment.TopEnd).statusBarsPadding().padding(8.dp),
                ) { Text("关闭", color = Color.White) }
            }
        }
    }
}

private fun decodePreview(bytes: ByteArray): Bitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
    var sample = 1
    while (bounds.outWidth / sample > 2048 || bounds.outHeight / sample > 2048) sample *= 2
    return BitmapFactory.decodeByteArray(
        bytes,
        0,
        bytes.size,
        BitmapFactory.Options().apply { inSampleSize = sample },
    )
}

private data class PickerEntry<T>(val value: T, val label: String)

@Composable
private fun <T> Picker(
    label: String,
    entries: List<PickerEntry<T>>,
    selected: T?,
    onSelect: (T) -> Unit,
    enabled: Boolean = true,
    emptyLabel: String = "暂无选项",
) {
    var expanded by remember { mutableStateOf(false) }
    val selectedLabel = entries.find { it.value == selected }?.label ?: emptyLabel
    Column(verticalArrangement = Arrangement.spacedBy(5.dp)) {
        Text(label, style = MaterialTheme.typography.labelLarge)
        Box(Modifier.fillMaxWidth()) {
            OutlinedButton(
                onClick = { expanded = true },
                enabled = enabled && entries.isNotEmpty(),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(selectedLabel, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                Text("▾")
            }
            DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
                entries.forEach { entry ->
                    DropdownMenuItem(
                        text = { Text(entry.label) },
                        onClick = {
                            expanded = false
                            onSelect(entry.value)
                        },
                    )
                }
            }
        }
    }
}

private fun providerStateLabel(state: String): String = when (state) {
    "not_installed" -> "未安装"
    "not_authenticated" -> "未登录"
    "starting" -> "正在启动"
    "ready" -> "可用"
    "crashed" -> "已崩溃"
    "protocol_incompatible" -> "协议不兼容"
    "offline" -> "离线"
    else -> state
}

private fun stateLabel(state: String): String = when (state) {
    "idle" -> "空闲"
    "running" -> "运行中"
    "needs_approval" -> "等待审批"
    "completed" -> "已完成"
    "failed" -> "失败"
    "interrupted" -> "已停止"
    "offline" -> "离线"
    "pending" -> "等待中"
    "declined" -> "已拒绝"
    else -> state
}

private fun statusMark(status: String): String = when (status) {
    "completed" -> "✓"
    "running" -> "→"
    "failed" -> "×"
    "interrupted" -> "■"
    else -> "•"
}
