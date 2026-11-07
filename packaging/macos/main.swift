// Reversal.app: a window around the local web UI. All the work happens in `reversal-engine`,
// the same program as the `reversal` command line; this file only adds what a web page can't
// do itself: start the engine, choose a folder and show a file in Finder.

import Cocoa
import WebKit

/// Accepts a folder dropped anywhere on the window.
final class DropWebView: WKWebView {
    var onFolder: ((URL) -> Void)?

    private func folder(_ info: NSDraggingInfo) -> URL? {
        let options: [NSPasteboard.ReadingOptionKey: Any] = [.urlReadingFileURLsOnly: true]
        let urls = info.draggingPasteboard.readObjects(forClasses: [NSURL.self], options: options)
        guard let url = urls?.first as? URL else { return nil }
        let isFolder = (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true
        return isFolder ? url : nil
    }

    override func draggingEntered(_ info: NSDraggingInfo) -> NSDragOperation {
        folder(info) == nil ? [] : .copy
    }

    override func draggingUpdated(_ info: NSDraggingInfo) -> NSDragOperation {
        folder(info) == nil ? [] : .copy
    }

    override func performDragOperation(_ info: NSDraggingInfo) -> Bool {
        guard let url = folder(info) else { return false }
        onFolder?(url)
        return true
    }
}

final class App: NSObject, NSApplicationDelegate, NSWindowDelegate, WKScriptMessageHandler {
    private static let lastFolderKey = "folder"
    private var window: NSWindow!
    private var web: DropWebView!
    private var engine: Process?
    /// The page says when it has work running in the engine.
    private var busy = false
    /// A folder the app was opened with, in the Dock or Finder, before it finished launching.
    private var launchFolder: URL?

    func applicationWillFinishLaunching(_: Notification) {
        NSApp.mainMenu = makeMenu()
    }

