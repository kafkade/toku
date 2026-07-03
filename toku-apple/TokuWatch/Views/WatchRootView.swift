import SwiftUI
import TokuKit

/// Root navigation for the watch companion: currently-reading list, glance stats,
/// and a sync affordance.
struct WatchRootView: View {
    @EnvironmentObject var appState: WatchAppState

    var body: some View {
        NavigationStack {
            Group {
                if let errorMessage = appState.errorMessage {
                    ContentUnavailableView(
                        "Unavailable",
                        systemImage: "exclamationmark.triangle",
                        description: Text(errorMessage)
                    )
                } else {
                    contentList
                }
            }
            .navigationTitle("Toku")
        }
        .task {
            appState.refresh()
            appState.performLaunchSync()
        }
    }

    @ViewBuilder
    private var contentList: some View {
        List {
            Section("Reading Now") {
                let reading = appState.currentlyReading
                if reading.isEmpty {
                    Text("No books in progress")
                        .foregroundStyle(.secondary)
                        .font(.footnote)
                } else {
                    ForEach(reading) { book in
                        NavigationLink {
                            LogProgressView(book: book)
                                .environmentObject(appState)
                        } label: {
                            CurrentlyReadingRow(book: book)
                        }
                    }
                }
            }

            Section {
                NavigationLink {
                    GlanceStatsView()
                        .environmentObject(appState)
                } label: {
                    Label("Stats", systemImage: "chart.bar.fill")
                }

                if let syncVM = appState.syncVM {
                    SyncRow(syncVM: syncVM)
                }
            }
        }
    }
}

/// A compact currently-reading row: title, author, and a thin progress hint.
struct CurrentlyReadingRow: View {
    let book: Book

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(book.title)
                .font(.headline)
                .lineLimit(2)
            Text(book.authorDisplay)
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .padding(.vertical, 2)
    }
}

/// A one-tap "sync now" row that reflects busy state.
private struct SyncRow: View {
    @ObservedObject var syncVM: SyncViewModel

    var body: some View {
        Button {
            syncVM.syncNow()
        } label: {
            HStack {
                Label("Sync", systemImage: "arrow.triangle.2.circlepath")
                Spacer()
                if syncVM.isBusy {
                    ProgressView()
                }
            }
        }
        .disabled(syncVM.isBusy)
    }
}
