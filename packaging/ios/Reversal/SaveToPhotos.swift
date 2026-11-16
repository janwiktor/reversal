// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Saves DNGs to the photo library as they are, as raw photos. The share sheet's own Save Image
// turns a file into an image before saving it, which for our DNGs is the small JPEG preview they
// carry, so the share sheet offers this in its place.

import Photos
import UIKit

final class SaveToPhotos: UIActivity {
    private var dngs: [URL] = []
    /// Says what went wrong, with a title and a message. Runs on the main queue.
    private let failed: (String, String) -> Void
    /// A roll of DNGs takes a while to copy, and the user may leave the app meanwhile.
    private var background = UIBackgroundTaskIdentifier.invalid

    init(failed: @escaping (String, String) -> Void) {
        self.failed = failed
        super.init()
    }

    override class var activityCategory: UIActivity.Category { .action }
    override var activityType: UIActivity.ActivityType? {
        UIActivity.ActivityType((Bundle.main.bundleIdentifier ?? "reversal") + ".save-to-photos")
    }
    override var activityTitle: String? { "Save to Photos" }
    override var activityImage: UIImage? { Self.icon }

    /// The share sheet scales an action's image up to its glyph size, so a symbol at its default
    /// size comes out soft. And the badge weighs on one corner, so a symbol centered by its box
    /// looks off center. This draws it large, on a square centered on where its ink is.
    private static let icon: UIImage? = {
        let config = UIImage.SymbolConfiguration(pointSize: 64, weight: .regular)
        guard let symbol = UIImage(systemName: "photo.badge.arrow.down", withConfiguration: config)?
            .withTintColor(.black, renderingMode: .alwaysOriginal)
        else { return nil }
        let format = UIGraphicsImageRendererFormat.preferred()
        format.scale = 1
        let glyph = UIGraphicsImageRenderer(size: symbol.size, format: format).image { _ in symbol.draw(at: .zero) }
        guard let ink = inkCenter(of: glyph) else { return nil }
        // The square reaches as far from the ink's center as the farthest edge of the symbol, and
        // a little farther: the sheet fills its glyph box with the square, and the symbol alone
        // would carry more ink than the sheet's own glyphs.
        let reach = max(ink.x, symbol.size.width - ink.x, ink.y, symbol.size.height - ink.y)
        let side = (reach * 2 * 1.18).rounded(.up)
        let icon = UIGraphicsImageRenderer(size: CGSize(width: side, height: side)).image { _ in
            symbol.draw(at: CGPoint(x: side / 2 - ink.x, y: side / 2 - ink.y))
        }
        return icon.withRenderingMode(.alwaysTemplate)
    }()

    /// The center of mass of an image's opaque pixels, in points from its top left.
    private static func inkCenter(of image: UIImage) -> CGPoint? {
        guard let cg = image.cgImage else { return nil }
        let width = cg.width, height = cg.height
        var alpha = [UInt8](repeating: 0, count: width * height)
        let drawn = alpha.withUnsafeMutableBytes { bytes in
            CGContext(
                data: bytes.baseAddress, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width,
                space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.alphaOnly.rawValue
            ).map { context in
                context.draw(cg, in: CGRect(x: 0, y: 0, width: width, height: height))
                return true
            } ?? false
        }
        guard drawn else { return nil }
        var mass = 0.0, x = 0.0, y = 0.0
        for row in 0..<height {
            for column in 0..<width {
                let a = Double(alpha[row * width + column])
                mass += a
                x += a * (Double(column) + 0.5)
                y += a * (Double(row) + 0.5)
            }
        }
        guard mass > 0 else { return nil }
        return CGPoint(x: x / mass / image.scale, y: y / mass / image.scale)
    }

    private static func dngs(in items: [Any]) -> [URL] {
        items.compactMap { $0 as? URL }.filter { $0.isFileURL && $0.pathExtension.lowercased() == "dng" }
    }

    override func canPerform(withActivityItems items: [Any]) -> Bool { !Self.dngs(in: items).isEmpty }
    override func prepare(withActivityItems items: [Any]) { dngs = Self.dngs(in: items) }

    override func perform() {
        background = UIApplication.shared.beginBackgroundTask { [weak self] in self?.endBackground() }
        PHPhotoLibrary.requestAuthorization(for: .addOnly) { [self] access in
            switch access {
            case .authorized: save(dngs, notSaved: [])
            case .restricted: finish("Reversal can't add to Photos", "Adding to Photos is restricted on this device")
            default: finish("Reversal can't add to Photos", "Allow it in Settings, under Privacy & Security, Photos")
            }
        }
    }

    /// One DNG at a time, so one that Photos refuses doesn't keep the others out, and the
    /// message can name it.
    private func save(_ left: [URL], notSaved: [String]) {
        guard let dng = left.first else {
            if notSaved.isEmpty { return finish(nil, nil) }
            let title = notSaved.count == dngs.count
                ? "Couldn't save to Photos" : "Couldn't save \(notSaved.count) of \(dngs.count) DNGs to Photos"
            return finish(title, notSaved.joined(separator: ", "))
        }
        PHPhotoLibrary.shared().performChanges {
            // The DNG goes in as the photo's original, so Photos keeps every pixel and shows it as raw.
            let options = PHAssetResourceCreationOptions()
            options.originalFilename = dng.lastPathComponent
            PHAssetCreationRequest.forAsset().addResource(with: .photo, fileURL: dng, options: options)
        } completionHandler: { saved, _ in
            self.save(Array(left.dropFirst()), notSaved: saved ? notSaved : notSaved + [dng.lastPathComponent])
        }
    }

    /// With no title, everything was saved.
    private func finish(_ title: String?, _ message: String?) {
        DispatchQueue.main.async {
            self.activityDidFinish(title == nil)
            if let title { self.failed(title, message ?? "") }
            self.endBackground()
        }
    }

    private func endBackground() {
        guard background != .invalid else { return }
        UIApplication.shared.endBackgroundTask(background)
        background = .invalid
    }
}
