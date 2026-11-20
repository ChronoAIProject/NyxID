package dev.nyxid.pdf

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.Matrix
import android.graphics.pdf.PdfRenderer
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.widget.ImageView
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import expo.modules.kotlin.viewevent.EventDispatcher
import expo.modules.kotlin.views.ExpoView
import java.io.File
import java.lang.ref.WeakReference
import java.util.concurrent.Executors
import java.util.concurrent.Future
import java.util.concurrent.atomic.AtomicBoolean

class NyxPdfThumbnailModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("NyxPdfThumbnail")
    View(NyxPdfThumbnailView::class) {
      Events("onPreviewError")
      Prop("uri") { view: NyxPdfThumbnailView, uri: String -> view.load(uri) }
      OnViewDestroys { view: NyxPdfThumbnailView -> view.dispose() }
    }
  }
}

class NyxPdfThumbnailView(context: Context, appContext: AppContext) : ExpoView(context, appContext) {
  override val shouldUseAndroidLayout = true
  private val onPreviewError by EventDispatcher()
  private val image = ImageView(context).apply {
    scaleType = ImageView.ScaleType.FIT_CENTER
    importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
  }
  private var uri: String? = null
  private var request: AtomicBoolean? = null
  private var task: Future<*>? = null

  init { addView(image, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT)) }

  fun load(uri: String) {
    if (this.uri == uri) return
    cancel()
    this.uri = uri
    if (isAttachedToWindow) start()
  }

  override fun onAttachedToWindow() {
    super.onAttachedToWindow()
    start()
  }

  override fun onDetachedFromWindow() {
    cancel()
    super.onDetachedFromWindow()
  }

  fun dispose() { cancel(); uri = null }

  private fun cancel() {
    request?.set(true)
    request = null
    task?.cancel(false)
    task = null
    image.setImageDrawable(null)
  }

  private fun start() {
    val uri = uri ?: return
    val cache = context.cacheDir
    val request = AtomicBoolean(false)
    this.request = request
    val target = WeakReference(this)
    task = renderer.submit {
      if (request.get()) return@submit
      val bitmap = try { render(uri, cache) } catch (_: Exception) { null }
      if (request.get()) { bitmap?.recycle(); return@submit }
      main.post {
        val view = target.get()
        if (request.get() || view == null || view.request !== request) { bitmap?.recycle() }
        else if (bitmap != null) { view.image.setImageBitmap(bitmap) }
        else { view.onPreviewError(emptyMap<String, Any>()) }
      }
    }
  }

  companion object {
    private val renderer = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())

    /** Only picker cache files; render page zero into a bounded, memory-only square. */
    private fun render(value: String, cacheDirectory: File): Bitmap {
      val uri = Uri.parse(value)
      require(uri.scheme == "file")
      require(uri.authority.isNullOrEmpty() || uri.authority == "localhost")
      val file = File(requireNotNull(uri.path)).canonicalFile
      require(file.path.startsWith(cacheDirectory.canonicalPath + File.separator))
      require(file.isFile && file.length() in 1..20L * 1024 * 1024)
      ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY).use { descriptor ->
        require(descriptor.statSize in 1..20L * 1024 * 1024)
        PdfRenderer(descriptor).use { document ->
          document.openPage(0).use { page ->
            require(page.width > 0 && page.height > 0)
            val bitmap = Bitmap.createBitmap(384, 384, Bitmap.Config.ARGB_8888)
            try {
              bitmap.eraseColor(Color.WHITE)
              val scale = minOf(384f / page.width, 384f / page.height)
              val transform = Matrix().apply {
                setScale(scale, scale)
                postTranslate((384 - page.width * scale) / 2, (384 - page.height * scale) / 2)
              }
              page.render(bitmap, null, transform, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
              return bitmap
            } catch (error: Exception) {
              bitmap.recycle()
              throw error
            }
          }
        }
      }
    }
  }
}
