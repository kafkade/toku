import SwiftUI
import TokuKit
import TokuKitUI

/// Full book detail view for iOS. Displays as a pushed view on iPhone
/// or in the detail pane on iPad.
struct BookDetailView: View {
    let bookID: String
    let ffi: TokuFFI
    @StateObject private var viewModel: BookDetailViewModel
    @State private var showProgressSheet = false

    init(bookID: String, ffi: TokuFFI) {
        self.bookID = bookID
        self.ffi = ffi
        _viewModel = StateObject(wrappedValue: BookDetailViewModel(ffi: ffi))
    }

    var body: some View {
        Group {
            if viewModel.isLoading {
                ProgressView()
            } else if let book = viewModel.book {
                ScrollView {
                    VStack(alignment: .leading, spacing: 24) {
                        headerSection(book)
                        actionButtons(book)
                        Divider()
                        metadataSection(book)
                        if !viewModel.tags.isEmpty {
                            Divider()
                            tagsSection
                        }
                    }
                    .padding()
                }
            } else if let error = viewModel.errorMessage {
                ContentUnavailableView(
                    "Error",
                    systemImage: "exclamationmark.triangle",
                    description: Text(error)
                )
            } else {
                ContentUnavailableView(
                    "No Book Selected",
                    systemImage: "book",
                    description: Text("Select a book from the library.")
                )
            }
        }
        .navigationTitle(viewModel.book?.title ?? "Book")
        .navigationBarTitleDisplayMode(.inline)
        .onChange(of: bookID) { _, newID in
            viewModel.loadBook(id: newID)
        }
        .onAppear {
            viewModel.loadBook(id: bookID)
        }
        .sheet(isPresented: $showProgressSheet) {
            if let book = viewModel.book {
                ProgressUpdateSheet(
                    book: book,
                    viewModel: nil,
                    onSave: { viewModel.loadBook(id: bookID) }
                )
                .presentationDetents([.medium])
            }
        }
    }

    // MARK: - Header

    @ViewBuilder
    private func headerSection(_ book: Book) -> some View {
        HStack(alignment: .top, spacing: 16) {
            // Cover placeholder
            RoundedRectangle(cornerRadius: 10)
                .fill(coverColor(for: book.title))
                .overlay {
                    VStack(spacing: 4) {
                        Text(book.title)
                            .font(.caption)
                            .fontWeight(.semibold)
                            .multilineTextAlignment(.center)
                            .lineLimit(3)
                    }
                    .padding(8)
                }
                .frame(width: 100, height: 150)
                .shadow(radius: 3)

            VStack(alignment: .leading, spacing: 8) {
                Text(book.title)
                    .font(.title2)
                    .fontWeight(.bold)
                if let subtitle = book.subtitle {
                    Text(subtitle)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                Text(book.authorDisplay)
                    .font(.headline)
                    .foregroundStyle(.secondary)

                HStack(spacing: 12) {
                    Label(book.status.displayName, systemImage: book.status.systemImage)
                        .font(.callout)
                        .foregroundStyle(statusColor(book.status))

                    if let stars = book.starRating {
                        StarRatingView(rating: stars)
                    }
                }
            }
        }
    }

    // MARK: - Action buttons

    @ViewBuilder
    private func actionButtons(_ book: Book) -> some View {
        HStack(spacing: 12) {
            if book.status == .reading {
                Button {
                    showProgressSheet = true
                } label: {
                    Label("Update Progress", systemImage: "bookmark.fill")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
            }

            Menu {
                ForEach(ReadingStatus.allCases) { status in
                    Button(status.displayName) {
                        updateStatus(book: book, status: status)
                    }
                }
            } label: {
                Label("Status", systemImage: "arrow.triangle.2.circlepath")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.bordered)
            .controlSize(.large)
        }
    }

    // MARK: - Metadata

    @ViewBuilder
    private func metadataSection(_ book: Book) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Details")
                .font(.headline)

            MetadataRow(label: "Format", value: book.format.displayName)

            if let pages = book.pageCount {
                MetadataRow(label: "Pages", value: "\(pages)")
            }

            if let pubDate = book.pubDate {
                MetadataRow(label: "Published", value: pubDate)
            }

            if let language = book.language {
                MetadataRow(label: "Language", value: language.uppercased())
            }
        }
    }

    // MARK: - Tags

    @ViewBuilder
    private var tagsSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Tags")
                .font(.headline)

            FlowLayout(spacing: 6) {
                ForEach(viewModel.tags) { tag in
                    Text(tag.name)
                        .font(.caption)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(.quaternary)
                        .clipShape(Capsule())
                }
            }
        }
    }

    // MARK: - Helpers

    private func updateStatus(book: Book, status: ReadingStatus) {
        let ffiRef = ffi
        Task {
            try? ffiRef.updateBookStatus(id: book.id, status: status)
            viewModel.loadBook(id: book.id)
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

    private func coverColor(for title: String) -> Color {
        let hash = abs(title.hashValue)
        let hue = Double(hash % 360) / 360.0
        return Color(hue: hue, saturation: 0.3, brightness: 0.9)
    }
}
