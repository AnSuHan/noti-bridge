package com.notibridge.app

import android.content.Context
import android.content.res.Configuration
import java.util.Locale

/**
 * 앱 언어. 사용자가 고르지 않으면 시스템 언어를 따른다.
 * 화면(Activity)과 서비스는 attachBaseContext 에서 [wrap] 한 컨텍스트로 문자열을 읽는다.
 */
object Locales {
    const val KEY = "language"
    val SUPPORTED = listOf("ko", "en")

    private fun prefs(context: Context) = context.getSharedPreferences("settings", Context.MODE_PRIVATE)

    /** "ko", "en", 또는 빈 문자열(자동). */
    fun setting(context: Context): String = prefs(context).getString(KEY, "").orEmpty()

    fun save(context: Context, language: String) {
        prefs(context).edit().putString(KEY, language.takeIf { it in SUPPORTED }).apply()
    }

    /** 실제로 쓰는 언어 코드. 자동이면 시스템 언어가 한국어일 때만 ko. */
    fun effective(context: Context): String = setting(context).ifEmpty {
        if (Locale.getDefault().language == "ko") "ko" else "en"
    }

    fun wrap(base: Context): Context {
        val language = setting(base)
        if (language.isEmpty()) return base
        val locale = Locale.forLanguageTag(language)
        val config = Configuration(base.resources.configuration)
        config.setLocale(locale)
        return base.createConfigurationContext(config)
    }
}
