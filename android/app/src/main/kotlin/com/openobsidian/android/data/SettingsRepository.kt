package com.openobsidian.android.data

import android.content.Context
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.map
import java.io.IOException

private val Context.settingsDataStore by preferencesDataStore(name = "settings")

class SettingsRepository(private val context: Context) {

    private val keyTheme     = stringPreferencesKey("theme")
    private val keyFontSize  = stringPreferencesKey("font_size")
    private val keySortOrder = stringPreferencesKey("sort_order")

    val settings: Flow<AppSettings> = context.settingsDataStore.data.map { prefs ->
        AppSettings(
            theme = when (prefs[keyTheme]) {
                AppTheme.LIGHT.name -> AppTheme.LIGHT
                AppTheme.DARK.name  -> AppTheme.DARK
                else                -> AppTheme.SYSTEM
            },
            previewFontSize = when (prefs[keyFontSize]) {
                PreviewFontSize.SMALL.name -> PreviewFontSize.SMALL
                PreviewFontSize.LARGE.name -> PreviewFontSize.LARGE
                else                       -> PreviewFontSize.MEDIUM
            },
            sortOrder = when (prefs[keySortOrder]) {
                SortOrder.NAME_DESC.name -> SortOrder.NAME_DESC
                SortOrder.MODIFIED.name  -> SortOrder.MODIFIED
                else                     -> SortOrder.NAME_ASC
            },
        )
    }

    suspend fun setTheme(theme: AppTheme) {
        context.settingsDataStore.edit { it[keyTheme] = theme.name }
    }

    suspend fun setPreviewFontSize(size: PreviewFontSize) {
        context.settingsDataStore.edit { it[keyFontSize] = size.name }
    }

    suspend fun setSortOrder(order: SortOrder) {
        context.settingsDataStore.edit { it[keySortOrder] = order.name }
    }

    // ── Atualizador (desligado por padrão; nada dele toca o vault) ───────────

    private val keyUpdateAuto      = booleanPreferencesKey("update_auto")
    private val keyUpdateLastCheck = longPreferencesKey("update_last_check")
    private val keyUpdateStatus    = stringPreferencesKey("update_last_status")
    private val keyUpdateAvailable = stringPreferencesKey("update_available")
    private val keyUpdateDismissed = stringPreferencesKey("update_dismissed")
    private val keyUpdatePending   = stringPreferencesKey("update_pending")
    private val keyUpdateNotice    = stringPreferencesKey("update_notice")

    val updatePrefs: Flow<UpdatePrefs> = context.settingsDataStore.data
        .catch { if (it is IOException) emit(emptyPreferences()) else throw it }
        .map { p ->
            UpdatePrefs(
                autoCheck = p[keyUpdateAuto] ?: false,
                lastCheckAt = p[keyUpdateLastCheck] ?: 0L,
                lastStatus = p[keyUpdateStatus],
                availableJson = p[keyUpdateAvailable],
                dismissedVersion = p[keyUpdateDismissed],
                pendingVersion = p[keyUpdatePending],
                installNotice = p[keyUpdateNotice],
            )
        }

    suspend fun setUpdateAuto(enabled: Boolean) {
        context.settingsDataStore.edit { it[keyUpdateAuto] = enabled }
    }

    /** Resultado de uma verificação: status ("uptodate", "available" ou "error:<FALHA>:<código>") e a release. */
    suspend fun setUpdateCheckResult(at: Long, status: String, availableJson: String?) {
        context.settingsDataStore.edit { p ->
            p[keyUpdateLastCheck] = at
            p[keyUpdateStatus] = status
            if (availableJson == null) p.remove(keyUpdateAvailable) else p[keyUpdateAvailable] = availableJson
        }
    }

    suspend fun setUpdateDismissed(version: String?) {
        context.settingsDataStore.edit { p ->
            if (version == null) p.remove(keyUpdateDismissed) else p[keyUpdateDismissed] = version
        }
    }

    /** Versão entregue ao instalador do sistema (ao abrir já nela, o app avisa que atualizou). */
    suspend fun setUpdatePending(version: String?) {
        context.settingsDataStore.edit { p ->
            if (version == null) p.remove(keyUpdatePending) else p[keyUpdatePending] = version
        }
    }

    /** Aviso para a próxima abertura ("ok:<versão>", "fail:<mensagem>" ou "aborted"); null apaga. */
    suspend fun setUpdateNotice(notice: String?) {
        context.settingsDataStore.edit { p ->
            if (notice == null) p.remove(keyUpdateNotice) else p[keyUpdateNotice] = notice
        }
    }
}

/** Estado guardado do atualizador. [autoCheck] nasce desligado: sem ele (ou o "verificar agora") não há rede. */
data class UpdatePrefs(
    val autoCheck: Boolean = false,
    val lastCheckAt: Long = 0L,
    val lastStatus: String? = null,
    val availableJson: String? = null,
    val dismissedVersion: String? = null,
    val pendingVersion: String? = null,
    val installNotice: String? = null,
)
