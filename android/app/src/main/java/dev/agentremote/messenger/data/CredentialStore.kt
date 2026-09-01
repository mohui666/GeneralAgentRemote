package dev.agentremote.messenger.data

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import dev.agentremote.messenger.model.StoredCredential
import org.json.JSONArray
import org.json.JSONObject
import java.nio.charset.StandardCharsets
import java.security.KeyStore
import java.util.UUID
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

class CredentialStore(context: Context) {
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)

    fun load(): List<StoredCredential> {
        val encodedCiphertext = preferences.getString(CIPHERTEXT, null) ?: return emptyList()
        val encodedIv = preferences.getString(IV, null) ?: return emptyList()
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(
            Cipher.DECRYPT_MODE,
            loadKey(),
            GCMParameterSpec(128, Base64.decode(encodedIv, Base64.NO_WRAP)),
        )
        val plaintext = cipher.doFinal(Base64.decode(encodedCiphertext, Base64.NO_WRAP))
        val array = JSONArray(String(plaintext, StandardCharsets.UTF_8))
        return buildList {
            for (index in 0 until array.length()) {
                val item = array.getJSONObject(index)
                add(
                    StoredCredential(
                        hostId = UUID.fromString(item.getString("hostId")),
                        deviceId = UUID.fromString(item.getString("deviceId")),
                        deviceToken = item.getString("deviceToken"),
                        origin = item.getString("origin"),
                        relay = item.getBoolean("relay"),
                        displayName = item.getString("displayName"),
                    ),
                )
            }
        }
    }

    fun save(credentials: List<StoredCredential>) {
        if (credentials.isEmpty()) {
            preferences.edit().clear().apply()
            return
        }
        val array = JSONArray()
        credentials.forEach { credential ->
            array.put(
                JSONObject()
                    .put("hostId", credential.hostId.toString())
                    .put("deviceId", credential.deviceId.toString())
                    .put("deviceToken", credential.deviceToken)
                    .put("origin", credential.origin)
                    .put("relay", credential.relay)
                    .put("displayName", credential.displayName),
            )
        }
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, loadKey())
        val ciphertext = cipher.doFinal(array.toString().toByteArray(StandardCharsets.UTF_8))
        preferences.edit()
            .putString(CIPHERTEXT, Base64.encodeToString(ciphertext, Base64.NO_WRAP))
            .putString(IV, Base64.encodeToString(cipher.iv, Base64.NO_WRAP))
            .apply()
    }

    private fun loadKey(): SecretKey {
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (keyStore.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(
            KeyGenParameterSpec.Builder(
                KEY_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .build(),
        )
        return generator.generateKey()
    }

    companion object {
        private const val PREFERENCES = "agent_remote_credentials"
        private const val CIPHERTEXT = "ciphertext"
        private const val IV = "iv"
        private const val KEY_ALIAS = "agent_remote_device_credentials_v1"
        private const val TRANSFORMATION = "AES/GCM/NoPadding"
    }
}
