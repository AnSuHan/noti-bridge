package com.notibridge.app.ui

import android.Manifest
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.PowerManager
import android.provider.Settings
import android.text.format.DateFormat
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.core.app.NotificationManagerCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import com.notibridge.app.BridgeEventHandler
import com.notibridge.app.Locales
import com.notibridge.app.NotiBridgeApp
import com.notibridge.app.R
import com.notibridge.app.service.BridgeService
import com.notibridge.core.AppPolicy
import com.notibridge.core.AppRule
import com.notibridge.core.CloudDevice
import com.notibridge.core.CloudStatus
import com.notibridge.core.ConnectionPath
import com.notibridge.core.FilterKind
import com.notibridge.core.NodeEvent
import com.notibridge.core.NotificationFilter
import com.notibridge.core.NotificationRecord
import com.notibridge.core.PeerInfo
import com.notibridge.core.ReleaseNote
import com.notibridge.core.inviteUri
import com.notibridge.core.releaseNotes
import kotlin.concurrent.thread
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class MainActivity : ComponentActivity() {
    override fun attachBaseContext(newBase: Context) {
        super.attachBaseContext(Locales.wrap(newBase))
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        handlePairingLink(intent)
        setContent {
            val context = LocalContext.current
            val dark = isSystemInDarkTheme()
            val dynamic = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
            val colors = when {
                dynamic && dark -> dynamicDarkColorScheme(context)
                dynamic -> dynamicLightColorScheme(context)
                dark -> darkColorScheme()
                else -> lightColorScheme()
            }
            MaterialTheme(colorScheme = colors) {
                Root(onLanguage = ::changeLanguage)
            }
        }
    }

    /** 언어를 저장하고 코어·알림 채널에 반영한 뒤 화면을 다시 만든다. */
    private fun changeLanguage(language: String) {
        Locales.save(this, language)
        NotiBridgeApp.of(this).applyLanguage()
        recreate()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handlePairingLink(intent)
    }

    // 시스템 카메라로 QR 을 찍어도 페어링을 시작한다. 양쪽 코드 확인은 그대로 거친다.
    private fun handlePairingLink(intent: Intent?) {
        val uri = intent?.data?.takeIf { it.scheme == "notibridge" }?.toString() ?: return
        val app = NotiBridgeApp.of(this)
        thread { runCatching { app.node().join(uri) } }
    }

    override fun onResume() {
        super.onResume()
        NotiBridgeApp.of(this).notifyChanged()
    }
}

private enum class Tab(val label: Int) {
    NOTIFICATIONS(R.string.tab_notifications),
    DEVICES(R.string.tab_devices),
    SETTINGS(R.string.tab_settings),
    ABOUT(R.string.tab_about),
}

@Composable
private fun Root(onLanguage: (String) -> Unit) {
    val context = LocalContext.current
    val app = remember { NotiBridgeApp.of(context) }
    val power = remember { context.getSystemService(PowerManager::class.java) }
    val revision by app.changes.collectAsStateWithLifecycle()

    val listenerOn = remember(revision) {
        NotificationManagerCompat.getEnabledListenerPackages(context).contains(context.packageName)
    }
    val batteryOk = remember(revision) { power.isIgnoringBatteryOptimizations(context.packageName) }
    val postNotifications = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        app.notifyChanged()
    }

    if (!listenerOn || !batteryOk) {
        Onboarding(
            listenerOn = listenerOn,
            onListener = {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    postNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
                }
                context.startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS))
            },
            onBattery = {
                context.startActivity(
                    Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS)
                        .setData(Uri.parse("package:${context.packageName}")),
                )
            },
        )
        return
    }

    LaunchedEffect(Unit) { BridgeService.start(context) }
    // 언어를 바꾸면 화면을 다시 만드므로, 보던 탭을 유지한다.
    var tab by rememberSaveable { mutableStateOf(Tab.NOTIFICATIONS) }
    Scaffold(
        bottomBar = {
            NavigationBar {
                Tab.entries.forEach {
                    NavigationBarItem(
                        selected = tab == it,
                        onClick = { tab = it },
                        icon = {},
                        label = { Text(stringResource(it.label)) },
                    )
                }
            }
        },
    ) { padding ->
        Column(Modifier.padding(padding).padding(16.dp).fillMaxSize()) {
            when (tab) {
                Tab.NOTIFICATIONS -> NotificationsTab(revision)
                Tab.DEVICES -> DevicesTab(revision)
                Tab.SETTINGS -> SettingsTab(revision, onLanguage)
                Tab.ABOUT -> AboutTab()
            }
        }
    }
    PairingDialog()
}

