package com.notibridge.app

import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.os.Build
import com.notibridge.core.AppPolicy
import com.notibridge.core.EventListener
import com.notibridge.core.NodeConfig
import com.notibridge.core.NodeEvent
import com.notibridge.core.NotiBridgeNode
import java.util.concurrent.CopyOnWriteArraySet
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

const val CHANNEL_STATUS = "status"
const val CHANNEL_MIRROR = "mirror"
private const val RETENTION_MS = 24L * 60 * 60 * 1000

fun interface BridgeEventHandler {
    fun onEvent(event: NodeEvent)
}

class NotiBridgeApp : Application(), EventListener {
    private val handlers = CopyOnWriteArraySet<BridgeEventHandler>()
    private val revision = MutableStateFlow(0L)

    /** 노드 상태가 바뀔 때마다 증가한다. UI 는 이 값을 구독해 다시 읽는다. */
    val changes = revision.asStateFlow()

    @Volatile
    private var started: NotiBridgeNode? = null

    val settings by lazy { getSharedPreferences("settings", Context.MODE_PRIVATE) }

    override fun onCreate() {
        super.onCreate()
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL_STATUS, getString(R.string.channel_status), NotificationManager.IMPORTANCE_MIN),
        )
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL_MIRROR, getString(R.string.channel_mirror), NotificationManager.IMPORTANCE_HIGH),
        )
    }

    @Synchronized
    fun node(): NotiBridgeNode {
        started?.let { return it }
        val vault = KeyVault(this)
        val config = NodeConfig(
            dataDir = filesDir.resolve("core").path,
            secretKey = vault.load("device_key"),
            storageKey = vault.load("storage_key"),
            deviceName = deviceName(),
            platform = "android",
            relayUrl = settings.getString(KEY_RELAY_URL, null),
            relayDisabled = false,
            mdnsEnabled = true,
            retentionMs = RETENTION_MS,
        )
        return NotiBridgeNode.start(config, this).also { started = it }
    }

    /** 릴레이 주소처럼 시작 설정이 바뀌면 노드를 다시 띄운다. */
    @Synchronized
    fun restartNode() {
        started?.stop()
        started = null
        node()
        notifyChanged()
    }

    fun addHandler(handler: BridgeEventHandler) {
        handlers.add(handler)
    }

    fun removeHandler(handler: BridgeEventHandler) {
        handlers.remove(handler)
    }

    override fun onEvent(event: NodeEvent) {
        handlers.forEach { it.onEvent(event) }
        notifyChanged()
    }

    fun notifyChanged() {
        revision.update { it + 1 }
    }

    fun setPolicy(pkg: String, policy: AppPolicy) {
        node().setAppRule(pkg, policy)
        notifyChanged()
    }

    private fun deviceName(): String = "${Build.MANUFACTURER} ${Build.MODEL}"

    companion object {
        const val KEY_RELAY_URL = "relay_url"

        fun of(context: Context): NotiBridgeApp = context.applicationContext as NotiBridgeApp
    }
}
