package dev.agentremote.messenger.debug

import android.app.Activity
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.util.Log
import dev.agentremote.messenger.MainActivity
import org.json.JSONObject

/** Debug-build-only command endpoint for deterministic on-device testing through ADB. */
class NativeDebugCommandReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val command = intent.getStringExtra(EXTRA_COMMAND).orEmpty().ifBlank { "status" }
        val result = if (command.equals("launch", ignoreCase = true)) {
            context.startActivity(
                Intent(context, MainActivity::class.java).addFlags(
                    Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP,
                ),
            )
            JSONObject()
                .put("ok", true)
                .put("command", "launch")
                .put("message", "Agent Remote 已启动；随后轮询 status 直到 app_not_ready 消失")
        } else {
            NativeDebugBridge.execute(command, intent.extras ?: Bundle.EMPTY)
        }
        val encoded = result.toString()
        runCatching {
            context.openFileOutput(RESULT_FILE, Context.MODE_PRIVATE).bufferedWriter().use {
                it.write(encoded)
            }
        }.onFailure { error ->
            Log.w(LOG_TAG, "cannot persist native debug result", error)
        }
        Log.i(LOG_TAG, encoded)
        setResultCode(if (result.optBoolean("ok")) Activity.RESULT_OK else Activity.RESULT_CANCELED)
        setResultData(encoded)
        setResultExtras(Bundle().apply { putString("json", encoded) })
    }

    companion object {
        const val ACTION = "dev.agentremote.messenger.DEBUG_COMMAND"
        const val EXTRA_COMMAND = "command"
        const val RESULT_FILE = "agent-remote-native-result.json"
        const val LOG_TAG = "AgentRemoteNative"
    }
}
