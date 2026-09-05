package dev.agentremote.messenger.ui

import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.collectIsDraggedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.outlined.Security
import androidx.compose.material.icons.outlined.TouchApp
import androidx.compose.material.icons.outlined.WarningAmber
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.ArrowDownward
import androidx.compose.material.icons.rounded.ArrowUpward
import androidx.compose.material.icons.rounded.AttachFile
import androidx.compose.material.icons.rounded.ChatBubbleOutline
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.History
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.Menu
import androidx.compose.material.icons.rounded.MoreHoriz
import androidx.compose.material.icons.rounded.PowerSettingsNew
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Stop
import androidx.compose.material.icons.rounded.AutoAwesome
import androidx.compose.material.icons.rounded.PushPin
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import dev.agentremote.messenger.model.Conversation
import dev.agentremote.messenger.model.PermissionModeOption
import dev.agentremote.messenger.model.PermissionRisk
import dev.agentremote.messenger.model.PromptAttachment
import dev.agentremote.messenger.model.ProjectSummary
import dev.agentremote.messenger.model.ProjectTreeScope
import dev.agentremote.messenger.model.ProviderCapability
import dev.agentremote.messenger.model.ProviderId
import dev.agentremote.messenger.model.SessionOption
import dev.agentremote.messenger.model.StoredCredential
import dev.agentremote.messenger.model.TimelineContent
import dev.agentremote.messenger.model.TimelineItem
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.UUID
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.distinctUntilChanged

