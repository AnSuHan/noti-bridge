package com.notibridge.app.service

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.net.ConnectivityManager
import android.net.Network
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import androidx.core.app.ServiceCompat
import com.notibridge.app.BridgeEventHandler
import com.notibridge.app.CHANNEL_MIRROR
import com.notibridge.app.CHANNEL_STATUS
import com.notibridge.app.NotiBridgeApp
import com.notibridge.app.R
import com.notibridge.app.ui.MainActivity
import com.notibridge.core.ConnectionPath
import com.notibridge.core.NodeEvent
import com.notibridge.core.NotificationRecord
import java.util.concurrent.Executors

/** 백그라운드 연결을 유지하고, 다른 기기에서 온 알림을 표시한다. */
class BridgeService : Service() {
    private val app get() = NotiBridgeApp.of(this)
    private val io = Executors.newSingleThreadExecutor()
    private var multicastLock: WifiManager.MulticastLock? = null

    private val handler = BridgeEventHandler { event ->
        when (event) {
            is NodeEvent.ConnectionChanged -> io.execute { updateStatus() }
            is NodeEvent.NotificationChanged -> io.execute { showMirror(event.notification) }
            is NodeEvent.NotificationRemoved -> io.execute { cancelMirror(event.nid) }
            else -> Unit
        }
    }

    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) = networkChanged()
        override fun onLost(network: Network) = networkChanged()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        ServiceCompat.startForeground(
            this,
            STATUS_ID,
            statusNotification(getString(R.string.status_connecting)),
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
            } else {
                0
            },
        )
        app.addHandler(handler)
        // mDNS 패킷은 멀티캐스트 락이 있어야 받을 수 있다.
        multicastLock = getSystemService(WifiManager::class.java).createMulticastLock("noti-bridge").apply {
            setReferenceCounted(false)
            acquire()
        }
        getSystemService(ConnectivityManager::class.java).registerDefaultNetworkCallback(networkCallback)
        io.execute {
            app.node()
            updateStatus()
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = START_STICKY

    private fun networkChanged() {
        io.execute { runCatching { app.node().networkChanged() } }
    }

    private fun statusNotification(text: String): Notification {
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE,
        )
        return Notification.Builder(this, CHANNEL_STATUS)
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setContentIntent(open)
            .setOngoing(true)
            .build()
    }

    private fun updateStatus() {
        val peers = runCatching { app.node().peers() }.getOrDefault(emptyList())
        val direct = peers.firstOrNull { it.path == ConnectionPath.DIRECT }
        val relay = peers.firstOrNull { it.path == ConnectionPath.RELAY }
        val text = when {
            direct != null -> getString(R.string.status_direct, direct.name)
            relay != null -> getString(R.string.status_relay, relay.name)
            else -> getString(R.string.status_connecting)
        }
        getSystemService(NotificationManager::class.java).notify(STATUS_ID, statusNotification(text))
    }

    private fun showMirror(n: NotificationRecord) {
        if (n.origin == app.node().deviceId()) return
        val manager = getSystemService(NotificationManager::class.java)
        val delete = PendingIntent.getBroadcast(
            this,
            n.nid.hashCode(),
            MirrorDismissReceiver.intent(this, n.nid),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val icon = runCatching { app.node().icon(n.iconHash) }.getOrNull()
        val builder = Notification.Builder(this, CHANNEL_MIRROR)
            .setSmallIcon(R.drawable.ic_stat_bridge)
            .setContentTitle(n.title.ifEmpty { n.appName })
            .setContentText(if (n.bodyHidden) getString(R.string.body_hidden) else n.body)
            .setSubText(n.appName)
            .setDeleteIntent(delete)
            .setAutoCancel(true)
            .setOnlyAlertOnce(true)
        if (icon != null) builder.setLargeIcon(Icon.createWithData(icon, 0, icon.size))
        manager.notify(n.nid, MIRROR_ID, builder.build())
    }

    private fun cancelMirror(nid: String) {
        getSystemService(NotificationManager::class.java).cancel(nid, MIRROR_ID)
    }

    override fun onDestroy() {
        app.removeHandler(handler)
        getSystemService(ConnectivityManager::class.java).unregisterNetworkCallback(networkCallback)
        multicastLock?.release()
        io.shutdown()
        super.onDestroy()
    }

    companion object {
        private const val STATUS_ID = 1
        private const val MIRROR_ID = 2

        fun start(context: Context) {
            context.startForegroundService(Intent(context, BridgeService::class.java))
        }
    }
}
