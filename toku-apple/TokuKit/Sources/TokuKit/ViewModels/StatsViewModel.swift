import Foundation

/// ViewModel for the statistics dashboard.
@MainActor
public final class StatsViewModel: ObservableObject {
    @Published public var stats: ReadingStats?
    @Published public var selectedYear: Int?
    @Published public var isLoading = false
    @Published public var errorMessage: String?

    private let ffi: TokuFFI
    private let queue = DispatchQueue(label: "dev.toku.stats", qos: .userInitiated)

    public init(ffi: TokuFFI) {
        self.ffi = ffi
    }

    /// Load statistics, optionally scoped to a year.
    public func loadStats() {
        isLoading = true
        errorMessage = nil

        queue.async { [weak self] in
            guard let self else { return }
            do {
                let result = try self.ffi.getStats(year: self.selectedYear)
                Task { @MainActor in
                    self.stats = result
                    self.isLoading = false
                }
            } catch {
                Task { @MainActor in
                    self.errorMessage = error.localizedDescription
                    self.isLoading = false
                }
            }
        }
    }
}
