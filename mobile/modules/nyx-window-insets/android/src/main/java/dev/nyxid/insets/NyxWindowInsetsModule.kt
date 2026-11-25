package dev.nyxid.insets

import android.os.Build
import android.view.WindowInsets
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

class NyxWindowInsetsModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("NyxWindowInsets")

    Function("contentBottom") {
      val activity = appContext.currentActivity
      if (activity == null || Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
        null
      } else {
        val window = activity.windowManager.currentWindowMetrics
        val systemBars = window.windowInsets.getInsets(WindowInsets.Type.systemBars())
        (window.bounds.bottom - systemBars.bottom) / activity.resources.displayMetrics.density.toDouble()
      }
    }
  }
}
