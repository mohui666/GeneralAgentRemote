package dev.agentremote.messenger.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
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
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.outlined.Security
import androidx.compose.material.icons.outlined.TouchApp
import androidx.compose.material.icons.outlined.WarningAmber
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.ArrowUpward
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
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.NavigationDrawerItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.sp
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

private val RemoteBlack = Color(0xFF000000)
private val RemoteSurface = Color(0xFF141414)
private val RemoteSurfaceRaised = Color(0xFF1E1E1F)
private val RemoteBorder = Color(0xFF343436)
private val RemoteText = Color(0xFFF5F5F6)
private val RemoteMuted = Color(0xFFA7A7AD)
private val RemotePurple = Color(0xFFA477ED)
private val RemoteGreen = Color(0xFF58D59A)
private val RemoteOrange = Color(0xFFFF6B2C)

private val AgentRemoteColors = darkColorScheme(
    primary = RemotePurple,
    onPrimary = Color.White,
    primaryContainer = Color(0xFF302342),
    onPrimaryContainer = Color(0xFFF4E9FF),
    secondary = Color(0xFFD0B7F4),
    secondaryContainer = Color(0xFF362B43),
    onSecondaryContainer = Color(0xFFF2E7FF),
    background = RemoteBlack,
    onBackground = RemoteText,
    surface = RemoteSurface,
    onSurface = RemoteText,
    surfaceVariant = RemoteSurfaceRaised,
    onSurfaceVariant = RemoteMuted,
    outline = RemoteBorder,
    error = Color(0xFFFF7483),
    errorContainer = Color(0xFF35161C),
    onErrorContainer = Color(0xFFFFDADF),
)

private val AgentRemoteTypography = Typography(
    displaySmall = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 34.sp, lineHeight = 40.sp, fontWeight = FontWeight.SemiBold),
    headlineMedium = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 26.sp, lineHeight = 32.sp, fontWeight = FontWeight.SemiBold),
    titleLarge = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 21.sp, lineHeight = 28.sp, fontWeight = FontWeight.SemiBold),
    titleMedium = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 18.sp, lineHeight = 24.sp, fontWeight = FontWeight.Medium),
    bodyLarge = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 17.sp, lineHeight = 27.sp, fontWeight = FontWeight.Normal),
    bodyMedium = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 15.sp, lineHeight = 22.sp, fontWeight = FontWeight.Normal),
    bodySmall = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 13.sp, lineHeight = 18.sp, fontWeight = FontWeight.Normal),
    labelLarge = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 15.sp, lineHeight = 20.sp, fontWeight = FontWeight.Medium),
    labelMedium = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 13.sp, lineHeight = 18.sp, fontWeight = FontWeight.Medium),
    labelSmall = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 12.sp, lineHeight = 16.sp, fontWeight = FontWeight.Medium),
)

@Composable
fun AgentRemoteTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = AgentRemoteColors, typography = AgentRemoteTypography, content = content)
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
            .padding(horizontal = 20.dp, vertical = 18.dp),
        verticalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column {
                Text("远程", style = MaterialTheme.typography.titleLarge)
                Text("Agent Remote", color = RemoteMuted, style = MaterialTheme.typography.bodySmall)
            }
            StatusCard(state.phase, state.online)
        }
        Spacer(Modifier.height(18.dp))
        Text("连接你的电脑", style = MaterialTheme.typography.displaySmall)
        Text(
            "在手机上继续电脑里的 Codex 与 Grok 会话。项目文件和 Agent 都留在 Host。",
            style = MaterialTheme.typography.bodyLarge,
            color = RemoteMuted,
        )
        Surface(
            modifier = Modifier.fillMaxWidth(),
            color = RemoteSurface,
            shape = RoundedCornerShape(28.dp),
            border = BorderStroke(1.dp, RemoteBorder),
        ) {
            Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                Text("配对新 Host", style = MaterialTheme.typography.titleLarge)
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
                    shape = RoundedCornerShape(18.dp),
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
                        shape = RoundedCornerShape(16.dp),
                        border = BorderStroke(1.dp, RemoteBorder),
                    ) {
                        RemoteIcon(RemoteGlyph.Scan, "扫码", Modifier.size(20.dp))
                        Spacer(Modifier.width(8.dp))
                        Text("扫码")
                    }
                    OutlinedButton(
                        onClick = { clipboard.getText()?.text?.let(viewModel::setPairLink) },
                        modifier = Modifier.weight(1f).height(48.dp),
                        shape = RoundedCornerShape(16.dp),
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
                    shape = RoundedCornerShape(18.dp),
                ) { Text(if (state.connecting) "正在连接…" else "连接并配对") }
            }
        }
        if (state.credentials.isNotEmpty()) {
            Text("已保存 Host", style = MaterialTheme.typography.titleLarge)
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
        color = RemoteSurfaceRaised,
        shape = RoundedCornerShape(999.dp),
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
        shape = RoundedCornerShape(24.dp),
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
                    shape = RoundedCornerShape(16.dp),
                ) { Text(if (connecting) "连接中" else "连接") }
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
    val selectedConversation = snapshot.conversations.find { it.id == state.selectedConversationId }
    var menuExpanded by rememberSaveable { mutableStateOf(false) }
    var sortMode by rememberSaveable { mutableStateOf(HomeSort.AGENT) }

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
                onNewConversation = {
                    viewModel.showNewConversation()
                    scope.launch { drawerState.close() }
                },
                onDisconnect = viewModel::disconnect,
            )
        },
    ) {
        Scaffold(
            containerColor = RemoteBlack,
            contentWindowInsets = WindowInsets(0, 0, 0, 0),
            topBar = {
                RemoteTopBar(
                    hostName = snapshot.hostName,
                    online = state.online,
                    showConversationActions = state.showingNewConversation || selectedConversation != null,
                    menuExpanded = menuExpanded,
                    sortMode = sortMode,
                    onNavigation = { scope.launch { drawerState.open() } },
                    onNewConversation = viewModel::showNewConversation,
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
                    selectedConversation != null -> ConversationScreen(state, selectedConversation, viewModel)
                    else -> RemoteHomeScreen(state, sortMode, viewModel)
                }
            }
        }
    }
    BackHandler(enabled = state.showingNewConversation || selectedConversation != null) {
        viewModel.showConversationList()
    }
}

