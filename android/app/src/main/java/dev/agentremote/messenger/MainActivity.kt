package dev.agentremote.messenger

import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.agentremote.messenger.ui.AgentRemoteTheme
import dev.agentremote.messenger.ui.RemoteApp
import dev.agentremote.messenger.ui.RemoteViewModel

class MainActivity : ComponentActivity() {
    private var sharedPairLink by mutableStateOf<String?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        sharedPairLink = extractSharedText(intent)
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
        )
        setContent {
            val remoteViewModel: RemoteViewModel = viewModel()
            LaunchedEffect(sharedPairLink) {
                sharedPairLink?.let(remoteViewModel::setPairLink)
            }
            AgentRemoteTheme {
                RemoteApp(remoteViewModel)
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        sharedPairLink = extractSharedText(intent)
    }

    private fun extractSharedText(intent: Intent?): String? = when (intent?.action) {
        Intent.ACTION_SEND -> intent.getStringExtra(Intent.EXTRA_TEXT)
        Intent.ACTION_VIEW -> intent.dataString
        else -> null
    }
}
