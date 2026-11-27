package dev.nyxid.performance

import android.app.Application
import android.content.Context
import expo.modules.core.interfaces.ApplicationLifecycleListener
import expo.modules.core.interfaces.Package

/** Expo invokes this during Application.onCreate, before any React Activity/JS. */
class NyxPerformancePackage : Package {
  override fun createApplicationLifecycleListeners(context: Context): List<ApplicationLifecycleListener> =
    listOf(object : ApplicationLifecycleListener {
      override fun onCreate(application: Application) { LaunchMonitor.initialize(application) }
    })
}
