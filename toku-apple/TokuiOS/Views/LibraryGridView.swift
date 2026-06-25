import SwiftUI
import TokuKit
import TokuKitUI

/// Responsive cover grid view for the book library.
///
/// On iPhone, shows 2 columns. On iPad, adapts to 3–4+ columns.
/// Supports search, status filter chips, pull-to-refresh, and
/// a visible progress button on currently-reading books.
struct LibraryGridView: View {
    @ObservedObject var viewModel: LibraryViewModel
    let ffi: TokuFFI?
    @State private var selectedBookID: String?
    @State private var progressBook: Book?
    @State private var statusFilter: ReadingStatus?

    private var columns: [GridItem] {
        [GridItem(.adaptive(minimum: 140, maximum: 180), spacing: 16)]
    }

    private var filteredBooks: [Book] {
        guard let filter = statusFilter else { return viewModel.books }
        return viewModel.books.filter { $0.status == filter }
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 0) {
                filterChips
                    .padding(.horizontal)
                    .padding(.bottom, 8)

                LazyVGrid(columns: columns, spacing: 20) {
                    ForEach(filteredBooks) { book in
                        NavigationLink(value: book.id) {
                            BookCard(book: book, onProgressTap: {
                                progressBook = book
                            })
                        }
                        .buttonStyle(.plain)
                        .contextMenu {
                            ForEach(ReadingStatus.allCases) { status in
                                Button(status.displayName) {
                                    viewModel.updateStatus(id: book.id, status: status)
                                }
                            }
                            Divider()
                            Button("Delete", role: .destructive) {
                                viewModel.deleteBook(id: book.id)
                            }
                        }
                    }
                }
                .padding(.horizontal)
            }
        }
        .refreshable {
            viewModel.loadBooks()
        }
        .overlay {
            if viewModel.isLoading {
                ProgressView("Loading…")
            } else if viewModel.books.isEmpty {
                ContentUnavailableView(
                    "No Books",
                    systemImage: "books.vertical",
                    description: Text("Add books or import from Goodreads to get started.")
                )
            }
        }
        .navigationTitle("Library")
        .navigationDestination(for: String.self) { bookID in
            if let ffi = ffi {
                BookDetailView(bookID: bookID, ffi: ffi)
            }
        }
        .searchable(
            text: Binding(
                get: { viewModel.searchText },
                set: { newValue in
                    viewModel.searchText = newValue
                    viewModel.loadBooks()
                }
            ),
            prompt: "Search books…"
        )
        .sheet(item: $progressBook) { book in
            ProgressUpdateSheet(book: book, viewModel: viewModel)
                .presentationDetents([.medium])
        }
        .onAppear { viewModel.loadBooks() }
    }

    // MARK: - Filter chips

    @ViewBuilder
    private var filterChips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                FilterChip(title: "All", isActive: statusFilter == nil) {
                    statusFilter = nil
                }
                ForEach(ReadingStatus.allCases) { status in
                    FilterChip(
                        title: status.displayName,
                        isActive: statusFilter == status
                    ) {
                        statusFilter = (statusFilter == status) ? nil : status
                    }
                }
            }
        }
    }
}

/// A single book card in the grid with an optional quick-progress button.
struct BookCard: View {
    let book: Book
    var onProgressTap: (() -> Void)?

    var body: some View {
        VStack(spacing: 8) {
            // Cover placeholder
            ZStack(alignment: .bottomTrailing) {
                RoundedRectangle(cornerRadius: 10)
                    .fill(coverColor(for: book.title))
                    .overlay {
                        VStack(spacing: 4) {
                            Text(book.title)
                                .font(.caption)
                                .fontWeight(.semibold)
                                .multilineTextAlignment(.center)
                                .lineLimit(3)
                            Text(book.authorDisplay)
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                        .padding(8)
                    }
                    .frame(height: 200)

                // Quick progress button for currently-reading books
                if book.status == .reading, let onTap = onProgressTap {
                    Button {
                        onTap()
                    } label: {
                        Image(systemName: "bookmark.fill")
                            .font(.caption)
                            .foregroundStyle(.white)
                            .padding(6)
                            .background(.tint, in: Circle())
                    }
                    .padding(6)
                }
            }

            // Status badge
            Label(book.status.displayName, systemImage: book.status.systemImage)
                .font(.caption2)
                .foregroundStyle(.secondary)

            // Rating
            if let stars = book.starRating {
                StarRatingView(rating: stars)
            }
        }
        .frame(width: 160)
    }

    private func coverColor(for title: String) -> Color {
        let hash = abs(title.hashValue)
        let hue = Double(hash % 360) / 360.0
        return Color(hue: hue, saturation: 0.3, brightness: 0.9)
    }
}

/// A tappable filter chip for status filtering.
struct FilterChip: View {
    let title: String
    let isActive: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(title)
                .font(.subheadline)
                .fontWeight(isActive ? .semibold : .regular)
                .padding(.horizontal, 14)
                .padding(.vertical, 7)
                .background(isActive ? Color.accentColor : Color(.secondarySystemBackground))
                .foregroundStyle(isActive ? .white : .primary)
                .clipShape(Capsule())
        }
        .buttonStyle(.plain)
    }
}
