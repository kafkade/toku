import SwiftUI
import TokuKit
import TokuKitUI

/// Sidebar navigation items.
enum SidebarItem: String, Identifiable, CaseIterable {
    case library = "Library"
    case libraryGrid = "Grid View"
    case stats = "Statistics"
    case importBooks = "Import"

    var id: String { rawValue }

    var systemImage: String {
        switch self {
        case .library: return "books.vertical"
        case .libraryGrid: return "square.grid.2x2"
        case .stats: return "chart.bar"
        case .importBooks: return "square.and.arrow.down"
        }
    }
}

/// Sidebar view with section-grouped navigation.
struct SidebarView: View {
    @Binding var selection: SidebarItem?
    @EnvironmentObject var appState: AppState

    var body: some View {
        List(selection: $selection) {
            Section("Library") {
                Label("All Books", systemImage: SidebarItem.library.systemImage)
                    .tag(SidebarItem.library)
                Label("Grid View", systemImage: SidebarItem.libraryGrid.systemImage)
                    .tag(SidebarItem.libraryGrid)
            }

            Section("Insights") {
                Label("Statistics", systemImage: SidebarItem.stats.systemImage)
                    .tag(SidebarItem.stats)
            }

            Section("Manage") {
                Label("Import", systemImage: SidebarItem.importBooks.systemImage)
                    .tag(SidebarItem.importBooks)
            }
        }
        .listStyle(.sidebar)
        .navigationTitle("Toku")
        .safeAreaInset(edge: .bottom) {
            if let syncVM = appState.syncVM {
                HStack {
                    SyncStatusBadge(viewModel: syncVM)
                    Spacer()
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
            }
        }
    }
}

