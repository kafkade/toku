import SwiftUI
import TokuKit
import UniformTypeIdentifiers

/// Import view with file picker and drag-and-drop support.
struct ImportView: View {
    @ObservedObject var viewModel: ImportViewModel
    var onImportComplete: () -> Void

    @State private var isDryRun = true
    @State private var isDropTargeted = false

    var body: some View {
        VStack(spacing: 24) {
            // Drop zone
            VStack(spacing: 16) {
                Image(systemName: "arrow.down.doc")
                    .font(.system(size: 48))
                    .foregroundStyle(isDropTargeted ? .accentColor : .secondary)

                Text("Drop a Goodreads CSV here")
                    .font(.title3)
                    .fontWeight(.medium)

                Text("or")
                    .foregroundStyle(.secondary)

                Button("Choose File…") {
                    openFilePicker()
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut("i", modifiers: .command)
            }
            .frame(maxWidth: .infinity)
            .padding(40)
            .background(
                RoundedRectangle(cornerRadius: 16)
                    .strokeBorder(
                        isDropTargeted ? Color.accentColor : .secondary.opacity(0.3),
                        style: StrokeStyle(lineWidth: 2, dash: [8])
                    )
            )
            .onDrop(of: [.fileURL], isTargeted: $isDropTargeted) { providers in
                handleDrop(providers)
            }

            // Options
            Toggle("Dry run (preview without importing)", isOn: $isDryRun)
                .toggleStyle(.checkbox)

            // Progress / Results
            if viewModel.isImporting {
                ProgressView("Importing…")
            }

            if let report = viewModel.report {
                reportView(report)
            }

            if let error = viewModel.errorMessage {
                Label(error, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.red)
            }

            Spacer()
        }
        .padding()
        .navigationTitle("Import")
    }

    // MARK: - Import report

    @ViewBuilder
    private func reportView(_ report: ImportReport) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(
                isDryRun ? "Dry Run Complete" : "Import Complete",
                systemImage: isDryRun ? "eye" : "checkmark.circle.fill"
            )
            .font(.headline)
            .foregroundStyle(isDryRun ? .orange : .green)

            Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 8) {
                GridRow {
                    Text("Total rows").foregroundStyle(.secondary)
                    Text("\(report.totalRows)").fontWeight(.medium)
                }
                GridRow {
                    Text("Imported").foregroundStyle(.secondary)
                    Text("\(report.imported)").fontWeight(.medium).foregroundStyle(.green)
                }
                GridRow {
                    Text("Skipped (duplicates)").foregroundStyle(.secondary)
                    Text("\(report.skipped)").fontWeight(.medium)
                }
                GridRow {
                    Text("Updated").foregroundStyle(.secondary)
                    Text("\(report.updated)").fontWeight(.medium).foregroundStyle(.blue)
                }
                if report.errors > 0 {
                    GridRow {
                        Text("Errors").foregroundStyle(.secondary)
                        Text("\(report.errors)").fontWeight(.medium).foregroundStyle(.red)
                    }
                }
            }

            if isDryRun {
                Button("Import for Real") {
                    // Re-run with dry_run = false — but we need the path again
                    // In a real app we'd store the last-used path
                }
                .buttonStyle(.borderedProminent)
                .disabled(true) // TODO: store path for re-import
            }
        }
        .padding()
        .background(.background.secondary)
        .clipShape(RoundedRectangle(cornerRadius: 12))
    }

    // MARK: - File handling

    private func openFilePicker() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [
            UTType(filenameExtension: "csv") ?? .commaSeparatedText,
        ]
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.message = "Select your Goodreads library export CSV"

        if panel.runModal() == .OK, let url = panel.url {
            runImport(path: url.path)
        }
    }

    private func handleDrop(_ providers: [NSItemProvider]) -> Bool {
        guard let provider = providers.first else { return false }
        provider.loadItem(forTypeIdentifier: "public.file-url", options: nil) { data, _ in
            guard let data = data as? Data,
                  let url = URL(dataRepresentation: data, relativeTo: nil) else { return }
            Task { @MainActor in
                runImport(path: url.path)
            }
        }
        return true
    }

    private func runImport(path: String) {
        viewModel.importGoodreads(path: path, dryRun: isDryRun)
        if !isDryRun {
            onImportComplete()
        }
    }
}
