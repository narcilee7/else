import Darwin
import Foundation

public struct ClientConfiguration: Sendable {
    public let runtimeURL: URL
    public let runtimeDirectory: URL
    public let runtimeToken: String?
    public let usesDemoRuntime: Bool

    public init(
        runtimeURL: URL,
        runtimeDirectory: URL = Self.defaultRuntimeDirectory,
        runtimeToken: String? = nil,
        usesDemoRuntime: Bool
    ) {
        self.runtimeURL = runtimeURL
        self.runtimeDirectory = runtimeDirectory
        self.runtimeToken = runtimeToken
        self.usesDemoRuntime = usesDemoRuntime
    }

    public static func fromEnvironment(
        _ environment: [String: String] = ProcessInfo.processInfo.environment
    ) -> ClientConfiguration {
        let value = environment["ELSE_RUNTIME_URL"] ?? "http://127.0.0.1:4180"
        let url = URL(string: value) ?? URL(string: "http://127.0.0.1:4180")!
        let runtimeDirectory = environment["ELSE_RUNTIME_DIR"].map {
            URL(fileURLWithPath: $0, isDirectory: true)
        } ?? defaultRuntimeDirectory
        let token = resolvedRuntimeToken(
            environment: environment,
            runtimeURL: url,
            runtimeDirectory: runtimeDirectory
        )
        return ClientConfiguration(
            runtimeURL: url,
            runtimeDirectory: runtimeDirectory,
            runtimeToken: token,
            usesDemoRuntime: environment["ELSE_DEMO"] == "1"
        )
    }

    public static var defaultRuntimeDirectory: URL {
        FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first!
        .appendingPathComponent("else", isDirectory: true)
        .appendingPathComponent("runtime", isDirectory: true)
    }

    public var runtimeTokenURL: URL {
        runtimeDirectory.appendingPathComponent("daemon-token", isDirectory: false)
    }

    public func currentRuntimeToken(
        environment: [String: String] = ProcessInfo.processInfo.environment
    ) -> String? {
        Self.resolvedRuntimeToken(
            environment: environment,
            runtimeURL: runtimeURL,
            runtimeDirectory: runtimeDirectory
        )
    }

    public static func resolvedRuntimeToken(
        environment: [String: String],
        runtimeURL: URL,
        runtimeDirectory: URL
    ) -> String? {
        if let token = normalized(environment["ELSE_RUNTIME_TOKEN"]) {
            return token
        }
        guard isLoopback(runtimeURL) else {
            return nil
        }
        return loadRuntimeToken(
            at: runtimeDirectory.appendingPathComponent("daemon-token")
        )
    }

    public static func loadRuntimeToken(at url: URL) -> String? {
        guard let attributes = try? FileManager.default.attributesOfItem(
            atPath: url.path
        ) else {
            return nil
        }
        guard attributes[.type] as? FileAttributeType == .typeRegular,
              let owner = attributes[.ownerAccountID] as? NSNumber,
              owner.uint32Value == getuid(),
              let permissions = attributes[.posixPermissions] as? NSNumber,
              permissions.intValue & 0o077 == 0,
              let contents = try? String(contentsOf: url, encoding: .utf8)
        else {
            return nil
        }
        return normalized(contents)
    }

    private static func normalized(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed?.isEmpty == false ? trimmed : nil
    }

    private static func isLoopback(_ url: URL) -> Bool {
        guard let host = url.host?.lowercased() else {
            return false
        }
        return host == "127.0.0.1"
            || host == "localhost"
            || host == "::1"
            || host == "[::1]"
    }
}
