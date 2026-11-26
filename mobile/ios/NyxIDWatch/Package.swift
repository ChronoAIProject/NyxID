// swift-tools-version:5.9
// Host-side test harness for the Apple Watch app's Foundation-only core.
// The watch target in NyxIDMobile.xcodeproj compiles Core/ directly; this
// package only exists so `swift test` can exercise it without a simulator.
import PackageDescription

let package = Package(
  name: "NyxIDWatchCore",
  platforms: [.macOS(.v13), .watchOS(.v10)],
  products: [
    .library(name: "NyxIDWatchCore", targets: ["NyxIDWatchCore"])
  ],
  targets: [
    .target(name: "NyxIDWatchCore", path: "Core"),
    .testTarget(
      name: "NyxIDWatchCoreTests",
      dependencies: ["NyxIDWatchCore"],
      path: "CoreTests"
    ),
  ]
)
