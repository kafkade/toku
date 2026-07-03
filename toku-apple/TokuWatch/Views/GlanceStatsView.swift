import SwiftUI
import TokuKit

/// Glanceable statistics for the wrist: books this year, current streak, pages,
/// and how many books are in progress.
struct GlanceStatsView: View {
    @EnvironmentObject var appState: WatchAppState

    var body: some View {
        ScrollView {
            if let stats = appState.statsVM?.stats {
                VStack(spacing: 8) {
                    GlanceStatRow(
                        icon: "checkmark.circle.fill",
                        tint: .green,
                        title: "Books Read",
                        value: "\(stats.booksRead)"
                    )
                    GlanceStatRow(
                        icon: "book.fill",
                        tint: .blue,
                        title: "Reading Now",
                        value: "\(stats.booksReading)"
                    )
                    if let streak = stats.currentStreak, streak > 0 {
                        GlanceStatRow(
                            icon: "flame.fill",
                            tint: .orange,
                            title: "Streak",
                            value: "\(streak)d"
                        )
                    }
                    GlanceStatRow(
                        icon: "doc.text",
                        tint: .gray,
                        title: "Total Pages",
                        value: "\(stats.totalPages)"
                    )
                    if let avg = stats.averageRating {
                        GlanceStatRow(
                            icon: "star.fill",
                            tint: .yellow,
                            title: "Avg Rating",
                            value: String(format: "%.1f★", avg / 2.0)
                        )
                    }
                }
                .padding(.horizontal, 4)
            } else if appState.statsVM?.isLoading == true {
                ProgressView()
                    .padding(.top, 24)
            } else {
                ContentUnavailableView(
                    "No Stats",
                    systemImage: "chart.bar",
                    description: Text("Track reading to see stats.")
                )
            }
        }
        .navigationTitle("Stats")
        .navigationBarTitleDisplayMode(.inline)
        .onAppear { appState.statsVM?.loadStats() }
    }
}

/// A single labelled stat row with an icon.
private struct GlanceStatRow: View {
    let icon: String
    let tint: Color
    let title: String
    let value: String

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: icon)
                .foregroundStyle(tint)
                .frame(width: 20)
            Text(title)
                .font(.footnote)
            Spacer()
            Text(value)
                .font(.headline)
                .monospacedDigit()
        }
        .padding(8)
        .background(.gray.opacity(0.15))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }
}
