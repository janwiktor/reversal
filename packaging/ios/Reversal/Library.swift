// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Where scans come from: folders used in place, and rolls imported into the app's Documents,
// which Files shows under On My iPad.

import Foundation
import PhotosUI
import UIKit
import UniformTypeIdentifiers

func isScan(_ name: String) -> Bool {
    name.withCString { reversal_is_scan($0) }
}

/// A count with its word: "1 scan", "36 scans".
func counted(_ n: Int, _ word: String) -> String {
    "\(n) \(word)\(n == 1 ? "" : "s")"
}

/// "iPad" or "iPhone", for messages that name the device.
var deviceName: String { UIDevice.current.userInterfaceIdiom == .pad ? "iPad" : "iPhone" }

/// A roll imported into the app.
struct Roll: Identifiable {
    let url: URL
    let frames: Int
    /// The DNGs saved from it, in its positives folder.
    let dngs: Int
    /// Everything in it: scans, DNGs and settings.
    let bytes: Int64
    var id: URL { url }
    var name: String { url.lastPathComponent }

    /// Such as "36 scans, 12 DNGs".
    var contents: String {
        let scans = counted(frames, "scan")
        return dngs == 0 ? scans : "\(scans), \(counted(dngs, "DNG"))"
    }

    static func size(_ bytes: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }

    /// The DNGs themselves, in name order.
    func dngFiles() -> [URL] { Roll.dngFiles(in: url) }

    static func dngFiles(in roll: URL) -> [URL] {
        let positives = roll.appendingPathComponent("positives", isDirectory: true)
        let files = (try? FileManager.default.contentsOfDirectory(at: positives, includingPropertiesForKeys: nil)) ?? []
        return files
            .filter { $0.pathExtension.lowercased() == "dng" }
            .sorted { $0.lastPathComponent.localizedStandardCompare($1.lastPathComponent) == .orderedAscending }
    }
}

enum Library {
    private static let lastFolderKey = "folder"

