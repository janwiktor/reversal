// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Where a session starts: choose how scans get into the app.

import SwiftUI

struct HomeView: View {
    let rolls: [Roll]
    let lastFolder: URL?
    let canClose: Bool
    let openFolder: () -> Void
    let importFolder: () -> Void
    let importFiles: () -> Void
    let importPhotos: () -> Void
    let open: (URL) -> Void
    let close: () -> Void

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Button(action: openFolder) { Label("Open Folder…", systemImage: "folder") }
                    if let lastFolder, !Library.isInApp(lastFolder) {
                        Button { open(lastFolder) } label: {
                            Label("Last folder: \(lastFolder.lastPathComponent)", systemImage: "clock")
                        }
                    }
                } header: {
                    Text("Work in place")
                } footer: {
                    Text("Use a folder in Files, on a memory card or on a drive, with nothing copied. DNGs go in a positives folder inside it, so it must be writable and stay connected.")
                }
                Section {
                    Button(action: importFolder) {
                        Label("Import Folder or Card…", systemImage: "sdcard")
                    }
                    Button(action: importFiles) {
                        Label("Import Files…", systemImage: "doc.on.doc")
                    }
                    Button(action: importPhotos) {
                        Label("Import from Photos…", systemImage: "photo.on.rectangle")
                    }
                } header: {
                    Text("Import")
                } footer: {
                    Text("Copy scans into Reversal, also from folders inside the one you choose. The originals stay where they are, and the card can go. From Photos, only raw photos come along.")
                }
                if !rolls.isEmpty {
                    Section("In Reversal") {
                        ForEach(rolls) { roll in
                            Button { open(roll.url) } label: {
                                LabeledContent(roll.name, value: "\(roll.frames)")
                            }
                        }
                    }
                }
            }
            .navigationTitle("Reversal")
            .toolbar {
                if canClose {
                    ToolbarItem(placement: .cancellationAction) { Button("Close", action: close) }
                }
            }
        }
    }
}
