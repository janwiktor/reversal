// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Reversal for iPhone and iPad: a window around the web UI, as on the Mac. The engine runs in this
// process, because an iOS app can't start another one. The Swift side adds only what a page can't do:
// reach files outside the app, and hand DNGs to Files, Photos and other apps.

import UIKit

@main
final class AppDelegate: UIResponder, UIApplicationDelegate {
    func application(
        _: UIApplication, configurationForConnecting session: UISceneSession,
        options _: UIScene.ConnectionOptions
    ) -> UISceneConfiguration {
        let config = UISceneConfiguration(name: nil, sessionRole: session.role)
        config.delegateClass = SceneDelegate.self
        return config
    }
}

final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?
    private let shell = ShellViewController()

    func scene(
        _ scene: UIScene, willConnectTo _: UISceneSession, options: UIScene.ConnectionOptions
    ) {
        guard let scene = scene as? UIWindowScene else { return }
        let window = UIWindow(windowScene: scene)
        window.rootViewController = shell
        window.makeKeyAndVisible()
        self.window = window
        opened = options.urlContexts.map(\.url)
    }

    /// Scans another app opened in Reversal while it was closed, until the shell is on screen.
    private var opened: [URL] = []

    func sceneDidBecomeActive(_: UIScene) {
        guard !opened.isEmpty else { return }
        let scans = opened
        opened = []
        // After the shell's own start, which opens the last folder or asks for one.
        DispatchQueue.main.async { self.shell.open(scans: scans) }
    }

    func scene(_: UIScene, openURLContexts contexts: Set<UIOpenURLContext>) {
        shell.open(scans: contexts.map(\.url))
    }

    func sceneDidEnterBackground(_: UIScene) { shell.rememberEdits() }
}
