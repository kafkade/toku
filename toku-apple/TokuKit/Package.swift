// swift-tools-version: 5.10
import PackageDescription

let package = Package(
    name: "TokuKit",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [
        .library(name: "TokuKit", targets: ["TokuKit"]),
        .library(name: "TokuKitUI", targets: ["TokuKitUI"]),
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
        .target(
            name: "TokuKitUI",
            dependencies: ["TokuKit"],
            path: "Sources/TokuKitUI"
        ),
    ]
)
