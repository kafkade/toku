import SwiftUI
import TokuKit

/// Simple star rating display (read-only).
public struct StarRatingView: View {
    public let rating: Double

    public init(rating: Double) {
        self.rating = rating
    }

    public var body: some View {
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
