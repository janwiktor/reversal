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

    init(failed: @escaping (String, String) -> Void) {
        self.failed = failed
        super.init()
    }

    override class var activityCategory: UIActivity.Category { .action }
    override var activityType: UIActivity.ActivityType? {
        UIActivity.ActivityType("io.github.janwiktor.reversal.save-to-photos")
    }
    override var activityTitle: String? { "Save to Photos" }
    override var activityImage: UIImage? { UIImage(systemName: "photo.badge.arrow.down") }

    override func canPerform(withActivityItems items: [Any]) -> Bool {
        items.contains { ($0 as? URL)?.isFileURL == true }
    }

    override func prepare(withActivityItems items: [Any]) {
        dngs = items.compactMap { $0 as? URL }.filter(\.isFileURL)
    }

    override func perform() {
        PHPhotoLibrary.requestAuthorization(for: .addOnly) { [self] access in
            guard access == .authorized || access == .limited else {
                return finish(false, "Reversal can't add to Photos", "Allow it in Settings, under Privacy & Security, Photos")
            }
            PHPhotoLibrary.shared().performChanges {
                // The DNG goes in as the photo's original, so Photos keeps every pixel and shows it as raw.
                for dng in self.dngs {
                    let options = PHAssetResourceCreationOptions()
                    options.originalFilename = dng.lastPathComponent
                    PHAssetCreationRequest.forAsset().addResource(with: .photo, fileURL: dng, options: options)
                }
            } completionHandler: { saved, error in
                let what = self.dngs.count == 1 ? "the DNG" : "the DNGs"
                self.finish(saved, "Couldn't save \(what) to Photos", error?.localizedDescription ?? "")
            }
        }
    }

    private func finish(_ saved: Bool, _ title: String, _ message: String) {
        DispatchQueue.main.async {
            self.activityDidFinish(saved)
            if !saved { self.failed(title, message) }
        }
    }
}
