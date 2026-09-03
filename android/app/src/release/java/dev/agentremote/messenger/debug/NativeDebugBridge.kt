package dev.agentremote.messenger.debug

import dev.agentremote.messenger.ui.RemoteViewModel

/** Release builds intentionally contain no native debug command implementation. */
object NativeDebugBridge {
    fun attach(@Suppress("UNUSED_PARAMETER") viewModel: RemoteViewModel) = Unit

    fun detach(@Suppress("UNUSED_PARAMETER") viewModel: RemoteViewModel) = Unit
}
