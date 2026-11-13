// Draws a "DEV" band across the lower part of every PNG in an
// .iconset folder, so Agent-Desk Dev never looks like the released Agent-Desk in the Dock,
// Finder, or the app switcher (scripts/build-app.sh, AGENT_DESK_DEV_BUILD=1).
//
//   swift scripts/badge-icon.swift <folder.iconset> [LABEL]
import AppKit
import Foundation

let arguments = CommandLine.arguments
guard arguments.count >= 2 else {
  FileHandle.standardError.write("usage: badge-icon.swift <folder.iconset> [LABEL]\n".data(using: .utf8)!)
  exit(2)
}
let folder = URL(fileURLWithPath: arguments[1])
let label = arguments.count >= 3 ? arguments[2] : "DEV"
let files = try FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil)
  .filter { $0.pathExtension == "png" }

for file in files {
  guard let source = NSImage(contentsOf: file),
    let rep = source.representations.first
  else { continue }
  let width = rep.pixelsWide
  let height = rep.pixelsHigh
  guard width > 0, height > 0,
    let bitmap = NSBitmapImageRep(
      bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height, bitsPerSample: 8,
      samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
      bytesPerRow: 0, bitsPerPixel: 0)
  else { continue }
  NSGraphicsContext.saveGraphicsState()
  NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
  let size = CGFloat(width)
  source.draw(in: NSRect(x: 0, y: 0, width: size, height: CGFloat(height)))
  // A band across the lower part of the icon, painted only over the icon's
  // own pixels so it follows its rounded shape.
  let band = size * 0.2
  let bottom = size * 0.12
  NSGraphicsContext.current!.compositingOperation = .sourceAtop
  NSColor(calibratedRed: 1.0, green: 0.48, blue: 0.0, alpha: 1.0).setFill()
  NSRect(x: 0, y: bottom, width: size, height: band).fill(using: .sourceAtop)
  if size >= 32 {
    let font = NSFont.systemFont(ofSize: band * 0.7, weight: .heavy)
    let attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: NSColor.white]
    let text = NSAttributedString(string: label, attributes: attributes)
    let textSize = text.size()
    text.draw(at: NSPoint(x: (size - textSize.width) / 2, y: bottom + (band - textSize.height) / 2))
  }
  NSGraphicsContext.restoreGraphicsState()
  guard let png = bitmap.representation(using: .png, properties: [:]) else { continue }
  try png.write(to: file)
}
