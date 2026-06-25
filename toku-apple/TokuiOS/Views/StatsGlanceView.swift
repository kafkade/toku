import SwiftUI
import Charts
import TokuKit
import TokuKitUI

/// Stats glance view — summary card showing books read this year,
/// current streak, and reading pace.
struct StatsGlanceView: View {
    @ObservedObject var viewModel: StatsViewModel
    @State private var selectedYear: Int? = Calendar.current.component(.year, from: Date())

    var body: some View {
        ScrollView {
            if viewModel.isLoading {
                ProgressView("Computing statistics…")
                    .padding(.top, 40)
            } else if let stats = viewModel.stats {
                VStack(alignment: .leading, spacing: 20) {
                    summarySection(stats)
                    if let monthly = stats.monthlyActivity, !monthly.isEmpty {
                        monthlyChart(monthly)
                    }
                    if let dist = stats.ratingDistribution, !dist.isEmpty {
                        ratingChart(dist)
                    }
                    if let breakdown = stats.formatBreakdown {
                        formatSection(breakdown)
                    }
                    if let authors = stats.topAuthors, !authors.isEmpty {
                        topAuthorsSection(authors)
                    }
                }
                .padding()
            } else if let error = viewModel.errorMessage {
                ContentUnavailableView(
                    "Error",
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
            ToolbarItem(placement: .topBarTrailing) {
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
        .onAppear {
            viewModel.selectedYear = selectedYear
            viewModel.loadStats()
        }
    }

    // MARK: - Summary cards

    @ViewBuilder
    private func summarySection(_ stats: ReadingStats) -> some View {
        LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 12) {
            StatCard(title: "Books Read", value: "\(stats.booksRead)", icon: "checkmark.circle.fill")
            StatCard(title: "Reading Now", value: "\(stats.booksReading)", icon: "book.fill")
            StatCard(title: "Total Pages", value: "\(stats.totalPages)", icon: "doc.text")

            if let avg = stats.averageRating {
                StatCard(title: "Avg Rating", value: String(format: "%.1f", avg / 2.0) + "★", icon: "star.fill")
            }
            if let streak = stats.currentStreak, streak > 0 {
                StatCard(title: "Streak", value: "\(streak) days", icon: "flame.fill")
            }
            if let longest = stats.longestStreak, longest > 0 {
                StatCard(title: "Best Streak", value: "\(longest) days", icon: "trophy.fill")
            }
        }
    }

    // MARK: - Charts

    @ViewBuilder
    private func monthlyChart(_ monthly: [MonthlyCount]) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Monthly Reading")
                .font(.headline)
            Chart(monthly, id: \.month) { item in
                BarMark(
                    x: .value("Month", item.month),
                    y: .value("Books", item.count)
                )
                .foregroundStyle(.green.gradient)
                .cornerRadius(4)
            }
            .frame(height: 180)
        }
        .padding()
        .background(.background.secondary)
        .clipShape(RoundedRectangle(cornerRadius: 12))
    }

    @ViewBuilder
    private func ratingChart(_ distribution: [Int]) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Rating Distribution")
                .font(.headline)
            Chart {
                ForEach(Array(distribution.enumerated()), id: \.offset) { index, count in
                    BarMark(
                        x: .value("Rating", "\(index)"),
                        y: .value("Books", count)
                    )
                    .foregroundStyle(.yellow.gradient)
                    .cornerRadius(4)
                }
            }
            .frame(height: 160)
        }
        .padding()
        .background(.background.secondary)
        .clipShape(RoundedRectangle(cornerRadius: 12))
    }

    @ViewBuilder
    private func formatSection(_ breakdown: FormatBreakdown) -> some View {
        let data: [(String, Int)] = [
            ("Physical", breakdown.physical),
            ("E-Book", breakdown.ebook),
            ("Audiobook", breakdown.audiobook),
        ].filter { $0.1 > 0 }

        if !data.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
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
                .frame(height: 180)
            }
            .padding()
            .background(.background.secondary)
            .clipShape(RoundedRectangle(cornerRadius: 12))
        }
    }

    @ViewBuilder
    private func topAuthorsSection(_ authors: [AuthorCount]) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Top Authors")
                .font(.headline)

            ForEach(authors.prefix(5), id: \.name) { author in
                HStack {
                    Text(author.name)
                    Spacer()
                    Text("\(author.count) books")
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                }
                .font(.callout)
            }
        }
        .padding()
        .background(.background.secondary)
        .clipShape(RoundedRectangle(cornerRadius: 12))
    }
}
