import SwiftUI

/// Standard macOS menu bar commands and keyboard shortcuts.
struct TokuCommands: Commands {
    var body: some Commands {
        // Replace the default New Window command
        CommandGroup(replacing: .newItem) {
            Button("Add Book…") {
                NotificationCenter.default.post(name: .tokuAddBook, object: nil)
            }
            .keyboardShortcut("n", modifiers: .command)
        }

        CommandMenu("Library") {
            Button("Import Goodreads CSV…") {
                NotificationCenter.default.post(name: .tokuImport, object: nil)
            }
            .keyboardShortcut("i", modifiers: .command)

            Divider()

            Button("Refresh Library") {
                NotificationCenter.default.post(name: .tokuRefresh, object: nil)
            }
            .keyboardShortcut("r", modifiers: .command)
        }
    }
}

// MARK: - Notification names for menu actions

extension Notification.Name {
    static let tokuAddBook = Notification.Name("tokuAddBook")
    static let tokuImport = Notification.Name("tokuImport")
    static let tokuRefresh = Notification.Name("tokuRefresh")
}
