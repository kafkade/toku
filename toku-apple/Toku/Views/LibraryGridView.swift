import SwiftUI
import TokuKit

/// Cover grid view as an alternative to the table.
struct LibraryGridView: View {
    @ObservedObject var viewModel: LibraryViewModel
    @Binding var selectedBookID: String?

    private let columns = [
        GridItem(.adaptive(minimum: 140, maximum: 180), spacing: 16),
    ]

    var body: some View {
        ScrollView {
            LazyVGrid(columns: columns, spacing: 20) {
                ForEach(viewModel.books) { book in
                    BookGridItem(book: book, isSelected: selectedBookID == book.id)
                        .onTapGesture {
                            selectedBookID = book.id
                        }
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
            .padding()
        }
        .overlay {
            if viewModel.isLoading {
                ProgressView("Loading…")
            } else if viewModel.books.isEmpty {
                ContentUnavailableView(
                    "No Books",
                    systemImage: "books.vertical",
                    description: Text("Add books to get started.")
                )
            }
        }
        .navigationTitle("Library")
        .onAppear { viewModel.loadBooks() }
    }
}

/// A single book card in the grid.
struct BookGridItem: View {
    let book: Book
    let isSelected: Bool

    var body: some View {
        VStack(spacing: 8) {
            // Placeholder cover
            RoundedRectangle(cornerRadius: 8)
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
                .shadow(radius: isSelected ? 4 : 1)
                .overlay(
                    RoundedRectangle(cornerRadius: 8)
                        .stroke(isSelected ? Color.accentColor : .clear, lineWidth: 3)
                )

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

    /// Generate a deterministic pastel color from the title.
    private func coverColor(for title: String) -> Color {
        let hash = abs(title.hashValue)
        let hue = Double(hash % 360) / 360.0
        return Color(hue: hue, saturation: 0.3, brightness: 0.9)
    }
}
