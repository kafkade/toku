import SwiftUI
import TokuKit

/// Shared sync conflict resolution screen used by both the macOS and iOS apps.
///
/// Lists unresolved sync conflicts (note/review edits that collided across
/// devices) and lets the user keep either the local or the remote value, one at
/// a time or all at once. All work is delegated to `SyncViewModel`, which runs
/// FFI calls off the main thread.
public struct ConflictResolutionView: View {
    @ObservedObject private var viewModel: SyncViewModel

    public init(viewModel: SyncViewModel) {
        self.viewModel = viewModel
    }

    public var body: some View {
        Group {
            if viewModel.conflicts.isEmpty {
                emptyState
            } else {
                conflictList
            }
        }
        .navigationTitle("Sync Conflicts")
        .disabled(viewModel.isBusy)
        .overlay {
            if viewModel.isBusy {
                ProgressView()
            }
        }
        .onAppear { viewModel.loadConflicts() }
    }

    // MARK: - Empty state

    private var emptyState: some View {
        ContentUnavailableView(
            "No Conflicts",
            systemImage: "checkmark.circle",
            description: Text("Your library is in sync across all devices.")
        )
    }

    // MARK: - Conflict list

    private var conflictList: some View {
        Form {
            Section {
                Text(
                    "^[\(viewModel.conflicts.count) conflict](inflect: true) need your review. Choose which version to keep."
                )
                .font(.subheadline)
                .foregroundStyle(.secondary)

                HStack {
                    Button("Keep All Local") {
                        viewModel.resolveAll(keep: .local)
                    }
                    Spacer()
                    Button("Keep All Remote") {
                        viewModel.resolveAll(keep: .remote)
                    }
                }
            }

            ForEach(viewModel.conflicts) { conflict in
                conflictSection(conflict)
            }

            if let error = viewModel.errorMessage {
                Section {
                    Label(error, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.red)
                }
            }
        }
        .formStyle(.grouped)
    }

    @ViewBuilder
    private func conflictSection(_ conflict: SyncConflict) -> some View {
        Section {
            valueRow(
                title: "This device",
                value: conflict.localValue,
                systemImage: "iphone"
            )
            Button {
                viewModel.resolve(conflict, keep: .local)
            } label: {
                Label("Keep This Device", systemImage: "checkmark.circle")
            }

            valueRow(
                title: "Other device",
                value: conflict.remoteValue,
                systemImage: "externaldrive.connected.to.line.below"
            )
            Button {
                viewModel.resolve(conflict, keep: .remote)
            } label: {
                Label("Keep Other Device", systemImage: "checkmark.circle")
            }
        } header: {
            Text(conflictTitle(conflict))
        }
    }

    private func valueRow(title: String, value: String?, systemImage: String) -> some View {
        HStack(alignment: .top) {
            Image(systemName: systemImage)
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Text(value?.isEmpty == false ? value! : "—")
                    .font(.body)
                    .textSelection(.enabled)
            }
        }
    }

    private func conflictTitle(_ conflict: SyncConflict) -> String {
        let entity = conflict.entityType.capitalized
        if let field = conflict.fieldName, !field.isEmpty {
            return "\(entity) · \(field)"
        }
        return entity
    }
}
