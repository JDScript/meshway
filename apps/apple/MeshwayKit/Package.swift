// swift-tools-version: 6.0
import PackageDescription

let package = Package(
  name: "MeshwayKit",
  platforms: [.macOS(.v14), .iOS(.v17)],
  products: [
    .library(name: "MeshwayUI", targets: ["MeshwayUI"]),
    .library(name: "MeshwayTunnel", targets: ["MeshwayTunnel"]),
  ],
  targets: [
    .binaryTarget(name: "meshway_apple_coreFFI", path: "../build/MeshwayCore.xcframework"),
    .target(name: "MeshwayBindings", dependencies: ["meshway_apple_coreFFI"]),
    .target(name: "MeshwayCore", dependencies: ["MeshwayBindings"]),
    .target(name: "MeshwayIPC"),
    .target(name: "MeshwayModel", dependencies: ["MeshwayCore", "MeshwayIPC"]),
    .target(name: "MeshwayUI", dependencies: ["MeshwayModel"]),
    .target(name: "MeshwayTunnel", dependencies: ["MeshwayIPC"]),
    .testTarget(
      name: "MeshwayKitTests", dependencies: ["MeshwayModel", "MeshwayCore"]),
    .testTarget(
      name: "MeshwayVisualTests", dependencies: ["MeshwayUI", "MeshwayModel", "MeshwayCore"]),
  ]
)
