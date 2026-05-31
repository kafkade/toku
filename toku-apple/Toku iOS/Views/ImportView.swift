import SwiftUI
import TokuKit
import UniformTypeIdentifiers

/// Import view adapted for iOS using `.fileImporter()`.
struct ImportView: View {
    @ObservedObject var viewModel: ImportViewModel
    var onImportComplete: () -> Void

    @State private var isDryRun = true
    @State private var showFilePicker = false
    @State private var lastImportPath: String?

    var body: some View {
        VStack(spacing: 24) {
            Spacer()

            Image(systemName: "arrow.down.doc")
                .font(.system(size: 48))
                .foregroundStyle(.secondary)

            Text("Import from Goodreads")
                .font(.title2)
                .fontWeight(.semibold)

            Text("Select your Goodreads library export CSV file.")
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal)

            Button {
                showFilePicker = true
            } label: {
                Label("Choose CSV File", systemImage: "doc.badge.plus")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .padding(.horizontal, 40)

            Toggle("Dry run (preview without importing)", isOn: $isDryRun)
                .padding(.horizontal, 40)

            if viewModel.isImporting {
                ProgressView("Importing…")
            }

            if let report = viewModel.report {
                reportView(report)
            }

            if let error = viewModel.errorMessage {
                Label(error, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.red)
                    .padding(.horizontal)
            }

            Spacer()
        }
        .navigationTitle("Import")
        .fileImporter(
            isPresented: $showFilePicker,
            allowedContentTypes: [
                UTType(filenameExtension: "csv") ?? .commaSeparatedText,
            ],
            allowsMultipleSelection: false
        ) { result in
            switch result {
            case .success(let urls):
                if let url = urls.first {
                    guard url.startAccessingSecurityScopedResource() else { return }
                    defer { url.stopAccessingSecurityScopedResource() }
                    let path = url.path
                    lastImportPath = path
                    viewModel.importGoodreads(path: path, dryRun: isDryRun)
                    if !isDryRun {
                        onImportComplete()
                    }
                }
            case .failure(let error):
                viewModel.errorMessage = error.localizedDescription
            }
        }
    }

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
                    Text("Skipped").foregroundStyle(.secondary)
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

            if isDryRun, let path = lastImportPath {
                Button("Import for Real") {
                    viewModel.importGoodreads(path: path, dryRun: false)
                    onImportComplete()
                }
                .buttonStyle(.borderedProminent)
            }
        }
        .padding()
        .background(.background.secondary)
        .clipShape(RoundedRectangle(cornerRadius: 12))
        .padding(.horizontal)
    }
}
