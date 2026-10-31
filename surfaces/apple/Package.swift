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
            dependencies: ["ElseClientCore"],
            // 契约 fixture 由 Rust 契约测试共用（core/else-protocol/tests/contract.rs），
            // 两边读同一份 JSON，形状漂移会同时打断两边。
            resources: [.copy("Fixtures")]
        ),
    ],
    swiftLanguageModes: [.v5]
)