private enum class HomeSort { AGENT, RECENT, ACTIVE }

@Composable
private fun RemoteDrawer(
    state: RemoteUiState,
    onProject: (UUID) -> Unit,
    onNewConversation: () -> Unit,
    onDisconnect: () -> Unit,
) {
    val snapshot = requireNotNull(state.snapshot)
    ModalDrawerSheet(
        modifier = Modifier.fillMaxHeight().widthIn(max = 330.dp),
        drawerContainerColor = RemoteSurface,
        drawerContentColor = RemoteText,
    ) {
        Column(Modifier.statusBarsPadding().padding(horizontal = 18.dp, vertical = 16.dp)) {
            Text("Agent Remote", style = MaterialTheme.typography.titleLarge)
            Row(
                Modifier.padding(top = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                OnlineDot(state.online)
                Spacer(Modifier.width(8.dp))
                Text(snapshot.hostName, color = RemoteMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Spacer(Modifier.height(10.dp))
            Button(
                onClick = onNewConversation,
                modifier = Modifier.fillMaxWidth().height(50.dp),
                shape = RoundedCornerShape(16.dp),
            ) {
                RemoteIcon(RemoteGlyph.Compose, null, Modifier.size(20.dp))
                Spacer(Modifier.width(8.dp))
                Text("新建会话")
            }
        }
        HorizontalDivider(color = RemoteBorder)
        Text(
            "项目",
            modifier = Modifier.padding(horizontal = 20.dp, vertical = 14.dp),
            color = RemoteMuted,
            style = MaterialTheme.typography.labelMedium,
        )
        LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(horizontal = 10.dp)) {
            items(snapshot.projects.filter { it.valid }, key = { it.id }) { project ->
                val count = snapshot.conversations.count { it.projectId == project.id }
                NavigationDrawerItem(
                    icon = { RemoteIcon(RemoteGlyph.Folder, null, Modifier.size(22.dp), RemoteMuted) },
                    label = {
                        Column {
                            Text(project.displayName, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text("$count 个会话", color = RemoteMuted, style = MaterialTheme.typography.labelSmall)
                        }
                    },
                    selected = state.selectedProjectId == project.id && state.selectedConversationId == null,
                    onClick = { onProject(project.id) },
                    colors = androidx.compose.material3.NavigationDrawerItemDefaults.colors(
                        selectedContainerColor = RemoteSurfaceRaised,
                        unselectedContainerColor = Color.Transparent,
                        selectedTextColor = RemoteText,
                        unselectedTextColor = RemoteText,
                    ),
                )
            }
        }
        HorizontalDivider(color = RemoteBorder)
        TextButton(
            onClick = onDisconnect,
            modifier = Modifier.fillMaxWidth().padding(10.dp).height(48.dp),
        ) {
            RemoteIcon(RemoteGlyph.Disconnect, null, Modifier.size(20.dp), MaterialTheme.colorScheme.error)
            Spacer(Modifier.width(8.dp))
            Text("断开 Host", color = MaterialTheme.colorScheme.error)
        }
        Spacer(Modifier.navigationBarsPadding())
    }
}

@Composable
private fun RemoteTopBar(
    hostName: String,
    online: Boolean,
    showConversationActions: Boolean,
    menuExpanded: Boolean,
    sortMode: HomeSort,
    onNavigation: () -> Unit,
    onNewConversation: () -> Unit,
    onToggleMenu: () -> Unit,
    onDismissMenu: () -> Unit,
    onSort: (HomeSort) -> Unit,
    onShowProjects: () -> Unit,
    onDisconnect: () -> Unit,
) {
    Box(
        Modifier
            .fillMaxWidth()
            .background(RemoteBlack)
            .statusBarsPadding()
            .padding(horizontal = 18.dp, vertical = 10.dp),
    ) {
        FloatingIconButton(
            glyph = RemoteGlyph.Menu,
            description = "打开项目导航",
            onClick = onNavigation,
            modifier = Modifier.align(Alignment.CenterStart),
        )
        Column(
            modifier = Modifier.align(Alignment.Center).widthIn(max = 120.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text("远程", style = MaterialTheme.typography.titleLarge)
            Row(verticalAlignment = Alignment.CenterVertically) {
                OnlineDot(online)
                Spacer(Modifier.width(6.dp))
                RemoteIcon(RemoteGlyph.Computer, null, Modifier.size(15.dp), RemoteMuted)
                Spacer(Modifier.width(5.dp))
                Text(
                    hostName,
                    color = RemoteMuted,
                    style = MaterialTheme.typography.bodySmall,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        if (showConversationActions) {
            Row(
                Modifier
                    .align(Alignment.CenterEnd)
                    .clip(RoundedCornerShape(26.dp))
                    .background(RemoteSurfaceRaised)
                    .border(1.dp, RemoteBorder, RoundedCornerShape(26.dp)),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IconButton(onClick = onNewConversation, modifier = Modifier.size(50.dp)) {
                    RemoteIcon(RemoteGlyph.Compose, "新建会话", Modifier.size(24.dp), RemoteText)
                }
                Box {
                    IconButton(onClick = onToggleMenu, modifier = Modifier.size(50.dp)) {
                        RemoteIcon(RemoteGlyph.More, "更多选项", Modifier.size(24.dp))
                    }
                    RemoteOverflowMenu(
                        expanded = menuExpanded,
                        sortMode = sortMode,
                        online = online,
                        onDismiss = onDismissMenu,
                        onSort = onSort,
                        onShowProjects = onShowProjects,
                        onNewConversation = onNewConversation,
                        onDisconnect = onDisconnect,
                    )
                }
            }
        } else {
            Box(Modifier.align(Alignment.CenterEnd)) {
                FloatingIconButton(
                    glyph = RemoteGlyph.More,
                    description = "更多选项",
                    onClick = onToggleMenu,
                )
                RemoteOverflowMenu(
                    expanded = menuExpanded,
                    sortMode = sortMode,
                    online = online,
                    onDismiss = onDismissMenu,
                    onSort = onSort,
                    onShowProjects = onShowProjects,
                    onNewConversation = onNewConversation,
                    onDisconnect = onDisconnect,
                )
            }
        }
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
    onNewConversation: () -> Unit,
    onDisconnect: () -> Unit,
) {
    DropdownMenu(
        expanded = expanded,
        onDismissRequest = onDismiss,
        modifier = Modifier.width(278.dp),
        shape = RoundedCornerShape(24.dp),
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
        RemoteMenuRow(RemoteGlyph.Compose, "新建会话") {
            onDismiss()
            onNewConversation()
        }
        RemoteMenuRow(RemoteGlyph.Disconnect, "断开 Host", tint = MaterialTheme.colorScheme.error) { onDisconnect() }
        HorizontalDivider(Modifier.padding(vertical = 8.dp), color = RemoteBorder)
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 10.dp),
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
        modifier = Modifier.padding(horizontal = 18.dp, vertical = 8.dp),
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
        Modifier.fillMaxWidth().height(50.dp).clickable(onClick = onClick).padding(horizontal = 18.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.width(24.dp), contentAlignment = Alignment.Center) {
            if (selected) RemoteIcon(RemoteGlyph.Check, null, Modifier.size(18.dp), tint)
        }
        Spacer(Modifier.width(10.dp))
        RemoteIcon(glyph, null, Modifier.size(22.dp), tint)
        Spacer(Modifier.width(14.dp))
        Text(label, color = tint, style = MaterialTheme.typography.bodyLarge)
    }
}

@Composable
private fun RemoteHomeScreen(state: RemoteUiState, sortMode: HomeSort, viewModel: RemoteViewModel) {
    val snapshot = requireNotNull(state.snapshot)
    val projects = snapshot.projects.filter { it.valid }
    val selectedProject = projects.find { it.id == state.selectedProjectId }
    var search by rememberSaveable { mutableStateOf("") }
    var projectMenuExpanded by rememberSaveable { mutableStateOf(false) }
    val filtered = snapshot.conversations
        .asSequence()
        .filter { selectedProject == null || it.projectId == selectedProject.id }
        .filter { search.isBlank() || it.title.contains(search.trim(), ignoreCase = true) }
        .let { conversations ->
            when (sortMode) {
                HomeSort.AGENT -> conversations.sortedWith(compareBy<Conversation> { it.provider.label }.thenByDescending { it.updatedAtMs })
                HomeSort.RECENT -> conversations.sortedByDescending { it.updatedAtMs }
                HomeSort.ACTIVE -> conversations.sortedWith(compareByDescending<Conversation> { it.running }.thenByDescending { it.updatedAtMs })
            }
        }
        .toList()

    Column(Modifier.fillMaxSize()) {
        Text(
            "项目",
            modifier = Modifier.padding(horizontal = 20.dp, vertical = 18.dp),
            style = MaterialTheme.typography.titleLarge,
        )
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(Modifier.weight(1f)) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .height(56.dp)
                        .clip(RoundedCornerShape(18.dp))
                        .clickable { projectMenuExpanded = true }
                        .padding(horizontal = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RemoteIcon(RemoteGlyph.Folder, null, Modifier.size(27.dp))
                    Spacer(Modifier.width(12.dp))
                    Text(
                        selectedProject?.displayName ?: "全部项目",
                        modifier = Modifier.weight(1f),
                        style = MaterialTheme.typography.titleLarge,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    RemoteIcon(RemoteGlyph.ChevronDown, null, Modifier.size(18.dp), RemoteMuted)
                }
                DropdownMenu(
                    expanded = projectMenuExpanded,
                    onDismissRequest = { projectMenuExpanded = false },
                    modifier = Modifier.widthIn(min = 260.dp),
                    shape = RoundedCornerShape(20.dp),
                    containerColor = RemoteSurfaceRaised,
                    border = BorderStroke(1.dp, RemoteBorder),
                ) {
                    projects.forEach { project ->
                        DropdownMenuItem(
                            leadingIcon = { RemoteIcon(RemoteGlyph.Folder, null, Modifier.size(20.dp), RemoteMuted) },
                            text = { Text(project.displayName, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                            trailingIcon = {
                                if (project.id == state.selectedProjectId) {
                                    RemoteIcon(RemoteGlyph.Check, null, Modifier.size(18.dp))
                                }
                            },
                            onClick = {
                                projectMenuExpanded = false
                                viewModel.selectProject(project.id)
                            },
                        )
                    }
                }
            }
            IconButton(onClick = viewModel::showNewConversation, modifier = Modifier.size(52.dp)) {
                RemoteIcon(RemoteGlyph.Compose, "在此项目中新建会话", Modifier.size(25.dp), RemoteMuted)
            }
        }
        if (filtered.isEmpty()) {
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
                items(filtered, key = { it.id }) { conversation ->
                    ConversationListItem(conversation) { viewModel.selectConversation(conversation.id) }
                }
            }
        }
        HomeBottomBar(
            search = search,
            onSearch = { search = it },
            onNewConversation = viewModel::showNewConversation,
        )
    }
}

@Composable
private fun ConversationListItem(conversation: Conversation, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).padding(vertical = 14.dp),
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
                "${conversation.provider.label} · ${stateLabel(conversation.state)}",
                modifier = Modifier.padding(top = 3.dp),
                color = RemoteMuted,
                style = MaterialTheme.typography.bodySmall,
            )
        }
        if (conversation.running) {
            Spacer(Modifier.width(10.dp))
            Surface(color = Color(0xFF153526), shape = RoundedCornerShape(999.dp)) {
                Text(
                    "进行中",
                    modifier = Modifier.padding(horizontal = 9.dp, vertical = 5.dp),
                    color = RemoteGreen,
                    style = MaterialTheme.typography.labelSmall,
                )
            }
        }
    }
}

@Composable
private fun HomeBottomBar(search: String, onSearch: (String) -> Unit, onNewConversation: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .windowInsetsPadding(WindowInsets.ime.union(WindowInsets.navigationBars))
            .padding(start = 18.dp, top = 12.dp, end = 18.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Surface(
            modifier = Modifier.weight(1f).height(52.dp),
            color = RemoteSurfaceRaised,
            shape = RoundedCornerShape(999.dp),
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
                            IconButton(onClick = { onSearch("") }, modifier = Modifier.size(40.dp)) {
                                RemoteIcon(RemoteGlyph.Close, "清除搜索", Modifier.size(17.dp), RemoteMuted)
                            }
                        }
                    }
                },
            )
        }
        FloatingIconButton(
            glyph = RemoteGlyph.Compose,
            description = "新建会话",
            onClick = onNewConversation,
            containerColor = RemotePurple,
            contentColor = Color.White,
            size = 54.dp,
        )
    }
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
            .padding(horizontal = 20.dp, vertical = 18.dp),
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        Text("新建会话", style = MaterialTheme.typography.headlineMedium)
        Text("选择项目与 Agent。模型和推理强度始终从当前 Provider 动态读取。", color = RemoteMuted)
        Surface(
            modifier = Modifier.fillMaxWidth(),
            color = RemoteSurface,
            shape = RoundedCornerShape(26.dp),
            border = BorderStroke(1.dp, RemoteBorder),
        ) {
            Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
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
                Picker(
                    label = "模型",
                    entries = capability?.models.orEmpty().map { PickerEntry(it.id, it.displayName) },
                    selected = state.selectedModel,
                    onSelect = viewModel::selectModel,
                    emptyLabel = "Provider 默认模型",
                )
                Picker(
                    label = "推理强度",
                    entries = model?.effortOptions.orEmpty().map { PickerEntry(it.id, it.displayName) },
                    selected = state.selectedEffort,
                    onSelect = viewModel::selectEffort,
                    emptyLabel = "Provider 默认强度",
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
            }
        }
        if (capability != null) {
            ProviderStatus(capability)
        }
        Button(
            onClick = viewModel::createConversation,
            enabled = state.online && !state.creatingConversation && state.selectedProjectId != null && state.selectedProvider != null && capability?.ready == true,
            modifier = Modifier.fillMaxWidth().height(54.dp),
            shape = RoundedCornerShape(18.dp),
        ) { Text(if (state.creatingConversation) "正在创建…" else "创建会话") }
        if (projects.isEmpty()) {
            Text("Host 没有有效授权项目。请先在电脑使用 project add。", color = MaterialTheme.colorScheme.error)
        }
    }
}

