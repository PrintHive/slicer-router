import AppKit
import ImageIO
import QuickLookThumbnailing

/// Draws a `.3mf` thumbnail: the preview the file carries, or a plain card with
/// its name, badged with the icon of the app that will open it.
@objc(ThumbnailProvider)
final class ThumbnailProvider: QLThumbnailProvider {
    override func provideThumbnail(
        for request: QLFileThumbnailRequest,
        _ handler: @escaping (QLThumbnailReply?, Error?) -> Void
    ) {
        let routed = Routed(file: request.fileURL, home: Self.home)
        let longestSide = max(request.maximumSize.width, request.maximumSize.height)
        let preview = routed.image.flatMap {
            decode($0, maxPixels: Int(longestSide * request.scale))
        }
        let badge = NSWorkspace.shared.icon(forFile: routed.openerApp ?? Self.routerApp)
        let name = request.fileURL.deletingPathExtension().lastPathComponent
        let size = canvasSize(for: preview, within: request.maximumSize)
        let reply = QLThumbnailReply(contextSize: size) {
            let canvas = CGRect(origin: .zero, size: size)
            if let preview {
                drawPreview(preview, in: canvas)
            } else {
                drawCard(named: name, in: canvas)
            }
            drawBadge(badge, in: canvas)
            return true
        }
        handler(reply, nil)
    }

    /// The Slicer Router app this extension ships in, from
    /// `Slicer Router.app/Contents/PlugIns/<extension>.appex`.
    private static let routerApp = Bundle.main.bundleURL
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .path

    /// The user's real home folder. In the sandbox `NSHomeDirectory()` is the
    /// extension's container instead.
    private static let home = getpwuid(getuid()).map { String(cString: $0.pointee.pw_dir) }
}

/// What the router decided for one file.
struct Routed {
    /// The `.app` whose icon is the badge, or nil when Slicer Router would ask
    /// which slicer to use. Also nil when that slicer was removed after the app
    /// last ran, because then the snapshot no longer says what opens the file.
    let openerApp: String?
    /// The preview image the file carries, PNG or JPEG.
    let image: Data?

    init(file: URL, home: String?) {
        let result = file.path.withCString { file in
            guard let home else { return sr_thumbnail(file, nil) }
            return home.withCString { sr_thumbnail(file, $0) }
        }
        defer { sr_thumbnail_free(result) }
        openerApp = result.opener_app
            .map { String(cString: $0) }
            .flatMap { FileManager.default.fileExists(atPath: $0) ? $0 : nil }
        image = result.image.map { Data(bytes: $0, count: result.image_len) }
    }
}

/// Decode `data` no larger than needed. An image whose header claims more than
/// 16 megapixels is refused before decoding, because a small compressed file can
/// expand to gigabytes and Finder thumbnails files straight from the internet.
func decode(_ data: Data, maxPixels: Int) -> CGImage? {
    guard let source = CGImageSourceCreateWithData(data as CFData, nil),
        let header = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
        let width = header[kCGImagePropertyPixelWidth] as? Int, (1...16_384).contains(width),
        let height = header[kCGImagePropertyPixelHeight] as? Int, (1...16_384).contains(height),
        width * height <= 4096 * 4096
    else { return nil }
    let options: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceThumbnailMaxPixelSize: max(maxPixels, 1),
    ]
    return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
}

/// The preview's shape scaled to fit `maximum`, or a square for the card.
func canvasSize(for preview: CGImage?, within maximum: CGSize) -> CGSize {
    guard let preview, preview.width > 0, preview.height > 0 else {
        let side = min(maximum.width, maximum.height)
        return CGSize(width: side, height: side)
    }
    let scale = min(
        maximum.width / CGFloat(preview.width),
        maximum.height / CGFloat(preview.height)
    )
    return CGSize(
        width: (CGFloat(preview.width) * scale).rounded(),
        height: (CGFloat(preview.height) * scale).rounded()
    )
}

func drawPreview(_ preview: CGImage, in canvas: CGRect) {
    guard let context = NSGraphicsContext.current?.cgContext else { return }
    context.interpolationQuality = .high
    context.draw(preview, in: canvas)
}

/// A neutral card with the file's name, for a file that carries no preview.
func drawCard(named name: String, in canvas: CGRect) {
    let side = min(canvas.width, canvas.height)
    let card = canvas.insetBy(dx: side * 0.06, dy: side * 0.06)
    let shape = NSBezierPath(roundedRect: card, xRadius: side * 0.08, yRadius: side * 0.08)
    NSColor(white: 0.96, alpha: 1).setFill()
    shape.fill()
    NSColor(white: 0.80, alpha: 1).setStroke()
    shape.lineWidth = max(1, side / 160)
    shape.stroke()

    let centered = NSMutableParagraphStyle()
    centered.alignment = .center
    centered.lineBreakMode = .byWordWrapping
    let label = NSAttributedString(
        string: "3MF",
        attributes: [
            .font: NSFont.systemFont(ofSize: side * 0.07, weight: .bold),
            .foregroundColor: NSColor(white: 0.55, alpha: 1),
            .paragraphStyle: centered,
        ]
    )
    let title = NSAttributedString(
        string: name,
        attributes: [
            .font: NSFont.systemFont(ofSize: side * 0.085, weight: .semibold),
            .foregroundColor: NSColor(white: 0.20, alpha: 1),
            .paragraphStyle: centered,
        ]
    )
    let inner = card.insetBy(dx: side * 0.08, dy: side * 0.08)
    let labelHeight = side * 0.1
    label.draw(
        with: CGRect(x: inner.minX, y: inner.maxY - labelHeight, width: inner.width, height: labelHeight),
        options: [.usesLineFragmentOrigin]
    )
    // The title is centered between the label and the bottom third, which the
    // badge covers.
    let free = CGRect(
        x: inner.minX,
        y: inner.minY + side * 0.3,
        width: inner.width,
        height: inner.height - side * 0.3 - labelHeight
    )
    let options: NSString.DrawingOptions = [.usesLineFragmentOrigin, .truncatesLastVisibleLine]
    let height = min(title.boundingRect(with: free.size, options: options).height, free.height)
    title.draw(
        with: CGRect(x: free.minX, y: free.midY - height / 2, width: free.width, height: height),
        options: options
    )
}

/// The opener's icon in the bottom-right corner, about a third of the canvas.
func drawBadge(_ icon: NSImage, in canvas: CGRect) {
    let side = min(canvas.width, canvas.height) * 0.35
    let frame = CGRect(x: canvas.maxX - side, y: canvas.minY, width: side, height: side)
    NSGraphicsContext.saveGraphicsState()
    let shadow = NSShadow()
    shadow.shadowColor = NSColor.black.withAlphaComponent(0.3)
    shadow.shadowBlurRadius = side * 0.05
    shadow.shadowOffset = NSSize(width: 0, height: -side * 0.02)
    shadow.set()
    icon.draw(in: frame, from: .zero, operation: .sourceOver, fraction: 1)
    NSGraphicsContext.restoreGraphicsState()
}
