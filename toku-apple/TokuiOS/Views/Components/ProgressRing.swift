import SwiftUI

/// Circular progress ring indicator.
public struct ProgressRing: View {
    public let progress: Double
    private let lineWidth: CGFloat = 10

    public init(progress: Double) {
        self.progress = progress
    }

    public var body: some View {
        ZStack {
            Circle()
                .stroke(.quaternary, lineWidth: lineWidth)

            Circle()
                .trim(from: 0, to: min(progress, 1.0))
                .stroke(
                    progress >= 1.0 ? Color.green : Color.accentColor,
                    style: StrokeStyle(lineWidth: lineWidth, lineCap: .round)
                )
                .rotationEffect(.degrees(-90))
                .animation(.easeInOut(duration: 0.3), value: progress)

            VStack(spacing: 2) {
                Text("\(Int(progress * 100))%")
                    .font(.title2)
                    .fontWeight(.bold)
                    .monospacedDigit()
            }
        }
    }
}
