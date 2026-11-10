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
    /// Whether this roll is the one that's open.
    let isOpen: (Roll) -> Bool
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

    var body: some View {
        NavigationStack {
            List {
                // What's already here comes first: after the first roll, that's why the app is opened.
                if !rolls.isEmpty {
                    Section {
                        ForEach(rolls) { roll in row(roll) }
                    } header: {
                        Text("Rolls")
                    } footer: {
                        Text("\(Roll.size(rolls.reduce(0) { $0 + $1.bytes })) on this \(deviceName), in Files under Reversal")
                    }
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
                    Text("Copy into Reversal")
                } footer: {
                    Text("Copies scans into Reversal, so the card can go. From Photos, only raw photos are copied")
                }
                Section {
                    Button(action: actions.openFolder) { Label("Open Folder…", systemImage: "folder") }
                    if let lastFolder, !Library.isInApp(lastFolder) {
                        Button { actions.open(lastFolder) } label: {
                            Label("Last folder: \(lastFolder.lastPathComponent)", systemImage: "clock")
                        }
                    }
                } header: {
                    Text("Open in place")
                } footer: {
                    Text("Works on a folder in Files, on a card or a drive, with nothing copied. DNGs go in a positives folder inside it")
                }
            }
            .onReceive(NotificationCenter.default.publisher(for: .rollsChanged)) { _ in refresh() }
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
            .alert(
                deleting.map { "Delete “\($0.name)”?" } ?? "", isPresented: shown($deleting), presenting: deleting
            ) { roll in
                Button("Delete Roll", role: .destructive) {
                    deleting = nil
                    actions.delete(roll) { refresh() }
                }
                Button("Cancel", role: .cancel) { deleting = nil }
            } message: { _ in
                Text("Its scans and DNGs are removed from Reversal. DNGs you haven't exported are lost. The originals you imported from stay")
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
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        // A roll is a thing, not a command: in the text color, not the tint.
                        // The label colors by name: inside a button, primary means the tint.
                        Text(roll.name).foregroundStyle(Color(uiColor: .label))
                        Text("\(roll.contents) · \(Roll.size(roll.bytes))")
                            .font(.footnote)
                            .foregroundStyle(Color(uiColor: .secondaryLabel))
                    }
                    Spacer()
                    if actions.isOpen(roll) {
                        Image(systemName: "checkmark").accessibilityLabel("Open")
                    }
                }
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
