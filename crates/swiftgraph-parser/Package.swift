// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "SwiftGraphParser",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "swiftgraph-parser", targets: ["SwiftGraphParser"]),
    ],
    dependencies: [
        .package(url: "https://github.com/swiftlang/swift-syntax.git", from: "600.0.0"),
    ],
    targets: [
        .executableTarget(
            name: "SwiftGraphParser",
            dependencies: [
                .product(name: "SwiftParser", package: "swift-syntax"),
                .product(name: "SwiftSyntax", package: "swift-syntax"),
            ]
        ),
    ]
)
