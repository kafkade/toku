import SwiftUI
import TokuKit
import TokuKitUI

/// Dedicated search view with recent searches and results.
struct SearchView: View {
    @ObservedObject var viewModel: LibraryViewModel
    let ffi: TokuFFI?
    @State private var searchText = ""
    @State private var searchResults: [Book] = []
    @State private var isSearching = false

    var body: some View {
        List {
            if searchText.isEmpty {
                Section {
                    ContentUnavailableView(
                        "Search Your Library",
                        systemImage: "magnifyingglass",
                        description: Text("Search by title, author, or keyword.")
                    )
                }
            } else if isSearching {
                Section {
                    HStack {
                        ProgressView()
                        Text("Searching…")
                            .foregroundStyle(.secondary)
                    }
                }
            } else if searchResults.isEmpty {
                Section {
                    ContentUnavailableView(
                        "No Results",
                        systemImage: "magnifyingglass",
                        description: Text("No books match \"\(searchText)\".")
                    )
                }
            } else {
                Section("\(searchResults.count) Results") {
                    ForEach(searchResults) { book in
                        NavigationLink(value: book.id) {
                            SearchResultRow(book: book)
                        }
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
        .navigationTitle("Search")
        .navigationDestination(for: String.self) { bookID in
            if let ffi = ffi {
                BookDetailView(bookID: bookID, ffi: ffi)
            }
        }
        .searchable(text: $searchText, prompt: "Search books…")
        .onSubmit(of: .search) {
            performSearch()
        }
        .onChange(of: searchText) { _, newValue in
            if newValue.isEmpty {
                searchResults = []
            }
        }
    }

    private func performSearch() {
        guard !searchText.isEmpty, let ffi = ffi else { return }
        isSearching = true

        Task {
            do {
                let results = try ffi.searchBooks(query: searchText)
                searchResults = results
            } catch {
                searchResults = []
            }
            isSearching = false
        }
    }
}

/// A compact row displaying a search result.
struct SearchResultRow: View {
    let book: Book

    var body: some View {
        HStack(spacing: 12) {
            // Mini cover
            RoundedRectangle(cornerRadius: 6)
                .fill(coverColor(for: book.title))
                .frame(width: 44, height: 64)
                .overlay {
                    Text(book.title.prefix(1))
                        .font(.title3)
                        .fontWeight(.bold)
                        .foregroundStyle(.white.opacity(0.7))
                }

            VStack(alignment: .leading, spacing: 4) {
                Text(book.title)
                    .font(.body)
                    .fontWeight(.medium)
                    .lineLimit(1)
                Text(book.authorDisplay)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                HStack(spacing: 8) {
                    Label(book.status.displayName, systemImage: book.status.systemImage)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    if let stars = book.starRating {
                        StarRatingView(rating: stars)
                    }
                }
            }
        }
        .padding(.vertical, 4)
    }

    private func coverColor(for title: String) -> Color {
        let hash = abs(title.hashValue)
        let hue = Double(hash % 360) / 360.0
        return Color(hue: hue, saturation: 0.3, brightness: 0.9)
    }
}