    static var documents: URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
    }

    static func rolls() -> [Roll] {
        let fm = FileManager.default
        let folders = (try? fm.contentsOfDirectory(
            at: documents, includingPropertiesForKeys: [.isDirectoryKey])) ?? []
        return folders
            .filter { (try? $0.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true }
            // Not the system's own: its inbox for files other apps send, and what Files hides.
            .filter { canName(roll: $0.lastPathComponent) }
            .map { folder in
                let names = (try? fm.contentsOfDirectory(atPath: folder.path)) ?? []
                var bytes: Int64 = 0
                for case let file as URL in fm.enumerator(at: folder, includingPropertiesForKeys: [.fileSizeKey]) ?? .init() {
                    bytes += Int64((try? file.resourceValues(forKeys: [.fileSizeKey]))?.fileSize ?? 0)
                }
                return Roll(
                    url: folder, frames: names.filter(isScan).count,
                    dngs: Roll.dngFiles(in: folder).count, bytes: bytes)
            }
            .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    /// A bookmark, not a path: it keeps access to a folder outside the app across launches, and
    /// the app's own container moves when it's updated.
    static func remember(folder: URL) {
        let bookmark = try? folder.bookmarkData(
            options: .minimalBookmark, includingResourceValuesForKeys: nil, relativeTo: nil)
        UserDefaults.standard.set(bookmark, forKey: lastFolderKey)
    }

    static func lastFolder() -> URL? {
        guard let bookmark = UserDefaults.standard.data(forKey: lastFolderKey) else { return nil }
        var stale = false
        return try? URL(resolvingBookmarkData: bookmark, bookmarkDataIsStale: &stale)
    }

    /// Where iOS puts copies of files that other apps send. The app may read and delete there.
    static var inbox: URL { documents.appendingPathComponent("Inbox", isDirectory: true) }

    static func isSystemFolder(_ name: String) -> Bool {
        name.caseInsensitiveCompare("Inbox") == .orderedSame
    }

    /// Whether a folder of that name can be a roll: the start screen lists it, and it isn't
    /// the inbox itself.
    static func canName(roll name: String) -> Bool {
        !name.isEmpty && !name.contains("/") && !name.hasPrefix(".") && !isSystemFolder(name)
    }

    /// The name for a roll of scans from a folder of that name, which a roll may not be able
    /// to take.
    static func rollName(from folder: String) -> String {
        canName(roll: folder) ? folder : "Imported \(today())"
    }

    static func isInInbox(_ url: URL) -> Bool { isInside(url, inbox) }

    /// Whether a file or folder lies somewhere in a folder, which itself doesn't count.
    private static func isInside(_ url: URL, _ folder: URL) -> Bool {
        url.resolvingSymlinksInPath().path.hasPrefix(folder.resolvingSymlinksInPath().path + "/")
    }

    /// The day here, not in Greenwich, to name a roll of things that came without a folder.
    static func today() -> String {
        Date().formatted(Date.ISO8601FormatStyle(timeZone: .current).year().month().day())
    }

    /// A name a roll can take: one folder's worth, and not another roll's.
    static func canRename(_ roll: Roll, to name: String) -> Bool {
        let name = name.trimmingCharacters(in: .whitespaces)
        guard canName(roll: name), name != roll.name else { return false }
        return !FileManager.default.fileExists(atPath: documents.appendingPathComponent(name).path)
    }

    static func rename(_ roll: Roll, to name: String) throws -> URL {
        let target = documents.appendingPathComponent(name.trimmingCharacters(in: .whitespaces), isDirectory: true)
        try FileManager.default.moveItem(at: roll.url, to: target)
        return target
    }

    static func delete(_ roll: Roll) throws {
        try FileManager.default.removeItem(at: roll.url)
    }

    /// The app's own folder of rolls counts, as do the rolls in it.
    static func isInApp(_ url: URL) -> Bool {
        url.resolvingSymlinksInPath() == documents.resolvingSymlinksInPath() || isInside(url, documents)
    }
}

/// Copies scans from files or folders, also on a memory card or in iCloud, into a roll in the
/// app. The sources are only read. Importing again adds only what isn't there yet.
final class Importer {
    struct Result {
        let roll: URL
        let copied: Int
        let skipped: Int
        let failed: [String]
        let canceled: Bool
    }

    private let lock = NSLock()
    private var canceled = false

    func cancel() {
        lock.withLock { canceled = true }
    }

    private var isCanceled: Bool { lock.withLock { canceled } }

    /// `progress` gets (files finished, files in all); both closures run on the main queue.
    func run(
        sources: [URL], rollName: String,
        progress: @escaping (Int, Int) -> Void, done: @escaping (Result) -> Void
    ) {
        DispatchQueue.global(qos: .userInitiated).async {
            let scoped = sources.filter { $0.startAccessingSecurityScopedResource() }
            defer { scoped.forEach { $0.stopAccessingSecurityScopedResource() } }
            let roll = Library.documents.appendingPathComponent(
                Library.rollName(from: rollName), isDirectory: true)
            var (copied, skipped) = (0, 0)
            var failed: [String] = []
            // A copy another app sent is ours to clear away, whether or not it reached a roll:
            // nothing lists the inbox, so what stayed there would take room unseen.
            defer {
                for source in sources where Library.isInInbox(source) {
                    try? FileManager.default.removeItem(at: source)
                }
            }
            do {
                let files = sources.flatMap(Self.scans)
                // No roll for nothing to put in it.
                if !files.isEmpty {
                    try FileManager.default.createDirectory(at: roll, withIntermediateDirectories: true)
                }
                DispatchQueue.main.async { progress(0, files.count) }
                for (index, file) in files.enumerated() where !self.isCanceled {
                    do {
                        try Self.copy(file, into: roll) ? (copied += 1) : (skipped += 1)
                    } catch {
                        failed.append("\(file.lastPathComponent): \(error.localizedDescription)")
                    }
                    DispatchQueue.main.async { progress(index + 1, files.count) }
                }
            } catch {
                failed.append(error.localizedDescription)
            }
            let result = Result(
                roll: roll, copied: copied, skipped: skipped, failed: failed, canceled: self.isCanceled)
            DispatchQueue.main.async { done(result) }
        }
    }

    /// A file, or the scans anywhere in a folder: cards keep them in folders inside DCIM. A
    /// positives folder holds results, not scans.
    private static func scans(in source: URL) -> [URL] {
        let keys: [URLResourceKey] = [.isDirectoryKey]
        guard (try? source.resourceValues(forKeys: Set(keys)))?.isDirectory == true else {
            // A file on its own has to be a scan too: a saved positive, say, isn't one.
            return isScan(source.lastPathComponent) ? [source] : []
        }
        guard let walk = FileManager.default.enumerator(at: source, includingPropertiesForKeys: keys)
        else { return [] }
        var found: [URL] = []
        for case let url as URL in walk {
            if (try? url.resourceValues(forKeys: Set(keys)))?.isDirectory == true {
                if url.lastPathComponent == "positives" { walk.skipDescendants() }
            } else if isScan(url.lastPathComponent) {
                found.append(url)
            }
        }
        return found.sorted { $0.path.localizedStandardCompare($1.path) == .orderedAscending }
    }

    /// Returns whether the file was copied, rather than being there already. Coordinated, so a
    /// file that's only in iCloud gets downloaded first.
    private static func copy(_ file: URL, into roll: URL) throws -> Bool {
        var coordination: NSError?
        var outcome: Swift.Result<Bool, Error> = .success(false)
        NSFileCoordinator().coordinate(
            readingItemAt: file, options: .withoutChanges, error: &coordination
        ) { readable in
            outcome = Swift.Result { try copyCoordinated(readable, named: file.lastPathComponent, into: roll) }
        }
        if let coordination { throw coordination }
        return try outcome.get()
    }

    static func copyCoordinated(_ file: URL, named name: String, into roll: URL) throws -> Bool {
        let fm = FileManager.default
        let size = { (url: URL) in try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize }
        // Same name and content: already here. Same name only: another frame, as on two cards
        // or in two of a card's folders. Uncompressed raws all have one size, so the size alone
        // can't tell, and a frame taken for another would be left on a card that gets wiped.
        var target = roll.appendingPathComponent(name)
        var number = 1
        while fm.fileExists(atPath: target.path) {
            if size(target) == size(file), fm.contentsEqual(atPath: target.path, andPath: file.path) { return false }
            number += 1
            let stem = (name as NSString).deletingPathExtension
            let ext = (name as NSString).pathExtension
            target = roll.appendingPathComponent("\(stem)-\(number).\(ext)")
        }
        // An interrupted copy mustn't look like a scan.
        let partial = roll.appendingPathComponent(".\(target.lastPathComponent).partial")
        try? fm.removeItem(at: partial)
        try fm.copyItem(at: file, to: partial)
        try fm.moveItem(at: partial, to: target)
        return true
    }
}

/// A raw file of a kind the engine doesn't list as a scan.
struct NotReadable: LocalizedError {
    var errorDescription: String? { "Reversal doesn't read this raw format" }
}

extension Notification.Name {
    /// The rolls in the app changed while the start screen may be showing them.
    static let rollsChanged = Notification.Name("rollsChanged")
}

/// Copies raw photos chosen from the photo library into a roll in the app. The system's picker
/// runs outside the app and hands over only what was chosen, so this needs no access to the
/// library. Photos that aren't raw are left out: a JPEG or HEIC of a negative has lost what the
/// conversion measures.
final class PhotosImporter {
    private let lock = NSLock()
    private var canceled = false
    /// The photo on its way, which may be coming down from iCloud.
    private var loading: Progress?

    func cancel() {
        lock.withLock { canceled = true }
        loading?.cancel()
    }

    private var isCanceled: Bool { lock.withLock { canceled } }

    /// The type to ask a photo for: its raw file, if it has one.
    private static func rawType(of photo: NSItemProvider) -> String? {
        photo.registeredTypeIdentifiers.first { UTType($0)?.conforms(to: .rawImage) == true }
    }

    /// `progress` gets (photos finished, photos in all); both closures run on the main queue.
    /// `notRaw` in the result counts the photos left out.
    func run(
        photos: [NSItemProvider], rollName: String,
        progress: @escaping (Int, Int) -> Void, done: @escaping (Importer.Result, _ notRaw: Int) -> Void
    ) {
        let roll = Library.documents.appendingPathComponent(rollName, isDirectory: true)
        let raws = photos.compactMap { photo in Self.rawType(of: photo).map { (photo, $0) } }
        let notRaw = photos.count - raws.count
        var (copied, skipped) = (0, 0)
        var failed: [String] = []
        let finish = {
            let result = Importer.Result(
                roll: roll, copied: copied, skipped: skipped, failed: failed, canceled: self.isCanceled)
            done(result, notRaw)
        }
        do {
            if !raws.isEmpty {
                try FileManager.default.createDirectory(at: roll, withIntermediateDirectories: true)
            }
        } catch {
            failed.append(error.localizedDescription)
            return finish()
        }
        progress(0, raws.count)
        // One at a time: each is tens of megabytes, and may come down from iCloud first.
        func next(_ index: Int) {
            guard index < raws.count, !isCanceled else { return finish() }
            let (photo, type) = raws[index]
            self.loading = photo.loadFileRepresentation(forTypeIdentifier: type) { file, error in
                // The file is the picker's copy, and is gone when this returns.
                var outcome: Result<Bool, Error> = .failure(error ?? CocoaError(.fileReadUnknown))
                if let file, !isScan(file.lastPathComponent) {
                    outcome = .failure(NotReadable())
                } else if let file {
                    outcome = Result { try Importer.copyCoordinated(file, named: file.lastPathComponent, into: roll) }
                }
                DispatchQueue.main.async {
                    switch outcome {
                    case .success(true): copied += 1
                    case .success(false): skipped += 1
                    case .failure(let error):
                        failed.append("\(photo.suggestedName ?? "A photo"): \(error.localizedDescription)")
                    }
                    progress(index + 1, raws.count)
                    next(index + 1)
                }
            }
        }
        next(0)
    }
}
