// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The window around the page: opens a folder in the engine, and answers what the page asks of
// its shell. The page is the same one the Mac app and a browser get.

import Photos
import SwiftUI
import UIKit
import UniformTypeIdentifiers
import WebKit

final class ShellViewController: UIViewController, WKScriptMessageHandlerWithReply, WKNavigationDelegate,
    UIDocumentPickerDelegate
{
    private enum Pick {
        case openFolder, importFolder, importFiles, saveToFiles
    }

    private var web: WKWebView!
    private let engine = Engine()
    private var folder: URL?
    /// The folder outside the app that we hold access to.
    private var scoped: URL?
    private var pick: Pick?
    private var backgroundWork = UIBackgroundTaskIdentifier.invalid
    /// The page says when it has work running in the engine.
    private var busy = false {
        didSet {
            guard busy != oldValue else { return }
            UIApplication.shared.isIdleTimerDisabled = busy
            if busy {
                backgroundWork = UIApplication.shared.beginBackgroundTask { [weak self] in self?.endBackgroundWork() }
            } else {
                endBackgroundWork()
            }
        }
    }

    override var preferredStatusBarStyle: UIStatusBarStyle { .lightContent }

    override func viewDidLoad() {
        super.viewDidLoad()
        // The page's --surround, so nothing flashes white or shows around it.
        view.backgroundColor = UIColor(white: 0.15, alpha: 1)

        let config = WKWebViewConfiguration()
        config.userContentController.addScriptMessageHandler(self, contentWorld: .page, name: "reversal")
        config.setURLSchemeHandler(EngineSchemeHandler(engine: engine), forURLScheme: Engine.scheme)
        config.defaultWebpagePreferences.preferredContentMode = .desktop
        web = WKWebView(frame: .zero, configuration: config)
        web.navigationDelegate = self
        web.isOpaque = false
        web.backgroundColor = view.backgroundColor
        web.scrollView.isScrollEnabled = false
        web.scrollView.bounces = false
        web.scrollView.contentInsetAdjustmentBehavior = .never
        web.allowsLinkPreview = false
        #if DEBUG
            web.isInspectable = true
        #endif

        web.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(web)
        NSLayoutConstraint.activate([
            web.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            web.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor),
            web.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            web.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
        ])
    }

    private var started = false

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        guard !started else { return }
        started = true
        // A card that's gone by now is no news: just ask what to open.
        guard let last = Library.lastFolder() else { return showHome() }
        show(folder: last) { [weak self] opened in
            if !opened { self?.showHome() }
        }
    }

    // MARK: Engine

    /// The engine holds one folder at a time. `done` hears whether this one opened; if it
    /// didn't, none is open.
    private func show(folder url: URL, done: @escaping (Bool) -> Void = { _ in }) {
        let access = url.startAccessingSecurityScopedResource()
        busy = false
        engine.open(url) { [weak self] opened in
            guard let self else { return }
            // The engine used the folder that was open until now.
            self.scoped?.stopAccessingSecurityScopedResource()
            self.scoped = nil
            if opened {
                self.scoped = access ? url : nil
                self.folder = url
                Library.remember(folder: url)
                self.web.load(URLRequest(url: Engine.page))
            } else {
                if access { url.stopAccessingSecurityScopedResource() }
                self.folder = nil
            }
            done(opened)
        }
    }

    /// The page stays in the app: nothing it shows leads anywhere else.
    func webView(
        _: WKWebView, decidePolicyFor action: WKNavigationAction,
        decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
    ) {
        decisionHandler(action.request.url?.scheme == Engine.scheme ? .allow : .cancel)
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        webView.reload()
    }

    func rememberEdits(then proceed: @escaping () -> Void = {}) {
        guard folder != nil else { return proceed() }
        var finished = false
        let finish = {
            guard !finished else { return }
            finished = true
            proceed()
        }
        web.callAsyncJavaScript(
            "await rememberSettings(true)", arguments: [:], in: nil, in: .page,
            completionHandler: { _ in finish() })
        // A page that doesn't answer mustn't keep the app from moving on.
        DispatchQueue.main.asyncAfter(deadline: .now() + 2, execute: finish)
    }

    /// Before the engine stops: ask if it has work running, then let the page store its edits.
    private func leaveFolder(then proceed: @escaping () -> Void) {
        guard busy else { return rememberEdits(then: proceed) }
        let warning = UIAlertController(
            title: "Reversal is still working",
            message: "Stopping now leaves the frames in progress unfinished.", preferredStyle: .alert)
        warning.addAction(UIAlertAction(title: "Keep Working", style: .cancel))
        warning.addAction(UIAlertAction(title: "Stop", style: .destructive) { [weak self] _ in
            self?.rememberEdits(then: proceed)
        })
        top.present(warning, animated: true)
    }

    private func endBackgroundWork() {
        guard backgroundWork != .invalid else { return }
        UIApplication.shared.endBackgroundTask(backgroundWork)
        backgroundWork = .invalid
    }

    // MARK: The page's requests

    /// What the page learns when it asks what this shell does differently: see `shell` in the
    /// page's `app.js`. The page stays the same everywhere, and adapts itself to this.
    private static let capabilities: [String: Any] = ["export": true]

    /// Requests from the page, which is served by our own engine, in this process. Every one
    /// gets an answer, since the page holds a promise for it.
    func userContentController(
        _: WKUserContentController, didReceive message: WKScriptMessage,
        replyHandler: @escaping (Any?, String?) -> Void
    ) {
        let body = message.body as? [String: Any] ?? [:]
        switch body["action"] as? String {
        case "capabilities": return replyHandler(Self.capabilities, nil)
        case "chooseFolder": showHome()
        case "busy": busy = body["busy"] as? Bool ?? false
        case "reveal": showExport()
        default: break
        }
        replyHandler(nil, nil)
    }

    // MARK: Getting scans in

    private var top: UIViewController {
        var controller: UIViewController = self
        while let next = controller.presentedViewController, !next.isBeingDismissed { controller = next }
        return controller
    }

    private func showHome() {
        let home = UIHostingController(rootView: HomeView(
            rolls: Library.rolls(),
            lastFolder: Library.lastFolder(),
            canClose: folder != nil,
            openFolder: { [weak self] in self?.choose(.openFolder, [.folder]) },
            importFolder: { [weak self] in self?.choose(.importFolder, [.folder]) },
            importFiles: { [weak self] in self?.choose(.importFiles, [.rawImage], many: true) },
            open: { [weak self] url in self?.switchTo(url) },
            close: { [weak self] in self?.dismiss(animated: true) }))
        home.modalPresentationStyle = .formSheet
        home.isModalInPresentation = folder == nil
        top.present(home, animated: true)
    }

    private func choose(_ what: Pick, _ types: [UTType], many: Bool = false) {
        pick = what
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: types, asCopy: false)
        picker.allowsMultipleSelection = many
        picker.delegate = self
        top.present(picker, animated: true)
    }

    func documentPicker(_: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        guard let first = urls.first else { return }
        switch pick {
        case .openFolder: switchTo(first)
        case .importFolder: runImport(urls, rollName: first.lastPathComponent)
        case .importFiles:
            let parent = first.deletingLastPathComponent().lastPathComponent
            runImport(urls, rollName: parent)
        case .saveToFiles, nil: break
        }
    }

    private func switchTo(_ url: URL) {
        leaveFolder { [weak self] in
            guard let self else { return }
            self.dismissAll {
                self.show(folder: url) { opened in
                    guard !opened else { return }
                    self.showHome()
                    self.top.present(self.alert("Could not open \(url.lastPathComponent)", "Check that it's still connected."), animated: true)
                }
            }
        }
    }

    private func dismissAll(then proceed: @escaping () -> Void) {
        if presentedViewController == nil { return proceed() }
        dismiss(animated: true, completion: proceed)
    }

    private func runImport(_ sources: [URL], rollName: String) {
        let importer = Importer()
        let panel = UIAlertController(title: "Importing", message: "Finding scans…", preferredStyle: .alert)
        panel.addAction(UIAlertAction(title: "Stop", style: .cancel) { _ in importer.cancel() })
        top.present(panel, animated: true)
        importer.run(
            sources: sources, rollName: rollName,
            progress: { finished, total in panel.message = "\(finished) of \(total)" },
            done: { [weak self] result in
                guard let self else { return }
                let finish = {
                    if !result.failed.isEmpty {
                        let list = result.failed.joined(separator: "\n")
                        self.top.present(self.alert("Could not import \(result.failed.count) files", list), animated: true)
                    } else if result.copied + result.skipped == 0 {
                        self.top.present(self.alert("No scans there", "Choose a folder or files with NEF, CR2, ARW, RAF or DNG scans."), animated: true)
                    } else {
                        self.switchTo(result.roll)
                    }
                }
                if panel.presentingViewController != nil {
                    panel.dismiss(animated: true, completion: finish)
                } else {
                    finish()
                }
            })
    }

    // MARK: Getting DNGs out

    /// The DNGs of the last save, from the engine.
    private func savedDngs(_ done: @escaping ([URL]) -> Void) {
        engine.request("GET", "/api/export/status") { response in
            let paths = response.json?["saved_paths"] as? [String] ?? []
            done(paths.map { URL(fileURLWithPath: $0) })
        }
    }

    private func showExport() {
        savedDngs { [weak self] dngs in
            guard let self, !dngs.isEmpty else { return }
            let title = dngs.count == 1 ? dngs[0].lastPathComponent : "\(dngs.count) DNGs"
            let sheet = UIAlertController(title: title, message: nil, preferredStyle: .actionSheet)
            sheet.addAction(UIAlertAction(title: "Open in Another App…", style: .default) { _ in self.share(dngs) })
            sheet.addAction(UIAlertAction(title: "Save to Photos", style: .default) { _ in self.saveToPhotos(dngs) })
            sheet.addAction(UIAlertAction(title: "Save to Files…", style: .default) { _ in self.saveToFiles(dngs) })
            sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel))
            self.anchor(sheet) { self.present(sheet, animated: true) }
        }
    }

    /// Points a popover at the page's button, as iPad requires.
    private func anchor(_ popover: UIViewController, then present: @escaping () -> Void) {
        web.evaluateJavaScript(
            "(() => { const r = document.getElementById('reveal').getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; })()"
        ) { [weak self] rect, _ in
            guard let self else { return }
            let r = (rect as? [Double]) ?? [Double(self.web.bounds.midX), Double(self.web.bounds.maxY), 0, 0]
            popover.popoverPresentationController?.sourceView = self.web
            popover.popoverPresentationController?.sourceRect = CGRect(x: r[0], y: r[1], width: r[2], height: r[3])
            present()
        }
    }

    private func share(_ dngs: [URL]) {
        let sheet = UIActivityViewController(activityItems: dngs, applicationActivities: nil)
        anchor(sheet) { [weak self] in self?.present(sheet, animated: true) }
    }

    private func saveToFiles(_ dngs: [URL]) {
        pick = .saveToFiles
        let picker = UIDocumentPickerViewController(forExporting: dngs, asCopy: true)
        picker.delegate = self
        present(picker, animated: true)
    }

    private func saveToPhotos(_ dngs: [URL]) {
        PHPhotoLibrary.requestAuthorization(for: .addOnly) { status in
            guard status == .authorized || status == .limited else {
                DispatchQueue.main.async {
                    self.present(self.alert("Reversal can't add to Photos", "Allow it in Settings, under Privacy & Security."), animated: true)
                }
                return
            }
            PHPhotoLibrary.shared().performChanges {
                for dng in dngs {
                    let options = PHAssetResourceCreationOptions()
                    options.originalFilename = dng.lastPathComponent
                    PHAssetCreationRequest.forAsset().addResource(with: .photo, fileURL: dng, options: options)
                }
            } completionHandler: { added, error in
                DispatchQueue.main.async {
                    let reply = added
                        ? self.alert("Saved to Photos", dngs.count == 1 ? dngs[0].lastPathComponent : "\(dngs.count) DNGs")
                        : self.alert("Photos did not accept this", error?.localizedDescription ?? "")
                    self.present(reply, animated: true)
                }
            }
        }
    }

    private func alert(_ title: String, _ message: String) -> UIAlertController {
        let alert = UIAlertController(title: title, message: message, preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "OK", style: .default))
        return alert
    }
}
