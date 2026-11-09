// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Keeps the engine's work going while the app is off screen. A roll takes minutes to save, and
// iOS gives an app that leaves the screen about half a minute. From iOS 26 the app can ask for a
// task that continues work the user started, which the system shows with its progress and lets
// the user stop. Before that, and where the system declines, the half minute is all there is,
// and the frames that didn't get saved are offered again when the roll is next open.

import BackgroundTasks
import UIKit

final class BackgroundWork {
    private let engine: Engine
    /// Follows the engine while it works: the page can't, once the app is off screen.
    private var poll: Timer?
    private var short = UIBackgroundTaskIdentifier.invalid
    /// A `BGContinuedProcessingTask`, once the system has started the one asked for.
    private var long: Any?
    /// The system took the time back, and the work has been told to stop for now.
    var onExpired: () -> Void = {}

    private static var registered = false
    private static weak var current: BackgroundWork?
    /// Listed in Info.plist, under BGTaskSchedulerPermittedIdentifiers.
    private static var identifier: String { (Bundle.main.bundleIdentifier ?? "reversal") + ".work" }

    init(engine: Engine) {
        self.engine = engine
    }

    /// The engine has started on a list of frames.
    func begin() {
        guard poll == nil else { return }
        UIApplication.shared.isIdleTimerDisabled = true
        short = UIApplication.shared.beginBackgroundTask { [weak self] in self?.endShort() }
        Self.current = self
        if #available(iOS 26.0, *) { askForLong() }
        poll = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in self?.check() }
    }

    /// The work is over, as the page says or as the engine does.
    func end(success: Bool = true) {
        guard poll != nil else { return }
        poll?.invalidate()
        poll = nil
        UIApplication.shared.isIdleTimerDisabled = false
        endShort()
        if #available(iOS 26.0, *), let task = long as? BGContinuedProcessingTask {
            task.setTaskCompleted(success: success)
        }
        long = nil
    }

    private func endShort() {
        guard short != .invalid else { return }
        UIApplication.shared.endBackgroundTask(short)
        short = .invalid
    }

    private func check() {
        engine.request("GET", "/api/export/status") { [weak self] response in
            guard let self, self.poll != nil, let status = response.json else { return }
            if #available(iOS 26.0, *) { self.show(status) }
            if status["running"] as? Bool != true { self.end() }
        }
    }

    @available(iOS 26.0, *)
    private func askForLong() {
        if !Self.registered {
            Self.registered = BGTaskScheduler.shared.register(
                forTaskWithIdentifier: Self.identifier, using: .main
            ) { task in
                guard let task = task as? BGContinuedProcessingTask, let work = Self.current, work.poll != nil
                else { return task.setTaskCompleted(success: true) }
                work.adopt(task)
            }
        }
        let request = BGContinuedProcessingTaskRequest(
            identifier: Self.identifier, title: "Reversal", subtitle: "Starting")
        // Now or not at all: the work runs either way, and a task that started later would
        // have nothing left to cover.
        request.strategy = .fail
        // Declined, or not offered on this device: the short task still covers a brief absence.
        try? BGTaskScheduler.shared.submit(request)
    }

    @available(iOS 26.0, *)
    private func adopt(_ task: BGContinuedProcessingTask) {
        long = task
        task.expirationHandler = { [weak self] in
            DispatchQueue.main.async {
                guard let self, self.poll != nil else { return }
                self.onExpired()
                self.end(success: false)
            }
        }
    }

    /// What the system shows of the work, on the lock screen and above other apps.
    @available(iOS 26.0, *)
    private func show(_ status: [String: Any]) {
        guard let task = long as? BGContinuedProcessingTask else { return }
        let total = status["total"] as? Int ?? 0
        let through = (status["done"] as? Int ?? 0) + (status["failed"] as? Int ?? 0)
        task.progress.totalUnitCount = Int64(max(total, 1))
        task.progress.completedUnitCount = Int64(through)
        let saving = status["task"] as? String == "save"
        task.updateTitle(saving ? "Saving DNGs" : "Matching the film base", subtitle: "\(through) of \(total)")
    }
}
