// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Reversal for iPad: a window around the web UI, as on the Mac. The engine runs in this process,
// because an iOS app can't start another one. The Swift side adds only what a page can't do:
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
        _ scene: UIScene, willConnectTo _: UISceneSession, options _: UIScene.ConnectionOptions
    ) {
        guard let scene = scene as? UIWindowScene else { return }
        let window = UIWindow(windowScene: scene)
        window.rootViewController = shell
        window.makeKeyAndVisible()
        self.window = window
    }

    func sceneDidBecomeActive(_: UIScene) { shell.checkEngine() }
    func sceneDidEnterBackground(_: UIScene) { shell.rememberEdits() }
}