@Composable
private fun Onboarding(listenerOn: Boolean, onListener: () -> Unit, onBattery: () -> Unit) {
    Scaffold { padding ->
        Column(
            Modifier.padding(padding).padding(24.dp).fillMaxSize(),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(stringResource(R.string.onboarding_title), style = MaterialTheme.typography.headlineMedium)
            if (!listenerOn) {
                Text(stringResource(R.string.onboarding_listener))
                Button(onClick = onListener) { Text(stringResource(R.string.onboarding_listener_button)) }
            } else {
                Text(stringResource(R.string.onboarding_battery))
                Button(onClick = onBattery) { Text(stringResource(R.string.onboarding_battery_button)) }
            }
        }
    }
}

@Composable
private fun NotificationsTab(revision: Long) {
    val context = LocalContext.current
    val app = remember { NotiBridgeApp.of(context) }
    val scope = rememberCoroutineScope()
    var items by remember { mutableStateOf(emptyList<NotificationRecord>()) }
    LaunchedEffect(revision) {
        items = withContext(Dispatchers.IO) { runCatching { app.node().activeNotifications() }.getOrDefault(emptyList()) }
    }
    if (items.isEmpty()) {
        Text(stringResource(R.string.no_notifications))
        return
    }
    LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        items(items, key = { it.nid }) { n ->
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(12.dp)) {
                    Text(n.appName, style = MaterialTheme.typography.labelMedium)
                    Text(n.title, style = MaterialTheme.typography.titleMedium)
                    Text(if (n.bodyHidden) stringResource(R.string.body_hidden) else n.body)
                    Row {
                        // 숨기기: 이 폰 목록에서만 내린다. 원본 기기의 알림은 그대로 둔다.
                        TextButton(onClick = { scope.launch(Dispatchers.IO) { app.node().hideLocal(n.nid) } }) {
                            Text(stringResource(R.string.hide))
                        }
                        TextButton(onClick = { scope.launch(Dispatchers.IO) { app.node().dismiss(n.nid) } }) {
                            Text(stringResource(R.string.dismiss))
                        }
                        TextButton(onClick = {
                            scope.launch(Dispatchers.IO) { app.node().addFilter(FilterKind.APP, n.appPkg) }
                        }) { Text(stringResource(R.string.mute_app)) }
                    }
                }
            }
        }
    }
}

@Composable
private fun pathLabel(path: ConnectionPath) = stringResource(
    when (path) {
        ConnectionPath.DIRECT -> R.string.path_direct
        ConnectionPath.RELAY -> R.string.path_relay
        ConnectionPath.DISCONNECTED -> R.string.path_disconnected
    },
)

@Composable
private fun DevicesTab(revision: Long) {
    val context = LocalContext.current
    val app = remember { NotiBridgeApp.of(context) }
    val scope = rememberCoroutineScope()
    var peers by remember { mutableStateOf(emptyList<PeerInfo>()) }
    var cloudDevices by remember { mutableStateOf(emptyList<CloudDevice>()) }
    var error by remember { mutableStateOf<String?>(null) }
    var qr by remember { mutableStateOf<Bitmap?>(null) }
    LaunchedEffect(revision) {
        peers = withContext(Dispatchers.IO) { runCatching { app.node().peers() }.getOrDefault(emptyList()) }
        // 페어링이 끝나면 이미 쓴 QR 을 치운다.
        if (peers.isNotEmpty()) qr = null
        cloudDevices = withContext(Dispatchers.IO) {
            runCatching { if (app.node().cloudStatus().signedIn) app.node().cloudDevices() else emptyList() }
                .getOrDefault(emptyList())
        }
    }
    val scanner = rememberLauncherForActivityResult(ScanContract()) { result ->
        val uri = result.contents ?: return@rememberLauncherForActivityResult
        scope.launch(Dispatchers.IO) {
            runCatching { app.node().join(uri) }.onFailure { error = it.message }
        }
    }
    LazyColumn(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = {
                    scanner.launch(ScanOptions().setDesiredBarcodeFormats(ScanOptions.QR_CODE).setBeepEnabled(false))
                }) { Text(stringResource(R.string.scan_qr)) }
                OutlinedButton(onClick = {
                    scope.launch(Dispatchers.IO) {
                        runCatching { qr = qrBitmap(inviteUri(app.node().createInvite())) }
                            .onFailure { error = it.message }
                    }
                }) { Text(stringResource(R.string.show_qr)) }
            }
        }
        qr?.let {
            item {
                Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.fillMaxWidth()) {
                    Image(it.asImageBitmap(), stringResource(R.string.pairing_qr), Modifier.size(260.dp))
                    Text(stringResource(R.string.scan_within))
                }
            }
        }
        error?.let { item { Text(it, color = MaterialTheme.colorScheme.error) } }
        if (peers.isEmpty()) item { Text(stringResource(R.string.no_peers)) }
        items(peers, key = { it.deviceId }) { peer ->
            Card(Modifier.fillMaxWidth()) {
                Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(peer.name, style = MaterialTheme.typography.titleMedium)
                        Text("${peer.platform} · ${pathLabel(peer.path)}")
                    }
                    TextButton(onClick = {
                        scope.launch(Dispatchers.IO) {
                            app.node().unpair(peer.deviceId)
                            app.notifyChanged()
                        }
                    }) { Text(stringResource(R.string.unpair)) }
                }
            }
        }
        if (cloudDevices.isNotEmpty()) {
            item { Text(stringResource(R.string.account_devices), style = MaterialTheme.typography.titleMedium) }
            items(cloudDevices, key = { "cloud-" + it.deviceId }) { d ->
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(12.dp)) {
                        val suffix = if (d.isSelf) " · " + stringResource(R.string.this_device) else ""
                        Text(d.name + suffix, style = MaterialTheme.typography.titleMedium)
                        val pairing = when {
                            d.isSelf -> ""
                            d.paired -> " · " + stringResource(R.string.paired)
                            else -> " · " + stringResource(R.string.not_paired)
                        }
                        Text("${d.platform} · v${d.version}$pairing")
                    }
                }
            }
        }
    }
}