@Composable
fun RemoteApp(viewModel: RemoteViewModel) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner, viewModel) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP || event == Lifecycle.Event.ON_DESTROY) {
                viewModel.flushState()
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }
    LaunchedEffect(state.error) {
        state.error?.let {
            snackbar.showSnackbar(it)
            viewModel.clearError()
        }
    }

    Box(
        Modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.background)
            .semantics { testTagsAsResourceId = true },
    ) {
        if (state.snapshot == null || state.showingConnections) {
            ConnectionScreen(state, viewModel)
        } else {
            ConversationShell(state, viewModel)
        }
        SnackbarHost(
            hostState = snackbar,
            modifier = Modifier.align(Alignment.BottomCenter).navigationBarsPadding().imePadding(),
        )
    }
    BackHandler(enabled = state.showingConnections && state.snapshot != null) {
        viewModel.hideConnections()
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
            .windowInsetsPadding(WindowInsets.ime.union(WindowInsets.navigationBars))
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 20.dp, vertical = 18.dp),
        verticalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (state.snapshot != null) {
                FloatingIconButton(RemoteGlyph.Back, "返回会话", viewModel::hideConnections)
            }
            Column {
                Text("远程", style = MaterialTheme.typography.titleLarge)
                Text("Agent Remote", color = RemoteMuted, style = MaterialTheme.typography.bodySmall)
            }
            StatusCard(state.phase, state.online)
        }
        Spacer(Modifier.height(18.dp))
        Text("连接你的电脑", style = MaterialTheme.typography.displaySmall)
        Text(
            "继续电脑上的会话，随时查看任务进展。",
            style = MaterialTheme.typography.bodyLarge,
            color = RemoteMuted,
        )
        Surface(
            modifier = Modifier.fillMaxWidth(),
            color = RemoteSurface,
            shape = RoundedCornerShape(12.dp),
            border = BorderStroke(1.dp, RemoteBorder),
        ) {
            Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                Text("添加电脑", style = MaterialTheme.typography.titleLarge)
                Text(
                    "在电脑运行 agent-remote-host pair，然后粘贴完整链接。也可以从浏览器把链接分享给本应用。",
                    style = MaterialTheme.typography.bodyMedium,
                    color = RemoteMuted,
                )
                OutlinedTextField(
                    value = state.pairLink,
                    onValueChange = viewModel::setPairLink,
                    modifier = Modifier.fillMaxWidth(),
                    label = { Text("配对链接") },
                    placeholder = { Text("http://host/#host=…&pair=…") },
                    minLines = 3,
                    shape = RoundedCornerShape(12.dp),
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
                        modifier = Modifier.weight(1f).height(48.dp),
                        shape = RoundedCornerShape(12.dp),
                        border = BorderStroke(1.dp, RemoteBorder),
                    ) {
                        RemoteIcon(RemoteGlyph.Scan, "扫码", Modifier.size(20.dp))
                        Spacer(Modifier.width(8.dp))
                        Text("扫码")
                    }
                    OutlinedButton(
                        onClick = { clipboard.getText()?.text?.let(viewModel::setPairLink) },
                        modifier = Modifier.weight(1f).height(48.dp),
                        shape = RoundedCornerShape(12.dp),
                        border = BorderStroke(1.dp, RemoteBorder),
                    ) {
                        RemoteIcon(RemoteGlyph.Paste, "粘贴", Modifier.size(20.dp))
                        Spacer(Modifier.width(8.dp))
                        Text("粘贴")
                    }
                }
                Button(
                    onClick = viewModel::pair,
                    enabled = state.pairLink.isNotBlank() && !state.connecting,
                    modifier = Modifier.fillMaxWidth().height(52.dp),
                    shape = RoundedCornerShape(12.dp),
                ) { Text(if (state.connecting) "正在连接…" else "连接并配对") }
            }
        }
        if (state.credentials.isNotEmpty()) {
            Text("保存的电脑", style = MaterialTheme.typography.titleLarge)
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
    Surface(
        modifier = Modifier.testTag("gar.connection.status"),
        color = RemoteSurfaceRaised,
        shape = RoundedCornerShape(10.dp),
        border = BorderStroke(1.dp, RemoteBorder),
    ) {
        Row(
            Modifier.padding(horizontal = 13.dp, vertical = 9.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            OnlineDot(online)
            Spacer(Modifier.width(8.dp))
            Text(text, style = MaterialTheme.typography.labelMedium, maxLines = 1)
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
    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = RemoteSurface,
        shape = RoundedCornerShape(12.dp),
        border = BorderStroke(1.dp, RemoteBorder),
    ) {
        Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                RemoteIcon(RemoteGlyph.Computer, null, Modifier.size(22.dp), RemoteMuted)
                Spacer(Modifier.width(10.dp))
                Text(credential.displayName, fontWeight = FontWeight.SemiBold, style = MaterialTheme.typography.titleMedium)
            }
            Text(credential.origin, style = MaterialTheme.typography.bodySmall, color = RemoteMuted)
            Text(
                if (credential.relay) "公开 Relay" else "直接连接",
                color = RemoteMuted,
                style = MaterialTheme.typography.labelMedium,
            )
            Row(
                Modifier.fillMaxWidth().padding(top = 6.dp),
                horizontalArrangement = Arrangement.End,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                TextButton(onClick = onForget) { Text("删除") }
                Spacer(Modifier.width(4.dp))
                Button(
                    onClick = onConnect,
                    enabled = !connecting,
                    shape = RoundedCornerShape(12.dp),
                ) { Text(if (connecting) "连接中" else "连接") }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ConversationShell(state: RemoteUiState, viewModel: RemoteViewModel) {
    val compactInput = useCompactInputLayout()
    val snapshot = requireNotNull(state.snapshot)
    val drawerState = androidx.compose.material3.rememberDrawerState(androidx.compose.material3.DrawerValue.Closed)
    val scope = rememberCoroutineScope()
    val selectedConversation = remember(
        snapshot.conversations,
        state.selectedConversationId,
        state.selectedProjectId,
        state.selectedProvider,
    ) {
        snapshot.conversations.find {
            it.id == state.selectedConversationId &&
                it.projectId == state.selectedProjectId &&
                it.provider == state.selectedProvider
        }
    }
    var menuExpanded by rememberSaveable { mutableStateOf(false) }
    var sortMode by rememberSaveable { mutableStateOf(HomeSort.AGENT) }
    var searchOpen by rememberSaveable(
        state.activeHostId, state.selectedProvider, state.selectedProjectId, state.selectedConversationId,
    ) { mutableStateOf(false) }

    ModalNavigationDrawer(
        drawerState = drawerState,
        drawerContent = {
            RemoteDrawer(
                state = state,
                onProject = { projectId ->
                    viewModel.selectProject(projectId)
                    viewModel.showConversationList()
                    scope.launch { drawerState.close() }
                },
                onConversation = { projectId, provider, conversationId ->
                    viewModel.selectConversation(projectId, provider, conversationId)
                    scope.launch { drawerState.close() }
                },
                onToggleProject = viewModel::toggleProjectExpanded,
                onProvider = viewModel::selectProvider,
                onProjectSearch = viewModel::setProjectSearch,
                onPinProject = viewModel::toggleProjectPin,
                onNewConversation = {
                    viewModel.showNewConversation()
                    scope.launch { drawerState.close() }
                },
                onRetryNow = viewModel::retryNow,
                onStopRetrying = viewModel::stopRetrying,
                onDisconnect = viewModel::disconnect,
            )
        },
    ) {
        Scaffold(
            containerColor = RemoteBackground,
            contentWindowInsets = WindowInsets(0, 0, 0, 0),
            topBar = {
                if (compactInput) {
                    Spacer(Modifier.fillMaxWidth().statusBarsPadding())
                } else RemoteTopBar(
                    hostName = snapshot.hostName,
                    conversation = selectedConversation.takeUnless { state.showingNewConversation },
                    title = if (state.showingNewConversation) "新对话" else "会话",
                    subtitle = listOfNotNull(state.selectedProvider?.label, snapshot.projects.find { it.id == state.selectedProjectId }?.displayName).joinToString(" · "),
                    onRename = viewModel::renameConversation,
                    onSearch = { searchOpen = !searchOpen },
                    online = state.online,
                    menuExpanded = menuExpanded,
                    sortMode = sortMode,
                    onNavigation = { scope.launch { drawerState.open() } },
                    onToggleMenu = { menuExpanded = !menuExpanded },
                    onDismissMenu = { menuExpanded = false },
                    onSort = {
                        sortMode = it
                        menuExpanded = false
                        viewModel.showConversationList()
                    },
                    onShowProjects = {
                        menuExpanded = false
                        scope.launch { drawerState.open() }
                    },
                    onManageConnections = {
                        menuExpanded = false
                        viewModel.showConnections()
                    },
                    onDisconnect = {
                        menuExpanded = false
                        viewModel.disconnect()
                    },
                )
            },
        ) { padding ->
            Box(Modifier.fillMaxSize().padding(padding)) {
                when {
                    state.showingNewConversation -> NewConversationScreen(state, viewModel)
                    selectedConversation != null -> ConversationScreen(
                        state, selectedConversation, viewModel, searchOpen, onCloseSearch = { searchOpen = false },
                    )
                    else -> RemoteHomeScreen(state, sortMode, viewModel, viewModel::toggleProjectExpanded)
                }
            }
        }
    }
    BackHandler(enabled = drawerState.isOpen || state.showingNewConversation || selectedConversation != null) {
        if (drawerState.isOpen) {
            scope.launch { drawerState.close() }
        } else if (searchOpen) {
            searchOpen = false
        } else {
            viewModel.showConversationList()
        }
    }
}

private enum class HomeSort { AGENT, RECENT, ACTIVE }

internal fun conversationsByProjectForProvider(
    conversations: List<Conversation>,
    provider: ProviderId?,
): Map<UUID, List<Conversation>> = conversations
    .asSequence()
    .filter { it.provider == provider }
    .groupBy(Conversation::projectId)
    .mapValues { (_, scoped) -> scoped.sortedByDescending(Conversation::updatedAtMs) }

internal fun projectTreeMatchesSearch(
    project: ProjectSummary,
    conversations: List<Conversation>,
    query: String,
): Boolean = query.isBlank() ||
    project.displayName.contains(query, ignoreCase = true) ||
    project.shortPath.contains(query, ignoreCase = true) ||
    conversations.any { it.title.contains(query, ignoreCase = true) }

internal fun usesCompactDrawerLayout(maxHeightDp: Float): Boolean = maxHeightDp <= 480f

internal fun availableAgentProviders(projects: List<ProjectSummary>): List<ProviderId> {
    val enabled = projects.asSequence()
        .filter(ProjectSummary::valid)
        .flatMap { it.enabledProviders.asSequence() }
        .toSet()
    return ProviderId.entries.filter(enabled::contains)
}

internal fun isHomeProjectExpanded(
    expandedScopes: Set<ProjectTreeScope>,
    hostId: UUID,
    provider: ProviderId?,
    projectId: UUID?,
): Boolean = provider != null && projectId != null &&
    projectTreeScope(hostId, provider, projectId) in expandedScopes

@Composable
private fun RemoteDrawer(
    state: RemoteUiState,
    onProject: (UUID) -> Unit,
    onConversation: (UUID, ProviderId, UUID) -> Unit,
    onToggleProject: (UUID) -> Unit,
    onProvider: (ProviderId) -> Unit,
    onProjectSearch: (String) -> Unit,
    onPinProject: (UUID) -> Unit,
    onNewConversation: () -> Unit,
    onRetryNow: () -> Unit,
    onStopRetrying: () -> Unit,
    onDisconnect: () -> Unit,
) {
    val snapshot = requireNotNull(state.snapshot)
    val provider = state.selectedProvider
    val availableProviders = remember(snapshot.projects) { availableAgentProviders(snapshot.projects) }
    val query = state.projectSearch.trim()
    val providerConversations = remember(snapshot.conversations, provider) {
        conversationsByProjectForProvider(snapshot.conversations, provider)
    }
    val matchingProjects = remember(snapshot.projects, providerConversations, provider, query) {
        snapshot.projects
            .asSequence()
            .filter { it.valid && provider in it.enabledProviders }
            .filter { project -> projectTreeMatchesSearch(project, providerConversations[project.id].orEmpty(), query) }
            .sortedWith(
                compareByDescending<ProjectSummary> { project ->
                    maxOf(
                        project.lastActivityAtMs ?: Long.MIN_VALUE,
                        providerConversations[project.id]?.maxOfOrNull(Conversation::updatedAtMs) ?: Long.MIN_VALUE,
                    )
                }.thenBy { it.displayName.lowercase() },
            )
            .toList()
    }
    val pinnedProjects = matchingProjects.filter { it.id in state.pinnedProjects }
    val recentProjects = state.recentProjects.mapNotNull { recentId ->
        matchingProjects.find { it.id == recentId && it.id !in state.pinnedProjects }
    }.distinctBy(ProjectSummary::id)
    val remainingProjects = matchingProjects.filter {
        it.id !in state.pinnedProjects && it.id !in state.recentProjects
    }
    BoxWithConstraints(Modifier.fillMaxHeight()) {
        val drawerWidth = minOf(280.dp, (maxWidth - 32.dp).coerceAtLeast(0.dp))
        val compactHeight = usesCompactDrawerLayout(maxHeight.value)
        ModalDrawerSheet(
            modifier = Modifier.fillMaxHeight().width(drawerWidth),
            drawerContainerColor = RemoteSurface,
            drawerContentColor = RemoteText,
        ) {
            Column(
                Modifier
                    .statusBarsPadding()
                    .padding(horizontal = if (compactHeight) 8.dp else 12.dp, vertical = if (compactHeight) 4.dp else 10.dp),
            ) {
                if (compactHeight) {
                    Column(Modifier.fillMaxWidth().height(44.dp), verticalArrangement = Arrangement.Center) {
                        Text("Agent Remote", style = MaterialTheme.typography.titleMedium, maxLines = 1)
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            OnlineDot(state.online)
                            Spacer(Modifier.width(6.dp))
                            Text(
                                snapshot.hostName,
                                color = RemoteMuted,
                                style = MaterialTheme.typography.labelSmall,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                    Spacer(Modifier.height(4.dp))
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        AgentSelector(
                            providers = availableProviders,
                            selected = provider,
                            modifier = Modifier.weight(1.2f),
                            onProvider = onProvider,
                        )
                        Button(
                            onClick = onNewConversation,
                            enabled = state.selectedProjectId != null && provider != null,
                            modifier = Modifier
                                .weight(1.35f)
                                .height(44.dp)
                                .testTag("gar.conversation.new"),
                            contentPadding = PaddingValues(horizontal = 4.dp),
                            shape = RoundedCornerShape(12.dp),
                        ) {
                            RemoteIcon(RemoteGlyph.Compose, null, Modifier.size(18.dp))
                            Spacer(Modifier.width(3.dp))
                            Text("新建", maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                } else {
                    Text("Agent Remote", style = MaterialTheme.typography.titleLarge, maxLines = 1)
                    Row(
                        Modifier.padding(top = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        OnlineDot(state.online)
                        Spacer(Modifier.width(7.dp))
                        Text(snapshot.hostName, color = RemoteMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    Spacer(Modifier.height(8.dp))
                    AgentSelector(
                        providers = availableProviders,
                        selected = provider,
                        modifier = Modifier.fillMaxWidth(),
                        onProvider = onProvider,
                    )
                    Spacer(Modifier.height(7.dp))
                    Button(
                        onClick = onNewConversation,
                        enabled = state.selectedProjectId != null && provider != null,
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(46.dp)
                            .testTag("gar.conversation.new"),
                        shape = RoundedCornerShape(12.dp),
                    ) {
                        RemoteIcon(RemoteGlyph.Compose, null, Modifier.size(19.dp))
                        Spacer(Modifier.width(7.dp))
                        Text("新建对话", maxLines = 1)
                    }
                }
            }
            HorizontalDivider(color = RemoteBorder)
            OutlinedTextField(
                value = state.projectSearch,
                onValueChange = onProjectSearch,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 10.dp, vertical = if (compactHeight) 4.dp else 7.dp)
                    .then(if (compactHeight) Modifier.height(48.dp) else Modifier)
                    .testTag("gar.tree.search"),
                singleLine = true,
                placeholder = { Text("搜索项目或对话", maxLines = 1) },
                leadingIcon = { RemoteIcon(RemoteGlyph.Search, null, Modifier.size(18.dp), RemoteMuted) },
                shape = RoundedCornerShape(12.dp),
            )
            LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(horizontal = 6.dp)) {
                listOf(
                    "固定" to pinnedProjects,
                    "最近" to recentProjects,
                    "全部项目" to remainingProjects,
                ).filter { it.second.isNotEmpty() }.forEach { (section, projects) ->
                    item(key = "project-section-$section") {
                        Text(
                            section,
                            modifier = Modifier.padding(
                                start = 12.dp,
                                top = if (compactHeight) 4.dp else 10.dp,
                                bottom = if (compactHeight) 1.dp else 3.dp,
                            ),
                            color = RemoteMuted,
                            style = MaterialTheme.typography.labelSmall,
                        )
                    }
                    projects.forEach { project ->
                        val conversations = providerConversations[project.id].orEmpty()
                        val matchingConversations = if (query.isBlank()) {
                            conversations
                        } else {
                            conversations.filter { it.title.contains(query, ignoreCase = true) }
                        }
                        val scope = provider?.let { projectTreeScope(snapshot.hostId, it, project.id) }
                        val expanded = scope in state.expandedProjectScopes ||
                            (query.isNotBlank() && matchingConversations.isNotEmpty())
                        item(key = "project-${provider?.wire}-${project.id}") {
                            DrawerProjectRow(
                                project = project,
                                status = snapshot.providerCapabilities.find {
                                    it.provider == provider && it.projectId == project.id
                                }?.state ?: if (project.valid) "available" else "unavailable",
                                updatedAtMs = maxOf(
                                    project.lastActivityAtMs ?: Long.MIN_VALUE,
                                    conversations.maxOfOrNull(Conversation::updatedAtMs) ?: Long.MIN_VALUE,
                                ).takeUnless { it == Long.MIN_VALUE },
                                conversationCount = conversations.size,
                                selected = state.selectedProjectId == project.id,
                                expanded = expanded,
                                pinned = project.id in state.pinnedProjects,
                                onSelect = { onProject(project.id) },
                                onToggle = { onToggleProject(project.id) },
                                onPin = { onPinProject(project.id) },
                            )
                        }
                        if (expanded) {
                            val visibleConversations = if (query.isBlank() || matchingConversations.isEmpty()) {
                                conversations
                            } else {
                                matchingConversations
                            }
                            items(
                                items = visibleConversations,
                                key = { "conversation-${it.provider.wire}-${it.projectId}-${it.id}" },
                            ) { conversation ->
                                DrawerConversationRow(
                                    conversation = conversation,
                                    selected = state.selectedConversationId == conversation.id &&
                                        state.selectedProjectId == conversation.projectId &&
                                        state.selectedProvider == conversation.provider,
                                    onClick = {
                                        onConversation(conversation.projectId, conversation.provider, conversation.id)
                                    },
                                )
                            }
                        }
                    }
                }
            }
            HorizontalDivider(color = RemoteBorder)
            if (compactHeight) {
                Row(Modifier.fillMaxWidth()) {
                    if (!state.online) {
                        TextButton(
                            onClick = if (state.retryEnabled) onStopRetrying else onRetryNow,
                            modifier = Modifier.weight(1f).height(44.dp),
                            contentPadding = PaddingValues(horizontal = 4.dp),
                        ) {
                            RemoteIcon(
                                if (state.retryEnabled) RemoteGlyph.Stop else RemoteGlyph.Recent,
                                null,
                                Modifier.size(18.dp),
                                RemoteMuted,
                            )
                            Spacer(Modifier.width(4.dp))
                            Text(
                                if (state.retryEnabled) "停止重连" else "立即重连",
                                color = RemoteMuted,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                    TextButton(
                        onClick = onDisconnect,
                        modifier = Modifier.weight(1f).height(44.dp),
                        contentPadding = PaddingValues(horizontal = 4.dp),
                    ) {
                        RemoteIcon(RemoteGlyph.Disconnect, null, Modifier.size(18.dp), MaterialTheme.colorScheme.error)
                        Spacer(Modifier.width(4.dp))
                        Text("断开 Host", color = MaterialTheme.colorScheme.error, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
            } else {
                if (!state.online) {
                    TextButton(
                        onClick = if (state.retryEnabled) onStopRetrying else onRetryNow,
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 8.dp).height(44.dp),
                    ) {
                        RemoteIcon(
                            if (state.retryEnabled) RemoteGlyph.Stop else RemoteGlyph.Recent,
                            null,
                            Modifier.size(19.dp),
                            RemoteMuted,
                        )
                        Spacer(Modifier.width(7.dp))
                        Text(if (state.retryEnabled) "停止自动重连" else "立即重连", color = RemoteMuted)
                    }
                }
                TextButton(
                    onClick = onDisconnect,
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 8.dp).height(44.dp),
                ) {
                    RemoteIcon(RemoteGlyph.Disconnect, null, Modifier.size(19.dp), MaterialTheme.colorScheme.error)
                    Spacer(Modifier.width(7.dp))
                    Text("断开 Host", color = MaterialTheme.colorScheme.error)
                }
            }
            Spacer(Modifier.navigationBarsPadding())
        }
    }
}

@Composable
private fun AgentSelector(
    providers: List<ProviderId>,
    selected: ProviderId?,
    modifier: Modifier,
    onProvider: (ProviderId) -> Unit,
) {
    var expanded by rememberSaveable(providers, selected) { mutableStateOf(false) }
    val canSwitch = providers.size > 1
    Box(modifier) {
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .height(44.dp)
                .testTag("gar.agent.selector")
                .then(if (canSwitch) Modifier.clickable { expanded = true } else Modifier),
            color = Color.Transparent,
            shape = RoundedCornerShape(12.dp),
            border = BorderStroke(1.dp, RemoteBorder),
        ) {
            Row(
                Modifier.fillMaxSize().padding(horizontal = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("Agent", color = RemoteMuted, style = MaterialTheme.typography.labelSmall)
                Spacer(Modifier.width(7.dp))
                Text(
                    selected?.label ?: "不可用",
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.labelMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (canSwitch) {
                    RemoteIcon(RemoteGlyph.ChevronDown, "选择 Agent", Modifier.size(18.dp), RemoteMuted)
                }
            }
        }
        DropdownMenu(
            expanded = expanded,
            onDismissRequest = { expanded = false },
            modifier = Modifier.widthIn(min = 168.dp, max = 220.dp),
        ) {
            providers.forEach { provider ->
                DropdownMenuItem(
                    text = { Text(provider.label, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    onClick = {
                        expanded = false
                        onProvider(provider)
                    },
                    modifier = Modifier.testTag("gar.agent.${provider.wire}"),
                    leadingIcon = {
                        if (provider == selected) {
                            RemoteIcon(RemoteGlyph.Check, null, Modifier.size(18.dp), RemoteText)
                        }
                    },
                    contentPadding = PaddingValues(horizontal = 12.dp),
                )
            }
        }
    }
}

@Composable
private fun DrawerProjectRow(
    project: ProjectSummary,
    status: String,
    updatedAtMs: Long?,
    conversationCount: Int,
    selected: Boolean,
    expanded: Boolean,
    pinned: Boolean,
    onSelect: () -> Unit,
    onToggle: () -> Unit,
    onPin: () -> Unit,
) {
    Surface(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 2.dp)
            .testTag("gar.project.${project.id}"),
        color = if (selected) RemoteSurfaceRaised else Color.Transparent,
        shape = RoundedCornerShape(12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconButton(
                onClick = onToggle,
                modifier = Modifier.size(44.dp).testTag("gar.project.${project.id}.toggle"),
            ) {
                RemoteIcon(
                    if (expanded) RemoteGlyph.ChevronDown else RemoteGlyph.ChevronRight,
                    if (expanded) "折叠 ${project.displayName}" else "展开 ${project.displayName}",
                    Modifier.size(18.dp),
                    RemoteMuted,
                )
            }
            Column(
                Modifier
                    .weight(1f)
                    .heightIn(min = 44.dp)
                    .clickable(onClick = onSelect),
                verticalArrangement = Arrangement.Center,
            ) {
                Text(project.displayName, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(
                    "${projectStatusLabel(status)} · ${activityLabel(updatedAtMs)} · $conversationCount",
                    color = RemoteMuted,
                    style = MaterialTheme.typography.labelSmall,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            IconButton(onClick = onPin, modifier = Modifier.size(44.dp)) {
                RemoteIcon(RemoteGlyph.Pin, if (pinned) "取消固定项目" else "固定项目",
                    Modifier.size(17.dp), if (pinned) RemoteAccent else RemoteMuted.copy(alpha = 0.55f))
            }
        }
    }
}

@Composable
private fun DrawerConversationRow(
    conversation: Conversation,
    selected: Boolean,
    onClick: () -> Unit,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .padding(start = 38.dp, end = 4.dp, top = 1.dp, bottom = 1.dp)
            .heightIn(min = 44.dp)
            .clip(RoundedCornerShape(10.dp))
            .background(if (selected) RemoteSurfaceRaised else Color.Transparent)
            .clickable(onClick = onClick)
            .padding(horizontal = 10.dp, vertical = 6.dp)
            .testTag("gar.conversation.${conversation.id}"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(conversation.title, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                "${stateLabel(conversation.state)} · ${activityLabel(conversation.updatedAtMs)}",
                color = RemoteMuted,
                style = MaterialTheme.typography.labelSmall,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

private fun projectStatusLabel(status: String): String = when (status) {
    "ready", "available" -> "可用"
    "syncing" -> "同步中"
    "offline" -> "离线"
    else -> status
}

private fun activityLabel(updatedAtMs: Long?): String {
    updatedAtMs ?: return "暂无活动"
    val elapsed = (System.currentTimeMillis() - updatedAtMs).coerceAtLeast(0L)
    return when {
        elapsed < 60_000L -> "刚刚"
        elapsed < 3_600_000L -> "${elapsed / 60_000L} 分钟前"
        elapsed < 86_400_000L -> "${elapsed / 3_600_000L} 小时前"
        else -> "${elapsed / 86_400_000L} 天前"
    }
}

@Composable
private fun RemoteTopBar(
    hostName: String,
    conversation: Conversation?,
    title: String,
    subtitle: String,
    onRename: (String) -> Unit,
    onSearch: () -> Unit,
    online: Boolean,
    menuExpanded: Boolean,
    sortMode: HomeSort,
    onNavigation: () -> Unit,
    onToggleMenu: () -> Unit,
    onDismissMenu: () -> Unit,
    onSort: (HomeSort) -> Unit,
    onShowProjects: () -> Unit,
    onManageConnections: () -> Unit,
    onDisconnect: () -> Unit,
) {
    Column(Modifier.fillMaxWidth().background(RemoteSurface).statusBarsPadding()) {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            FloatingIconButton(
                glyph = RemoteGlyph.Menu,
                description = "打开项目导航",
                onClick = onNavigation,
                modifier = Modifier.testTag("gar.drawer.open"),
            )
            Column(Modifier.weight(1f).padding(horizontal = 8.dp)) {
                if (conversation != null) {
                    ConversationHeader(conversation, onRename)
                } else {
                    Text(title, style = MaterialTheme.typography.titleMedium)
                    Text(subtitle.ifBlank { hostName }, color = RemoteMuted, style = MaterialTheme.typography.labelSmall,
                        maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            if (conversation != null) {
                FloatingIconButton(
                    RemoteGlyph.Search, "搜索当前对话", onSearch,
                    modifier = Modifier.testTag("gar.timeline.search.open"),
                )
            }
            Row(Modifier.clip(RoundedCornerShape(20.dp))
                .background(if (online) RemoteSuccess.copy(alpha = 0.09f) else RemoteSurfaceRaised)
                .padding(horizontal = 7.dp, vertical = 5.dp)
                .testTag("gar.connection.status").semantics {
                contentDescription = "$hostName · ${if (online) "在线" else "离线"}"
            }, verticalAlignment = Alignment.CenterVertically) {
                OnlineDot(online)
                Spacer(Modifier.width(4.dp))
                Text(if (online) "在线" else "离线", style = MaterialTheme.typography.labelSmall,
                    color = if (online) RemoteSuccess else RemoteMuted)
            }
            Box {
                FloatingIconButton(RemoteGlyph.More, "更多选项", onToggleMenu)
                RemoteOverflowMenu(menuExpanded, sortMode, online, onDismissMenu, onSort, onShowProjects, onManageConnections, onDisconnect)
            }
        }
        HorizontalDivider(color = RemoteBorder.copy(alpha = 0.65f))
    }
}

@Composable
private fun RemoteOverflowMenu(
    expanded: Boolean,
    sortMode: HomeSort,
    online: Boolean,
    onDismiss: () -> Unit,
    onSort: (HomeSort) -> Unit,
    onShowProjects: () -> Unit,
    onManageConnections: () -> Unit,
    onDisconnect: () -> Unit,
) {
    DropdownMenu(
        expanded = expanded,
        onDismissRequest = onDismiss,
        modifier = Modifier.widthIn(min = 168.dp, max = 220.dp),
        shape = RoundedCornerShape(12.dp),
        containerColor = RemoteSurfaceRaised,
        border = BorderStroke(1.dp, RemoteBorder),
    ) {
        MenuSectionLabel("整理")
        RemoteMenuRow(RemoteGlyph.Folder, "按 Agent 排序", sortMode == HomeSort.AGENT) { onSort(HomeSort.AGENT) }
        RemoteMenuRow(RemoteGlyph.Recent, "按时间倒序", sortMode == HomeSort.RECENT) { onSort(HomeSort.RECENT) }
        RemoteMenuRow(RemoteGlyph.Chat, "进行中优先", sortMode == HomeSort.ACTIVE) { onSort(HomeSort.ACTIVE) }
        HorizontalDivider(Modifier.padding(vertical = 8.dp), color = RemoteBorder)
        MenuSectionLabel("管理")
        RemoteMenuRow(RemoteGlyph.Folder, "项目列表") { onShowProjects() }
        RemoteMenuRow(RemoteGlyph.Computer, "管理连接") { onManageConnections() }
        RemoteMenuRow(RemoteGlyph.Disconnect, "断开 Host", tint = MaterialTheme.colorScheme.error) { onDisconnect() }
        HorizontalDivider(Modifier.padding(vertical = 8.dp), color = RemoteBorder)
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            OnlineDot(online)
            Spacer(Modifier.width(10.dp))
            Text(if (online) "Host 在线" else "Host 离线", color = RemoteMuted)
        }
    }
}

@Composable
private fun MenuSectionLabel(label: String) {
    Text(
        label,
        modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
        color = RemoteMuted,
        style = MaterialTheme.typography.labelMedium,
    )
}

@Composable
private fun RemoteMenuRow(
    glyph: RemoteGlyph,
    label: String,
    selected: Boolean = false,
    tint: Color = RemoteText,
    onClick: () -> Unit,
) {
    Row(
        Modifier.fillMaxWidth().height(48.dp).clickable(onClick = onClick).padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.width(24.dp), contentAlignment = Alignment.Center) {
            if (selected) RemoteIcon(RemoteGlyph.Check, null, Modifier.size(18.dp), tint)
        }
        Spacer(Modifier.width(7.dp))
        RemoteIcon(glyph, null, Modifier.size(22.dp), tint)
        Spacer(Modifier.width(9.dp))
        Text(
            label,
            modifier = Modifier.weight(1f),
            color = tint,
            style = MaterialTheme.typography.bodyMedium,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun RemoteHomeScreen(
    state: RemoteUiState,
    sortMode: HomeSort,
    viewModel: RemoteViewModel,
    onToggleProject: (UUID) -> Unit,
) {
    val snapshot = requireNotNull(state.snapshot)
    val selectedProject = snapshot.projects.find { it.valid && it.id == state.selectedProjectId }
    val projectExpanded = isHomeProjectExpanded(
        expandedScopes = state.expandedProjectScopes,
        hostId = snapshot.hostId,
        provider = state.selectedProvider,
        projectId = selectedProject?.id,
    )
    var search by rememberSaveable { mutableStateOf("") }
    val filtered = remember(snapshot.conversations, selectedProject?.id, state.selectedProvider, search, sortMode) {
        val normalizedSearch = search.trim()
        snapshot.conversations
            .asSequence()
            .filter { selectedProject != null && it.projectId == selectedProject.id }
            .filter { state.selectedProvider == null || it.provider == state.selectedProvider }
            .filter { normalizedSearch.isBlank() || it.title.contains(normalizedSearch, ignoreCase = true) }
            .let { conversations ->
                when (sortMode) {
                    HomeSort.AGENT -> conversations.sortedWith(
                        compareBy<Conversation> { it.provider.label }.thenByDescending { it.updatedAtMs },
                    )
                    HomeSort.RECENT -> conversations.sortedByDescending { it.updatedAtMs }
                    HomeSort.ACTIVE -> conversations.sortedWith(
                        compareByDescending<Conversation> { it.running }.thenByDescending { it.updatedAtMs },
                    )
                }
            }
            .toList()
    }

    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(start = 20.dp, end = 12.dp, top = 8.dp),
            verticalAlignment = Alignment.CenterVertically) {
            Text("当前项目", color = RemoteMuted, style = MaterialTheme.typography.labelMedium, modifier = Modifier.weight(1f))
            TextButton(onClick = viewModel::showNewConversation, enabled = selectedProject != null,
                modifier = Modifier.testTag("gar.home.new")) {
                RemoteIcon(RemoteGlyph.Compose, null, Modifier.size(18.dp))
                Spacer(Modifier.width(5.dp))
                Text("新对话")
            }
        }
        if (selectedProject == null) {
            Row(
                Modifier.fillMaxWidth().heightIn(min = 44.dp).padding(horizontal = 20.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RemoteIcon(RemoteGlyph.Folder, null, Modifier.size(24.dp))
                Spacer(Modifier.width(10.dp))
                Text(
                    "请从侧栏选择项目",
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        } else {
            HomeProjectRow(
                project = selectedProject,
                expanded = projectExpanded,
                onToggle = { onToggleProject(selectedProject.id) },
            )
        }
        if (!projectExpanded) {
            Spacer(Modifier.weight(1f))
        } else if (filtered.isEmpty()) {
            Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    RemoteIcon(RemoteGlyph.Chat, null, Modifier.size(34.dp), RemoteMuted)
                    Spacer(Modifier.height(12.dp))
                    Text(if (search.isBlank()) "这个项目还没有会话" else "没有匹配的聊天记录", color = RemoteMuted)
                }
            }
        } else {
            LazyColumn(
                modifier = Modifier.weight(1f),
                contentPadding = PaddingValues(horizontal = 20.dp, vertical = 8.dp),
            ) {
                items(filtered, key = { it.id }, contentType = { "conversation" }) { conversation ->
                    ConversationListItem(conversation) { viewModel.selectConversation(conversation.id) }
                }
            }
        }
        HomeBottomBar(
            search = search,
            onSearch = { search = it },
        )
    }
}

@Composable
private fun HomeProjectRow(
    project: ProjectSummary,
    expanded: Boolean,
    onToggle: () -> Unit,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = 4.dp)
            .heightIn(min = 44.dp)
            .clip(RoundedCornerShape(12.dp))
            .clickable(onClick = onToggle)
            .padding(start = 8.dp)
            .testTag("gar.home.project.${project.id}"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RemoteIcon(RemoteGlyph.Folder, null, Modifier.size(24.dp))
        Spacer(Modifier.width(10.dp))
        Text(
            project.displayName,
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.titleMedium,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        IconButton(
            onClick = onToggle,
            modifier = Modifier.size(44.dp).testTag("gar.home.project.${project.id}.toggle"),
        ) {
            RemoteIcon(
                if (expanded) RemoteGlyph.ChevronDown else RemoteGlyph.ChevronRight,
                if (expanded) "折叠 ${project.displayName}" else "展开 ${project.displayName}",
                Modifier.size(19.dp),
                RemoteMuted,
            )
        }
    }
}

@Composable
private fun ConversationListItem(conversation: Conversation, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 44.dp)
            .clickable(onClick = onClick)
            .padding(vertical = 14.dp)
            .testTag("gar.home.conversation.${conversation.id}"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                conversation.title,
                style = MaterialTheme.typography.titleMedium,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                "${conversation.provider.label} · ${stateLabel(conversation.state)} · ${activityLabel(conversation.updatedAtMs)}",
                modifier = Modifier.padding(top = 3.dp),
                color = RemoteMuted,
                style = MaterialTheme.typography.bodySmall,
            )
        }
        if (conversation.running) {
            Spacer(Modifier.width(10.dp))
            Box(Modifier.size(6.dp).background(RemoteSuccess, CircleShape))
        }
    }
    HorizontalDivider(color = RemoteBorder.copy(alpha = 0.5f))
}

@Composable
private fun HomeBottomBar(search: String, onSearch: (String) -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .windowInsetsPadding(WindowInsets.ime.union(WindowInsets.navigationBars))
            .padding(start = 16.dp, top = 8.dp, end = 16.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Surface(
            modifier = Modifier.weight(1f).height(52.dp),
            color = RemoteSurfaceRaised,
            shape = RoundedCornerShape(10.dp),
            border = BorderStroke(1.dp, RemoteBorder),
        ) {
            BasicTextField(
                value = search,
                onValueChange = onSearch,
                singleLine = true,
                textStyle = MaterialTheme.typography.bodyLarge.copy(color = RemoteText),
                modifier = Modifier.fillMaxSize().semantics { contentDescription = "搜索聊天记录" },
                decorationBox = { input ->
                    Row(
                        Modifier.fillMaxSize().padding(horizontal = 16.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RemoteIcon(RemoteGlyph.Search, null, Modifier.size(22.dp), RemoteMuted)
                        Spacer(Modifier.width(10.dp))
                        Box(Modifier.weight(1f)) {
                            if (search.isEmpty()) Text("搜索聊天记录", color = RemoteMuted, style = MaterialTheme.typography.bodyLarge)
                            input()
                        }
                        if (search.isNotEmpty()) {
                            IconButton(onClick = { onSearch("") }, modifier = Modifier.size(44.dp)) {
                                RemoteIcon(RemoteGlyph.Close, "清除搜索", Modifier.size(17.dp), RemoteMuted)
                            }
                        }
                    }
                },
            )
        }
    }
}

@Composable
private fun NewConversationScreen(state: RemoteUiState, viewModel: RemoteViewModel) {
    val snapshot = requireNotNull(state.snapshot)
    val capability = snapshot.providerCapabilities.find {
        it.projectId == state.selectedProjectId && it.provider == state.selectedProvider
    }
    Column(
        Modifier
            .fillMaxSize()
            .windowInsetsPadding(WindowInsets.ime.union(WindowInsets.navigationBars)),
    ) {
        Box(
            Modifier.weight(1f).fillMaxWidth(),
            contentAlignment = Alignment.Center,
        ) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                RemoteIcon(RemoteGlyph.Chat, null, Modifier.size(34.dp), RemoteMuted)
                Spacer(Modifier.height(10.dp))
                Text("开始一段对话", style = MaterialTheme.typography.titleMedium)
                Spacer(Modifier.height(6.dp))
                Text("描述任务，或附上相关文件", color = RemoteMuted, style = MaterialTheme.typography.bodyMedium)
                capability?.limitation?.let {
                    Text(it, color = RemoteMuted, style = MaterialTheme.typography.labelSmall)
                }
            }
        }
        Composer(
            draft = state.draft,
            running = false,
            online = state.online && capability?.ready == true,
            supportsSteer = false,
            sessionOptions = emptyList(),
            capability = capability,
            selectedModel = state.selectedModel,
            selectedEffort = state.selectedEffort,
            selectedPermission = state.selectedPermission,
            promptAttachments = state.promptAttachments,
            sendStatus = state.sendStatus,
            sendFailure = state.sendFailure,
            onDraft = viewModel::setDraft,
            onSend = viewModel::sendMessage,
            onRetry = viewModel::retryPendingSend,
            onSteer = {},
            onInterrupt = {},
            onSessionOption = { option, value ->
                when (option) {
                    "model" -> viewModel.selectModel(value)
                    "reasoning_effort" -> viewModel.selectEffort(value)
                    "permission_mode" -> viewModel.selectPermission(value)
                }
            },
            onAttachments = viewModel::addPromptAttachments,
            onRemoveAttachment = viewModel::removePromptAttachment,
        )
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ConversationScreen(
    state: RemoteUiState,
    conversation: Conversation,
    viewModel: RemoteViewModel,
    searchOpen: Boolean,
    onCloseSearch: () -> Unit,
) {
    val snapshot = requireNotNull(state.snapshot)
    val capability = remember(snapshot.providerCapabilities, conversation.projectId, conversation.provider) {
        snapshot.providerCapabilities.find {
            it.projectId == conversation.projectId && it.provider == conversation.provider
        }
    }
    val timeline = state.timelineByConversation[conversation.id].orEmpty()
    val conversationScope = "${snapshot.hostId}/${conversation.provider.wire}/${conversation.projectId}/${conversation.id}"
    val timelineGrouper = remember(conversationScope) { TimelineBlockGrouper() }
    val timelineBlocks = remember(timeline) { timelineGrouper.update(timeline) }
    val listState = rememberSaveable(conversationScope, saver = LazyListState.Saver) { LazyListState() }
    val scrollScope = rememberCoroutineScope()
    var followingTail by remember(conversationScope) { mutableStateOf(true) }
    var navigatingHistory by remember(conversationScope) { mutableStateOf(false) }
    val dragging by listState.interactionSource.collectIsDraggedAsState()
    val currentlyDragging by rememberUpdatedState(dragging)
    val currentlySearching by rememberUpdatedState(searchOpen)
    var query by rememberSaveable(conversationScope) { mutableStateOf("") }
    var selectedMatch by rememberSaveable(conversationScope) { mutableStateOf<String?>(null) }
    val matches = remember(timeline, query) { messageSearchMatches(timeline, query) }
    val matchIndex = matches.indexOfFirst { it.toString() == selectedMatch }.coerceAtLeast(0)
    val activeMatch = if (searchOpen) matches.getOrNull(matchIndex) else null
    val approvals = remember(timeline, conversation.running) { unresolvedApprovalItems(timeline, conversation.running) }
    val compactSearch = searchOpen && LocalConfiguration.current.orientation == Configuration.ORIENTATION_LANDSCAPE
    val openApproval = {
        navigatingHistory = true
        onCloseSearch()
        followingTail = false
        val index = approvals.firstOrNull()?.let { timelineBlockIndex(timelineBlocks, it.id) } ?: -1
        if (index >= 0) scrollScope.launch { listState.animateScrollToItem(index + 1) }
        Unit
    }
    val approvalUnavailableReason = when {
        !conversation.running -> "本轮已结束，无法再确认"
        !state.online -> "连接后可确认"
        else -> null
    }
    val tail = timeline.lastOrNull()?.let { it.id to it.revision }
    var lastReadTail by remember(conversationScope) { mutableStateOf(tail) }
    val hasNewContent = lastReadTail != null && tail != lastReadTail
    LaunchedEffect(tail, followingTail) {
        if (followingTail) lastReadTail = tail
    }
    LaunchedEffect(listState) {
        snapshotFlow { Triple(currentlyDragging, !listState.canScrollForward, currentlySearching) }
            .distinctUntilChanged()
            .collect { (isDragging, atBottom, searching) ->
                if (searching) followingTail = false
                else if (isDragging) {
                    navigatingHistory = false
                    followingTail = atBottom
                } else if (atBottom && !navigatingHistory) followingTail = true
            }
    }
    LaunchedEffect(timelineBlocks.lastOrNull()?.key, timeline.lastOrNull()?.revision, searchOpen) {
        if (followingTail && !searchOpen && timelineBlocks.isNotEmpty()) {
            listState.scrollToItem(timelineBlocks.size + 1)
        }
    }
    LaunchedEffect(activeMatch) {
        activeMatch?.let { itemId ->
            val index = timelineBlockIndex(timelineBlocks, itemId)
            if (index >= 0) {
                navigatingHistory = true
                followingTail = false
                listState.animateScrollToItem(index + 1)
            }
        }
    }
    Column(
        Modifier
            .fillMaxSize()
            .windowInsetsPadding(WindowInsets.ime.union(WindowInsets.navigationBars)),
    ) {
        if (searchOpen) {
            ConversationSearchBar(
                query = query,
                onQuery = { query = it; selectedMatch = null },
                matchIndex = matchIndex,
                matchCount = matches.size,
                onPrevious = { selectedMatch = matches[(matchIndex - 1 + matches.size) % matches.size].toString() },
                onNext = { selectedMatch = matches[(matchIndex + 1) % matches.size].toString() },
                onClose = onCloseSearch,
                approvalCount = approvals.size,
                onApproval = openApproval,
            )
        }
        if (approvals.isNotEmpty() && !compactSearch) {
            Row(
                Modifier.fillMaxWidth().background(RemoteWarning.copy(alpha = 0.08f))
                    .padding(start = 18.dp, end = 8.dp).testTag("gar.timeline.approvals"),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("${approvals.size} 项等待确认", color = RemoteWarning, modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.labelMedium)
                TextButton(onClick = openApproval,
                    modifier = Modifier.testTag("gar.timeline.approvals.open")) { Text("查看") }
            }
        }
        if (timeline.isNotEmpty() && conversation.id in state.historyErrors) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                Text("历史更新失败，已保留缓存", modifier = Modifier.weight(1f), color = RemoteMuted)
                TextButton(onClick = viewModel::retryConversationHistory,
                    enabled = state.online && conversation.id !in state.historyLoading) { Text("重试") }
            }
        }
        if (timeline.isEmpty()) {
            Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    RemoteIcon(RemoteGlyph.Chat, null, Modifier.size(36.dp), RemoteMuted)
                    Spacer(Modifier.height(12.dp))
                    Text(
                        when {
                            conversation.id in state.historyLoading -> "正在加载消息…"
                            conversation.id in state.historyErrors -> "消息加载失败"
                            !state.online -> "连接后加载消息"
                            conversation.id in state.historyExhausted -> "此会话暂无消息"
                            else -> "消息尚未加载"
                        },
                        color = RemoteMuted,
                    )
                    state.historyErrors[conversation.id]?.let { error ->
                        Text(error, color = RemoteMuted, modifier = Modifier.padding(16.dp))
                    }
                    if (state.online && conversation.id !in state.historyLoading && conversation.id !in state.historyExhausted) {
                        TextButton(onClick = viewModel::retryConversationHistory) { Text("重新加载消息") }
                    }
                }
            }
        } else {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                LazyColumn(
                    state = listState,
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(start = 16.dp, top = 8.dp, end = 16.dp, bottom = 20.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    item("load-older") {
                        if (conversation.id !in state.historyExhausted) {
                            TextButton(onClick = viewModel::loadOlder, enabled = state.online && conversation.id !in state.historyLoading,
                                modifier = Modifier.fillMaxWidth()) {
                                Text(if (conversation.id in state.historyLoading) "正在加载消息…" else "加载更早消息", color = RemoteMuted)
                            }
                        }
                    }
                    items(
                        items = timelineBlocks,
                        key = { it.key },
                        contentType = {
                            when (it) {
                                is TimelineBlock.Single -> "timeline"
                                is TimelineBlock.Activity -> "activity"
                            }
                        },
                    ) { block ->
                        when (block) {
                            is TimelineBlock.Single -> TimelineCard(
                                item = block.item,
                                providerLabel = conversation.provider.label,
                                attachment = (block.item.content as? TimelineContent.Image)?.let {
                                    state.attachments[it.attachmentId]
                                },
                                approvalPending = (block.item.content as? TimelineContent.Approval)?.approvalId in state.pendingApprovals,
                                approvalUnavailableReason = approvalUnavailableReason,
                                highlighted = block.item.id == activeMatch,
                                onApproval = viewModel::resolveApproval,
                            )
                            is TimelineBlock.Activity -> ActivityTimelineCard(
                                items = block.items,
                                attachments = state.attachments,
                                pendingApprovals = state.pendingApprovals,
                                approvalUnavailableReason = approvalUnavailableReason,
                                onApproval = viewModel::resolveApproval,
                            )
                        }
                    }
                    item("timeline-end") { Spacer(Modifier.height(1.dp)) }
                }
                if ((!followingTail || searchOpen) && listState.canScrollForward) {
                    OutlinedButton(
                        onClick = {
                            navigatingHistory = false
                            onCloseSearch()
                            followingTail = true
                            scrollScope.launch { listState.animateScrollToItem(timelineBlocks.size + 1) }
                        },
                        modifier = Modifier.align(Alignment.BottomEnd).padding(12.dp).testTag("gar.timeline.latest"),
                        shape = RoundedCornerShape(10.dp),
                        colors = androidx.compose.material3.ButtonDefaults.outlinedButtonColors(containerColor = RemoteSurface),
                        contentPadding = PaddingValues(horizontal = 12.dp),
                    ) {
                        RemoteIcon(RemoteGlyph.Latest, null, Modifier.size(16.dp))
                        Spacer(Modifier.width(6.dp))
                        Text(if (hasNewContent) "有新内容 ↓" else "回到最新", style = MaterialTheme.typography.labelMedium)
                    }
                }
                CodexScrollbar(
                    state = listState,
                    modifier = Modifier.align(Alignment.CenterEnd).padding(vertical = 10.dp, horizontal = 4.dp),
                )
            }
        }
        if (!compactSearch) Composer(
            draft = state.draft,
            running = conversation.running,
            online = state.online,
            supportsSteer = capability?.supportsSteer == true,
            sessionOptions = conversation.sessionOptions,
            existingConversation = true,
            capability = capability,
            selectedModel = conversation.selectedModel,
            selectedEffort = conversation.selectedEffort,
            selectedPermission = conversation.sessionOptions
                .find { it.id == "permission_mode" }
                ?.currentValue,
            promptAttachments = state.promptAttachments,
            sendStatus = state.sendStatus,
            sendFailure = state.sendFailure,
            onDraft = viewModel::setDraft,
            onSend = viewModel::sendMessage,
            onRetry = viewModel::retryPendingSend,
            onSteer = viewModel::steer,
            onInterrupt = viewModel::interrupt,
            onSessionOption = viewModel::setSessionOption,
            onAttachments = viewModel::addPromptAttachments,
            onRemoveAttachment = viewModel::removePromptAttachment,
        )
    }
}

@Composable
private fun ConversationSearchBar(
    query: String,
    onQuery: (String) -> Unit,
    matchIndex: Int,
    matchCount: Int,
    onPrevious: () -> Unit,
    onNext: () -> Unit,
    onClose: () -> Unit,
    approvalCount: Int,
    onApproval: () -> Unit,
) {
    val focusRequester = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    val compact = LocalConfiguration.current.orientation == Configuration.ORIENTATION_LANDSCAPE
    LaunchedEffect(Unit) { focusRequester.requestFocus() }
    val searchField: @Composable (Modifier) -> Unit = { modifier ->
        OutlinedTextField(
            value = query,
            onValueChange = onQuery,
            placeholder = { Text("搜索已加载消息") },
            singleLine = true,
            leadingIcon = { RemoteIcon(RemoteGlyph.Search, null, Modifier.size(18.dp), RemoteMuted) },
            trailingIcon = {
                IconButton(onClick = { keyboard?.hide(); onClose() }, modifier = Modifier.testTag("gar.timeline.search.close")) {
                    RemoteIcon(RemoteGlyph.Close, "关闭搜索", Modifier.size(20.dp), RemoteMuted)
                }
            },
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            keyboardActions = KeyboardActions(onSearch = { keyboard?.hide() }),
            modifier = modifier.focusRequester(focusRequester).testTag("gar.timeline.search.input"),
        )
    }
    val controls: @Composable (Modifier) -> Unit = { modifier ->
        Row(modifier, verticalAlignment = Alignment.CenterVertically) {
            Text(
                when {
                    query.isBlank() -> if (compact) "已加载消息" else "搜索已加载的用户和 Agent 消息"
                    matchCount == 0 -> if (compact) "无匹配" else "已加载消息中没有匹配项"
                    compact -> "${matchIndex + 1}/$matchCount · 已加载"
                    else -> "${matchIndex + 1} / $matchCount 条匹配 · 已加载消息"
                },
                modifier = Modifier.weight(1f).testTag("gar.timeline.search.count"),
                color = RemoteMuted,
                style = MaterialTheme.typography.labelSmall,
            )
            IconButton(onClick = { keyboard?.hide(); onPrevious() }, enabled = matchCount > 0,
                modifier = Modifier.testTag("gar.timeline.search.previous")) {
                RemoteIcon(RemoteGlyph.Send, "上一条匹配", Modifier.size(18.dp), RemoteMuted)
            }
            IconButton(onClick = { keyboard?.hide(); onNext() }, enabled = matchCount > 0,
                modifier = Modifier.testTag("gar.timeline.search.next")) {
                RemoteIcon(RemoteGlyph.Latest, "下一条匹配", Modifier.size(18.dp), RemoteMuted)
            }
            if (compact && approvalCount > 0) {
                TextButton(onClick = { keyboard?.hide(); onApproval() },
                    modifier = Modifier.testTag("gar.timeline.approvals.open")) {
                    Text("确认 $approvalCount", color = RemoteWarning)
                }
            }
        }
    }
    if (compact) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically) {
            searchField(Modifier.weight(1f))
            controls(Modifier.width(if (approvalCount > 0) 250.dp else 180.dp).padding(start = 8.dp))
        }
    } else {
        Column(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp)) {
            searchField(Modifier.fillMaxWidth())
            controls(Modifier.fillMaxWidth())
        }
    }
}

@Composable
private fun ConversationHeader(conversation: Conversation, onRename: (String) -> Unit) {
    var editing by rememberSaveable(conversation.id) { mutableStateOf(false) }
    var title by remember(conversation.id, conversation.title) { mutableStateOf(conversation.title) }
    Row(
        Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f).clickable { editing = true }) {
            Text(
                conversation.title,
                style = MaterialTheme.typography.titleMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                "${conversation.provider.label} · ${stateLabel(conversation.state)}",
                color = RemoteMuted,
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
    if (editing) {
        Dialog(onDismissRequest = { editing = false }) {
            Surface(
                color = RemoteSurfaceRaised,
                shape = RoundedCornerShape(12.dp),
                border = BorderStroke(1.dp, RemoteBorder),
            ) {
                Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text("重命名对话", style = MaterialTheme.typography.titleLarge)
                    OutlinedTextField(
                        value = title,
                        onValueChange = { title = it },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                        TextButton(onClick = { editing = false }) { Text("取消") }
                        Button(
                            onClick = {
                                onRename(title)
                                editing = false
                            },
                            enabled = title.isNotBlank(),
                        ) { Text("保存") }
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Composer(
    draft: String,
    running: Boolean,
    online: Boolean,
    supportsSteer: Boolean,
    sessionOptions: List<SessionOption>,
    existingConversation: Boolean = false,
    capability: ProviderCapability?,
    selectedModel: String?,
    selectedEffort: String?,
    selectedPermission: String?,
    promptAttachments: List<PromptAttachment>,
    sendStatus: SendStatus,
    sendFailure: String?,
    onDraft: (String) -> Unit,
    onSend: () -> Unit,
    onRetry: () -> Unit,
    onSteer: () -> Unit,
    onInterrupt: () -> Unit,
    onSessionOption: (String, String) -> Unit,
    onAttachments: (List<Uri>) -> Unit,
    onRemoveAttachment: (UUID) -> Unit,
) {
    val compactInput = useCompactInputLayout()
    val inputEnabled = !running || supportsSteer
    var settingsOpen by rememberSaveable { mutableStateOf(false) }
    val effectiveOptions = remember(
        sessionOptions,
        existingConversation,
        capability,
        selectedModel,
        selectedEffort,
        selectedPermission,
    ) {
        if (existingConversation || sessionOptions.isNotEmpty()) {
            sessionOptions
        } else {
            newConversationOptions(capability, selectedModel, selectedEffort, selectedPermission)
        }
    }
    val permission = effectiveOptions.find { it.id == "permission_mode" }
    val model = effectiveOptions.find { it.id == "model" || it.category == "model" }
    val effort = effectiveOptions.find { it.id == "reasoning_effort" || it.id == "thought_level" || it.category == "thought_level" }
    val modelLabel = listOfNotNull(model?.let(::sessionOptionValueLabel), effort?.let(::sessionOptionValueLabel)?.removeSuffix(" Effort"))
        .joinToString(" · ").ifEmpty { "会话设置" }
    val canSend = online && draft.isNotBlank() && sendStatus == SendStatus.IDLE
    val attachmentTypes = capability?.attachments?.allowedMimeTypes.orEmpty().toTypedArray()
    val attachmentLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenMultipleDocuments(),
        onAttachments,
    )
    Column(
        Modifier
            .fillMaxWidth()
            .background(RemoteBackground)
            .padding(start = 12.dp, top = if (compactInput) 4.dp else 8.dp, end = 12.dp, bottom = if (compactInput) 4.dp else 6.dp),
    ) {
        if (promptAttachments.isNotEmpty()) {
            Row(
                Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(bottom = 7.dp),
                horizontalArrangement = Arrangement.spacedBy(7.dp),
            ) {
                promptAttachments.forEach { attachment ->
                    Surface(
                        color = RemoteSurfaceRaised,
                        shape = RoundedCornerShape(12.dp),
                        border = BorderStroke(1.dp, RemoteBorder),
                    ) {
                        Row(
                            Modifier.padding(start = 10.dp, top = 5.dp, bottom = 5.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                attachment.fileName,
                                modifier = Modifier.widthIn(max = 160.dp),
                                style = MaterialTheme.typography.labelSmall,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            IconButton(
                                onClick = { onRemoveAttachment(attachment.id) },
                                modifier = Modifier.size(44.dp),
                            ) {
                                RemoteIcon(RemoteGlyph.Close, "移除附件", Modifier.size(14.dp), RemoteMuted)
                            }
                        }
                    }
                }
            }
        }
        Surface(
            modifier = Modifier.fillMaxWidth(),
            color = RemoteSurface,
            shape = RoundedCornerShape(20.dp),
            border = BorderStroke(1.dp, RemoteBorder),
        ) {
            Column(Modifier.padding(if (compactInput) 3.dp else 5.dp)) {
                BasicTextField(
                    value = draft,
                    onValueChange = onDraft,
                    enabled = inputEnabled,
                    textStyle = MaterialTheme.typography.bodyLarge.copy(color = RemoteText),
                    cursorBrush = SolidColor(RemoteAccent),
                    modifier = Modifier.fillMaxWidth().heightIn(min = if (compactInput) 36.dp else 48.dp, max = if (compactInput) 44.dp else 116.dp)
                        .padding(horizontal = 11.dp, vertical = if (compactInput) 5.dp else 10.dp)
                        .testTag("gar.composer.input")
                        .semantics { contentDescription = if (running) "输入追加指令" else "输入消息" },
                    maxLines = if (compactInput) 1 else 4,
                    decorationBox = { input ->
                        Box {
                            if (draft.isEmpty()) Text(
                                when {
                                    running && !supportsSteer -> "等待当前任务完成…"
                                    running -> "补充一条指令…"
                                    !online -> "离线，仍可编辑草稿"
                                    else -> "输入消息…"
                                }, color = RemoteMuted, style = MaterialTheme.typography.bodyLarge)
                            input()
                        }
                    },
                )
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(3.dp)) {
                    IconButton(
                        onClick = { attachmentLauncher.launch(attachmentTypes) },
                        enabled = inputEnabled && sendStatus == SendStatus.IDLE && capability?.attachments?.supported == true &&
                            promptAttachments.size < capability.attachments.maxCount &&
                            promptAttachments.sumOf { it.bytes.size.toLong() } < capability.attachments.maxTotalBytes,
                        modifier = Modifier.size(44.dp).testTag("gar.composer.attach"),
                    ) {
                        RemoteIcon(RemoteGlyph.Attach, "添加附件", Modifier.size(20.dp), RemoteMuted)
                    }
                    permission?.let {
                        SessionSettingsChip(
                            label = sessionOptionValueLabel(it), enabled = online && !running,
                            onClick = { settingsOpen = true },
                            modifier = Modifier.weight(0.8f).testTag("gar.session.permission"),
                        )
                    }
                    if (permission == null && effectiveOptions.isNotEmpty()) Spacer(Modifier.weight(1f))
                    if (effectiveOptions.isNotEmpty()) {
                        SessionSettingsChip(
                            label = modelLabel, enabled = online && !running,
                            onClick = { settingsOpen = true },
                            modifier = (if (permission == null) Modifier.widthIn(max = 208.dp)
                                else Modifier.weight(1.4f)).testTag("gar.session.settings"),
                        )
                    } else {
                        Spacer(Modifier.weight(1f))
                    }
                    if (!running || supportsSteer) {
                        IconButton(
                            onClick = if (running) onSteer else onSend,
                            enabled = canSend,
                            modifier = Modifier.size(44.dp).testTag("gar.composer.send"),
                        ) {
                            Box(Modifier.size(36.dp).background(if (canSend) RemoteAccent else RemoteAccent.copy(alpha = 0.10f), CircleShape),
                                contentAlignment = Alignment.Center) {
                                RemoteIcon(RemoteGlyph.Send, if (running) "追加指令" else "发送", Modifier.size(20.dp),
                                    if (canSend) MaterialTheme.colorScheme.onPrimary else RemoteAccent.copy(alpha = 0.5f))
                            }
                        }
                    }
                    if (running) {
                        IconButton(onClick = onInterrupt, enabled = online,
                            modifier = Modifier.size(44.dp).testTag("gar.composer.stop")) {
                            RemoteIcon(RemoteGlyph.Stop, "停止", Modifier.size(22.dp), MaterialTheme.colorScheme.error)
                        }
                    }
                }
            }
        }
        if (sendStatus != SendStatus.IDLE) {
            Row(
                Modifier.fillMaxWidth().heightIn(min = 44.dp).padding(horizontal = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    when (sendStatus) {
                        SendStatus.SENDING -> "正在发送…"
                        SendStatus.QUEUED -> "已排队，等待 Host 确认"
                        SendStatus.FAILED -> sendFailure ?: "发送失败，草稿已保留"
                        SendStatus.IDLE -> ""
                    },
                    modifier = Modifier.weight(1f),
                    color = if (sendStatus == SendStatus.FAILED) MaterialTheme.colorScheme.error else RemoteMuted,
                    style = MaterialTheme.typography.labelSmall,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
                if (sendStatus == SendStatus.FAILED) {
                    TextButton(
                        onClick = onRetry,
                        enabled = online,
                        modifier = Modifier.heightIn(min = 44.dp).testTag("gar.send.retry"),
                        contentPadding = PaddingValues(horizontal = 10.dp),
                    ) {
                        Text("重试")
                    }
                }
            }
        }

    }
    if (settingsOpen) {
        SessionSettingsSheet(
            options = effectiveOptions,
            permissionModes = capability?.permissionModes.orEmpty(),
            enabled = online && !running,
            onDismiss = { settingsOpen = false },
            onSelect = onSessionOption,
        )
    }
}

@Composable
private fun useCompactInputLayout(): Boolean =
    LocalConfiguration.current.orientation == Configuration.ORIENTATION_LANDSCAPE &&
        WindowInsets.ime.getBottom(LocalDensity.current) > 0

private fun newConversationOptions(
    capability: ProviderCapability?,
    selectedModel: String?,
    selectedEffort: String?,
    selectedPermission: String?,
): List<SessionOption> {
    capability ?: return emptyList()
    val model = capability.models.find { it.id == selectedModel } ?: capability.models.firstOrNull()
    return buildList {
        if (capability.models.isNotEmpty()) {
            add(
                SessionOption(
                    id = "model",
                    displayName = "模型",
                    category = null,
                    currentValue = selectedModel ?: capability.models.first().id,
                    values = capability.models.map {
                        dev.agentremote.messenger.model.SessionOptionValue(it.id, it.displayName)
                    },
                ),
            )
        }
        if (model?.effortOptions?.isNotEmpty() == true) {
            add(
                SessionOption(
                    id = "reasoning_effort",
                    displayName = "推理强度",
                    category = null,
                    currentValue = selectedEffort ?: model.defaultEffort ?: model.effortOptions.first().id,
                    values = model.effortOptions.map {
                        dev.agentremote.messenger.model.SessionOptionValue(it.id, it.displayName)
                    },
                ),
            )
        }
        if (capability.permissionModes.isNotEmpty()) {
            add(
                SessionOption(
                    id = "permission_mode",
                    displayName = "权限",
                    category = null,
                    currentValue = selectedPermission
                        ?: capability.defaultPermissionMode
                        ?: capability.permissionModes.first().id,
                    values = capability.permissionModes.map {
                        dev.agentremote.messenger.model.SessionOptionValue(it.id, it.displayName)
                    },
                ),
            )
        }
    }
}

@Composable
private fun SessionSettingsChip(
    label: String,
    enabled: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        modifier = modifier.height(44.dp).clip(RoundedCornerShape(12.dp)).clickable(enabled = enabled, onClick = onClick).semantics { contentDescription = label },
        color = RemoteSurfaceRaised,
        shape = RoundedCornerShape(12.dp),
    ) {
        Row(
            Modifier.padding(horizontal = 9.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.Center,
        ) {
            Text(
                label,
                modifier = Modifier.weight(1f, fill = false),
                color = if (enabled) RemoteText else RemoteMuted,
                style = MaterialTheme.typography.labelSmall,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Spacer(Modifier.width(3.dp))
            RemoteIcon(RemoteGlyph.ChevronDown, null, Modifier.size(12.dp), RemoteMuted)
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SessionSettingsSheet(
    options: List<SessionOption>,
    permissionModes: List<PermissionModeOption>,
    enabled: Boolean,
    onDismiss: () -> Unit,
    onSelect: (String, String) -> Unit,
) {
    var activeOptionId by rememberSaveable { mutableStateOf<String?>(null) }
    var pendingElevatedPermission by remember { mutableStateOf<String?>(null) }
    val activeOption = options.find { it.id == activeOptionId }
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = androidx.compose.material3.rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = RemoteSurfaceRaised,
        contentColor = RemoteText,
    ) {
        if (activeOption == null) {
            Column(
                Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(start = 20.dp, end = 20.dp, bottom = 24.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Text("会话设置", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(bottom = 10.dp))
                options.forEach { option ->
                    SessionSettingRow(option = option, enabled = enabled) { activeOptionId = option.id }
                }
                if (!enabled) {
                    Text("Agent 运行时不能修改这些设置", color = RemoteMuted, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 8.dp))
                }
            }
        } else {
            Column(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, bottom = 20.dp)) {
                TextButton(onClick = { activeOptionId = null }) {
                    RemoteIcon(RemoteGlyph.Back, null, Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text("会话设置")
                }
                Text(
                    sessionOptionLabel(activeOption),
                    style = MaterialTheme.typography.titleLarge,
                    modifier = Modifier.padding(horizontal = 4.dp, vertical = 8.dp),
                )
                LazyColumn(Modifier.fillMaxWidth().weight(1f, fill = false)) {
                    items(activeOption.values, key = { it.value }) { value ->
                        val selected = value.value == activeOption.currentValue
                        val permission = activeOption.id == "permission_mode"
                        val permissionMode = permissionModes.find { it.id == value.value }
                        val elevated = permissionMode?.risk == PermissionRisk.ELEVATED
                        val selectedColor = if (elevated && selected) RemoteWarning else RemoteText
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .heightIn(min = if (permission) 68.dp else 58.dp)
                                .clip(RoundedCornerShape(12.dp))
                                .clickable(enabled = enabled) {
                                    if (elevated) {
                                        pendingElevatedPermission = value.value
                                    } else {
                                        onSelect(activeOption.id, value.value)
                                        activeOptionId = null
                                    }
                                }
                                .background(
                                    when {
                                        elevated && selected -> RemoteWarning.copy(alpha = 0.08f)
                                        selected -> RemoteSurfaceRaised
                                        else -> Color.Transparent
                                    },
                                )
                                .border(
                                    1.dp,
                                    if (elevated && selected) RemoteWarning.copy(alpha = 0.55f) else Color.Transparent,
                                    RoundedCornerShape(12.dp),
                                )
                                .padding(horizontal = 16.dp, vertical = 10.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            if (permission) {
                                Icon(
                                    imageVector = permissionIcon(permissionMode?.risk),
                                    contentDescription = null,
                                    modifier = Modifier.size(22.dp),
                                    tint = selectedColor,
                                )
                                Spacer(Modifier.width(12.dp))
                            }
                            Column(Modifier.weight(1f)) {
                                Text(
                                    sessionOptionValueLabel(activeOption, value.value),
                                    color = selectedColor,
                                    fontWeight = FontWeight.Medium,
                                )
                                permissionMode?.description?.let {
                                    Text(it, color = RemoteMuted, style = MaterialTheme.typography.bodySmall)
                                }
                            }
                            if (selected) RemoteIcon(RemoteGlyph.Check, null, Modifier.size(19.dp), selectedColor)
                        }
                    }
                }
            }
        }
        Spacer(Modifier.navigationBarsPadding())
    }
    pendingElevatedPermission?.let { permission ->
        Dialog(onDismissRequest = { pendingElevatedPermission = null }) {
            Surface(
                color = RemoteSurfaceRaised,
                shape = RoundedCornerShape(12.dp),
                border = BorderStroke(1.dp, RemoteWarning.copy(alpha = 0.4f)),
            ) {
                Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text("确认高风险权限", style = MaterialTheme.typography.titleLarge)
                    Text(
                        permissionModes.find { it.id == permission }?.description
                            ?: "此模式可能允许无需逐次确认的写入或命令。",
                        color = RemoteMuted,
                    )
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                        TextButton(onClick = { pendingElevatedPermission = null }) { Text("取消") }
                        Button(onClick = {
                            onSelect("permission_mode", permission)
                            pendingElevatedPermission = null
                            activeOptionId = null
                        }) { Text("确认切换") }
                    }
                }
            }
        }
    }
}

@Composable
private fun SessionSettingRow(option: SessionOption, enabled: Boolean, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .height(58.dp)
            .clip(RoundedCornerShape(12.dp))
            .clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(sessionOptionLabel(option), modifier = Modifier.weight(0.7f), fontWeight = FontWeight.Medium)
        Text(sessionOptionValueLabel(option), modifier = Modifier.weight(1f), color = RemoteMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Spacer(Modifier.width(10.dp))
        RemoteIcon(RemoteGlyph.ChevronRight, null, Modifier.size(20.dp), RemoteMuted)
    }
}

private fun sessionOptionLabel(option: SessionOption): String = when (option.id) {
    "model" -> "模型"
    "reasoning_effort", "thought_level" -> "推理强度"
    "permission_mode" -> "权限"
    else -> option.displayName
}

private fun sessionOptionValueLabel(option: SessionOption): String =
    sessionOptionValueLabel(option, option.currentValue)

private fun sessionOptionValueLabel(option: SessionOption, value: String): String =
    option.values.find { it.value == value }?.displayName ?: value

private fun permissionIcon(risk: PermissionRisk?): ImageVector = when (risk) {
    PermissionRisk.ELEVATED -> Icons.Outlined.WarningAmber
    PermissionRisk.STANDARD -> Icons.Outlined.Security
    null -> Icons.Outlined.TouchApp
}

@Composable
private fun CodexScrollbar(state: LazyListState, modifier: Modifier = Modifier) {
    val layout = state.layoutInfo
    val visibleCount = layout.visibleItemsInfo.size
    val totalCount = layout.totalItemsCount
    if (visibleCount == 0 || totalCount <= visibleCount || (!state.canScrollBackward && !state.canScrollForward)) return
    BoxWithConstraints(modifier.fillMaxHeight().width(7.dp)) {
        val thumbHeight = (maxHeight * (visibleCount.toFloat() / totalCount))
            .coerceIn(minOf(38.dp, maxHeight), maxHeight)
        val lastStartIndex = (totalCount - visibleCount).coerceAtLeast(1)
        val firstItemSize = layout.visibleItemsInfo.firstOrNull()?.size?.coerceAtLeast(1) ?: 1
        val fractionalIndex = state.firstVisibleItemIndex + state.firstVisibleItemScrollOffset.toFloat() / firstItemSize
        val progress = (fractionalIndex / lastStartIndex).coerceIn(0f, 1f)
        val thumbOffset = (maxHeight - thumbHeight) * progress
        Box(
            Modifier
                .align(Alignment.TopCenter)
                .offset(y = thumbOffset)
                .width(3.dp)
                .height(thumbHeight)
                .background(
                    RemoteMuted.copy(alpha = if (state.isScrollInProgress) 0.7f else 0.3f),
                    CircleShape,
                ),
        )
    }
}

internal sealed interface TimelineBlock {
    val key: String
    val startIndex: Int
    val endExclusive: Int

    data class Single(val item: TimelineItem, override val startIndex: Int) : TimelineBlock {
        override val key: String = item.id.toString()
        override val endExclusive: Int = startIndex + 1
    }

    data class Activity(
        val items: List<TimelineItem>,
        override val startIndex: Int,
    ) : TimelineBlock {
        override val key: String = "activity-${items.first().id}"
        override val endExclusive: Int = startIndex + items.size
    }
}

internal data class TimelineGrouping(
    val items: List<TimelineItem>,
    val blocks: List<TimelineBlock>,
)

internal class TimelineBlockGrouper {
    private var grouping = TimelineGrouping(emptyList(), emptyList())

    fun update(items: List<TimelineItem>): List<TimelineBlock> {
        grouping = updateTimelineGrouping(grouping, items)
        return grouping.blocks
    }
}

internal fun updateTimelineGrouping(
    previous: TimelineGrouping,
    items: List<TimelineItem>,
): TimelineGrouping {
    if (previous.items.isEmpty()) return TimelineGrouping(items, groupTimeline(items))
    var commonPrefix = 0
    val comparableCount = minOf(previous.items.size, items.size)
    while (commonPrefix < comparableCount && previous.items[commonPrefix].sameTimelineVersion(items[commonPrefix])) {
        commonPrefix++
    }
    if (commonPrefix == previous.items.size && commonPrefix == items.size) return previous

    var rebuildStart = commonPrefix
    val affectedBlock = previous.blocks.firstOrNull { commonPrefix < it.endExclusive }
    if (affectedBlock != null) {
        rebuildStart = affectedBlock.startIndex
    }
    if (items.getOrNull(rebuildStart)?.content?.isActivity() == true) {
        val precedingActivity = previous.blocks.lastOrNull { it.endExclusive == rebuildStart } as? TimelineBlock.Activity
        if (precedingActivity != null) rebuildStart = precedingActivity.startIndex
    }
    rebuildStart = rebuildStart.coerceAtMost(items.size)
    val preserved = previous.blocks.takeWhile { it.endExclusive <= rebuildStart }
    return TimelineGrouping(
        items = items,
        blocks = preserved + groupTimeline(items.drop(rebuildStart), startIndex = rebuildStart),
    )
}

private fun TimelineItem.sameTimelineVersion(other: TimelineItem): Boolean =
    id == other.id && revision == other.revision && createdAtMs == other.createdAtMs

internal fun groupTimeline(items: List<TimelineItem>, startIndex: Int = 0): List<TimelineBlock> = buildList {
    var activity = mutableListOf<TimelineItem>()
    var activityStart = startIndex
    fun flush() {
        if (activity.isNotEmpty()) {
            add(TimelineBlock.Activity(activity, activityStart))
            activity = mutableListOf()
        }
    }
    items.forEachIndexed { localIndex, item ->
        if (item.content.isActivity()) {
            if (activity.isEmpty()) activityStart = startIndex + localIndex
            activity += item
        } else {
            flush()
            add(TimelineBlock.Single(item, startIndex + localIndex))
        }
    }
    flush()
}

private fun TimelineContent.isActivity(): Boolean = when (this) {
    is TimelineContent.Progress,
    is TimelineContent.ToolCall,
    is TimelineContent.Command,
    is TimelineContent.FileChange,
    -> true
    is TimelineContent.Approval -> resolvedOption != null
    else -> false
}

@Composable
private fun ActivityTimelineCard(
    items: List<TimelineItem>,
    attachments: Map<UUID, ByteArray>,
    pendingApprovals: Set<UUID>,
    approvalUnavailableReason: String?,
    onApproval: (UUID, String) -> Unit,
) {
    var expanded by rememberSaveable(items.first().id) { mutableStateOf(false) }
    val labels = remember(items) {
        items.groupingBy {
            when (it.content) {
                is TimelineContent.FileChange -> "文件"
                is TimelineContent.Command -> "命令"
                is TimelineContent.Approval -> "审批"
                is TimelineContent.Error -> "错误"
                is TimelineContent.Progress -> if (it.content.kind == "test") "测试" else "进度"
                else -> "工具"
            }
        }.eachCount().entries.joinToString(" · ") { "${it.key} ${it.value}" }
    }
    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = RemoteSurface,
        shape = RoundedCornerShape(12.dp),
        border = BorderStroke(1.dp, RemoteBorder),
    ) {
        Column {
            Row(
                Modifier.fillMaxWidth().heightIn(min = 44.dp).testTag("gar.activity.${items.first().id}").clickable { expanded = !expanded }.padding(horizontal = 10.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RemoteIcon(if (expanded) RemoteGlyph.ChevronDown else RemoteGlyph.ChevronRight, null, Modifier.size(18.dp), RemoteMuted)
                Spacer(Modifier.width(8.dp))
                Text("活动 · $labels", modifier = Modifier.weight(1f), style = MaterialTheme.typography.labelMedium)
                Text("${items.size} 项", color = RemoteMuted, style = MaterialTheme.typography.labelSmall)
            }
            if (expanded) {
                HorizontalDivider(color = RemoteBorder)
                Column(Modifier.padding(10.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    items.forEach { item ->
                        TimelineCard(
                            item = item,
                            attachment = (item.content as? TimelineContent.Image)?.let {
                                attachments[it.attachmentId]
                            },
                            approvalPending = (item.content as? TimelineContent.Approval)?.approvalId in pendingApprovals,
                            approvalUnavailableReason = approvalUnavailableReason,
                            onApproval = onApproval,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun TimelineCard(
    item: TimelineItem,
    providerLabel: String = "Agent",
    attachment: ByteArray?,
    approvalPending: Boolean,
    approvalUnavailableReason: String? = null,
    highlighted: Boolean = false,
    onApproval: (UUID, String) -> Unit,
) {
    Column(
        if (highlighted) Modifier.fillMaxWidth()
            .border(1.dp, RemoteAccent, RoundedCornerShape(10.dp)).padding(6.dp)
            .testTag("gar.timeline.search.match.${item.id}") else Modifier.fillMaxWidth(),
    ) {
        when (val content = item.content) {
            is TimelineContent.UserMessage -> MessageBubble(content.text, user = true, messageId = item.id, createdAtMs = item.createdAtMs)
            is TimelineContent.AgentMessage -> if (content.phase == "reasoning_summary") {
                ReasoningSummary(content.text, item.id, highlighted)
            } else {
                MessageBubble(content.text, user = false, messageId = item.id,
                    createdAtMs = item.createdAtMs, label = providerLabel)
            }
            is TimelineContent.Progress -> GenericTimelineCard(
                title = "${content.kind} · ${content.label}",
                status = content.status,
                body = content.detail,
            )
            is TimelineContent.Plan -> GenericTimelineCard(
                title = "计划",
                body = content.steps.joinToString("\n") { "${statusMark(it.status)} ${it.text}" },
            )
            is TimelineContent.ToolCall -> ToolTimelineCard(content)
            is TimelineContent.Command -> CodeTimelineCard(content)
            is TimelineContent.FileChange -> GenericTimelineCard(
                title = "文件 · ${content.changeKind}",
                status = content.status,
                body = content.relativePath,
            )
            is TimelineContent.Approval -> ApprovalCard(content, approvalPending, approvalUnavailableReason, onApproval)
            is TimelineContent.Image -> ImageCard(content.alt, attachment)
            is TimelineContent.Error -> GenericTimelineCard(
                title = "错误 · ${content.code}",
                body = content.message,
                error = true,
            )
        }
    }
}

@Composable
private fun ToolTimelineCard(content: TimelineContent.ToolCall) {
    val input = meaningfulToolSummary(content.inputSummary)
    val output = meaningfulToolSummary(content.outputSummary)?.takeUnless { it == input }
    val displayName = when (content.name) {
        "web_search" -> "网页搜索"
        "image_generation" -> "图片生成"
        "contextCompaction" -> "上下文压缩"
        "subAgentActivity" -> "子 Agent"
        else -> content.name
    }
    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = Color.Transparent,
        shape = RoundedCornerShape(12.dp),
    ) {
        Column(Modifier.padding(8.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text("工具 · $displayName", fontWeight = FontWeight.SemiBold)
                    if (displayName != content.name) {
                        Text(content.name, color = RemoteMuted, style = MaterialTheme.typography.labelSmall)
                    }
                }
                Text(stateLabel(content.status), color = RemoteMuted, style = MaterialTheme.typography.labelMedium)
            }
            input?.let { ToolSummaryRow("输入", it) }
            output?.let { ToolSummaryRow("结果", it) }
        }
    }
}

@Composable
private fun ToolSummaryRow(label: String, value: String) {
    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        Text(label, color = RemoteMuted, style = MaterialTheme.typography.labelSmall)
        Text(
            value,
            color = RemoteText,
            style = MaterialTheme.typography.bodySmall,
            fontFamily = FontFamily.Monospace,
            maxLines = 8,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

private fun meaningfulToolSummary(value: String?): String? {
    val summary = value?.trim().orEmpty()
    if (summary.isEmpty() || summary == "[]" || summary == "{}" || summary == "null" || summary == "\"\"") {
        return null
    }
    return if (summary.startsWith('{') || summary.startsWith('[')) {
        "Provider 返回了结构化详情（已隐藏）"
    } else {
        summary
    }
}

@Composable
private fun ReasoningSummary(text: String, messageId: UUID, highlighted: Boolean) {
    var expanded by rememberSaveable(messageId) { mutableStateOf(false) }
    val clipboard = LocalClipboardManager.current
    LaunchedEffect(highlighted) {
        if (highlighted) expanded = true
    }
    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = RemoteSurfaceRaised,
        shape = RoundedCornerShape(14.dp),
    ) {
        Column {
            Row(
                Modifier.fillMaxWidth().heightIn(min = 44.dp)
                    .testTag("gar.reasoning.$messageId")
                    .semantics { stateDescription = if (expanded) "已展开" else "已收起" }
                    .clickable { expanded = !expanded }
                    .padding(horizontal = 12.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RemoteIcon(RemoteGlyph.Reasoning, null, Modifier.size(17.dp), RemoteAccent)
                Spacer(Modifier.width(8.dp))
                Text("推理摘要", modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.labelMedium, color = RemoteMuted)
                Text(if (expanded) "收起" else "展开", color = RemoteMuted,
                    style = MaterialTheme.typography.labelSmall)
                Spacer(Modifier.width(4.dp))
                RemoteIcon(if (expanded) RemoteGlyph.ChevronDown else RemoteGlyph.ChevronRight,
                    null, Modifier.size(16.dp), RemoteMuted)
            }
            if (expanded) {
                Column(Modifier.padding(horizontal = 14.dp).testTag("gar.reasoning.$messageId.content")) {
                    MarkdownText(text, contentKey = messageId.toString())
                    TextButton(
                        onClick = { clipboard.setText(AnnotatedString(text)) },
                        modifier = Modifier.align(Alignment.End).testTag("gar.message.$messageId.copy"),
                    ) {
                        RemoteIcon(RemoteGlyph.Copy, null, Modifier.size(16.dp))
                        Spacer(Modifier.width(5.dp))
                        Text("复制摘要", style = MaterialTheme.typography.labelSmall)
                    }
                }
            }
        }
    }
}

@Composable
private fun MessageBubble(text: String, user: Boolean, messageId: UUID, createdAtMs: Long, label: String = "你") {
    val clipboard = LocalClipboardManager.current
    val time = remember(createdAtMs) { SimpleDateFormat("HH:mm", Locale.getDefault()).format(Date(createdAtMs)) }
    var copied by remember(text) { mutableStateOf(false) }
    val content: @Composable () -> Unit = {
        Column(Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 5.dp)) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                if (!user) {
                    Box(Modifier.size(24.dp).background(RemoteAccent.copy(alpha = 0.09f), RoundedCornerShape(7.dp)),
                        contentAlignment = Alignment.Center) {
                        RemoteIcon(RemoteGlyph.Chat, null, Modifier.size(14.dp), RemoteAccent)
                    }
                    Spacer(Modifier.width(7.dp))
                }
                Text(label, color = if (user) RemoteMuted else RemoteText,
                    style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.SemiBold)
                Text(time, color = RemoteMuted, style = MaterialTheme.typography.labelSmall, modifier = Modifier.weight(1f).padding(start = 8.dp))
                IconButton(
                    onClick = { clipboard.setText(AnnotatedString(text)); copied = true },
                    modifier = Modifier.size(44.dp).testTag("gar.message.$messageId.copy"),
                ) {
                    RemoteIcon(if (copied) RemoteGlyph.Check else RemoteGlyph.Copy,
                        if (copied) "已复制" else "复制消息", Modifier.size(16.dp), RemoteMuted)
                }
            }
            MarkdownText(text, modifier = Modifier.padding(bottom = 10.dp), contentKey = messageId.toString())
        }
    }
    if (user) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
            Surface(Modifier.fillMaxWidth(0.9f), color = MaterialTheme.colorScheme.primaryContainer,
                shape = RoundedCornerShape(topStart = 18.dp, topEnd = 6.dp, bottomEnd = 18.dp, bottomStart = 18.dp)) { content() }
        }
    } else {
        Surface(Modifier.fillMaxWidth(), color = RemoteSurface,
            shape = RoundedCornerShape(18.dp), border = BorderStroke(1.dp, RemoteBorder.copy(alpha = 0.65f))) { content() }
    }
}

@Composable
private fun GenericTimelineCard(
    title: String,
    status: String? = null,
    body: String? = null,
    error: Boolean = false,
) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = if (error) MaterialTheme.colorScheme.errorContainer else Color.Transparent,
        shape = RoundedCornerShape(12.dp),
        border = if (error) BorderStroke(1.dp, MaterialTheme.colorScheme.error.copy(alpha = 0.25f)) else null,
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row {
                Text(title, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
                status?.let { Text(stateLabel(it), color = RemoteMuted, style = MaterialTheme.typography.labelMedium) }
            }
            body?.let { Text(it, color = if (error) MaterialTheme.colorScheme.onErrorContainer else RemoteText) }
        }
    }
}

@Composable
private fun CodeTimelineCard(content: TimelineContent.Command) {
    val clipboard = LocalClipboardManager.current
    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = Color.Transparent,
        shape = RoundedCornerShape(12.dp),
    ) {
        Column(Modifier.padding(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("命令 · ${stateLabel(content.status)}", fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
                IconButton(
                    onClick = {
                        clipboard.setText(AnnotatedString(listOfNotNull(content.command, content.output).joinToString("\n\n")))
                    },
                    modifier = Modifier.size(44.dp),
                ) {
                    RemoteIcon(RemoteGlyph.Copy, "复制命令和输出", Modifier.size(20.dp), RemoteMuted)
                }
            }
            val scroll = rememberScrollState()
            Surface(color = RemoteSurfaceRaised, shape = RoundedCornerShape(12.dp)) {
                Text(
                    content.command,
                    fontFamily = FontFamily.Monospace,
                    color = RemoteText,
                    modifier = Modifier.fillMaxWidth().horizontalScroll(scroll).padding(13.dp),
                )
            }
            content.relativeCwd?.let { Text("cwd: $it", color = RemoteMuted, style = MaterialTheme.typography.bodySmall) }
            content.output?.let {
                Text(it, color = RemoteMuted, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall)
            }
            content.exitCode?.let { Text("exit $it", color = RemoteMuted, style = MaterialTheme.typography.labelSmall) }
        }
    }
}

@Composable
private fun ApprovalCard(
    content: TimelineContent.Approval,
    pending: Boolean,
    unavailableReason: String?,
    onApproval: (UUID, String) -> Unit,
) {
    Surface(
        color = RemoteWarning.copy(alpha = 0.07f),
        shape = RoundedCornerShape(12.dp),
        border = BorderStroke(1.dp, RemoteWarning.copy(alpha = 0.3f)),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(Modifier.padding(17.dp), verticalArrangement = Arrangement.spacedBy(11.dp)) {
            Text(when {
                content.resolvedOption != null -> "已处理"
                unavailableReason != null -> unavailableReason
                else -> "等待你的确认"
            }, color = RemoteWarning, fontWeight = FontWeight.SemiBold, style = MaterialTheme.typography.titleMedium)
            Text(content.prompt)
            if (content.resolvedOption != null) {
                Text("已选择：${content.options.find { it.id == content.resolvedOption }?.label ?: content.resolvedOption}", color = RemoteMuted, fontWeight = FontWeight.Medium)
            } else {
                content.options.forEach { option ->
                    OutlinedButton(
                        onClick = { onApproval(content.approvalId, option.id) },
                        enabled = !pending && unavailableReason == null,
                        modifier = Modifier.fillMaxWidth().height(48.dp),
                        shape = RoundedCornerShape(12.dp),
                        border = BorderStroke(1.dp, RemoteWarning.copy(alpha = 0.4f)),
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
    Surface(
        modifier = Modifier.fillMaxWidth().clickable(enabled = bitmap != null) { fullscreen = true },
        color = RemoteSurface,
        shape = RoundedCornerShape(12.dp),
        border = BorderStroke(1.dp, RemoteBorder),
    ) {
        Column(Modifier.padding(10.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (bitmap == null) {
                Box(Modifier.fillMaxWidth().height(180.dp), contentAlignment = Alignment.Center) {
                    Text(if (bytes == null) "正在读取图片…" else "无法解码图片", color = RemoteMuted)
                }
            } else {
                Image(
                    bitmap = bitmap.asImageBitmap(),
                    contentDescription = alt,
                    contentScale = ContentScale.Fit,
                    modifier = Modifier.fillMaxWidth().height(240.dp).clip(RoundedCornerShape(12.dp)).background(RemoteSurfaceRaised),
                )
            }
            Text(alt, modifier = Modifier.padding(horizontal = 5.dp, vertical = 2.dp), color = RemoteMuted, style = MaterialTheme.typography.bodySmall)
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

private enum class RemoteGlyph(val imageVector: ImageVector) {
    Menu(Icons.Rounded.Menu),
    Back(Icons.AutoMirrored.Rounded.ArrowBack),
    More(Icons.Rounded.MoreHoriz),
    Computer(Icons.Rounded.Computer),
    Folder(Icons.Rounded.Folder),
    Compose(Icons.Rounded.Add),
    Disconnect(Icons.Rounded.PowerSettingsNew),
    Recent(Icons.Rounded.History),
    Chat(Icons.Rounded.ChatBubbleOutline),
    Check(Icons.Rounded.Check),
    ChevronDown(Icons.Rounded.KeyboardArrowDown),
    ChevronRight(Icons.Rounded.ChevronRight),
    Search(Icons.Rounded.Search),
    Close(Icons.Rounded.Close),
    Send(Icons.Rounded.ArrowUpward),
    Latest(Icons.Rounded.ArrowDownward),
    Stop(Icons.Rounded.Stop),
    Attach(Icons.Rounded.AttachFile),
    Copy(Icons.Rounded.ContentCopy),
    Scan(Icons.Rounded.QrCodeScanner),
    Paste(Icons.Rounded.ContentPaste),
    Reasoning(Icons.Rounded.AutoAwesome),
    Pin(Icons.Rounded.PushPin),
}

@Composable
private fun OnlineDot(online: Boolean) {
    Box(
        Modifier
            .size(7.dp)
            .background(if (online) RemoteSuccess else Color.Transparent, CircleShape)
            .border(1.dp, if (online) RemoteSuccess else RemoteMuted, CircleShape)
            .semantics { contentDescription = if (online) "在线" else "离线" },
    )
}

@Composable
private fun FloatingIconButton(
    glyph: RemoteGlyph,
    description: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    containerColor: Color = Color.Transparent,
    contentColor: Color = RemoteText,
    size: Dp = 44.dp,
) {
    IconButton(
        onClick = onClick,
        modifier = modifier
            .size(size)
            .background(containerColor, RoundedCornerShape(8.dp)),
    ) {
        RemoteIcon(glyph, description, Modifier.size(24.dp), contentColor)
    }
}

@Composable
private fun RemoteIcon(
    glyph: RemoteGlyph,
    description: String?,
    modifier: Modifier = Modifier,
    tint: Color = LocalContentColor.current,
) {
    Icon(
        imageVector = glyph.imageVector,
        contentDescription = description,
        modifier = modifier,
        tint = tint,
    )
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
