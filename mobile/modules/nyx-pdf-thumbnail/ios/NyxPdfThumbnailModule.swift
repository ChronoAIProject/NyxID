import ExpoModulesCore
import UIKit

public class NyxPdfThumbnailModule: Module {
  public func definition() -> ModuleDefinition {
    Name("NyxPdfThumbnail")
    View(NyxPdfThumbnailView.self) {
      Events("onPreviewError")
      Prop("uri") { (view: NyxPdfThumbnailView, uri: String) in view.load(uri) }
    }
  }
}

final class NyxPdfThumbnailView: ExpoView {
  let onPreviewError = EventDispatcher()
  private let image = UIImageView()
  private var uri: String?
  private var request: PdfThumbnailRequest?
  private static let renderer = DispatchQueue(label: "dev.nyxid.pdf-thumbnail", qos: .userInitiated)

  required init(appContext: AppContext? = nil) {
    super.init(appContext: appContext)
    image.contentMode = .scaleAspectFit
    image.isAccessibilityElement = false
    addSubview(image)
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    image.frame = bounds
  }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    cancel()
    if window != nil { start() }
  }

  deinit { request?.cancel() }

  func load(_ uri: String) {
    guard self.uri != uri else { return }
    cancel()
    self.uri = uri
    if window != nil { start() }
  }

  private func cancel() {
    request?.cancel()
    request = nil
    image.image = nil
  }

  private func start() {
    guard let uri,
      let cache = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first
    else { return }
    let request = PdfThumbnailRequest()
    self.request = request
    Self.renderer.async { [weak self] in
      guard !request.cancelled else { return }
      let rendered = autoreleasepool { renderPdfThumbnail(uri, cacheDirectory: cache) }
      guard !request.cancelled else { return }
      DispatchQueue.main.async { [weak self] in
        guard let self, self.request === request, !request.cancelled else { return }
        if let rendered { self.image.image = UIImage(cgImage: rendered) }
        else { self.onPreviewError([:]) }
      }
    }
  }
}

private final class PdfThumbnailRequest {
  private let lock = NSLock()
  private var stopped = false
  var cancelled: Bool { lock.lock(); defer { lock.unlock() }; return stopped }
  func cancel() { lock.lock(); defer { lock.unlock() }; stopped = true }
}
