import AppKit
import CoreText
import Foundation
import PDFKit
import Vision

// The assistant's documents (`assistant::media::document`): a PDF's text page
// by page, with Vision's OCR for pages that carry no text (scans), its pages
// as PNG images, OCR of any image, and a PDF laid out from structured blocks
// with Core Text. One entry point takes a JSON request and returns JSON; no
// call shows UI or needs the main thread.

private func reply(_ value: Any) -> UnsafeMutablePointer<CChar>? {
    guard JSONSerialization.isValidJSONObject(value),
          let data = try? JSONSerialization.data(withJSONObject: value)
    else { return strdup("{\"error\":\"unencodable reply\"}") }
    return strdup(String(decoding: data, as: UTF8.self))
}

private func failure(_ message: String) -> UnsafeMutablePointer<CChar>? {
    reply(["error": message])
}

@_cdecl("hex_documents_free")
public func hexDocumentsFree(_ string: UnsafeMutablePointer<CChar>?) {
    free(string)
}

@_cdecl("hex_documents_run")
public func hexDocumentsRun(_ request: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? {
    guard let request,
          let data = String(cString: request).data(using: .utf8),
          let object = try? JSONSerialization.jsonObject(with: data),
          let json = object as? [String: Any],
          let op = json["op"] as? String
    else { return failure("unreadable request") }
    switch op {
    case "pdf_read": return pdfRead(json)
    case "ocr": return ocrImage(json)
    case "pdf_write": return pdfWrite(json)
    default: return failure("unknown op \(op)")
    }
}

// MARK: Reading

/// Text Vision recognizes in `image`, line by line (top to bottom), with each
/// line's box in the image's fractions from the top left.
private func recognize(_ image: CGImage) -> [(String, Float, CGRect)] {
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = true
    if #available(macOS 13.0, *) {
        request.automaticallyDetectsLanguage = true
    }
    let handler = VNImageRequestHandler(cgImage: image, options: [:])
    do { try handler.perform([request]) } catch { return [] }
    let found = (request.results ?? []).compactMap { observation -> (String, Float, CGRect)? in
        guard let best = observation.topCandidates(1).first else { return nil }
        let box = observation.boundingBox
        let flipped = CGRect(x: box.minX, y: 1 - box.maxY, width: box.width, height: box.height)
        return (best.string, best.confidence, flipped)
    }
    return found.sorted { a, b in
        abs(a.2.minY - b.2.minY) > 0.01 ? a.2.minY < b.2.minY : a.2.minX < b.2.minX
    }
}

/// `page` drawn white-backed at `width` pixels wide.
private func render(_ page: PDFPage, width: Int) -> CGImage? {
    let bounds = page.bounds(for: .mediaBox)
    guard bounds.width > 0, bounds.height > 0 else { return nil }
    let scale = CGFloat(width) / bounds.width
    let height = Int((bounds.height * scale).rounded())
    guard let context = CGContext(
        data: nil, width: width, height: max(height, 1), bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else { return nil }
    context.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: width, height: height))
    context.scaleBy(x: scale, y: scale)
    context.translateBy(x: -bounds.minX, y: -bounds.minY)
    page.draw(with: .mediaBox, to: context)
    return context.makeImage()
}

private func writePNG(_ image: CGImage, to path: String) -> Bool {
    let url = URL(fileURLWithPath: path) as CFURL
    guard let destination = CGImageDestinationCreateWithURL(url, "public.png" as CFString, 1, nil)
    else { return false }
    CGImageDestinationAddImage(destination, image, nil)
    return CGImageDestinationFinalize(destination)
}

private func pdfRead(_ json: [String: Any]) -> UnsafeMutablePointer<CChar>? {
    guard let path = json["path"] as? String,
          let document = PDFDocument(url: URL(fileURLWithPath: path))
    else { return failure("not a readable PDF") }
    if document.isLocked { return failure("the PDF is locked with a password") }
    let count = document.pageCount
    let first = max(1, json["first"] as? Int ?? 1)
    let last = min(count, json["last"] as? Int ?? count)
    let ocr = json["ocr"] as? String ?? "auto"
    let imagesDir = json["images_dir"] as? String
    let imagePages = Set(json["image_pages"] as? [Int] ?? [])
    let width = json["image_width"] as? Int ?? 1200
    let maxChars = json["max_chars"] as? Int ?? 200_000
    let deadline = Date().addingTimeInterval(json["seconds"] as? Double ?? 90)
    var pages: [[String: Any]] = []
    var images: [String] = []
    var total = 0
    var cut = false
    if first <= last {
        for index in (first - 1)..<last {
            guard let page = document.page(at: index) else { continue }
            if total >= maxChars || Date() > deadline { cut = true; break }
            var text = page.string ?? ""
            var recognized = false
            let bare = text.trimmingCharacters(in: .whitespacesAndNewlines).count < 16
            if ocr == "always" || (ocr == "auto" && bare) {
                if let image = render(page, width: 2000) {
                    let lines = recognize(image).map { $0.0 }
                    if !lines.isEmpty || ocr == "always" {
                        text = lines.joined(separator: "\n")
                        recognized = true
                    }
                }
            }
            total += text.count
            pages.append(["n": index + 1, "text": text, "ocr": recognized])
            if let imagesDir, imagePages.contains(index + 1), let image = render(page, width: width) {
                let file = (imagesDir as NSString).appendingPathComponent("page-\(index + 1).png")
                if writePNG(image, to: file) { images.append(file) }
            }
        }
    }
    let attributes = document.documentAttributes ?? [:]
    return reply([
        "page_count": count,
        "title": attributes[PDFDocumentAttribute.titleAttribute] as? String ?? "",
        "author": attributes[PDFDocumentAttribute.authorAttribute] as? String ?? "",
        "pages": pages,
        "images": images,
        "cut": cut,
    ])
}

