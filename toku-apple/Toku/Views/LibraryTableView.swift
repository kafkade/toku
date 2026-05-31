import SwiftUI
import TokuKit

/// Multi-column table view for the library with sortable columns.
struct LibraryTableView: View {
    @ObservedObject var viewModel: LibraryViewModel
    @Binding var selectedBookID: String?
    @State private var sortOrder = [KeyPathComparator(\Book.title)]

    var body: some View {
        Table(viewModel.books, selection: $selectedBookID, sortOrder: $sortOrder) {
            TableColumn("Title", value: \.title) { book in
                VStack(alignment: .leading, spacing: 2) {
                    Text(book.title)
                        .fontWeight(.medium)
                    if let subtitle = book.subtitle {
                        Text(subtitle)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .width(min: 150, ideal: 250)

            TableColumn("Author", value: \.authorDisplay)
                .width(min: 100, ideal: 180)

            TableColumn("Status") { book in
                Label(book.status.displayName, systemImage: book.status.systemImage)
                    .foregroundStyle(statusColor(book.status))
            }
            .width(min: 80, ideal: 120)

            TableColumn("Rating") { book in
                if let stars = book.starRating {
                    StarRatingView(rating: stars)
                } else {
                    Text("—")
                        .foregroundStyle(.tertiary)
                }
            }
            .width(min: 80, ideal: 100)

            TableColumn("Format") { book in
                Text(book.format.displayName)
                    .foregroundStyle(.secondary)
            }
            .width(min: 60, ideal: 80)

            TableColumn("Pages") { book in
                if let pages = book.pageCount {
                    Text("\(pages)")
                        .monospacedDigit()
                } else {
                    Text("—")
                        .foregroundStyle(.tertiary)
                }
            }
            .width(min: 50, ideal: 60)
        }
        .onChange(of: sortOrder) { _, _ in
            viewModel.books.sort(using: sortOrder)
        }
        .contextMenu(forSelectionType: String.self) { ids in
            if let id = ids.first {
                Button("Delete Book", role: .destructive) {
                    viewModel.deleteBook(id: id)
                }
            }
        }
        .overlay {
            if viewModel.isLoading {
                ProgressView("Loading…")
            } else if viewModel.books.isEmpty {
                ContentUnavailableView(
                    "No Books",
                    systemImage: "books.vertical",
                    description: Text(viewModel.searchText.isEmpty
                        ? "Add books to get started."
                        : "No results for \"\(viewModel.searchText)\".")
                )
            }
        }
        .navigationTitle("Library")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    ForEach(LibraryViewModel.SortOrder.allCases) { order in
                        Button(order.displayName) {
                            viewModel.sortOrder = order
                            viewModel.loadBooks()
                        }
                    }
                } label: {
                    Label("Sort", systemImage: "arrow.up.arrow.down")
                }
            }
        }
        .onAppear {
            viewModel.loadBooks()
        }
    }

    private func statusColor(_ status: ReadingStatus) -> Color {
        switch status {
        case .wantToRead: return .blue
        case .reading: return .green
        case .read: return .secondary
        case .onHold: return .orange
        case .didNotFinish: return .red
        }
    }
}

/// Simple star rating display (read-only).
struct StarRatingView: View {
    let rating: Double

    var body: some View {
        HStack(spacing: 1) {
            ForEach(1...5, id: \.self) { star in
                let filled = Double(star) <= rating
                let half = Double(star) - 0.5 == rating
                Image(systemName: filled ? "star.fill" : (half ? "star.leadinghalf.filled" : "star"))
                    .foregroundStyle(.yellow)
                    .font(.caption2)
            }
        }
    }
}
