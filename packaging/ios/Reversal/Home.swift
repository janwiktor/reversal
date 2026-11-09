// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Where a session starts: choose how scans get into the app, and look after the rolls in it.

import SwiftUI

/// What the start screen can ask of the shell. Each change to a roll ends with `done`, so the
/// list shows what's there now.
struct HomeActions {
    let openFolder: () -> Void
    let importFolder: () -> Void
    let importFiles: () -> Void
    let importPhotos: () -> Void
    let open: (URL) -> Void
    let close: () -> Void
    let exportDngs: (Roll) -> Void
    let rename: (Roll, String, @escaping () -> Void) -> Void
    let delete: (Roll, @escaping () -> Void) -> Void
    /// Whether a folder is open behind the start screen, to go back to.
    let hasOpenFolder: () -> Bool
}

struct HomeView: View {
    let lastFolder: URL?
    let actions: HomeActions

    @State private var rolls = Library.rolls()
    @State private var canClose: Bool
    @State private var renaming: Roll?
    @State private var newName = ""
    @State private var deleting: Roll?

    init(lastFolder: URL?, actions: HomeActions) {
        self.lastFolder = lastFolder
        self.actions = actions
        _canClose = State(initialValue: actions.hasOpenFolder())
    }

    /// After a roll changed: what's there now, and whether one is still open.
    private func refresh() {
        rolls = Library.rolls()
        canClose = actions.hasOpenFolder()
    }

    private var device: String { UIDevice.current.userInterfaceIdiom == .pad ? "iPad" : "iPhone" }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Button(action: actions.openFolder) { Label("Open Folder…", systemImage: "folder") }
                    if let lastFolder, !Library.isInApp(lastFolder) {
                        Button { actions.open(lastFolder) } label: {
                            Label("Last folder: \(lastFolder.lastPathComponent)", systemImage: "clock")
                        }
                    }
                } header: {
                    Text("Work in place")
                } footer: {
                    Text("Use a folder in Files, on a memory card or on a drive, with nothing copied. DNGs go in a positives folder inside it, so it must be writable and stay connected.")
                }
                Section {
                    Button(action: actions.importFolder) {
                        Label("Import Folder or Card…", systemImage: "sdcard")
                    }
                    Button(action: actions.importFiles) {
                        Label("Import Files…", systemImage: "doc.on.doc")
                    }
                    Button(action: actions.importPhotos) {
                        Label("Import from Photos…", systemImage: "photo.on.rectangle")
                    }
                } header: {
                    Text("Import")
                } footer: {
                    Text("Copy scans into Reversal, also from folders inside the one you choose. The originals stay where they are, and the card can go. From Photos, only raw photos come along.")
                }
                if !rolls.isEmpty {
                    Section {
                        ForEach(rolls) { roll in row(roll) }
                    } header: {
                        Text("In Reversal")
                    } footer: {
                        Text("\(Roll.size(rolls.reduce(0) { $0 + $1.bytes })) on this \(device), in Files under Reversal. Deleting a roll frees its space.")
                    }
                }
            }
            .navigationTitle("Reversal")
            .toolbar {
                if canClose {
                    ToolbarItem(placement: .cancellationAction) { Button("Close", action: actions.close) }
                }
            }
            .alert("Rename Roll", isPresented: shown($renaming), presenting: renaming) { roll in
                TextField("Name", text: $newName)
                Button("Cancel", role: .cancel) { renaming = nil }
                Button("Rename") {
                    renaming = nil
                    actions.rename(roll, newName) { refresh() }
                }
                .disabled(!Library.canRename(roll, to: newName))
            }
            .confirmationDialog(
                deleting.map { "Delete \($0.name)?" } ?? "", isPresented: shown($deleting),
                titleVisibility: .visible, presenting: deleting
            ) { roll in
                Button("Delete \(roll.contents)", role: .destructive) {
                    deleting = nil
                    actions.delete(roll) { refresh() }
                }
                Button("Cancel", role: .cancel) { deleting = nil }
            } message: { _ in
                Text("This removes them from Reversal for good. What you imported them from, and DNGs you exported, stay where they are.")
            }
        }
    }

    /// Whether there's a roll to ask about; putting the question away forgets the roll.
    private func shown(_ roll: Binding<Roll?>) -> Binding<Bool> {
        Binding(get: { roll.wrappedValue != nil }, set: { if !$0 { roll.wrappedValue = nil } })
    }

    /// A roll opens with a tap. What else can be done with it is behind its own button, where
    /// a swipe alone would hide it.
    private func row(_ roll: Roll) -> some View {
        HStack {
            Button { actions.open(roll.url) } label: {
                VStack(alignment: .leading, spacing: 2) {
                    Text(roll.name)
                    Text("\(roll.contents) · \(Roll.size(roll.bytes))")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            Menu {
                Button { actions.exportDngs(roll) } label: { Label("Export DNGs…", systemImage: "square.and.arrow.up") }
                    .disabled(roll.dngs == 0)
                Button {
                    newName = roll.name
                    renaming = roll
                } label: { Label("Rename…", systemImage: "pencil") }
                Button(role: .destructive) { deleting = roll } label: { Label("Delete…", systemImage: "trash") }
            } label: {
                Image(systemName: "ellipsis.circle")
                    .frame(width: 44, height: 44)
                    .contentShape(Rectangle())
            }
            .accessibilityLabel("More for \(roll.name)")
        }
        .buttonStyle(.borderless)
        .swipeActions {
            Button(role: .destructive) { deleting = roll } label: { Label("Delete", systemImage: "trash") }
        }
    }
}
