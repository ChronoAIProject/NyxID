package dev.nyxid.focus

import android.content.Context
import com.facebook.react.views.view.ReactViewGroup
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

class NyxFocusBoundaryModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("NyxFocusBoundary")

    View(NyxFocusBoundaryView::class) {
      Prop("blocked") { view, blocked: Boolean ->
        view.setBlocked(blocked)
      }
    }
  }
}

class NyxFocusBoundaryView(context: Context) : ReactViewGroup(context) {
  init {
    isFocusable = false
    importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
  }

  fun setBlocked(blocked: Boolean) {
    descendantFocusability = if (blocked) FOCUS_BLOCK_DESCENDANTS else FOCUS_AFTER_DESCENDANTS
    // A focused Home button can open the drawer using Enter. Blocking future
    // Tab searches must also release that existing keyboard focus.
    if (blocked) findFocus()?.clearFocus()
  }
}