private func ocrImage(_ json: [String: Any]) -> UnsafeMutablePointer<CChar>? {
    guard let path = json["path"] as? String,
          let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
    else { return failure("not a readable image") }
    let lines = recognize(image).map { line -> [String: Any] in
        let box = line.2
        return [
            "text": line.0,
            "confidence": Double(line.1),
            "box": [box.minX, box.minY, box.width, box.height].map { Double($0) },
        ]
    }
    return reply([
        "width": image.width, "height": image.height,
        "text": lines.map { $0["text"] as? String ?? "" }.joined(separator: "\n"),
        "lines": lines,
    ])
}

// MARK: Writing

/// Lays blocks out on pages, top to bottom, starting a page when one is full.
private final class Layout {
    let context: CGContext
    let page: CGRect
    let margin: CGFloat = 56
    var y: CGFloat = 0
    var pages = 0
    var open = false

    init(context: CGContext, page: CGRect) {
        self.context = context
        self.page = page
    }

    var width: CGFloat { page.width - 2 * margin }
    var bottom: CGFloat { margin }

    func newPage() {
        if open { context.endPDFPage() }
        context.beginPDFPage(nil)
        open = true
        pages += 1
        y = page.height - margin
    }

    func ensure(_ height: CGFloat) {
        if !open || (y - height < bottom && y < page.height - margin - 1) { newPage() }
    }

    func finish() {
        if !open { newPage() }
        context.endPDFPage()
        context.closePDF()
    }

    /// `text` from the cursor, flowing onto new pages as needed.
    func flow(_ text: NSAttributedString, indent: CGFloat = 0, after: CGFloat = 6) {
        guard text.length > 0 else { return }
        let framesetter = CTFramesetterCreateWithAttributedString(text)
        var start = 0
        while start < text.length {
            ensure(14)
            let rect = CGRect(x: margin + indent, y: bottom, width: width - indent, height: y - bottom)
            let path = CGPath(rect: rect, transform: nil)
            let frame = CTFramesetterCreateFrame(framesetter, CFRange(location: start, length: 0), path, nil)
            let visible = CTFrameGetVisibleStringRange(frame)
            if visible.length == 0 { newPage(); continue }
            CTFrameDraw(frame, context)
            let size = CTFramesetterSuggestFrameSizeWithConstraints(
                framesetter, visible, nil, CGSize(width: rect.width, height: .greatestFiniteMagnitude), nil)
            start = visible.location + visible.length
            if start < text.length {
                newPage()
            } else {
                y -= size.height + after
            }
        }
    }

    func height(of text: NSAttributedString, width: CGFloat) -> CGFloat {
        let framesetter = CTFramesetterCreateWithAttributedString(text)
        return CTFramesetterSuggestFrameSizeWithConstraints(
            framesetter, CFRange(location: 0, length: 0), nil,
            CGSize(width: width, height: .greatestFiniteMagnitude), nil).height
    }

    func draw(_ text: NSAttributedString, in rect: CGRect) {
        let framesetter = CTFramesetterCreateWithAttributedString(text)
        let frame = CTFramesetterCreateFrame(
            framesetter, CFRange(location: 0, length: 0), CGPath(rect: rect, transform: nil), nil)
        CTFrameDraw(frame, context)
    }

