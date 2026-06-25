import SwiftUI
import TokuKit

/// Quick progress update sheet — designed for ≤3 taps from the library view.
///
/// Flow: Tap progress button on card (1) → adjust page (2) → tap Save (3).
///
/// Note: Full reading progress logging requires `toku_log_progress` FFI function
/// which is available in toku-db but not yet exposed via toku-ffi. For now, this
/// updates the book's reading status to "reading" as a side effect.
struct ProgressUpdateSheet: View {
    let book: Book
    var viewModel: LibraryViewModel?
    var onSave: (() -> Void)?
    @Environment(\.dismiss) private var dismiss

    @State private var currentPage: Int
    @State private var isSaving = false

    init(book: Book, viewModel: LibraryViewModel? = nil, onSave: (() -> Void)? = nil) {
        self.book = book
        self.viewModel = viewModel
        self.onSave = onSave
        _currentPage = State(initialValue: 0)
    }

    private var totalPages: Int {
        book.pageCount ?? 0
    }

    private var progressFraction: Double {
        guard totalPages > 0 else { return 0 }
        return Double(currentPage) / Double(totalPages)
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 24) {
                // Book info
                VStack(spacing: 4) {
                    Text(book.title)
                        .font(.headline)
                        .multilineTextAlignment(.center)
                    Text(book.authorDisplay)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }

                // Progress ring
                ProgressRing(progress: progressFraction)
                    .frame(width: 120, height: 120)

                // Page input
                VStack(spacing: 12) {
                    if totalPages > 0 {
                        Text("Page \(currentPage) of \(totalPages)")
                            .font(.title3)
                            .monospacedDigit()

                        Slider(
                            value: Binding(
                                get: { Double(currentPage) },
                                set: { currentPage = Int($0) }
                            ),
                            in: 0...Double(totalPages),
                            step: 1
                        )
                        .padding(.horizontal)

                        HStack {
                            Button("-10") { currentPage = max(0, currentPage - 10) }
                                .buttonStyle(.bordered)
                            Button("-1") { currentPage = max(0, currentPage - 1) }
                                .buttonStyle(.bordered)
                            Spacer()
                            Button("+1") { currentPage = min(totalPages, currentPage + 1) }
                                .buttonStyle(.bordered)
                            Button("+10") { currentPage = min(totalPages, currentPage + 10) }
                                .buttonStyle(.bordered)
                        }
                        .padding(.horizontal)
                    } else {
                        Text("No page count set for this book")
                            .foregroundStyle(.secondary)

                        HStack {
                            Text("Page:")
                            TextField("Current page", value: $currentPage, format: .number)
                                .keyboardType(.numberPad)
                                .textFieldStyle(.roundedBorder)
                                .frame(width: 100)
                        }
                    }
                }

                Spacer()
            }
            .padding()
            .navigationTitle("Update Progress")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        saveProgress()
                    }
                    .fontWeight(.semibold)
                    .disabled(isSaving)
                }
            }
        }
    }

    private func saveProgress() {
        isSaving = true

        // Ensure the book is marked as "reading" if it isn't already
        if book.status != .reading {
            viewModel?.updateStatus(id: book.id, status: .reading)
        }

        // TODO: Call toku_log_progress FFI function when available.
        // Currently, toku-db has `repo.log_progress()` but toku-ffi
        // does not expose it. For now, we update the status only.

        onSave?()
        dismiss()
    }
}
