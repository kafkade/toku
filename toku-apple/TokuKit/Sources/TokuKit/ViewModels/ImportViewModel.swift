import Foundation

/// ViewModel for the import screen.
@MainActor
public final class ImportViewModel: ObservableObject {
    @Published public var report: ImportReport?
    @Published public var isImporting = false
    @Published public var errorMessage: String?

    private let ffi: TokuFFI
    private let queue = DispatchQueue(label: "dev.toku.import", qos: .userInitiated)

    public init(ffi: TokuFFI) {
        self.ffi = ffi
    }

    /// Run a Goodreads CSV import.
    public func importGoodreads(path: String, dryRun: Bool = false) {
        isImporting = true
        errorMessage = nil
        report = nil

        queue.async { [weak self] in
            guard let self else { return }
            do {
                let result = try self.ffi.importGoodreads(csvPath: path, dryRun: dryRun)
                Task { @MainActor in
                    self.report = result
                    self.isImporting = false
                }
            } catch {
                Task { @MainActor in
                    self.errorMessage = error.localizedDescription
                    self.isImporting = false
                }
            }
        }
    }
}
