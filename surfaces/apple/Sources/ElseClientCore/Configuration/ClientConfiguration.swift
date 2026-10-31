import Foundation

public struct ClientConfiguration: Sendable {
    public let runtimeURL: URL
    public let runtimeToken: String?
    public let usesDemoRuntime: Bool

    public init(
        runtimeURL: URL,
        runtimeToken: String? = nil,
        usesDemoRuntime: Bool
    ) {
        self.runtimeURL = runtimeURL
        self.runtimeToken = runtimeToken
        self.usesDemoRuntime = usesDemoRuntime
    }

    public static func fromEnvironment(
        _ environment: [String: String] = ProcessInfo.processInfo.environment
    ) -> ClientConfiguration {
        let value = environment["ELSE_RUNTIME_URL"] ?? "http://127.0.0.1:4180"
        let url = URL(string: value) ?? URL(string: "http://127.0.0.1:4180")!
        return ClientConfiguration(
            runtimeURL: url,
            runtimeToken: environment["ELSE_RUNTIME_TOKEN"],
            usesDemoRuntime: environment["ELSE_DEMO"] == "1"
        )
    }
}