@Composable
private fun ProviderStatus(capability: ProviderCapability) {
    Surface(
        color = if (capability.ready) Color(0xFF13291F) else RemoteSurfaceRaised,
        shape = RoundedCornerShape(20.dp),
        border = BorderStroke(1.dp, if (capability.ready) Color(0xFF285A40) else RemoteBorder),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(Modifier.padding(15.dp), verticalAlignment = Alignment.Top) {
            OnlineDot(capability.ready)
            Spacer(Modifier.width(10.dp))
            Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Text("${capability.provider.label} · ${providerStateLabel(capability.state)}", fontWeight = FontWeight.SemiBold)
                capability.version?.let { Text(it, color = RemoteMuted, style = MaterialTheme.typography.bodySmall) }
                capability.detail?.let { Text(it, color = RemoteMuted, style = MaterialTheme.typography.bodySmall) }
                capability.limitation?.let { Text(it, color = RemoteMuted, style = MaterialTheme.typography.bodySmall) }
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
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
    Column(
        Modifier
            .fillMaxSize()
            .windowInsetsPadding(WindowInsets.ime.union(WindowInsets.navigationBars)),
    ) {
        ConversationHeader(conversation)
        if (timeline.isEmpty()) {
            Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    RemoteIcon(RemoteGlyph.Chat, null, Modifier.size(36.dp), RemoteMuted)
                    Spacer(Modifier.height(12.dp))
                    Text("发送第一条消息开始", color = RemoteMuted)
                }
            }
        } else {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                LazyColumn(
                    state = listState,
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(start = 20.dp, top = 14.dp, end = 25.dp, bottom = 14.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
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
                CodexScrollbar(
                    state = listState,
                    modifier = Modifier.align(Alignment.CenterEnd).padding(vertical = 10.dp, horizontal = 4.dp),
                )
            }
        }
        Composer(
            draft = state.draft,
            running = conversation.running,
            online = state.online,
            supportsSteer = capability?.supportsSteer == true,
            sessionOptions = conversation.sessionOptions,
            onDraft = viewModel::setDraft,
            onSend = viewModel::sendMessage,
            onSteer = viewModel::steer,
            onInterrupt = viewModel::interrupt,
            onSessionOption = viewModel::setSessionOption,
        )
    }
}

@Composable
private fun ConversationHeader(conversation: Conversation) {
    Row(
        Modifier.fillMaxWidth().background(RemoteBlack).padding(horizontal = 20.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                conversation.title,
                style = MaterialTheme.typography.titleMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                conversation.provider.label,
                color = RemoteMuted,
                style = MaterialTheme.typography.bodySmall,
            )
        }
        StatePill(conversation.state)
    }
}

