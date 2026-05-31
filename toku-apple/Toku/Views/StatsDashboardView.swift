import SwiftUI
import Charts
import TokuKit

/// Statistics dashboard using Swift Charts.
struct StatsDashboardView: View {
    @ObservedObject var viewModel: StatsViewModel
    @State private var selectedYear: Int? = nil

    var body: some View {
        ScrollView {
            if viewModel.isLoading {
                ProgressView("Computing statistics…")
                    .padding(.top, 40)
            } else if let stats = viewModel.stats {
                VStack(alignment: .leading, spacing: 24) {
                    summaryCards(stats)
                    if let dist = stats.ratingDistribution, !dist.isEmpty {
                        ratingChart(dist)
                    }
                    if let breakdown = stats.formatBreakdown {
                        formatChart(breakdown)
                    }
                    if let authors = stats.topAuthors, !authors.isEmpty {
                        topAuthorsChart(authors)
                    }
                    if let monthly = stats.monthlyActivity, !monthly.isEmpty {
                        monthlyChart(monthly)
                    }
                }
                .padding()
            } else if let error = viewModel.errorMessage {
                ContentUnavailableView(
                    "Error loading statistics",
                    systemImage: "chart.bar.xaxis",
                    description: Text(error)
                )
            } else {
                ContentUnavailableView(
                    "No Statistics",
                    systemImage: "chart.bar.xaxis",
                    description: Text("Add books and track reading to see statistics.")
                )
            }
        }
        .navigationTitle("Statistics")
        .toolbar {
            ToolbarItem {
                Picker("Year", selection: $selectedYear) {
                    Text("All Time").tag(nil as Int?)
                    ForEach((2020...Calendar.current.component(.year, from: Date())).reversed(),
                            id: \.self) { year in
                        Text(String(year)).tag(year as Int?)
                    }
                }
                .onChange(of: selectedYear) { _, newValue in
                    viewModel.selectedYear = newValue
                    viewModel.loadStats()
                }
            }
        }
        .onAppear { viewModel.loadStats() }
    }

    // MARK: - Summary cards

    @ViewBuilder
    private func summaryCards(_ stats: ReadingStats) -> some View {
        LazyVGrid(columns: [
            GridItem(.flexible()),
            GridItem(.flexible()),
            GridItem(.flexible()),
            GridItem(.flexible()),
        ], spacing: 16) {
            StatCard(title: "Total Books", value: "\(stats.totalBooks)", icon: "books.vertical")
            StatCard(title: "Books Read", value: "\(stats.booksRead)", icon: "checkmark.circle.fill")
            StatCard(title: "Currently Reading", value: "\(stats.booksReading)", icon: "book.fill")
            StatCard(title: "Total Pages", value: "\(stats.totalPages)", icon: "doc.text")

            if let avg = stats.averageRating {
                StatCard(title: "Avg Rating", value: String(format: "%.1f", avg / 2.0) + "★", icon: "star.fill")
            }
            if let streak = stats.currentStreak, streak > 0 {
                StatCard(title: "Current Streak", value: "\(streak) days", icon: "flame.fill")
            }
            if let longest = stats.longestStreak, longest > 0 {
                StatCard(title: "Longest Streak", value: "\(longest) days", icon: "trophy.fill")
            }
        }
    }

    // MARK: - Charts

    @ViewBuilder
    private func ratingChart(_ distribution: [Int]) -> some View {
        VStack(alignment: .leading) {
            Text("Rating Distribution")
                .font(.headline)
            Chart {
                ForEach(Array(distribution.enumerated()), id: \.offset) { index, count in
                    BarMark(
                        x: .value("Rating", "\(index)"),
                        y: .value("Books", count)
                    )
                    .foregroundStyle(.yellow.gradient)
                }
            }
            .frame(height: 200)
        }
        .padding()
        .background(.background.secondary)
        .clipShape(RoundedRectangle(cornerRadius: 12))
    }

    @ViewBuilder
    private func formatChart(_ breakdown: FormatBreakdown) -> some View {
        let data: [(String, Int)] = [
            ("Physical", breakdown.physical),
            ("E-Book", breakdown.ebook),
            ("Audiobook", breakdown.audiobook),
        ].filter { $0.1 > 0 }

        if !data.isEmpty {
            VStack(alignment: .leading) {
                Text("Format Breakdown")
                    .font(.headline)
                Chart(data, id: \.0) { item in
                    SectorMark(
                        angle: .value("Count", item.1),
                        innerRadius: .ratio(0.5),
                        angularInset: 2
                    )
                    .foregroundStyle(by: .value("Format", item.0))
                }
                .frame(height: 200)
            }
            .padding()
            .background(.background.secondary)
            .clipShape(RoundedRectangle(cornerRadius: 12))
        }
    }

    @ViewBuilder
    private func topAuthorsChart(_ authors: [AuthorCount]) -> some View {
        VStack(alignment: .leading) {
            Text("Top Authors")
                .font(.headline)
            Chart(authors.prefix(10), id: \.name) { author in
                BarMark(
                    x: .value("Books", author.count),
                    y: .value("Author", author.name)
                )
                .foregroundStyle(.blue.gradient)
            }
            .frame(height: CGFloat(min(authors.count, 10)) * 30)
        }
        .padding()
        .background(.background.secondary)
        .clipShape(RoundedRectangle(cornerRadius: 12))
    }

    @ViewBuilder
    private func monthlyChart(_ monthly: [MonthlyCount]) -> some View {
        VStack(alignment: .leading) {
            Text("Monthly Reading")
                .font(.headline)
            Chart(monthly, id: \.month) { item in
                BarMark(
                    x: .value("Month", item.month),
                    y: .value("Books", item.count)
                )
                .foregroundStyle(.green.gradient)
            }
            .frame(height: 200)
        }
        .padding()
        .background(.background.secondary)
        .clipShape(RoundedRectangle(cornerRadius: 12))
    }
}

/// A small stat card for the summary grid.
struct StatCard: View {
    let title: String
    let value: String
    let icon: String

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: icon)
                .font(.title2)
                .foregroundStyle(.tint)
            Text(value)
                .font(.title3)
                .fontWeight(.bold)
                .monospacedDigit()
            Text(title)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding()
        .background(.background.secondary)
        .clipShape(RoundedRectangle(cornerRadius: 12))
    }
}
