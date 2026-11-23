import CoreGraphics
import Foundation

/// A bounded, memory-only first page. No network reads or derived cache files.
func renderPdfThumbnail(_ uri: String, cacheDirectory: URL) -> CGImage? {
  guard let selected = URL(string: uri), selected.isFileURL,
    selected.host == nil || selected.host == "" || selected.host == "localhost"
  else { return nil }
  let url = selected.resolvingSymlinksInPath().standardizedFileURL
  let root = cacheDirectory.resolvingSymlinksInPath().standardizedFileURL.path
  guard url.path.hasPrefix(root + "/"),
    let attributes = try? url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey]),
    attributes.isRegularFile == true,
    let size = attributes.fileSize, size > 0, size <= 20 * 1024 * 1024,
    let document = CGPDFDocument(url as CFURL), document.isUnlocked,
    let page = document.page(at: 1)
  else { return nil }
  let bounds = page.getBoxRect(.cropBox)
  guard bounds.width.isFinite, bounds.height.isFinite,
    bounds.width > 0, bounds.height > 0,
    let context = CGContext(
      data: nil, width: 384, height: 384, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
  else { return nil }
  let square = CGRect(x: 0, y: 0, width: 384, height: 384)
  context.setFillColor(CGColor(gray: 1, alpha: 1))
  context.fill(square)
  context.concatenate(page.getDrawingTransform(.cropBox, rect: square, rotate: 0, preserveAspectRatio: true))
  context.clip(to: bounds)
  context.drawPDFPage(page)
  return context.makeImage()
}
