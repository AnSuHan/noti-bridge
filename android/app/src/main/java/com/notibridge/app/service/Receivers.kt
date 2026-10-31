package com.notibridge.app.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import com.notibridge.app.NotiBridgeApp
import kotlin.concurrent.thread

class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED) BridgeService.start(context)
    }
}

/** 폰에 표시된 PC 알림을 사용자가 지우면 원본 기기로 삭제를 요청한다. */
class MirrorDismissReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val nid = intent.getStringExtra(EXTRA_NID) ?: return
        val app = NotiBridgeApp.of(context)
        val pending = goAsync()
        thread {
            try {
                runCatching { app.node().dismiss(nid) }
            } finally {
                pending.finish()
            }
        }
    }

    companion object {
        private const val EXTRA_NID = "nid"

        fun intent(context: Context, nid: String): Intent =
            Intent(context, MirrorDismissReceiver::class.java).putExtra(EXTRA_NID, nid)
    }
}
