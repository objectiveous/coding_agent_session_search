// swift-tools-version: 6.0
import PackageDescription

// Swift round-trip test target for the cass-ffi UniFFI bridge.
//
// Strategy: consume the staticlib + UniFFI-generated Swift module directly,
// not the CassFFI.xcframework that the parent OneApp workspace builds. The
// xcframework is a packaging artifact owned by OneApp; cass-ffi itself
// stays buildable + testable standalone.
//
// Prereqs (or run `mise run cassffi-setup` from the OneApp workspace):
//   1. From cass-ffi/: cargo build --release --lib
//      (produces target/release/libcass_ffi.{a,dylib} + uniffi-bindgen)
//   2. ./target/release/uniffi-bindgen generate
//        --library ./target/release/libcass_ffi.dylib --language swift
//        --out-dir ./generated
//   3. cp generated/cass_ffi.swift swift-tests/Sources/CassFFIBindings/
//      cp generated/cass_ffiFFI.h swift-tests/Sources/cass_ffiFFI/
//      cp generated/cass_ffiFFI.modulemap swift-tests/Sources/cass_ffiFFI/module.modulemap
//
// Two targets, intentional split:
//   - cass_ffiFFI: a SystemLibrary target whose only role is to expose the
//     UniFFI-emitted C header as a Clang module literally named `cass_ffiFFI`.
//     The generated swift file does `#if canImport(cass_ffiFFI) import cass_ffiFFI`,
//     so the module name has to match exactly — SwiftPM regular targets
//     name the module after the target, but SystemLibrary lets us use the
//     name from the modulemap directly.
//   - CassFFIBindings: contains the UniFFI-generated cass_ffi.swift. Depends
//     on cass_ffiFFI for the C symbols. Links the staticlib + libc++/libz
//     for the cass transitives' C++/zlib symbols.

let package = Package(
    name: "CassFFITests",
    platforms: [.macOS(.v14)],
    targets: [
        .systemLibrary(
            name: "cass_ffiFFI",
            path: "Sources/cass_ffiFFI"
        ),
        .target(
            name: "CassFFIBindings",
            dependencies: ["cass_ffiFFI"],
            path: "Sources/CassFFIBindings",
            linkerSettings: [
                .unsafeFlags([
                    "-L../target/release",
                    "-lcass_ffi",
                    "-lc++",
                    "-lz"
                ])
            ]
        ),
        .testTarget(
            name: "CassFFITests",
            dependencies: ["CassFFIBindings"],
            path: "Tests/CassFFITests"
            // Fixture is reached via #filePath at runtime (see
            // CassFFITests.swift `fixtureRoot()`).
        )
    ]
)
