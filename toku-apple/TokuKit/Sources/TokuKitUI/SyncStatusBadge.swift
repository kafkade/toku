import SwiftUI
import TokuKit

/// Compact sync status indicator for the macOS sidebar and iOS tab/sidebar.
///
/// Renders nothing until sync is configured. Once configured it shows a green
/// "Synced" badge, or an orange conflict badge when conflicts await review.
/// Tapping the conflict badge opens the shared `ConflictResolutionView`.
public struct SyncStatusBadge: View {
    @ObservedObject private var viewModel: SyncViewModel
    @State private var showingConflicts = false

    public init(viewModel: SyncViewModel) {
        self.viewModel = viewModel
    }

    public var body: some View {
        Group {
            if viewModel.isConfigured {
                Button {
                    if viewModel.hasConflicts {
                        showingConflicts = true
                    } else {
                        viewModel.syncNow()
                    }
                } label: {
                    badgeLabel
                }
                .buttonStyle(.plain)
                .help(viewModel.hasConflicts
                    ? "Resolve sync conflicts"
                    : "Synced — tap to sync now")
            }
        }
        .onAppear { viewModel.refresh() }
        .sheet(isPresented: $showingConflicts) {
            NavigationStack {
                ConflictResolutionView(viewModel: viewModel)
                    .toolbar {
                        ToolbarItem(placement: .confirmationAction) {
                            Button("Done") { showingConflicts = false }
                        }
                    }
            }
        }
    }

    @ViewBuilder
    private var badgeLabel: some View {
        if viewModel.hasConflicts {
            Label(
                "^[\(viewModel.conflictCount) conflict](inflect: true)",
                systemImage: "exclamationmark.triangle.fill"
            )
            .font(.caption)
            .foregroundStyle(.orange)
        } else {
            Label("Synced", systemImage: "checkmark.icloud")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }
}
