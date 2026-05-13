import Foundation
import Testing
@testable import CassFFIBindings

/// Live Swift round-trip tests for the cass-ffi UniFFI bridge.
///
/// Mirrors `cass-ffi/tests/integration.rs` cross-language: same fixture
/// (`tests/fixtures/search_demo_data`), same queries, same assertions.
/// Divergent results between Rust and Swift sides indicate an FFI bug
/// rather than a fixture bug.
///
/// No mocks — every test opens a real `CassEngine` against a real cass
/// data dir per CLAUDE.md's no-mocks policy.

/// Resolves the shared cass-ffi/tests/fixtures/search_demo_data fixture
/// relative to this source file. Avoiding SwiftPM resources keeps the
/// fixture single-source-of-truth shared with the Rust integration tests
/// (cass-ffi/tests/integration.rs uses the exact same dir).
private func fixtureRoot() -> URL {
    let thisFile = URL(fileURLWithPath: #filePath)
    // .../cass-ffi/swift-tests/Tests/CassFFITests/CassFFITests.swift
    //   -> ../../../tests/fixtures/search_demo_data
    return thisFile
        .deletingLastPathComponent()        // Tests/CassFFITests/
        .deletingLastPathComponent()        // Tests/
        .deletingLastPathComponent()        // swift-tests/
        .deletingLastPathComponent()        // cass-ffi/
        .appendingPathComponent("tests")
        .appendingPathComponent("fixtures")
        .appendingPathComponent("search_demo_data")
}

private func copyFixtureToTempdir() throws -> URL {
    let fm = FileManager.default
    let tempdir = fm.temporaryDirectory.appendingPathComponent(
        "cassffi-test-\(UUID().uuidString)",
        isDirectory: true
    )
    try fm.createDirectory(at: tempdir, withIntermediateDirectories: true)
    try copyRecursive(from: fixtureRoot(), to: tempdir)
    return tempdir
}

private func copyRecursive(from src: URL, to dst: URL) throws {
    let fm = FileManager.default
    let contents = try fm.contentsOfDirectory(
        at: src,
        includingPropertiesForKeys: [.isDirectoryKey],
        options: []
    )
    for child in contents {
        let target = dst.appendingPathComponent(child.lastPathComponent)
        let isDirectory = (try child.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) ?? false
        if isDirectory {
            try fm.createDirectory(at: target, withIntermediateDirectories: true)
            try copyRecursive(from: child, to: target)
        } else {
            try fm.copyItem(at: child, to: target)
        }
    }
}


@Test
func probeVersionRoundTrips() async throws {
    let engine = CassEngine.probe()
    #expect(engine.version() == "0.1.0")
}

@Test
func openAndLexicalSearchReturnsHit() async throws {
    let dataDir = try copyFixtureToTempdir()
    defer { try? FileManager.default.removeItem(at: dataDir) }

    let engine = try CassEngine.open(dataDir: dataDir.path)
    let hits = try await engine.search(
        query: "aider",
        opts: .lexical(limit: 10)
    )
    #expect(!hits.isEmpty)
    if let first = hits.first {
        #expect(first.content.lowercased().contains("aider"))
        #expect(!first.sourcePath.isEmpty)
    }
}

@Test
func closeYieldsEngineClosedError() async throws {
    let dataDir = try copyFixtureToTempdir()
    defer { try? FileManager.default.removeItem(at: dataDir) }

    let engine = try CassEngine.open(dataDir: dataDir.path)
    try engine.close()

    do {
        _ = try await engine.search(query: "aider", opts: .lexical(limit: 10))
        Issue.record("expected search-after-close to throw, got success")
    } catch let error as CassError {
        switch error {
        case .Failed(_, let kind, _, _, _):
            #expect(kind == "engine-closed")
        }
    }
}

@Test
func listWorkspacesRoundTrips() async throws {
    let dataDir = try copyFixtureToTempdir()
    defer { try? FileManager.default.removeItem(at: dataDir) }

    let engine = try CassEngine.open(dataDir: dataDir.path)
    let workspaces = try await engine.listWorkspaces()

    #expect(!workspaces.isEmpty)
    if let first = workspaces.first {
        #expect(!first.path.isEmpty)
    }
}

@Test
func loadConversationRoundTrips() async throws {
    let dataDir = try copyFixtureToTempdir()
    defer { try? FileManager.default.removeItem(at: dataDir) }

    let engine = try CassEngine.open(dataDir: dataDir.path)
    let hits = try await engine.search(query: "aider", opts: .lexical(limit: 1))
    #expect(!hits.isEmpty)
    guard let first = hits.first else { return }

    let conversation = try await engine.loadConversation(sourcePath: first.sourcePath)
    #expect(conversation != nil)
    if let conversation {
        #expect(conversation.sourcePath == first.sourcePath)
        #expect(!conversation.messages.isEmpty)
        #expect(!conversation.agentSlug.isEmpty)
    }
}

@Test
func expandAroundReturnsAWindow() async throws {
    let dataDir = try copyFixtureToTempdir()
    defer { try? FileManager.default.removeItem(at: dataDir) }

    let engine = try CassEngine.open(dataDir: dataDir.path)
    let hits = try await engine.search(query: "aider", opts: .lexical(limit: 1))
    guard let first = hits.first else {
        Issue.record("expected at least one hit for 'aider' fixture")
        return
    }
    let conversation = try await engine.loadConversation(sourcePath: first.sourcePath)
    guard let conversation, let conversationId = conversation.id,
          let anchorIdx = conversation.messages.first?.idx else {
        Issue.record("expected conversation with id and at least one message")
        return
    }

    let window = try await engine.expandAround(
        conversationId: conversationId,
        messageIdx: anchorIdx,
        before: 0,
        after: 1
    )
    #expect(!window.isEmpty)
    #expect(window.first?.idx == anchorIdx)
}