private fun qrBitmap(text: String): Bitmap {
    val size = 720
    val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, size, size)
    val bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
    for (x in 0 until size) {
        for (y in 0 until size) {
            bitmap.setPixel(x, y, if (matrix[x, y]) 0xFF000000.toInt() else 0xFFFFFFFF.toInt())
        }
    }
    return bitmap
}

@Composable
private fun SectionTitle(text: String) {
    Text(text, style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(top = 8.dp))
}

@Composable
private fun SettingsTab(revision: Long, onLanguage: (String) -> Unit) {
    val context = LocalContext.current
    val app = remember { NotiBridgeApp.of(context) }
    val scope = rememberCoroutineScope()
    var relay by remember { mutableStateOf(app.settings.getString(NotiBridgeApp.KEY_RELAY_URL, "").orEmpty()) }
    var rules by remember { mutableStateOf(emptyList<AppRule>()) }
    var filters by remember { mutableStateOf(emptyList<NotificationFilter>()) }
    var cloud by remember { mutableStateOf<CloudStatus?>(null) }
    var pkg by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(revision) {
        withContext(Dispatchers.IO) {
            rules = runCatching { app.node().appRules() }.getOrDefault(emptyList())
            filters = runCatching { app.node().filters() }.getOrDefault(emptyList())
            cloud = runCatching { app.node().cloudStatus() }.getOrNull()
        }
    }
    val policies = listOf(
        AppPolicy.ALLOW to stringResource(R.string.policy_allow),
        AppPolicy.HIDE_BODY to stringResource(R.string.policy_hide_body),
        AppPolicy.BLOCK to stringResource(R.string.policy_block),
    )
    LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item {
            SectionTitle(stringResource(R.string.language))
            val current = Locales.setting(context)
            val options = listOf("" to stringResource(R.string.language_auto), "ko" to "한국어", "en" to "English")
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                options.forEachIndexed { index, (value, label) ->
                    SegmentedButton(
                        selected = current == value,
                        onClick = { if (current != value) onLanguage(value) },
                        shape = SegmentedButtonDefaults.itemShape(index, options.size),
                    ) { Text(label) }
                }
            }
        }
        item {
            FiltersSection(filters) { action -> scope.launch(Dispatchers.IO) { runCatching(action).onFailure { error = it.message } } }
        }
        item {
            cloud?.let { status ->
                AccountSection(status) { action ->
                    scope.launch(Dispatchers.IO) {
                        runCatching(action).onFailure { error = it.message }
                        app.notifyChanged()
                    }
                }
            }
        }
        error?.let { item { Text(it, color = MaterialTheme.colorScheme.error) } }
        item {
            HorizontalDivider()
            SectionTitle(stringResource(R.string.relay))
            OutlinedTextField(
                value = relay,
                onValueChange = { relay = it },
                label = { Text(stringResource(R.string.relay_label)) },
                modifier = Modifier.fillMaxWidth(),
            )
            Button(onClick = {
                app.settings.edit().putString(NotiBridgeApp.KEY_RELAY_URL, relay.trim().ifEmpty { null }).apply()
                scope.launch(Dispatchers.IO) { app.restartNode() }
            }) { Text(stringResource(R.string.relay_save)) }
        }
        item {
            SectionTitle(stringResource(R.string.app_rules))
            OutlinedTextField(
                value = pkg,
                onValueChange = { pkg = it },
                label = { Text(stringResource(R.string.app_rules_label)) },
                modifier = Modifier.fillMaxWidth(),
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                policies.forEach { (policy, label) ->
                    OutlinedButton(onClick = {
                        val target = pkg.trim()
                        if (target.isNotEmpty()) {
                            scope.launch(Dispatchers.IO) { app.setPolicy(target, policy) }
                            pkg = ""
                        }
                    }) { Text(label) }
                }
            }
        }
        items(rules, key = { it.appPkg }) { rule ->
            Column {
                Text(rule.appPkg)
                SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                    policies.forEachIndexed { index, (policy, label) ->
                        SegmentedButton(
                            selected = rule.policy == policy,
                            onClick = { scope.launch(Dispatchers.IO) { app.setPolicy(rule.appPkg, policy) } },
                            shape = SegmentedButtonDefaults.itemShape(index, policies.size),
                        ) { Text(label) }
                    }
                }
            }
        }
    }
}

