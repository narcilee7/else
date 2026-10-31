// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "ElseApple",
    platforms: [
        .macOS(.v14),
    ],
    products: [
        .library(name: "ElseClientCore", targets: ["ElseClientCore"]),
        .executable(name: "ElseMac", targets: ["ElseMac"]),
        .executable(name: "ElseNotifier", targets: ["ElseNotifier"]),
    ],
    targets: [
        .target(name: "ElseClientCore"),
        .executableTarget(
            name: "ElseMac",
            dependencies: ["ElseClientCore"]
        ),
        .executableTarget(
            name: "ElseNotifier",
            dependencies: ["ElseClientCore"]
        ),
        .testTarget(
            name: "ElseClientCoreTests",
            dependencies: ["ElseClientCore"]
        ),
    ],
    swiftLanguageModes: [.v5]
)