@Composable
private fun StatePill(state: String) {
    val colors = when (state) {
        "running" -> Triple(Color(0xFF153526), RemoteGreen, Color(0xFF285A40))
        "needs_approval" -> Triple(Color(0xFF362816), Color(0xFFFFC869), Color(0xFF6D5329))
        "failed" -> Triple(MaterialTheme.colorScheme.errorContainer, MaterialTheme.colorScheme.error, Color(0xFF71313B))
        "offline" -> Triple(RemoteSurfaceRaised, RemoteMuted, RemoteBorder)
        else -> Triple(RemoteSurfaceRaised, RemoteText, RemoteBorder)
    }
    Surface(color = colors.first, shape = RoundedCornerShape(999.dp), border = BorderStroke(1.dp, colors.third)) {
        Text(
            stateLabel(state),
            Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
            color = colors.second,
            style = MaterialTheme.typography.labelMedium,
        )
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
    onDraft: (String) -> Unit,
    onSend: () -> Unit,
    onSteer: () -> Unit,
    onInterrupt: () -> Unit,
    onSessionOption: (String, String) -> Unit,
) {
    val inputEnabled = online && (!running || supportsSteer)
    var settingsOpen by rememberSaveable { mutableStateOf(false) }
    Column(
        Modifier
            .fillMaxWidth()
            .background(RemoteBlack)
            .padding(start = 14.dp, top = 12.dp, end = 14.dp),
    ) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.Bottom,
        ) {
            Surface(
                modifier = Modifier.weight(1f).heightIn(min = 56.dp, max = 144.dp),
                color = RemoteSurfaceRaised,
                shape = RoundedCornerShape(28.dp),
                border = BorderStroke(1.dp, RemoteBorder),
            ) {
                Row(
                    Modifier.fillMaxWidth().padding(start = 18.dp, end = 5.dp, top = 5.dp, bottom = 5.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    BasicTextField(
                        value = draft,
                        onValueChange = onDraft,
                        enabled = inputEnabled,
                        textStyle = MaterialTheme.typography.bodyLarge.copy(color = if (inputEnabled) RemoteText else RemoteMuted),
                        modifier = Modifier
                            .weight(1f)
                            .padding(vertical = 9.dp)
                            .semantics {
                                contentDescription = when {
                                    running && !supportsSteer -> "当前 Agent 不支持追加指令"
                                    running -> "输入追加指令"
                                    else -> "回复 Agent"
                                }
                            },
                        maxLines = 5,
                        decorationBox = { input ->
                            Box {
                                if (draft.isEmpty()) {
                                    Text(
                                        when {
                                            running && !supportsSteer -> "当前 Agent 不支持追加指令"
                                            running -> "输入追加指令…"
                                            else -> "回复 Agent"
                                        },
                                        color = RemoteMuted,
                                    )
                                }
                                input()
                            }
                        },
                    )
                    if (!running || supportsSteer) {
                        IconButton(
                            onClick = if (running) onSteer else onSend,
                            enabled = online && draft.isNotBlank(),
                            modifier = Modifier.size(46.dp).background(Color.White, CircleShape),
                        ) {
                            RemoteIcon(RemoteGlyph.Send, if (running) "追加指令" else "发送", Modifier.size(22.dp), Color.Black)
                        }
                    }
                }
            }
            if (running) {
                IconButton(
                    onClick = onInterrupt,
                    enabled = online,
                    modifier = Modifier.size(56.dp).border(1.dp, Color(0xFF70343D), CircleShape).background(Color(0xFF2B171B), CircleShape),
                ) {
                    RemoteIcon(RemoteGlyph.Stop, "停止", Modifier.size(22.dp), MaterialTheme.colorScheme.error)
                }
            }
        }
        if (sessionOptions.isNotEmpty()) {
            SessionSettingsBar(
                options = sessionOptions,
                enabled = online && !running,
                onClick = { settingsOpen = true },
            )
        }
    }
    if (settingsOpen) {
        SessionSettingsSheet(
            options = sessionOptions,
            enabled = online && !running,
            onDismiss = { settingsOpen = false },
            onSelect = onSessionOption,
        )
    }
}

