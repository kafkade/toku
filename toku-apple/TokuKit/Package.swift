// swift-tools-version: 5.10
import PackageDescription

let package = Package(
    name: "TokuKit",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "TokuKit", targets: ["TokuKit"]),
    ],
    targets: [
        .systemLibrary(
            name: "CTokuFFI",
            path: "Sources/CTokuFFI",
            pkgConfig: nil,
            providers: nil
        ),
        .target(
            name: "TokuKit",
            dependencies: ["CTokuFFI"],
            path: "Sources/TokuKit"
        ),
    ]
)
