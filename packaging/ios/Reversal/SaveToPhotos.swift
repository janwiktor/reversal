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
    override var activityImage: UIImage? { UIImage(systemName: "photo.badge.arrow.down") }

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