@Composable
private fun SessionSettingsBar(options: List<SessionOption>, enabled: Boolean, onClick: () -> Unit) {
    val model = options.find { it.id == "model" }
    val effort = options.find { it.id == "reasoning_effort" || it.id == "thought_level" }
    val permission = options.find { it.id == "permission_mode" }
    val primary = listOfNotNull(
        model?.let(::sessionOptionValueLabel),
        effort?.let(::sessionOptionValueLabel),
    ).joinToString(" · ").ifEmpty { "会话设置" }
    Row(
        Modifier.fillMaxWidth().padding(top = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        SessionSettingsChip(
            label = primary,
            enabled = enabled,
            onClick = onClick,
            modifier = Modifier.weight(1f),
        )
        permission?.let {
            SessionSettingsChip(
                label = sessionOptionValueLabel(it),
                enabled = enabled,
                onClick = onClick,
                modifier = Modifier.widthIn(min = 100.dp, max = 132.dp),
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
        modifier = modifier.height(40.dp).clip(RoundedCornerShape(14.dp)).clickable(enabled = enabled, onClick = onClick),
        color = RemoteSurfaceRaised,
        shape = RoundedCornerShape(14.dp),
        border = BorderStroke(1.dp, RemoteBorder),
    ) {
        Row(
            Modifier.fillMaxSize().padding(horizontal = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.Center,
        ) {
            Text(
                label,
                modifier = Modifier.weight(1f),
                color = if (enabled) RemoteText else RemoteMuted,
                style = MaterialTheme.typography.labelMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Spacer(Modifier.width(7.dp))
            RemoteIcon(RemoteGlyph.ChevronDown, null, Modifier.size(14.dp), RemoteMuted)
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SessionSettingsSheet(
    options: List<SessionOption>,
    enabled: Boolean,
    onDismiss: () -> Unit,
    onSelect: (String, String) -> Unit,
) {
    var activeOptionId by rememberSaveable { mutableStateOf<String?>(null) }
    val activeOption = options.find { it.id == activeOptionId }
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = RemoteSurfaceRaised,
        contentColor = RemoteText,
    ) {
        if (activeOption == null) {
            Column(
                Modifier.fillMaxWidth().padding(start = 20.dp, end = 20.dp, bottom = 24.dp),
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
                    if (activeOption.id == "permission_mode") "应如何批准 ChatGPT 操作？" else sessionOptionLabel(activeOption),
                    style = MaterialTheme.typography.titleLarge,
                    modifier = Modifier.padding(horizontal = 4.dp, vertical = 8.dp),
                )
                LazyColumn(Modifier.fillMaxWidth().heightIn(max = 430.dp)) {
                    items(activeOption.values, key = { it.value }) { value ->
                        val selected = value.value == activeOption.currentValue
                        val permission = activeOption.id == "permission_mode"
                        val selectedColor = if (permission && selected) RemoteOrange else RemoteText
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .heightIn(min = if (permission) 68.dp else 58.dp)
                                .clip(RoundedCornerShape(18.dp))
                                .clickable(enabled = enabled) {
                                    onSelect(activeOption.id, value.value)
                                    activeOptionId = null
                                }
                                .background(
                                    when {
                                        permission && selected -> RemoteOrange.copy(alpha = 0.08f)
                                        selected -> Color(0xFF303033)
                                        else -> Color.Transparent
                                    },
                                )
                                .border(
                                    1.dp,
                                    if (permission && selected) RemoteOrange.copy(alpha = 0.55f) else Color.Transparent,
                                    RoundedCornerShape(18.dp),
                                )
                                .padding(horizontal = 16.dp, vertical = 10.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            if (permission) {
                                Icon(
                                    imageVector = permissionIcon(value.value),
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
                                if (permission) permissionDescription(value.value)?.let {
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
}

@Composable
private fun SessionSettingRow(option: SessionOption, enabled: Boolean, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .height(58.dp)
            .clip(RoundedCornerShape(17.dp))
            .clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(sessionOptionLabel(option), modifier = Modifier.weight(1f), fontWeight = FontWeight.Medium)
        Text(sessionOptionValueLabel(option), color = RemoteMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
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

private fun sessionOptionValueLabel(option: SessionOption, value: String): String = when {
    option.id == "permission_mode" -> when (value) {
        "request_approval" -> "请求批准"
        "auto_review" -> "帮我批准"
        "full_access" -> "完全访问权限"
        else -> option.values.find { it.value == value }?.displayName ?: value
    }
    option.id == "reasoning_effort" || option.id == "thought_level" -> when (value) {
        "minimal" -> "最低"
        "low" -> "低"
        "medium" -> "中等"
        "high" -> "高"
        "xhigh" -> "极高"
        "max" -> "最高"
        "ultra" -> "超高"
        else -> option.values.find { it.value == value }?.displayName ?: value
    }
    else -> option.values.find { it.value == value }?.displayName ?: value
}

private fun permissionDescription(value: String): String? = when (value) {
    "request_approval" -> "编辑外部文件和使用互联网时始终询问"
    "auto_review" -> "仅对检测到的风险操作请求批准"
    "full_access" -> "可不受限制地访问互联网和你电脑上的任何文件"
    else -> null
}

private fun permissionIcon(value: String): ImageVector = when (value) {
    "request_approval" -> Icons.Outlined.TouchApp
    "auto_review" -> Icons.Outlined.Security
    else -> Icons.Outlined.WarningAmber
}

@Composable
private fun CodexScrollbar(state: LazyListState, modifier: Modifier = Modifier) {
    val layout = state.layoutInfo
    val visibleCount = layout.visibleItemsInfo.size
    val totalCount = layout.totalItemsCount
    if (visibleCount == 0 || totalCount <= visibleCount || (!state.canScrollBackward && !state.canScrollForward)) return
    BoxWithConstraints(modifier.fillMaxHeight().width(7.dp)) {
        val thumbHeight = (maxHeight * (visibleCount.toFloat() / totalCount)).coerceIn(38.dp, maxHeight)
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
                    if (state.isScrollInProgress) Color(0xFF9B9B9F) else Color(0xFF66666A),
                    CircleShape,
                ),
        )
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
        is TimelineContent.ToolCall -> ToolTimelineCard(content)
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
        color = RemoteSurface,
        shape = RoundedCornerShape(22.dp),
        border = BorderStroke(1.dp, RemoteBorder),
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
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
    return summary.takeUnless { it.isEmpty() || it == "[]" || it == "{}" || it == "null" || it == "\"\"" }
}

@Composable
private fun MessageBubble(text: String, user: Boolean, label: String = "你") {
    if (user) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
            Surface(
                modifier = Modifier.fillMaxWidth(0.86f),
                color = RemoteSurfaceRaised,
                shape = RoundedCornerShape(24.dp),
                border = BorderStroke(1.dp, RemoteBorder),
            ) {
                Text(text, modifier = Modifier.padding(horizontal = 17.dp, vertical = 14.dp), style = MaterialTheme.typography.bodyLarge)
            }
        }
    } else {
        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(7.dp)) {
            Text(label, color = RemoteMuted, style = MaterialTheme.typography.labelMedium)
            Text(text, style = MaterialTheme.typography.bodyLarge)
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
    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = if (error) MaterialTheme.colorScheme.errorContainer else RemoteSurface,
        shape = RoundedCornerShape(22.dp),
        border = BorderStroke(1.dp, if (error) Color(0xFF71313B) else RemoteBorder),
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
        color = RemoteSurface,
        shape = RoundedCornerShape(22.dp),
        border = BorderStroke(1.dp, RemoteBorder),
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
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
            Surface(color = Color(0xFF0B0B0B), shape = RoundedCornerShape(14.dp)) {
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
    onApproval: (UUID, String) -> Unit,
) {
    Surface(
        color = Color(0xFF2D2315),
        shape = RoundedCornerShape(22.dp),
        border = BorderStroke(1.dp, Color(0xFF6D5329)),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(Modifier.padding(17.dp), verticalArrangement = Arrangement.spacedBy(11.dp)) {
            Text("需要你的许可", color = Color(0xFFFFC869), fontWeight = FontWeight.SemiBold, style = MaterialTheme.typography.titleMedium)
            Text(content.prompt)
            if (content.resolvedOption != null) {
                Text("已选择：${content.resolvedOption}", color = RemoteMuted, fontWeight = FontWeight.Medium)
            } else {
                content.options.forEach { option ->
                    OutlinedButton(
                        onClick = { onApproval(content.approvalId, option.id) },
                        enabled = !pending,
                        modifier = Modifier.fillMaxWidth().height(48.dp),
                        shape = RoundedCornerShape(16.dp),
                        border = BorderStroke(1.dp, Color(0xFF765D35)),
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
        shape = RoundedCornerShape(22.dp),
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
                    modifier = Modifier.fillMaxWidth().height(240.dp).clip(RoundedCornerShape(15.dp)).background(Color(0xFF090909)),
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
    Column(verticalArrangement = Arrangement.spacedBy(7.dp)) {
        Text(label, color = RemoteMuted, style = MaterialTheme.typography.labelMedium)
        Box(Modifier.fillMaxWidth()) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .height(52.dp)
                    .clip(RoundedCornerShape(16.dp))
                    .background(RemoteSurfaceRaised)
                    .border(1.dp, RemoteBorder, RoundedCornerShape(16.dp))
                    .clickable(enabled = enabled && entries.isNotEmpty()) { expanded = true }
                    .padding(horizontal = 15.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    selectedLabel,
                    color = if (enabled && entries.isNotEmpty()) RemoteText else RemoteMuted,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                RemoteIcon(RemoteGlyph.ChevronDown, null, Modifier.size(18.dp), RemoteMuted)
            }
            DropdownMenu(
                expanded = expanded,
                onDismissRequest = { expanded = false },
                shape = RoundedCornerShape(18.dp),
                containerColor = RemoteSurfaceRaised,
                border = BorderStroke(1.dp, RemoteBorder),
            ) {
                entries.forEach { entry ->
                    DropdownMenuItem(
                        text = { Text(entry.label) },
                        trailingIcon = {
                            if (entry.value == selected) RemoteIcon(RemoteGlyph.Check, null, Modifier.size(18.dp))
                        },
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
    Stop(Icons.Rounded.Stop),
    Copy(Icons.Rounded.ContentCopy),
    Scan(Icons.Rounded.QrCodeScanner),
    Paste(Icons.Rounded.ContentPaste),
}

@Composable
private fun OnlineDot(online: Boolean) {
    Box(
        Modifier
            .size(7.dp)
            .background(if (online) RemoteGreen else RemoteMuted, CircleShape)
            .semantics { contentDescription = if (online) "在线" else "离线" },
    )
}

@Composable
private fun FloatingIconButton(
    glyph: RemoteGlyph,
    description: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    containerColor: Color = RemoteSurfaceRaised,
    contentColor: Color = RemoteText,
    size: Dp = 52.dp,
) {
    IconButton(
        onClick = onClick,
        modifier = modifier
            .size(size)
            .background(containerColor, CircleShape)
            .border(1.dp, if (containerColor == RemotePurple) Color.Transparent else RemoteBorder, CircleShape),
    ) {
        RemoteIcon(glyph, description, Modifier.size(24.dp), contentColor)
    }
}

@Composable
private fun RemoteIcon(
    glyph: RemoteGlyph,
    description: String?,
    modifier: Modifier = Modifier,
    tint: Color = RemoteText,
) {
    Icon(
        imageVector = glyph.imageVector,
        contentDescription = description,
        modifier = modifier,
        tint = tint,
    )
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
