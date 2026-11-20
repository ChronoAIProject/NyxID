import CoreGraphics
import Foundation

let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
let cache = directory.appendingPathComponent("cache")
try FileManager.default.createDirectory(at: cache, withIntermediateDirectories: true)
defer { try? FileManager.default.removeItem(at: directory) }

func pdf(_ name: String, options: [String: Any] = [:]) -> URL {
  let url = cache.appendingPathComponent(name)
  var media = CGRect(x: 0, y: 0, width: 200, height: 300)
  var crop = CGRect(x: 50, y: 50, width: 100, height: 200)
  let cropData = Data(bytes: &crop, count: MemoryLayout<CGRect>.size)
  let context = CGContext(url as CFURL, mediaBox: &media, options as CFDictionary)!
  context.beginPDFPage([kCGPDFContextCropBox: cropData] as CFDictionary)
  // Out-of-crop red must never bleed into the white letterbox. The first
  // page is green inside the crop; the second page is entirely blue.
  context.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
  context.fill(media)
  context.setFillColor(CGColor(red: 0, green: 1, blue: 0, alpha: 1))
  context.fill(crop)
  context.endPDFPage()
  context.beginPDFPage(nil)
  context.setFillColor(CGColor(red: 0, green: 0, blue: 1, alpha: 1))
  context.fill(media)
  context.endPDFPage()
  context.closePDF()
  return url
}

func pixel(_ image: CGImage, _ x: Int, _ y: Int) -> [UInt8] {
  let data = image.dataProvider!.data! as Data
  let offset = y * image.bytesPerRow + x * 4
  return Array(data[offset..<(offset + 4)])
}

let document = pdf("first-page.pdf")
let image = renderPdfThumbnail(document.absoluteString, cacheDirectory: cache)!
precondition(image.width == 384 && image.height == 384)
let center = pixel(image, 192, 192)
precondition(center[0] < 10 && center[1] > 240 && center[2] < 10 && center[3] == 255, "must render the first page")
precondition(pixel(image, 20, 192) == [255, 255, 255, 255], "must clip crop marks from letterbox")
precondition(pixel(image, 364, 192) == [255, 255, 255, 255])

let unlocked = pdf("unlocked.pdf", options: [kCGPDFContextOwnerPassword as String: "owner"])
let encrypted = CGPDFDocument(unlocked as CFURL)!
precondition(encrypted.isEncrypted && encrypted.isUnlocked)
precondition(renderPdfThumbnail(unlocked.absoluteString, cacheDirectory: cache) != nil)
let locked = pdf("locked.pdf", options: [
  kCGPDFContextOwnerPassword as String: "owner",
  kCGPDFContextUserPassword as String: "secret",
])
precondition(renderPdfThumbnail(locked.absoluteString, cacheDirectory: cache) == nil)

let outside = directory.appendingPathComponent("outside.pdf")
try FileManager.default.copyItem(at: document, to: outside)
let link = cache.appendingPathComponent("escape.pdf")
try FileManager.default.createSymbolicLink(at: link, withDestinationURL: outside)
for uri in [outside.absoluteString, link.absoluteString, "file://remote" + document.path,
  "https://example.com/file.pdf", "relative.pdf"] {
  precondition(renderPdfThumbnail(uri, cacheDirectory: cache) == nil, "must reject non-cache inputs")
}
let empty = cache.appendingPathComponent("empty.pdf")
try Data().write(to: empty)
precondition(renderPdfThumbnail(empty.absoluteString, cacheDirectory: cache) == nil)
let corrupt = cache.appendingPathComponent("corrupt.pdf")
try Data("not a PDF".utf8).write(to: corrupt)
precondition(renderPdfThumbnail(corrupt.absoluteString, cacheDirectory: cache) == nil)
let oversized = cache.appendingPathComponent("oversized.pdf")
try FileManager.default.copyItem(at: document, to: oversized)
let handle = try FileHandle(forWritingTo: oversized)
try handle.truncate(atOffset: 20 * 1024 * 1024 + 1)
try handle.close()
precondition(renderPdfThumbnail(oversized.absoluteString, cacheDirectory: cache) == nil)
print("PDF thumbnail rendering, first page, crop, encryption, cache boundary and size proofs passed")