@Composable
private fun FiltersSection(filters: List<NotificationFilter>, run: (() -> Unit) -> Unit) {
    val context = LocalContext.current
    val app = remember { NotiBridgeApp.of(context) }
    var kind by remember { mutableStateOf(FilterKind.KEYWORD) }
    var pattern by remember { mutableStateOf("") }
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        HorizontalDivider()
        SectionTitle(stringResource(R.string.filters))
        Text(stringResource(R.string.filters_hint), style = MaterialTheme.typography.bodySmall)
        val kinds = listOf(FilterKind.KEYWORD to R.string.filter_keyword, FilterKind.APP to R.string.filter_app)
        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
            kinds.forEachIndexed { index, (value, label) ->
                SegmentedButton(
                    selected = kind == value,
                    onClick = { kind = value },
                    shape = SegmentedButtonDefaults.itemShape(index, kinds.size),
                ) { Text(stringResource(label)) }
            }
        }
        OutlinedTextField(
            value = pattern,
            onValueChange = { pattern = it },
            label = {
                Text(stringResource(if (kind == FilterKind.APP) R.string.filter_app_label else R.string.filter_keyword_label))
            },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
        )
        Button(onClick = {
            val target = pattern.trim()
            if (target.isNotEmpty()) {
                val selected = kind
                run { app.node().addFilter(selected, target) }
                pattern = ""
            }
        }) { Text(stringResource(R.string.add)) }
        if (filters.isEmpty()) Text(stringResource(R.string.no_filters), style = MaterialTheme.typography.bodySmall)
        filters.forEach { f ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                val label = stringResource(if (f.kind == FilterKind.APP) R.string.filter_app else R.string.filter_keyword)
                Text("[$label] ${f.pattern}", Modifier.weight(1f))
                TextButton(onClick = { run { app.node().removeFilter(f.id) } }) { Text(stringResource(R.string.remove)) }
            }
        }
    }
}

