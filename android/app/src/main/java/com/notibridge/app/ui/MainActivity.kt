package com.notibridge.app.ui

import android.Manifest
import android.content.Intent
import android.graphics.Bitmap
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.PowerManager
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
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
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.app.NotificationManagerCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import com.notibridge.app.BridgeEventHandler
import com.notibridge.app.NotiBridgeApp
import com.notibridge.app.R
import com.notibridge.app.service.BridgeService
import com.notibridge.core.AppPolicy
import com.notibridge.core.AppRule
import com.notibridge.core.ConnectionPath
import com.notibridge.core.NodeEvent
import com.notibridge.core.NotificationRecord
import com.notibridge.core.PeerInfo
import com.notibridge.core.inviteUri
import kotlin.concurrent.thread
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class MainActivity : ComponentActivity() {
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
                Root()
            }
        }
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

private enum class Tab(val label: String) { NOTIFICATIONS("알림"), DEVICES("기기"), SETTINGS("설정") }

@Composable
private fun Root() {
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
    var tab by remember { mutableStateOf(Tab.NOTIFICATIONS) }
    Scaffold(
        bottomBar = {
            NavigationBar {
                Tab.entries.forEach {
                    NavigationBarItem(
                        selected = tab == it,
                        onClick = { tab = it },
                        icon = {},
                        label = { Text(it.label) },
                    )
                }
            }
        },
    ) { padding ->
        Column(Modifier.padding(padding).padding(16.dp).fillMaxSize()) {
            when (tab) {
                Tab.NOTIFICATIONS -> NotificationsTab(revision)
                Tab.DEVICES -> DevicesTab(revision)
                Tab.SETTINGS -> SettingsTab(revision)
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
            Text("시작하기", style = MaterialTheme.typography.headlineMedium)
            if (!listenerOn) {
                Text("1단계: 다른 앱의 알림을 읽으려면 알림 접근 권한이 필요합니다.")
                Button(onClick = onListener) { Text("알림 접근 허용") }
            } else {
                Text("2단계: 백그라운드 연결이 끊기지 않도록 배터리 최적화에서 제외해 주세요.")
                Button(onClick = onBattery) { Text("배터리 최적화 제외") }
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
        Text("표시할 알림이 없습니다.")
        return
    }
    LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        items(items, key = { it.nid }) { n ->
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(12.dp)) {
                    Text(n.appName, style = MaterialTheme.typography.labelMedium)
                    Text(n.title, style = MaterialTheme.typography.titleMedium)
                    Text(if (n.bodyHidden) context.getString(R.string.body_hidden) else n.body)
                    TextButton(onClick = { scope.launch(Dispatchers.IO) { app.node().dismiss(n.nid) } }) {
                        Text("지우기")
                    }
                }
            }
        }
    }
}

private fun pathLabel(path: ConnectionPath) = when (path) {
    ConnectionPath.DIRECT -> "직접 연결"
    ConnectionPath.RELAY -> "중계 연결"
    ConnectionPath.DISCONNECTED -> "연결 끊김"
}

@Composable
private fun DevicesTab(revision: Long) {
    val context = LocalContext.current
    val app = remember { NotiBridgeApp.of(context) }
    val scope = rememberCoroutineScope()
    var peers by remember { mutableStateOf(emptyList<PeerInfo>()) }
    var error by remember { mutableStateOf<String?>(null) }
    var qr by remember { mutableStateOf<Bitmap?>(null) }
    LaunchedEffect(revision) {
        peers = withContext(Dispatchers.IO) { runCatching { app.node().peers() }.getOrDefault(emptyList()) }
    }
    val scanner = rememberLauncherForActivityResult(ScanContract()) { result ->
        val uri = result.contents ?: return@rememberLauncherForActivityResult
        scope.launch(Dispatchers.IO) {
            runCatching { app.node().join(uri) }.onFailure { error = it.message }
        }
    }
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = {
                scanner.launch(ScanOptions().setDesiredBarcodeFormats(ScanOptions.QR_CODE).setBeepEnabled(false))
            }) { Text("QR 스캔으로 추가") }
            OutlinedButton(onClick = {
                scope.launch(Dispatchers.IO) {
                    runCatching { qr = qrBitmap(inviteUri(app.node().createInvite())) }
                        .onFailure { error = it.message }
                }
            }) { Text("QR 표시") }
        }
        qr?.let {
            Image(it.asImageBitmap(), "페어링 QR", Modifier.size(260.dp).align(Alignment.CenterHorizontally))
            Text("5분 안에 다른 기기로 스캔하세요.")
        }
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
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
                        }) { Text("해제") }
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

private val policies = listOf(AppPolicy.ALLOW to "허용", AppPolicy.HIDE_BODY to "본문 숨김", AppPolicy.BLOCK to "차단")

@Composable
private fun SettingsTab(revision: Long) {
    val context = LocalContext.current
    val app = remember { NotiBridgeApp.of(context) }
    val scope = rememberCoroutineScope()
    var relay by remember { mutableStateOf(app.settings.getString(NotiBridgeApp.KEY_RELAY_URL, "").orEmpty()) }
    var rules by remember { mutableStateOf(emptyList<AppRule>()) }
    var pkg by remember { mutableStateOf("") }
    LaunchedEffect(revision) {
        rules = withContext(Dispatchers.IO) { runCatching { app.node().appRules() }.getOrDefault(emptyList()) }
    }
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text("릴레이", style = MaterialTheme.typography.titleMedium)
        OutlinedTextField(
            value = relay,
            onValueChange = { relay = it },
            label = { Text("릴레이 주소 (비우면 iroh 공개 릴레이)") },
            modifier = Modifier.fillMaxWidth(),
        )
        Button(onClick = {
            app.settings.edit().putString(NotiBridgeApp.KEY_RELAY_URL, relay.trim().ifEmpty { null }).apply()
            scope.launch(Dispatchers.IO) { app.restartNode() }
        }) { Text("저장 후 재연결") }

        Text("앱별 미러링", style = MaterialTheme.typography.titleMedium)
        OutlinedTextField(
            value = pkg,
            onValueChange = { pkg = it },
            label = { Text("패키지 이름 (예: com.kakao.talk)") },
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
        LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
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
}

private data class PendingCode(val name: String, val code: String)

/** 페어링 중 6자리 코드 확인과 결과를 보여준다. */
@Composable
private fun PairingDialog() {
    val context = LocalContext.current
    val app = remember { NotiBridgeApp.of(context) }
    var pending by remember { mutableStateOf<PendingCode?>(null) }
    var message by remember { mutableStateOf<String?>(null) }
    DisposableEffect(app) {
        val handler = BridgeEventHandler { event ->
            when (event) {
                is NodeEvent.PairingCode -> pending = PendingCode(event.deviceName, event.code)
                is NodeEvent.PairingFinished -> {
                    pending = null
                    message = if (event.accepted) "${event.deviceName} 연결 완료" else "페어링이 취소되었습니다."
                }
                is NodeEvent.PairingFailed -> {
                    pending = null
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
            title = { Text("확인 코드") },
            text = { Text("${p.name} 화면의 코드와 같은지 확인하세요.\n\n${p.code}") },
            confirmButton = { TextButton(onClick = { app.node().confirmPairing(true) }) { Text("일치") } },
            dismissButton = { TextButton(onClick = { app.node().confirmPairing(false) }) { Text("다름") } },
        )
    }
    message?.let {
        AlertDialog(
            onDismissRequest = { message = null },
            text = { Text(it) },
            confirmButton = { TextButton(onClick = { message = null }) { Text("확인") } },
        )
    }
}
