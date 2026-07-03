import WidgetKit
import SwiftUI

/// Timeline entry carrying the shared reading snapshot.
struct TokuComplicationEntry: TimelineEntry {
    let date: Date
    let snapshot: WatchSnapshot
}

/// Provides complication timelines from the shared `WatchSnapshot` written by the
/// watch app. Reads are best-effort: when no snapshot exists yet (or the App Group
/// is not provisioned), a placeholder is shown.
struct TokuComplicationProvider: TimelineProvider {
    func placeholder(in context: Context) -> TokuComplicationEntry {
        TokuComplicationEntry(date: Date(), snapshot: .placeholder)
    }

    func getSnapshot(in context: Context, completion: @escaping (TokuComplicationEntry) -> Void) {
        completion(TokuComplicationEntry(date: Date(), snapshot: currentSnapshot()))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<TokuComplicationEntry>) -> Void) {
        let entry = TokuComplicationEntry(date: Date(), snapshot: currentSnapshot())
        // Refresh roughly hourly; the app also nudges via WidgetCenter on data changes.
        let next = Calendar.current.date(byAdding: .hour, value: 1, to: Date()) ?? Date().addingTimeInterval(3600)
        completion(Timeline(entries: [entry], policy: .after(next)))
    }

    private func currentSnapshot() -> WatchSnapshot {
        WatchSnapshotStore.load() ?? .placeholder
    }
}

/// The complication view. Adapts to the watch-face families it supports.
struct TokuComplicationView: View {
    @Environment(\.widgetFamily) private var family
    let entry: TokuComplicationEntry

    var body: some View {
        switch family {
        case .accessoryInline:
            Text(inlineText)
        case .accessoryCircular:
            circular
        default:
            rectangular
        }
    }

    private var inlineText: String {
        if let title = entry.snapshot.currentBookTitle {
            return "📖 \(title)"
        }
        return "🔥 \(entry.snapshot.currentStreak)d"
    }

    private var circular: some View {
        VStack(spacing: 0) {
            Image(systemName: "flame.fill")
                .font(.caption2)
            Text("\(entry.snapshot.currentStreak)")
                .font(.headline)
                .monospacedDigit()
        }
    }

    private var rectangular: some View {
        VStack(alignment: .leading, spacing: 1) {
            Label {
                Text(entry.snapshot.currentBookTitle ?? "No book in progress")
                    .lineLimit(1)
            } icon: {
                Image(systemName: "book.fill")
            }
            .font(.headline)

            if let author = entry.snapshot.currentBookAuthor {
                Text(author)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Text("🔥 \(entry.snapshot.currentStreak)d · \(entry.snapshot.booksThisYear) this year")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }
}

/// The complication widget: current book + reading streak on the watch face.
struct TokuComplication: Widget {
    let kind = "TokuComplication"

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: kind, provider: TokuComplicationProvider()) { entry in
            TokuComplicationView(entry: entry)
                .containerBackground(.clear, for: .widget)
        }
        .configurationDisplayName("Toku")
        .description("Your current book and reading streak.")
        .supportedFamilies([
            .accessoryInline,
            .accessoryCircular,
            .accessoryRectangular,
        ])
    }
}

@main
struct TokuComplicationBundle: WidgetBundle {
    var body: some Widget {
        TokuComplication()
    }
}
