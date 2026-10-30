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
    ],
    targets: [
        .target(name: "ElseClientCore"),
        .executableTarget(
            name: "ElseMac",
            dependencies: ["ElseClientCore"]
        ),
        .testTarget(
            name: "ElseClientCoreTests",
            dependencies: ["ElseClientCore"]
        ),
    ],
    swiftLanguageModes: [.v5]
)
