// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The window around the page: opens a folder in the engine, and answers what the page asks of
// its shell. The page is the same one the Mac app and a browser get.

import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers
import WebKit

final class ShellViewController: UIViewController, WKScriptMessageHandlerWithReply, WKNavigationDelegate,
    UIDocumentPickerDelegate, PHPickerViewControllerDelegate
{
    private enum Pick {
        case openFolder, importFolder, importFiles
    }

    private var web: WKWebView!
    private let engine = Engine()
    private lazy var pageRequests = EngineSchemeHandler(engine: engine)
    private var folder: URL?
    /// The folder outside the app that we hold access to.
    private var scoped: URL?
    private var pick: Pick?
    private lazy var backgroundWork = BackgroundWork(engine: engine)
    /// The page says when it has work running in the engine.
    private var busy = false {
        didSet {
            guard busy != oldValue else { return }
            busy ? backgroundWork.begin() : backgroundWork.end()
        }
    }

    override var preferredStatusBarStyle: UIStatusBarStyle { .lightContent }

    override func viewDidLoad() {
        super.viewDidLoad()
        // The page's --surround, so nothing flashes white or shows around it.
        view.backgroundColor = UIColor(white: 0.15, alpha: 1)

        let config = WKWebViewConfiguration()
        config.userContentController.addScriptMessageHandler(self, contentWorld: .page, name: "reversal")
        config.setURLSchemeHandler(pageRequests, forURLScheme: Engine.scheme)
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
            // To the screen's edge: the page fills the strip under the home indicator with what's
            // at its foot, and keeps its controls clear of it.
            web.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            web.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            web.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
        ])
    }

    private var started = false
    private var heat: NSObjectProtocol?

    /// Stops the engine's work for now. The frames in progress finish, and the page or the
    /// next visit to this folder offers the rest.
    private func stopForNow(then proceed: @escaping () -> Void = {}) {
        engine.request(
            "POST", "/api/export/cancel", headers: Engine.json, body: Data(#"{"later":true}"#.utf8)
        ) { _ in proceed() }
    }

    /// A device at its hottest slows everything down and may shut the app. A save is the one
    /// thing here that keeps it hot, so it rests, and the rest of it waits.
    private func watchHeat() {
        heat = NotificationCenter.default.addObserver(
            forName: ProcessInfo.thermalStateDidChangeNotification, object: nil, queue: .main
        ) { [weak self] _ in
            guard let self, self.busy, ProcessInfo.processInfo.thermalState == .critical else { return }
            self.stopForNow()
            let device = UIDevice.current.userInterfaceIdiom == .pad ? "iPad" : "iPhone"
            self.top.present(
                self.alert("Saving paused", "This \(device) is too hot to go on. Save the rest when it has cooled."),
                animated: true)
        }
    }

    override func didReceiveMemoryWarning() {
        super.didReceiveMemoryWarning()
        engine.trim()
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        guard !started else { return }
        started = true
        watchHeat()
        backgroundWork.onExpired = { [weak self] stopped in
            guard let self else { return stopped() }
            self.stopForNow(then: stopped)
        }
        // Scans that another app opened in Reversal come after the usual start.
        let then = { [weak self] in
            guard let self else { return }
            self.ready = true
            self.importWaitingScans()
        }
        // A card that's gone by now is no news: just ask what to open.
        guard let last = Library.lastFolder() else { return showHome(then: then) }
        show(folder: last) { [weak self] opened in
            opened ? then() : self?.showHome(then: then)
        }
    }

    // MARK: Engine

    /// The engine holds one folder at a time. `done` hears whether this one opened; if it
    /// didn't, none is open.
    private func show(folder url: URL, done: @escaping (Bool) -> Void = { _ in }) {
        let access = url.startAccessingSecurityScopedResource()
        busy = false
        pageRequests.paused = true
        engine.open(url) { [weak self] opened in
            guard let self else { return }
            self.pageRequests.paused = false
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

    /// Before another folder opens: ask if this one has work running, stop it, then let the
    /// page store its edits.
    private func leaveFolder(then proceed: @escaping () -> Void) {
        guard busy else { return rememberEdits(then: proceed) }
        let warning = UIAlertController(
            title: "Reversal is still working",
            message: "Stopping now finishes the frames in progress. The rest are offered when you open this folder again.", preferredStyle: .alert)
        warning.addAction(UIAlertAction(title: "Keep Working", style: .cancel))
        warning.addAction(UIAlertAction(title: "Stop", style: .destructive) { [weak self] _ in
            self?.stopWork { self?.rememberEdits(then: proceed) }
        })
        top.present(warning, animated: true)
    }

    /// Stops the engine's work and waits for the frames in progress. They still write to this
    /// folder, which the app may lose its access to once another one opens.
    private func stopWork(then proceed: @escaping () -> Void) {
        let stopping = UIAlertController(title: "Stopping…", message: "Finishing the frames in progress", preferredStyle: .alert)
        top.present(stopping, animated: true) { [weak self] in
            // For now: the rest is offered when this folder is next open.
            self?.stopForNow {
                self?.whenIdle { self?.putAway(stopping, then: proceed) }
            }
        }
    }

    private func whenIdle(_ proceed: @escaping () -> Void) {
        engine.request("GET", "/api/export/status") { [weak self] response in
            guard let self, response.json?["running"] as? Bool == true else { return proceed() }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { self.whenIdle(proceed) }
        }
    }

    // MARK: The page's requests

    /// What the page learns when it asks what this shell does differently: see `shell` in the
    /// page's `app.js`. The page stays the same everywhere, and adapts itself to this.
    private static let capabilities: [String: Any] = ["export": true, "rolls": true]

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
        case "reveal":
            buttonPlace = Self.place(body["from"])
            savedDngs { [weak self] dngs in self?.showExport(dngs) }
        case "export":
            buttonPlace = Self.place(body["from"])
            positives(of: body["files"] as? [String] ?? []) { [weak self] dngs in self?.showExport(dngs) }
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

    private func showHome(then: (() -> Void)? = nil) {
        let actions = HomeActions(
            openFolder: { [weak self] in self?.choose(.openFolder, [.folder]) },
            importFolder: { [weak self] in self?.choose(.importFolder, [.folder]) },
            importFiles: { [weak self] in self?.choose(.importFiles, [.rawImage], many: true) },
            importPhotos: { [weak self] in self?.choosePhotos() },
            open: { [weak self] url in self?.switchTo(url) },
            close: { [weak self] in self?.dismiss(animated: true) },
            exportDngs: { [weak self] roll in
                self?.buttonPlace = nil
                self?.showExport(roll.dngFiles())
            },
            rename: { [weak self] roll, name, done in self?.rename(roll, to: name, then: done) },
            delete: { [weak self] roll, done in self?.delete(roll, then: done) },
            hasOpenFolder: { [weak self] in self?.folder != nil },
            isOpen: { [weak self] roll in self?.folder?.standardizedFileURL == roll.url.standardizedFileURL })
        let home = UIHostingController(rootView: HomeView(lastFolder: Library.lastFolder(), actions: actions))
        home.modalPresentationStyle = .formSheet
        home.isModalInPresentation = folder == nil
        self.home = home
        top.present(home, animated: true, completion: then)
    }

    /// The start screen, while it's up. With no folder open it can't be put away.
    private weak var home: UIViewController?

    /// A roll that's open has to close before its folder moves or goes: the engine holds it,
    /// and may be saving into it. `change` then runs with the engine closed.
    private func closing(_ roll: Roll, then change: @escaping (_ wasOpen: Bool) -> Void) {
        let isOpen = folder?.standardizedFileURL == roll.url.standardizedFileURL
        guard isOpen else { return change(false) }
        let close = { [weak self] in
            self?.engine.close {
                self?.folder = nil
                self?.busy = false
                self?.home?.isModalInPresentation = true
                change(true)
            }
        }
        busy ? stopWork(then: { close() }) : close()
    }

    /// Opens a roll again behind the start screen, which stays up, after a change to it.
    private func reopen(_ url: URL, then done: @escaping () -> Void) {
        show(folder: url) { [weak self] opened in
            self?.home?.isModalInPresentation = !opened
            done()
        }
    }

    private func rename(_ roll: Roll, to name: String, then done: @escaping () -> Void) {
        closing(roll) { [weak self] wasOpen in
            guard let self else { return }
            var now = roll.url
            do {
                now = try Library.rename(roll, to: name)
            } catch {
                self.top.present(self.alert("Could not rename \(roll.name)", error.localizedDescription), animated: true)
            }
            wasOpen ? self.reopen(now, then: done) : done()
        }
    }

    private func delete(_ roll: Roll, then done: @escaping () -> Void) {
        closing(roll) { [weak self] wasOpen in
            guard let self else { return }
            do {
                try Library.delete(roll)
                // The page still shows the roll that's gone, behind a start screen that now stays.
                if wasOpen { self.web.loadHTMLString("", baseURL: Engine.page) }
            } catch {
                self.top.present(self.alert("Could not delete \(roll.name)", error.localizedDescription), animated: true)
                if wasOpen { return self.reopen(roll.url, then: done) }
            }
            done()
        }
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
        case nil: break
        }
    }

    /// The start is over: a folder is open or the start screen is up.
    private var ready = false
    /// An import is on screen. One at a time: each has a panel, and ends by opening its roll.
    private var importing = false
    /// Scans other apps opened in Reversal, until it can take them.
    private var waitingScans: [URL] = []

    /// Scans opened from another app, such as Files or Mail. They join a roll and that roll
    /// opens: one named for their folder if they were opened where they are, or for the day if
    /// they came as copies, which iOS leaves in the app's inbox.
    func open(scans: [URL]) {
        waitingScans += scans
        importWaitingScans()
    }

    private func importWaitingScans() {
        guard ready, !importing, let first = waitingScans.first else { return }
        let copies = Library.isInInbox(first)
        let scans = waitingScans.filter { Library.isInInbox($0) == copies }
        waitingScans.removeAll { Library.isInInbox($0) == copies }
        let roll = copies ? "Opened \(Library.today())" : first.deletingLastPathComponent().lastPathComponent
        runImport(scans, rollName: roll)
    }

    private func choosePhotos() {
        // Without a library to read, the picker runs on its own and asks for no access.
        var choice = PHPickerConfiguration()
        choice.filter = .images
        choice.selectionLimit = 0
        // As they are: a raw photo must not arrive as a JPEG made from it.
        choice.preferredAssetRepresentationMode = .current
        let picker = PHPickerViewController(configuration: choice)
        picker.delegate = self
        top.present(picker, animated: true)
    }

    func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
        picker.dismiss(animated: true) { [weak self] in
            guard let self, !results.isEmpty else { return }
            // The day here, not in Greenwich.
            self.runPhotosImport(results.map(\.itemProvider), rollName: "Photos \(Library.today())")
        }
    }

    private func runPhotosImport(_ photos: [NSItemProvider], rollName: String) {
        let importer = PhotosImporter()
        let panel = UIAlertController(title: "Importing", message: "Getting photos…", preferredStyle: .alert)
        panel.addAction(UIAlertAction(title: "Stop", style: .cancel) { _ in importer.cancel() })
        importing = true
        top.present(panel, animated: true) { [weak self] in
            importer.run(
                photos: photos, rollName: rollName,
                progress: { finished, total in panel.message = "\(finished) of \(total)" },
                done: { result, notRaw in
                    self?.imported(result, panel: panel, notRaw: notRaw)
                })
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

    /// Dismisses a panel from what presented it, which also puts away anything shown over it.
    /// Asked of the panel itself, a dismissal takes only what's over it and leaves it up.
    private func putAway(_ panel: UIViewController, then proceed: @escaping () -> Void) {
        guard let presenter = panel.presentingViewController else { return proceed() }
        presenter.dismiss(animated: true, completion: proceed)
    }

    private func runImport(_ sources: [URL], rollName: String) {
        let importer = Importer()
        let panel = UIAlertController(title: "Importing", message: "Finding scans…", preferredStyle: .alert)
        panel.addAction(UIAlertAction(title: "Stop", style: .cancel) { _ in importer.cancel() })
        importing = true
        // Started once the panel is up: one that's still arriving can't be dismissed.
        top.present(panel, animated: true) { [weak self] in
            self?.run(importer, sources, rollName: rollName, panel: panel)
        }
    }

    private func run(_ importer: Importer, _ sources: [URL], rollName: String, panel: UIAlertController) {
        importer.run(
            sources: sources, rollName: rollName,
            progress: { finished, total in panel.message = "\(finished) of \(total)" },
            done: { [weak self] result in self?.imported(result, panel: panel) })
    }

    /// How an import ends: its roll opens, or there's a word on why there's nothing to open.
    /// `notRaw` counts chosen photos that were left out for not being raw.
    private func imported(_ result: Importer.Result, panel: UIAlertController, notRaw: Int = 0) {
        let photos = notRaw == 1 ? "1 photo isn't raw" : "\(notRaw) photos aren't raw"
        let finish = {
            NotificationCenter.default.post(name: .rollsChanged, object: nil)
            if !result.failed.isEmpty, !result.canceled {
                let files = result.failed.count == 1 ? "1 file" : "\(result.failed.count) files"
                let note = UIAlertController(
                    title: "Could not import \(files)", message: result.failed.joined(separator: "\n"),
                    preferredStyle: .alert)
                // The import isn't over until this is read: scans that came in meanwhile wait
                // for it, and then go first. Otherwise what did arrive is still worth opening.
                note.addAction(UIAlertAction(title: "OK", style: .default) { _ in
                    self.importing = false
                    if !self.waitingScans.isEmpty {
                        self.importWaitingScans()
                    } else if result.copied + result.skipped > 0 {
                        self.switchTo(result.roll)
                    }
                })
                self.top.present(note, animated: true)
                return
            }
            self.importing = false
            if !self.waitingScans.isEmpty {
                // More came in meanwhile: on to those, and the last import opens its roll.
                self.importWaitingScans()
            } else if result.canceled {
                // Stopped: nothing to say, and nothing opens over what the user went on to.
            } else if result.copied + result.skipped == 0 {
                let none = notRaw > 0
                    ? self.alert("No raw photos there", "Reversal converts the raw file of a scan. A JPEG or HEIC has lost what it measures.")
                    : self.alert("No scans there", "Choose a folder or files with NEF, CR2, ARW, RAF or DNG scans.")
                self.top.present(none, animated: true)
            } else if notRaw > 0 {
                let arrived = result.copied + result.skipped
                let note = UIAlertController(
                    title: "Imported \(arrived == 1 ? "1 raw photo" : "\(arrived) raw photos")",
                    message: "\(photos) and \(notRaw == 1 ? "was" : "were") left out", preferredStyle: .alert)
                note.addAction(UIAlertAction(title: "OK", style: .default) { _ in self.switchTo(result.roll) })
                self.top.present(note, animated: true)
            } else {
                self.switchTo(result.roll)
            }
        }
        putAway(panel, then: finish)
    }

    // MARK: Getting DNGs out

    /// The DNGs of the last save, from the engine. None while the engine is on to other work.
    private func savedDngs(_ done: @escaping ([URL]) -> Void) {
        engine.request("GET", "/api/export/status") { response in
            let status = response.json
            let finishedSave = status?["task"] as? String == "save" && status?["running"] as? Bool == false
            let paths = finishedSave ? status?["saved_paths"] as? [String] ?? [] : []
            done(paths.map { URL(fileURLWithPath: $0) })
        }
    }

    /// The DNGs of these frames, for the ones that have one.
    private func positives(of files: [String], _ done: @escaping ([URL]) -> Void) {
        let body = (try? JSONSerialization.data(withJSONObject: ["files": files])) ?? Data()
        engine.request("POST", "/api/positives", headers: Engine.json, body: body) { response in
            let paths = response.json?["paths"] as? [String] ?? []
            done(paths.map { URL(fileURLWithPath: $0) })
        }
    }

    /// Hands DNGs on with the system's share sheet, which has Photos, Files and other apps.
    private func showExport(_ dngs: [URL]) {
        guard !dngs.isEmpty else { return }
        let sheet = UIActivityViewController(activityItems: dngs, applicationActivities: nil)
        anchor(sheet)
        top.present(sheet, animated: true)
    }

    /// Where the page's button is, in the web view, as the page said with its request.
    private var buttonPlace: CGRect?

    private static func place(_ from: Any?) -> CGRect? {
        guard let r = from as? [Double], r.count == 4 else { return nil }
        return CGRect(x: r[0], y: r[1], width: r[2], height: r[3])
    }

    /// Points a popover at the page's button, as iPad requires, or at the middle of what's on
    /// top when the request didn't come from the page.
    private func anchor(_ popover: UIViewController) {
        let (view, place): (UIView, CGRect) =
            if let buttonPlace, top === self { (web, buttonPlace) } else {
                (top.view, CGRect(x: top.view.bounds.midX, y: top.view.bounds.midY, width: 0, height: 0))
            }
        popover.popoverPresentationController?.sourceView = view
        popover.popoverPresentationController?.sourceRect = place
        if view !== web { popover.popoverPresentationController?.permittedArrowDirections = [] }
    }

    private func alert(_ title: String, _ message: String) -> UIAlertController {
        let alert = UIAlertController(title: title, message: message, preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "OK", style: .default))
        return alert
    }
}
