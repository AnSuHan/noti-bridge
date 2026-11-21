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
import androidx.core.app.NotificationCompat
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
    // 개수가 아니라 바이트로 제한한다. 96px PNG 는 대개 수 KB 라 수십 개 앱 아이콘이 들어간다.
    private val icons = object : LruCache<String, ByteArray>(ICON_CACHE_BYTES) {
        override fun sizeOf(key: String, value: ByteArray) = value.size
    }

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
    // 음악 재생 같은 미디어 알림도 알림이 아니라 재생 상태라 보내지 않는다(곡이 바뀔 때마다 PC 에 뜨고, 지우면 재생이 멈춘다).
    private fun shouldMirror(sbn: StatusBarNotification): Boolean {
        val n = sbn.notification
        return sbn.packageName != packageName &&
            !sbn.isOngoing &&
            n.flags and Notification.FLAG_GROUP_SUMMARY == 0 &&
            !isMedia(n)
    }

    private fun isMedia(n: Notification): Boolean =
        n.extras.containsKey(Notification.EXTRA_MEDIA_SESSION) ||
            n.category == Notification.CATEGORY_TRANSPORT ||
            n.extras.getString(Notification.EXTRA_TEMPLATE).orEmpty().contains("MediaStyle")

    private fun capture(sbn: StatusBarNotification) {
        val n = sbn.notification
        val extras = n.extras
        // Android 15+ 는 OTP 처럼 보이는 알림(숫자가 든 메시지 포함)을 알림 리스너에 가려서 준다:
        // 제목은 앱 이름, 본문은 시스템 문구 "Sensitive notification content hidden". 이 문구를 본문으로 보내지 않는다.
        val redacted = isRedacted(extras.getCharSequence(Notification.EXTRA_TEXT))
        val body = if (redacted) "" else extractBody(n)
        val title = extractTitle(n)
        app.node().capturePosted(
            CapturedNotification(
                key = sbn.key,
                appPkg = sbn.packageName,
                appName = appLabel(sbn.packageName),
                title = title,
                body = body,
                actions = n.actions.orEmpty().mapIndexed { index, action ->
                    ActionInfo(index.toString(), action.title?.toString().orEmpty(), action.replyInput() != null)
                },
                iconPng = appIcon(sbn.packageName),
                // Android 15+ 는 민감 알림(OTP 등)의 본문을 비워서 준다. 이를 구분하는 공개 API 는 없어,
                // 대화형·목록형까지 모두 읽고도 본문이 없을 때만 숨겨진 것으로 본다.
                bodyHidden = redacted || (Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM && body.isBlank()),
            ),
        )
    }

    /**
     * 카카오톡·Slack 은 MessagingStyle(대화형) 알림이라 EXTRA_TEXT 가 비거나 요약 문구만 있는 경우가 있다.
     * 대화 메시지 → 긴 본문 → 본문 → 목록형 줄 → 요약 순으로 처음 비어 있지 않은 값을 쓴다.
     * 예전에는 EXTRA_BIG_TEXT 가 빈 문자열이면 그대로 써서 본문이 비고 '내용 숨겨짐'으로 보였다.
     */
    private fun extractBody(n: Notification): String {
        val extras = n.extras
        val messaging = runCatching { NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(n) }.getOrNull()
        val messages = messaging?.messages.orEmpty().filter { !it.text.isNullOrBlank() }
        if (messages.isNotEmpty()) {
            val group = messaging?.isGroupConversation == true
            return messages.takeLast(MAX_MESSAGES).joinToString("\n") { m ->
                val sender = m.person?.name?.toString()
                if (group && !sender.isNullOrBlank()) "$sender: ${m.text}" else m.text.toString()
            }
        }
        val lines = extras.getCharSequenceArray(Notification.EXTRA_TEXT_LINES)
            ?.filter { !it.isNullOrBlank() }
            ?.takeLast(MAX_MESSAGES)
            ?.joinToString("\n")
        return listOf(
            extras.getCharSequence(Notification.EXTRA_BIG_TEXT),
            extras.getCharSequence(Notification.EXTRA_TEXT),
            lines,
            extras.getCharSequence(Notification.EXTRA_SUMMARY_TEXT),
            n.tickerText,
        ).firstOrNull { !it.isNullOrBlank() }?.toString().orEmpty()
    }

    private val redactedText: String? by lazy {
        val res = android.content.res.Resources.getSystem()
        val id = res.getIdentifier("redacted_notification_message", "string", "android")
        if (id == 0) null else runCatching { res.getString(id) }.getOrNull()
    }

    private fun isRedacted(text: CharSequence?): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM &&
            text != null && redactedText != null && text.toString().trim() == redactedText

    /** 단체 대화는 대화방 이름을, 그 밖에는 큰 제목 → 제목 순으로 쓴다. */
    private fun extractTitle(n: Notification): String {
        val extras = n.extras
        val conversation = extras.getCharSequence(Notification.EXTRA_CONVERSATION_TITLE)
        return listOf(
            conversation,
            extras.getCharSequence(Notification.EXTRA_TITLE_BIG),
            extras.getCharSequence(Notification.EXTRA_TITLE),
        ).firstOrNull { !it.isNullOrBlank() }?.toString().orEmpty()
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
            bitmap.recycle()
            out.toByteArray().also { icons.put(pkg, it) }
        }.getOrNull()
    }

    private fun handle(event: NodeEvent) {
        when (event) {
            is NodeEvent.CancelLocal -> cancelIfClearable(event.key)
            is NodeEvent.InvokeAction -> findAction(event.key, event.actionId)?.actionIntent?.send()
            is NodeEvent.SendReply -> sendReply(event)
            is NodeEvent.DndChanged -> requestInterruptionFilter(
                if (event.enabled) INTERRUPTION_FILTER_PRIORITY else INTERRUPTION_FILTER_ALL,
            )
            else -> Unit
        }
    }

    /** 원격 삭제 요청이 와도 미디어·고정 알림은 지우지 않는다. 지우면 음악 재생 같은 앱이 멈춘다. */
    private fun cancelIfClearable(key: String) {
        val sbn = activeNotifications?.firstOrNull { it.key == key } ?: return
        val n = sbn.notification
        if (isMedia(n) || sbn.isOngoing || n.flags and Notification.FLAG_NO_CLEAR != 0) {
            Log.i(TAG, "미디어·고정 알림이라 원격 삭제를 건너뜀: ${sbn.packageName}")
            return
        }
        cancelNotification(key)
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
        const val ICON_CACHE_BYTES = 256 * 1024
        /** 대화형·목록형 알림에서 본문으로 가져올 최근 메시지 수. */
        const val MAX_MESSAGES = 3
    }
}
