import ElseClientCore
import Foundation

enum AppDependencies {
    static func configuration() -> ClientConfiguration {
        let environment = ProcessInfo.processInfo.environment
        var configuration = ClientConfiguration.fromEnvironment(environment)
        if CommandLine.arguments.contains("--demo") {
            configuration = ClientConfiguration(
                runtimeURL: configuration.runtimeURL,
                runtimeToken: configuration.runtimeToken,
                usesDemoRuntime: true
            )
        }
        return configuration
    }

    @MainActor
    static func client(for configuration: ClientConfiguration) -> any RuntimeClient {
        if configuration.usesDemoRuntime {
            return DemoRuntimeClient()
        }

        let environment = ProcessInfo.processInfo.environment
        let sessionConfiguration = URLSessionConfiguration.ephemeral
        sessionConfiguration.waitsForConnectivity = false
        sessionConfiguration.timeoutIntervalForRequest = 8
        sessionConfiguration.timeoutIntervalForResource = 20
        return HTTPRuntimeClient(
            baseURL: configuration.runtimeURL,
            token: configuration.runtimeToken,
            tokenProvider: {
                configuration.currentRuntimeToken(environment: environment)
            },
            session: URLSession(configuration: sessionConfiguration)
        )
    }
}
