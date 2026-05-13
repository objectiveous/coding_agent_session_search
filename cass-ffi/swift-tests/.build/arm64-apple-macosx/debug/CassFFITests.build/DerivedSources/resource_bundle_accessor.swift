import Foundation

extension Foundation.Bundle {
    static nonisolated let module: Bundle = {
        let mainPath = Bundle.main.bundleURL.appendingPathComponent("CassFFITests_CassFFITests.bundle").path
        let buildPath = "/Users/kenpo/SoftwareProjects/collection-one/OneWorkspace/Vendor/Cass/coding_agent_session_search/cass-ffi/swift-tests/.build/arm64-apple-macosx/debug/CassFFITests_CassFFITests.bundle"

        let preferredBundle = Bundle(path: mainPath)

        guard let bundle = preferredBundle ?? Bundle(path: buildPath) else {
            // Users can write a function called fatalError themselves, we should be resilient against that.
            Swift.fatalError("could not load resource bundle: from \(mainPath) or \(buildPath)")
        }

        return bundle
    }()
}