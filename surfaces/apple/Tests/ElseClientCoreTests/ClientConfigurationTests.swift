import Foundation
import Testing
@testable import ElseClientCore

@Suite("Client configuration")
struct ClientConfigurationTests {
    @Test
    func environmentTokenTakesPriority() throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        try writeToken("file-token", in: directory, permissions: 0o600)

        let configuration = ClientConfiguration.fromEnvironment([
            "ELSE_RUNTIME_URL": "http://127.0.0.1:4180",
            "ELSE_RUNTIME_DIR": directory.path,
            "ELSE_RUNTIME_TOKEN": "environment-token",
        ])

        #expect(configuration.runtimeToken == "environment-token")
        #expect(configuration.runtimeDirectory == directory)
    }

    @Test
    func readsPrivateDaemonTokenForLoopbackRuntime() throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        try writeToken("daemon-token\n", in: directory, permissions: 0o600)

        let configuration = ClientConfiguration.fromEnvironment([
            "ELSE_RUNTIME_URL": "http://127.0.0.1:4180",
            "ELSE_RUNTIME_DIR": directory.path,
        ])

        #expect(configuration.runtimeToken == "daemon-token")
        #expect(configuration.runtimeTokenURL == directory.appendingPathComponent("daemon-token"))
    }

    @Test
    func missingTokenLeavesRuntimeUnconfigured() throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }

        let configuration = ClientConfiguration.fromEnvironment([
            "ELSE_RUNTIME_URL": "http://127.0.0.1:4180",
            "ELSE_RUNTIME_DIR": directory.path,
        ])

        #expect(configuration.runtimeToken == nil)
    }

    @Test
    func rejectsTokenFileVisibleToOtherUsers() throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        try writeToken("public-token", in: directory, permissions: 0o644)

        let configuration = ClientConfiguration.fromEnvironment([
            "ELSE_RUNTIME_URL": "http://127.0.0.1:4180",
            "ELSE_RUNTIME_DIR": directory.path,
        ])

        #expect(configuration.runtimeToken == nil)
    }

    @Test
    func doesNotSendLocalDaemonTokenToRemoteRuntime() throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        try writeToken("local-only", in: directory, permissions: 0o600)

        let configuration = ClientConfiguration.fromEnvironment([
            "ELSE_RUNTIME_URL": "https://runtime.example.com",
            "ELSE_RUNTIME_DIR": directory.path,
        ])

        #expect(configuration.runtimeToken == nil)
    }

    private func temporaryDirectory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
            "else-client-configuration-\(UUID().uuidString)",
            isDirectory: true
        )
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true
        )
        return directory
    }

    private func writeToken(
        _ token: String,
        in directory: URL,
        permissions: Int
    ) throws {
        let url = directory.appendingPathComponent("daemon-token")
        try Data(token.utf8).write(to: url)
        try FileManager.default.setAttributes(
            [.posixPermissions: permissions],
            ofItemAtPath: url.path
        )
    }
}
