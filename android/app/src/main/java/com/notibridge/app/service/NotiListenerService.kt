package com.notibridge.app.service

import android.app.Notification
import android.app.NotificationManager
import android.app.RemoteInput
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Build
import android.os.Bundle
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log
import android.util.LruCache
import androidx.core.content.ContextCompat
import com.notibridge.app.BridgeEventHandler
import com.notibridge.app.NotiBridgeApp
import com.notibridge.core.ActionInfo
import com.notibridge.core.CapturedNotification
import com.notibridge.core.NodeEvent
import com.notibridge.core.RemovalReason
import java.io.ByteArrayOutputStream
import java.util.concurrent.Executors

/** 폰의 알림을 코어로 넘기고, 원격 요청(삭제·액션·답장·방해 금지)을 OS 에 반영한다. */
class NotiListenerService : NotificationListenerService() {
    private val app get() = NotiBridgeApp.of(this)
    private val worker = Executors.newSingleThreadExecutor()
    private val icons = LruCache<String, ByteArray>(64)

    private val handler = BridgeEventHandler { event ->
        worker.execute { runCatching { handle(event) }.onFailure { Log.w(TAG, "원격 요청 처리 실패", it) } }
    }

    override fun onListenerConnected() {
        app.addHandler(handler)
        // 백그라운드 시작이 막힌 경우에도 리스너 자체는 노드를 돌릴 수 있으므로 실패를 무시한다.
        runCatching { ContextCompat.startForegroundService(this, Intent(this, BridgeService::class.java)) }
        worker.execute {
            // 리스너가 끊겨 있던 동안 쌓인 알림을 다시 읽어 맞춘다. 내용이 같으면 코어가 무시한다.
            runCatching { activeNotifications?.filter(::shouldMirror)?.forEach(::capture) }
        }
    }

    override fun onListenerDisconnected() {
        app.removeHandler(handler)
    }

    override fun onNotificationPosted(sbn: StatusBarNotification) {
        if (!shouldMirror(sbn)) return
        worker.execute { runCatching { capture(sbn) }.onFailure { Log.w(TAG, "알림 캡처 실패", it) } }
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification, rankingMap: RankingMap, reason: Int) {
        if (!shouldMirror(sbn)) return
        val removal = if (reason == REASON_TIMEOUT) RemovalReason.EXPIRED else RemovalReason.DISMISSED
        worker.execute { runCatching { app.node().captureRemoved(sbn.key, removal) } }
    }

    override fun onInterruptionFilterChanged(interruptionFilter: Int) {
        val enabled = interruptionFilter != NotificationManager.INTERRUPTION_FILTER_ALL
        worker.execute { runCatching { app.node().setDnd(enabled) } }
    }

    // 요약 알림은 하위 알림으로 전달되고, 진행 중 알림은 사용자가 지울 수 없어 미러링하지 않는다.
    private fun shouldMirror(sbn: StatusBarNotification): Boolean {
        val n = sbn.notification
        return sbn.packageName != packageName &&
            !sbn.isOngoing &&
            n.flags and Notification.FLAG_GROUP_SUMMARY == 0
    }

    private fun capture(sbn: StatusBarNotification) {
        val n = sbn.notification
        val extras = n.extras
        val body = extras.getCharSequence(Notification.EXTRA_BIG_TEXT)
            ?: extras.getCharSequence(Notification.EXTRA_TEXT)
        app.node().capturePosted(
            CapturedNotification(
                key = sbn.key,
                appPkg = sbn.packageName,
                appName = appLabel(sbn.packageName),
                title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty(),
                body = body?.toString().orEmpty(),
                actions = n.actions.orEmpty().mapIndexed { index, action ->
                    ActionInfo(index.toString(), action.title?.toString().orEmpty(), action.replyInput() != null)
                },
                iconPng = appIcon(sbn.packageName),
                // Android 15+ 는 민감 알림의 본문을 비워서 준다. 이를 구분하는 공개 API 는 없다.
                bodyHidden = Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM && body.isNullOrBlank(),
            ),
        )
    }

    private fun Notification.Action.replyInput(): RemoteInput? =
        remoteInputs?.firstOrNull { it.allowFreeFormInput }

    private fun appLabel(pkg: String): String = runCatching {
        packageManager.getApplicationLabel(packageManager.getApplicationInfo(pkg, 0)).toString()
    }.getOrDefault(pkg)

    private fun appIcon(pkg: String): ByteArray? {
        icons.get(pkg)?.let { return it }
        return runCatching {
            val drawable = packageManager.getApplicationIcon(pkg)
            val bitmap = Bitmap.createBitmap(ICON_SIZE, ICON_SIZE, Bitmap.Config.ARGB_8888)
            drawable.setBounds(0, 0, ICON_SIZE, ICON_SIZE)
            drawable.draw(Canvas(bitmap))
            val out = ByteArrayOutputStream()
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)
            out.toByteArray().also { icons.put(pkg, it) }
        }.getOrNull()
    }

    private fun handle(event: NodeEvent) {
        when (event) {
            is NodeEvent.CancelLocal -> cancelNotification(event.key)
            is NodeEvent.InvokeAction -> findAction(event.key, event.actionId)?.actionIntent?.send()
            is NodeEvent.SendReply -> sendReply(event)
            is NodeEvent.DndChanged -> requestInterruptionFilter(
                if (event.enabled) INTERRUPTION_FILTER_PRIORITY else INTERRUPTION_FILTER_ALL,
            )
            else -> Unit
        }
    }

    private fun findAction(key: String, actionId: String): Notification.Action? {
        val sbn = activeNotifications?.firstOrNull { it.key == key } ?: return null
        return sbn.notification.actions?.getOrNull(actionId.toIntOrNull() ?: return null)
    }

    private fun sendReply(event: NodeEvent.SendReply) {
        val action = findAction(event.key, event.actionId) ?: return
        val input = action.replyInput() ?: return
        val intent = Intent().addFlags(Intent.FLAG_RECEIVER_FOREGROUND)
        RemoteInput.addResultsToIntent(
            arrayOf(input),
            intent,
            Bundle().apply { putCharSequence(input.resultKey, event.text) },
        )
        action.actionIntent.send(this, 0, intent)
    }

    override fun onDestroy() {
        app.removeHandler(handler)
        worker.shutdown()
        super.onDestroy()
    }

    private companion object {
        const val TAG = "NotiListener"
        const val ICON_SIZE = 96
    }
}
