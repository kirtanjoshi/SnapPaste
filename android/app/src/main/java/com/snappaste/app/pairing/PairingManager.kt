package com.snappaste.app.pairing

import android.content.Context
import android.util.Log
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKeys
import java.util.UUID

/**
 * Handles the secure generation, persistence, and retrieval of pairing tokens.
 *
 * ## Design (per Architecture.md §7)
 *
 * Tokens are generated on first run and stored securely using [EncryptedSharedPreferences]
 * to prevent extraction from rooted devices or backup snooping. A subsequent connection
 * must transmit this token in an `AUTH` packet before any image traffic is accepted.
 */
class PairingManager(private val context: Context) {

    companion object {
        private const val TAG = "PairingManager"
        private const val PREFS_FILE = "snappaste_secure_prefs"
        private const val KEY_PAIRING_TOKEN = "pairing_token"
    }

    // Initialize EncryptedSharedPreferences lazily.
    private val sharedPreferences by lazy {
        val masterKeyAlias = MasterKeys.getOrCreate(MasterKeys.AES256_GCM_SPEC)
        EncryptedSharedPreferences.create(
            PREFS_FILE,
            masterKeyAlias,
            context,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM
        )
    }

    /**
     * Retrieves the existing pairing token, or generates and saves a new one if not present.
     */
    @Synchronized
    fun getOrGenerateToken(): String {
        var token = sharedPreferences.getString(KEY_PAIRING_TOKEN, null)
        if (token == null) {
            token = UUID.randomUUID().toString().replace("-", "")
            sharedPreferences.edit().putString(KEY_PAIRING_TOKEN, token).apply()
            Log.i(TAG, "Generated and saved new pairing token.")
        }
        return token
    }

    /**
     * Forcefully updates or sets a pairing token.
     * Useful for diagnostics or syncing with the desktop token manually.
     */
    @Synchronized
    fun saveToken(token: String) {
        sharedPreferences.edit().putString(KEY_PAIRING_TOKEN, token).apply()
        Log.i(TAG, "Saved pairing token manually.")
    }

    /**
     * Clears the stored token, forcing a new generation next time.
     */
    @Synchronized
    fun clearToken() {
        sharedPreferences.edit().remove(KEY_PAIRING_TOKEN).apply()
        Log.i(TAG, "Cleared pairing token.")
    }
}
