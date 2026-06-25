import SwiftUI
import TokuKit

/// Root view with NavigationSplitView: sidebar + content + detail.
struct ContentView: View {
    @EnvironmentObject var appState: AppState
    @State private var selection: SidebarItem? = .library
    @State private var selectedBookID: String?
    @State private var columnVisibility: NavigationSplitViewVisibility = .all

    var body: some View {
        if let errorMessage = appState.errorMessage {
            VStack(spacing: 16) {
                Image(systemName: "exclamationmark.triangle")
                    .font(.largeTitle)
                    .foregroundStyle(.red)
                Text("Unable to start Toku")
                    .font(.headline)
                Text(errorMessage)
                    .foregroundStyle(.secondary)
            }
            .padding()
        } else {
            NavigationSplitView(columnVisibility: $columnVisibility) {
                SidebarView(selection: $selection)
            } content: {
                switch selection {
                case .library:
                    if let vm = appState.libraryVM {
                        LibraryTableView(viewModel: vm, selectedBookID: $selectedBookID)
                    }
                case .libraryGrid:
                    if let vm = appState.libraryVM {
                        LibraryGridView(viewModel: vm, selectedBookID: $selectedBookID)
                    }
                case .stats:
                    if let vm = appState.statsVM {
                        StatsDashboardView(viewModel: vm)
                    }
                case .importBooks:
                    if let vm = appState.importVM {
                        ImportView(viewModel: vm, onImportComplete: {
                            appState.libraryVM?.loadBooks()
                        })
                    }
                case .none:
                    Text("Select an item from the sidebar")
                        .foregroundStyle(.secondary)
                }
            } detail: {
                if let bookID = selectedBookID, let ffi = appState.ffi {
                    BookDetailView(bookID: bookID, ffi: ffi)
                } else {
                    Text("Select a book to see details")
                        .foregroundStyle(.secondary)
                        .frame(minWidth: 250)
                }
            }
            .searchable(
                text: Binding(
                    get: { appState.libraryVM?.searchText ?? "" },
                    set: { newValue in
                        appState.libraryVM?.searchText = newValue
                        appState.libraryVM?.loadBooks()
                    }
                ),
                placement: .toolbar,
                prompt: "Search books..."
            )
            .task {
                appState.performLaunchSync()
            }
        }
    }
}