    func applicationDidFinishLaunching(_: Notification) {
        let config = WKWebViewConfiguration()
        config.userContentController.add(self, name: "reversal")
        web = DropWebView(frame: .zero, configuration: config)
        web.onFolder = { [weak self] folder in self?.leaveFolder(then: { self?.show(folder: folder) }) }
        web.unregisterDraggedTypes()
        web.registerForDraggedTypes([.fileURL])

        let screen = NSScreen.main?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1440, height: 900)
        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: min(1500, screen.width), height: min(960, screen.height)),
            styleMask: [.titled, .closable, .resizable, .miniaturizable],
            backing: .buffered, defer: false)
        window.title = "Reversal"
        // The title bar carries on from the page's top bar, in any system appearance.
        window.appearance = NSAppearance(named: .darkAqua)
        window.titlebarAppearsTransparent = true
        window.backgroundColor = NSColor(calibratedWhite: 0.15, alpha: 1)
        window.delegate = self
        window.contentMinSize = NSSize(width: 960, height: 600)
        window.contentView = web
        window.center()
        window.setFrameAutosaveName("main")
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)

        let saved = UserDefaults.standard.string(forKey: Self.lastFolderKey)
            .map { URL(fileURLWithPath: $0, isDirectory: true) }
            .flatMap { FileManager.default.fileExists(atPath: $0.path) ? $0 : nil }
        if let folder = launchFolder ?? saved {
            show(folder: folder)
        } else {
            chooseFolder()
        }
    }

    func application(_: NSApplication, open urls: [URL]) {
        guard let url = urls.first else { return }
        let isFolder = (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true
        let folder = isFolder ? url : url.deletingLastPathComponent()
        if web == nil {
            launchFolder = folder
        } else {
            leaveFolder(then: { [weak self] in self?.show(folder: folder) })
        }
    }

    /// Closing the window goes through the same checks as quitting.
    func windowShouldClose(_: NSWindow) -> Bool {
        NSApp.terminate(nil)
        return false
    }

    func applicationShouldTerminate(_: NSApplication) -> NSApplication.TerminateReply {
        leaveFolder(
            then: { NSApp.reply(toApplicationShouldTerminate: true) },
            otherwise: { NSApp.reply(toApplicationShouldTerminate: false) })
        return .terminateLater
    }

    func applicationWillTerminate(_: Notification) { stopEngine() }

    /// Before the engine stops: ask if it has work running, then let the page store its
    /// latest edits.
    private func leaveFolder(then proceed: @escaping () -> Void, otherwise stay: @escaping () -> Void = {}) {
        guard engine != nil else { return proceed() }
        if busy {
            let alert = NSAlert()
            alert.messageText = "Reversal is still working"
            alert.informativeText = "Stopping now leaves the frames in progress unfinished."
            alert.addButton(withTitle: "Keep Working")
            alert.addButton(withTitle: "Stop")
            guard alert.runModal() == .alertSecondButtonReturn else { return stay() }
        }
        var finished = false
        let finish = {
            guard !finished else { return }
            finished = true
            proceed()
        }
        web.callAsyncJavaScript(
            "await rememberSettings(true)", arguments: [:], in: nil, in: .page,
            completionHandler: { _ in finish() })
        // A page that doesn't answer mustn't stop the app from closing.
        DispatchQueue.main.asyncAfter(deadline: .now() + 2, execute: finish)
    }

    @objc func chooseFolder() {
        guard window.attachedSheet == nil else { return }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.message = "Choose a folder of negative scans."
        panel.prompt = "Open"
        panel.beginSheetModal(for: window) { [weak self] response in
            guard let self else { return }
            if response == .OK, let url = panel.url {
                self.leaveFolder(then: { self.show(folder: url) })
            } else if self.engine == nil {
                NSApp.terminate(nil)
            }
        }
    }

    /// The engine serves one folder, so a new folder gets a new engine.
    private func show(folder: URL) {
        stopEngine()
        busy = false
        UserDefaults.standard.set(folder.path, forKey: Self.lastFolderKey)
        window.title = "Reversal: \(folder.lastPathComponent)"

        let process = Process()
        process.executableURL = Bundle.main.url(forAuxiliaryExecutable: "reversal-engine")
        process.arguments = ["ui", folder.path, "--port", "0", "--exit-with-stdin"]
        process.qualityOfService = .userInitiated
        // The engine exits when this pipe closes, also if the app crashes.
        process.standardInput = Pipe()
        let output = Pipe()
        process.standardOutput = output

        var pending = ""
        output.fileHandleForReading.readabilityHandler = { [weak self] handle in
            // Always read, or the engine would block once the pipe fills up.
            let data = handle.availableData
            guard !data.isEmpty else {
                handle.readabilityHandler = nil  // it ended
                return
            }
            let text = String(decoding: data, as: UTF8.self)
            guard !pending.isEmpty || text.contains("http") else { return }
            pending += text
            guard let range = pending.range(of: #"http://127\.0\.0\.1:\d+"#, options: .regularExpression),
                let url = URL(string: String(pending[range]))
            else { return }
            pending = ""
            DispatchQueue.main.async {
                guard let self, self.engine === process else { return }
                self.web.load(URLRequest(url: url))
            }
        }
        process.terminationHandler = { [weak self] ended in
            DispatchQueue.main.async {
                guard let self, self.engine === ended else { return }
                self.engine = nil
                self.fail("The conversion engine stopped unexpectedly (status \(ended.terminationStatus)).")
            }
        }
        do {
            try process.run()
            engine = process
        } catch {
            fail("Could not start the conversion engine: \(error.localizedDescription)")
        }
    }

    private func stopEngine() {
        let running = engine
        engine = nil
        running?.terminate()
    }

    private func fail(_ message: String) {
        let alert = NSAlert()
        alert.messageText = "Reversal needs to quit"
        alert.informativeText = message
        alert.runModal()
        NSApp.terminate(nil)
    }

    /// Requests from the page. It is served by our own engine, on this machine.
    func userContentController(_: WKUserContentController, didReceive message: WKScriptMessage) {
        guard let body = message.body as? [String: Any] else { return }
        switch body["action"] as? String {
        case "chooseFolder":
            chooseFolder()
        case "busy":
            busy = body["busy"] as? Bool ?? false
        case "reveal":
            if let path = body["path"] as? String {
                NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
            }
        default:
            break
        }
    }

    private func makeMenu() -> NSMenu {
        let bar = NSMenu()
        func add(_ title: String, _ items: [NSMenuItem]) -> NSMenu {
            let menu = NSMenu(title: title)
            items.forEach(menu.addItem)
            let holder = NSMenuItem()
            holder.submenu = menu
            bar.addItem(holder)
            return menu
        }
        func item(_ title: String, _ action: Selector?, _ key: String = "") -> NSMenuItem {
            NSMenuItem(title: title, action: action, keyEquivalent: key)
        }
        _ = add("Reversal", [
            item("About Reversal", #selector(NSApplication.orderFrontStandardAboutPanel(_:))),
            .separator(),
            item("Hide Reversal", #selector(NSApplication.hide(_:)), "h"),
            .separator(),
            item("Quit Reversal", #selector(NSApplication.terminate(_:)), "q"),
        ])
        _ = add("File", [
            item("Open Folder…", #selector(chooseFolder), "o"),
            item("Close Window", #selector(NSWindow.performClose(_:)), "w"),
        ])
        _ = add("Edit", [
            item("Cut", #selector(NSText.cut(_:)), "x"),
            item("Copy", #selector(NSText.copy(_:)), "c"),
            item("Paste", #selector(NSText.paste(_:)), "v"),
            item("Select All", #selector(NSText.selectAll(_:)), "a"),
        ])
        NSApp.windowsMenu = add("Window", [
            item("Minimize", #selector(NSWindow.performMiniaturize(_:)), "m"),
            item("Zoom", #selector(NSWindow.performZoom(_:))),
        ])
        return bar
    }
}

let delegate = App()
NSApplication.shared.delegate = delegate
NSApp.setActivationPolicy(.regular)
NSApp.run()