    func table(_ rows: [[String]], header: Bool) {
        let columns = rows.map(\.count).max() ?? 0
        guard columns > 0 else { return }
        // Columns share the width by their longest text, each at least a little.
        var weights = [CGFloat](repeating: 1, count: columns)
        for row in rows {
            for (index, cell) in row.enumerated() {
                weights[index] = max(weights[index], min(CGFloat(cell.count), 40))
            }
        }
        let sum = weights.reduce(0, +)
        let widths = weights.map { max(width * $0 / sum, 36) }
        let scale = width / widths.reduce(0, +)
        let columnWidths = widths.map { $0 * scale }
        let pad: CGFloat = 4
        for (rowIndex, row) in rows.enumerated() {
            let bold = header && rowIndex == 0
            let texts = (0..<columns).map { index in
                styled(index < row.count ? row[index] : "", size: 9.5, bold: bold)
            }
            let rowHeight = zip(texts, columnWidths).map { height(of: $0, width: $1 - 2 * pad) }.max() ?? 0
            let total = rowHeight + 2 * pad
            ensure(total)
            var x = margin
            for (text, columnWidth) in zip(texts, columnWidths) {
                let cell = CGRect(x: x, y: y - total, width: columnWidth, height: total)
                if bold {
                    context.setFillColor(CGColor(gray: 0.9, alpha: 1))
                    context.fill(cell)
                }
                context.setStrokeColor(CGColor(gray: 0.55, alpha: 1))
                context.setLineWidth(0.5)
                context.stroke(cell)
                draw(text, in: cell.insetBy(dx: pad, dy: pad))
                x += columnWidth
            }
            y -= total
        }
        y -= 8
    }

    func image(_ path: String, width wanted: CGFloat?) {
        guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
              let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
        else {
            flow(styled("[image not found: \(path)]", size: 10, bold: false))
            return
        }
        var drawnWidth = min(wanted ?? width, width)
        var drawnHeight = drawnWidth * CGFloat(image.height) / CGFloat(max(image.width, 1))
        let room = page.height - 2 * margin
        if drawnHeight > room {
            drawnWidth *= room / drawnHeight
            drawnHeight = room
        }
        ensure(drawnHeight)
        context.draw(image, in: CGRect(x: margin, y: y - drawnHeight, width: drawnWidth, height: drawnHeight))
        y -= drawnHeight + 10
    }
}

private func styled(_ text: String, size: CGFloat, bold: Bool, color: CGColor? = nil) -> NSAttributedString {
    let font = bold ? NSFont.boldSystemFont(ofSize: size) : NSFont.systemFont(ofSize: size)
    let paragraph = NSMutableParagraphStyle()
    paragraph.lineSpacing = size * 0.2
    var attributes: [NSAttributedString.Key: Any] = [.font: font, .paragraphStyle: paragraph]
    attributes[.foregroundColor] = NSColor(cgColor: color ?? CGColor(gray: 0.08, alpha: 1)) ?? NSColor.black
    return NSAttributedString(string: text, attributes: attributes)
}

private func pdfWrite(_ json: [String: Any]) -> UnsafeMutablePointer<CChar>? {
    guard let out = json["out"] as? String else { return failure("no output path") }
    let letter = (json["page"] as? String) == "letter"
    var box = letter ? CGRect(x: 0, y: 0, width: 612, height: 792) : CGRect(x: 0, y: 0, width: 595, height: 842)
    if (json["landscape"] as? Bool) == true { box = CGRect(x: 0, y: 0, width: box.height, height: box.width) }
    var info: [CFString: Any] = [kCGPDFContextCreator: "Agent-Desk"]
    if let title = json["title"] as? String, !title.isEmpty { info[kCGPDFContextTitle] = title }
    guard let context = CGContext(URL(fileURLWithPath: out) as CFURL, mediaBox: &box, info as CFDictionary)
    else { return failure("couldn't create \(out)") }
    let layout = Layout(context: context, page: box)
    layout.newPage()
    if let title = json["title"] as? String, !title.isEmpty {
        layout.flow(styled(title, size: 22, bold: true), after: 12)
    }
    for block in json["blocks"] as? [[String: Any]] ?? [] {
        let text = block["text"] as? String ?? ""
        switch block["type"] as? String ?? "paragraph" {
        case "heading":
            let level = max(1, min(block["level"] as? Int ?? 1, 4))
            let size: CGFloat = [18, 15, 13, 12][level - 1]
            layout.ensure(size * 3)
            layout.y -= 6
            layout.flow(styled(text, size: size, bold: true), after: 6)
        case "bullets":
            let ordered = block["ordered"] as? Bool ?? false
            for (index, item) in (block["items"] as? [String] ?? []).enumerated() {
                let mark = ordered ? "\(index + 1)." : "•"
                layout.flow(styled("\(mark)\t\(item)", size: 11, bold: false), indent: 12, after: 3)
            }
            layout.y -= 4
        case "table":
            let rows = (block["rows"] as? [[Any]] ?? []).map { $0.map { "\($0)" } }
            layout.table(rows, header: block["header"] as? Bool ?? true)
        case "image":
            layout.image(block["path"] as? String ?? "", width: (block["width"] as? Double).map { CGFloat($0) })
        case "page_break":
            layout.newPage()
        case "quote":
            layout.flow(styled(text, size: 11, bold: false, color: CGColor(gray: 0.35, alpha: 1)), indent: 18)
        case "code":
            let code = NSAttributedString(string: text, attributes: [
                .font: NSFont.monospacedSystemFont(ofSize: 9.5, weight: .regular),
            ])
            layout.flow(code, indent: 8)
        default:
            layout.flow(styled(text, size: 11, bold: false))
        }
    }
    layout.finish()
    return reply(["pages": layout.pages, "out": out])
}
