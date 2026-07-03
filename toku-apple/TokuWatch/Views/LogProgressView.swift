import SwiftUI
import TokuKit

/// Quick progress + status logging for a single book, driven by the Digital Crown.
///
/// Saving records an append-only page-progress entry via `logProgress` and marks the
/// book as `reading`. Quick actions let the user mark the book finished or move it to
/// another status straight from the wrist.
struct LogProgressView: View {
    let book: Book
    @EnvironmentObject var appState: WatchAppState
    @Environment(\.dismiss) private var dismiss

    @State private var page: Double = 0
    @State private var crownPage: Double = 0

    private var totalPages: Int { book.pageCount ?? 0 }

    private var progressFraction: Double {
        guard totalPages > 0 else { return 0 }
        return min(1, max(0, page / Double(totalPages)))
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 12) {
                header

                if totalPages > 0 {
                    crownDial
                } else {
                    Text("No page count set")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }

                Button {
                    saveProgress()
                } label: {
                    Label("Save Progress", systemImage: "square.and.arrow.down")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .disabled(totalPages == 0)

                quickActions
            }
            .padding(.horizontal, 4)
        }
        .navigationTitle("Log")
        .navigationBarTitleDisplayMode(.inline)
    }

    // MARK: - Sections

    private var header: some View {
        VStack(spacing: 2) {
            Text(book.title)
                .font(.headline)
                .multilineTextAlignment(.center)
                .lineLimit(2)
            Text(book.authorDisplay)
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }

    private var crownDial: some View {
        VStack(spacing: 6) {
            ZStack {
                Circle()
                    .stroke(.gray.opacity(0.25), lineWidth: 8)
                Circle()
                    .trim(from: 0, to: progressFraction)
                    .stroke(.green.gradient, style: StrokeStyle(lineWidth: 8, lineCap: .round))
                    .rotationEffect(.degrees(-90))
                    .animation(.easeInOut, value: progressFraction)
                VStack(spacing: 0) {
                    Text("\(Int(page))")
                        .font(.title2).monospacedDigit()
                    Text("of \(totalPages)")
                        .font(.caption2).foregroundStyle(.secondary)
                }
            }
            .frame(width: 96, height: 96)
            .focusable()
            .digitalCrownRotation(
                $crownPage,
                from: 0,
                through: Double(totalPages),
                by: 1,
                sensitivity: .medium,
                isContinuous: false,
                isHapticFeedbackEnabled: true
            )
            .onChange(of: crownPage) { _, newValue in
                page = newValue.rounded()
            }

            Text("\(Int(progressFraction * 100))%")
                .font(.caption).foregroundStyle(.secondary).monospacedDigit()
        }
    }

    private var quickActions: some View {
        VStack(spacing: 6) {
            Button {
                updateStatus(.read)
            } label: {
                Label("Mark Finished", systemImage: "checkmark.circle.fill")
                    .frame(maxWidth: .infinity)
            }
            .tint(.green)

            Button {
                updateStatus(.onHold)
            } label: {
                Label("On Hold", systemImage: "pause.circle")
                    .frame(maxWidth: .infinity)
            }
            .tint(.orange)
        }
        .buttonStyle(.bordered)
        .padding(.top, 4)
    }

    // MARK: - Actions

    private func saveProgress() {
        // Record an append-only page progress entry; also ensure the book is marked
        // as reading so the currently-reading list and complication stay accurate.
        if book.status != .reading {
            appState.libraryVM?.updateStatus(id: book.id, status: .reading)
        }
        appState.libraryVM?.logProgress(id: book.id, type: .page, value: Int(page)) {
            appState.refresh()
        }
        appState.updateComplicationSnapshot()
        dismiss()
    }

    private func updateStatus(_ status: ReadingStatus) {
        appState.libraryVM?.updateStatus(id: book.id, status: status)
        appState.updateComplicationSnapshot()
        dismiss()
    }
}