@Composable
private fun AccountSection(status: CloudStatus, run: (() -> Unit) -> Unit) {
    val context = LocalContext.current
    val app = remember { NotiBridgeApp.of(context) }
    var apiKey by remember { mutableStateOf("") }
    var projectId by remember { mutableStateOf("") }
    var email by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var editing by remember { mutableStateOf(false) }
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        HorizontalDivider()
        SectionTitle(stringResource(R.string.account))
        Text(stringResource(R.string.account_hint), style = MaterialTheme.typography.bodySmall)
        when {
            status.signedIn -> {
                Text(stringResource(R.string.signed_in_as, status.email))
                val last = if (status.lastSync > 0) {
                    DateFormat.getDateFormat(context).format(status.lastSync) + " " +
                        DateFormat.getTimeFormat(context).format(status.lastSync)
                } else {
                    stringResource(R.string.never)
                }
                Text(stringResource(R.string.last_sync, last) + " · " + status.projectId, style = MaterialTheme.typography.bodySmall)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(onClick = { run { app.node().cloudSync() } }) { Text(stringResource(R.string.sync_now)) }
                    OutlinedButton(onClick = { run { app.node().cloudSignOut() } }) { Text(stringResource(R.string.sign_out)) }
                }
            }
            !status.configured || editing -> {
                if (!status.configured) Text(stringResource(R.string.firebase_not_configured), style = MaterialTheme.typography.bodySmall)
                OutlinedTextField(apiKey, { apiKey = it }, Modifier.fillMaxWidth(), label = { Text(stringResource(R.string.api_key)) }, singleLine = true)
                OutlinedTextField(projectId, { projectId = it }, Modifier.fillMaxWidth(), label = { Text(stringResource(R.string.project_id)) }, singleLine = true)
                Button(onClick = {
                    val (key, project) = apiKey to projectId
                    run { app.node().cloudSetConfig(key, project) }
                    editing = false
                }) { Text(stringResource(R.string.save)) }
            }
            else -> {
                OutlinedTextField(
                    email,
                    { email = it },
                    Modifier.fillMaxWidth(),
                    label = { Text(stringResource(R.string.email)) },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email),
                )
                OutlinedTextField(
                    password,
                    { password = it },
                    Modifier.fillMaxWidth(),
                    label = { Text(stringResource(R.string.password)) },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Button(onClick = {
                        val (e, p) = email to password
                        run { app.node().cloudSignIn(e, p, false) }
                        password = ""
                    }) { Text(stringResource(R.string.sign_in)) }
                    OutlinedButton(onClick = {
                        val (e, p) = email to password
                        run { app.node().cloudSignIn(e, p, true) }
                        password = ""
                    }) { Text(stringResource(R.string.sign_up)) }
                    TextButton(onClick = { editing = true }) { Text(stringResource(R.string.change_project)) }
                }
                Text(status.projectId, style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}

@Composable
private fun AboutTab() {
    val context = LocalContext.current
    val version = remember {
        runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull().orEmpty()
    }
    var notes by remember { mutableStateOf(emptyList<ReleaseNote>()) }
    LaunchedEffect(Unit) { notes = withContext(Dispatchers.IO) { releaseNotes() } }
    LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item {
            Text("noti-bridge", style = MaterialTheme.typography.headlineSmall)
            Text(stringResource(R.string.version, version))
            SectionTitle(stringResource(R.string.release_notes))
        }
        items(notes, key = { it.version }) { note ->
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text("v${note.version} · ${note.date}", style = MaterialTheme.typography.titleMedium)
                    note.items.forEach { Text("• $it", style = MaterialTheme.typography.bodyMedium) }
                }
            }
        }
    }
}

private data class PendingCode(val name: String, val code: String)

/** 페어링 중 6자리 코드 확인과 결과를 보여준다. '일치'를 누르면 상대의 확인을 기다리는 중임을 표시한다. */
@Composable
private fun PairingDialog() {
    val context = LocalContext.current
    val app = remember { NotiBridgeApp.of(context) }
    var pending by remember { mutableStateOf<PendingCode?>(null) }
    var waiting by remember { mutableStateOf(false) }
    var message by remember { mutableStateOf<String?>(null) }
    DisposableEffect(app) {
        val handler = BridgeEventHandler { event ->
            when (event) {
                is NodeEvent.PairingCode -> {
                    pending = PendingCode(event.deviceName, event.code)
                    waiting = false
                }
                is NodeEvent.PairingFinished -> {
                    pending = null
                    waiting = false
                    message = if (event.accepted) {
                        context.getString(R.string.pair_done, event.deviceName)
                    } else {
                        context.getString(R.string.pair_cancelled)
                    }
                }
                is NodeEvent.PairingFailed -> {
                    pending = null
                    waiting = false
                    message = event.reason
                }
                else -> Unit
            }
        }
        app.addHandler(handler)
        onDispose { app.removeHandler(handler) }
    }
    pending?.let { p ->
        AlertDialog(
            onDismissRequest = {},
            title = { Text(stringResource(R.string.confirm_code)) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(stringResource(R.string.confirm_code_hint, p.name))
                    Text(p.code, style = MaterialTheme.typography.headlineMedium)
                    if (waiting) {
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                            CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                            Text(stringResource(R.string.waiting_peer, p.name))
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(enabled = !waiting, onClick = {
                    waiting = true
                    thread { app.node().confirmPairing(true) }
                }) { Text(stringResource(R.string.match)) }
            },
            dismissButton = {
                TextButton(enabled = !waiting, onClick = { thread { app.node().confirmPairing(false) } }) {
                    Text(stringResource(R.string.mismatch))
                }
            },
        )
    }
    message?.let {
        AlertDialog(
            onDismissRequest = { message = null },
            text = { Text(it) },
            confirmButton = { TextButton(onClick = { message = null }) { Text(stringResource(R.string.ok)) } },
        )
    }
}
