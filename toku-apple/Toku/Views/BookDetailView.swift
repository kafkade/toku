import SwiftUI
import TokuKit

/// Inspector panel showing full book details.
struct BookDetailView: View {
    let bookID: String
    let ffi: TokuFFI
    @StateObject private var viewModel: BookDetailViewModel

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
                    VStack(alignment: .leading, spacing: 20) {
                        headerSection(book)
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
                Text("Select a book")
                    .foregroundStyle(.secondary)
            }
        }
        .frame(minWidth: 250, idealWidth: 300)
        .onChange(of: bookID) { _, newID in
            viewModel.loadBook(id: newID)
        }
        .onAppear {
            viewModel.loadBook(id: bookID)
        }
    }

    // MARK: - Sections

    @ViewBuilder
    private func headerSection(_ book: Book) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(book.title)
                .font(.title2)
                .fontWeight(.bold)
            if let subtitle = book.subtitle {
                Text(subtitle)
                    .font(.title3)
                    .foregroundStyle(.secondary)
            }
            Text(book.authorDisplay)
                .font(.headline)
                .foregroundStyle(.secondary)

            HStack(spacing: 12) {
                Label(book.status.displayName, systemImage: book.status.systemImage)
                    .font(.callout)

                if let stars = book.starRating {
                    StarRatingView(rating: stars)
                }
            }
        }
    }

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
}

/// A labeled row for metadata display.
struct MetadataRow: View {
    let label: String
    let value: String

    var body: some View {
        HStack {
            Text(label)
                .foregroundStyle(.secondary)
                .frame(width: 80, alignment: .leading)
            Text(value)
        }
        .font(.callout)
    }
}

/// Simple horizontal flow layout for tags.
struct FlowLayout: Layout {
    var spacing: CGFloat = 8

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let result = arrange(proposal: proposal, subviews: subviews)
        return result.size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let result = arrange(proposal: proposal, subviews: subviews)
        for (index, position) in result.positions.enumerated() {
            subviews[index].place(at: CGPoint(
                x: bounds.minX + position.x,
                y: bounds.minY + position.y
            ), proposal: .unspecified)
        }
    }

    private func arrange(proposal: ProposedViewSize, subviews: Subviews) -> (size: CGSize, positions: [CGPoint]) {
        let maxWidth = proposal.width ?? .infinity
        var positions: [CGPoint] = []
        var x: CGFloat = 0
        var y: CGFloat = 0
        var rowHeight: CGFloat = 0
        var maxX: CGFloat = 0

        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x + size.width > maxWidth, x > 0 {
                x = 0
                y += rowHeight + spacing
                rowHeight = 0
            }
            positions.append(CGPoint(x: x, y: y))
            rowHeight = max(rowHeight, size.height)
            x += size.width + spacing
            maxX = max(maxX, x)
        }

        return (CGSize(width: maxX, height: y + rowHeight), positions)
    }
}
