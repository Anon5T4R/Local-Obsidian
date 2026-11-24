package com.openobsidian.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.ui.Alignment
import androidx.compose.ui.platform.LocalContext
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import com.openobsidian.android.data.AppSettings
import com.openobsidian.android.data.AppTheme
import com.openobsidian.android.data.LocalAppSettings
import com.openobsidian.android.ui.InstallPermissionDialog
import com.openobsidian.android.ui.UpdateActions
import com.openobsidian.android.ui.UpdateBanner
import com.openobsidian.android.ui.UpdateViewModel
import com.openobsidian.android.ui.noticeText
import com.openobsidian.android.ui.screens.SettingsScreen
import com.openobsidian.android.ui.screens.VaultScreen
import com.openobsidian.android.ui.screens.WelcomeScreen
import com.openobsidian.android.ui.theme.LocalNotebookTheme
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {

    private val repo         by lazy { (application as LocalNotebookApp).vaultRepo }
    private val settingsRepo by lazy { (application as LocalNotebookApp).settingsRepo }
    private val updates: UpdateViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        installSplashScreen()
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        setContent {
            val settings by settingsRepo.settings.collectAsState(initial = AppSettings())
            val scope    = rememberCoroutineScope()

            val isDark = when (settings.theme) {
                AppTheme.DARK   -> true
                AppTheme.LIGHT  -> false
                AppTheme.SYSTEM -> isSystemInDarkTheme()
            }

            var showSettings by remember { mutableStateOf(false) }

            val updateState by updates.state.collectAsState()
            val updateActions = remember {
                UpdateActions(
                    onAutoCheck = updates::setAutoCheck,
                    onCheckNow = updates::checkNow,
                    onUpdate = updates::startUpdate,
                    onDismissBanner = updates::dismissBanner,
                )
            }
            val snackbar = remember { SnackbarHostState() }
            val context = LocalContext.current
            LaunchedEffect(updateState.notice) {
                val notice = updateState.notice ?: return@LaunchedEffect
                updates.consumeNotice()
                snackbar.showSnackbar(noticeText(context, notice))
            }

            CompositionLocalProvider(LocalAppSettings provides settings) {
                LocalNotebookTheme(darkTheme = isDark) {
                    Surface(modifier = Modifier.fillMaxSize()) {
                        Box(Modifier.fillMaxSize()) {
                            Column(Modifier.fillMaxSize()) {
                            // O cartão de atualização fica acima da tela; as telas já respeitam a barra de status,
                            // então com o cartão visível ela é "consumida" aqui para não ser contada duas vezes.
                            val showBanner = updateState.showBanner
                            if (showBanner) {
                                Box(Modifier.windowInsetsPadding(WindowInsets.statusBars)) {
                                    UpdateBanner(updateState, updateActions)
                                }
                            }
                            Box(
                                Modifier
                                    .fillMaxSize()
                                    .then(if (showBanner) Modifier.consumeWindowInsets(WindowInsets.statusBars) else Modifier),
                            ) {
                                if (showSettings) {
                                    SettingsScreen(
                                        settings          = settings,
                                        onThemeChange     = { scope.launch { settingsRepo.setTheme(it) } },
                                        onFontSizeChange  = { scope.launch { settingsRepo.setPreviewFontSize(it) } },
                                        onSortOrderChange = { scope.launch { settingsRepo.setSortOrder(it) } },
                                        updateState       = updateState,
                                        updateActions     = updateActions,
                                        onClose           = { showSettings = false },
                                    )
                                } else {
                                    val current by repo.lastVault.collectAsState(initial = null)
                                    when {
                                        current.isNullOrBlank() ->
                                            WelcomeScreen(repo = repo)
                                        else ->
                                            VaultScreen(
                                                vaultUri       = current!!,
                                                repo           = repo,
                                                onOpenSettings = { showSettings = true },
                                            )
                                    }
                                }
                            }
                        }
                        SnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).navigationBarsPadding())
                        }
                    }
                    if (updateState.needsInstallPermission) {
                        InstallPermissionDialog(
                            onOpen = { updates.openInstallPermission(context) },
                            onDismiss = updates::dismissInstallPermission,
                        )
                    }
                }
            }
        }
    }

    override fun onResume() {
        super.onResume()
        updates.onResume()
    }
}
