import AppKit
import ElseClientCore
import Foundation
import ServiceManagement

@MainActor
final class RuntimeServiceManager: ObservableObject {
    enum Status: Equatable {
        case enabled
        case requiresApproval
        case notRegistered
        case notFound
        case unavailable

        var label: String {
            switch self {
            case .enabled: "后台运行"
            case .requiresApproval: "等待系统批准"
            case .notRegistered: "未启动"
            case .notFound: "Helper 未就绪"
            case .unavailable: "状态不可用"
            }
        }

        var isRegistered: Bool {
            self == .enabled || self == .requiresApproval
        }
    }

    @Published private(set) var status: Status
    @Published private(set) var notifierStatus: Status
    @Published private(set) var tokenConfigured: Bool
    @Published private(set) var isEnabledByUser: Bool
    @Published private(set) var errorMessage: String?

    let configuration: ClientConfiguration

    private let runtimeAgent: SMAppService
    private let notifierAgent: SMAppService
    private let defaults: UserDefaults

    init(
        configuration: ClientConfiguration,
        defaults: UserDefaults = .standard
    ) {
        self.configuration = configuration
        self.defaults = defaults
        runtimeAgent = SMAppService.agent(plistName: PreferenceKey.runtimeAgentPlist)
        notifierAgent = SMAppService.agent(plistName: PreferenceKey.notifierAgentPlist)
        let runtimeStatus = Self.map(runtimeAgent.status)
        let notifierStatus = Self.map(notifierAgent.status)
        status = Self.combined([runtimeStatus, notifierStatus])
        self.notifierStatus = notifierStatus
        tokenConfigured = configuration.currentRuntimeToken() != nil
        isEnabledByUser = defaults.object(forKey: PreferenceKey.enabled) as? Bool ?? true
    }

    var runtimeURL: URL {
        configuration.runtimeURL
    }

    var runtimeDirectory: URL {
        configuration.runtimeDirectory
    }

    var hasRegisteredAgent: Bool {
        status.isRegistered || notifierStatus.isRegistered
    }

    var allAgentsEnabled: Bool {
        status == .enabled && notifierStatus == .enabled
    }

    func refresh() {
        let runtimeStatus = Self.map(runtimeAgent.status)
        notifierStatus = Self.map(notifierAgent.status)
        status = Self.combined([runtimeStatus, notifierStatus])
        tokenConfigured = configuration.currentRuntimeToken() != nil
    }

    func ensureEnabledIfNeeded() {
        refresh()
        guard isEnabledByUser, status == .notRegistered else {
            return
        }
        register()
    }

    func start() {
        isEnabledByUser = true
        defaults.set(true, forKey: PreferenceKey.enabled)
        refresh()
        let statuses = [Self.map(runtimeAgent.status), Self.map(notifierAgent.status)]
        if statuses.contains(.requiresApproval) {
            SMAppService.openSystemSettingsLoginItems()
            return
        }
        if statuses.contains(.notRegistered) || statuses.contains(.notFound) {
            registerMissingAgents()
        }
    }

    func stop() async {
        isEnabledByUser = false
        defaults.set(false, forKey: PreferenceKey.enabled)
        guard hasRegisteredAgent else {
            refresh()
            return
        }
        var errors: [String] = []
        for service in [runtimeAgent, notifierAgent] where Self.map(service.status).isRegistered {
            do {
                try await service.unregister()
            } catch {
                errors.append(error.localizedDescription)
            }
        }
        errorMessage = errors.first
        refresh()
    }

    func restart() async {
        isEnabledByUser = true
        defaults.set(true, forKey: PreferenceKey.enabled)
        var errors: [String] = []
        for service in [runtimeAgent, notifierAgent] where Self.map(service.status).isRegistered {
            do {
                try await service.unregister()
            } catch {
                errors.append(error.localizedDescription)
            }
        }
        registerMissingAgents(initialErrors: errors)
        refresh()
    }

    func openSystemApprovalSettings() {
        SMAppService.openSystemSettingsLoginItems()
    }

    func openRuntimeDirectory() {
        try? FileManager.default.createDirectory(
            at: runtimeDirectory,
            withIntermediateDirectories: true
        )
        try? FileManager.default.setAttributes(
            [.posixPermissions: 0o700],
            ofItemAtPath: runtimeDirectory.path
        )
        NSWorkspace.shared.open(runtimeDirectory)
    }

    func waitUntilReachable(timeout: TimeInterval = 8) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        let session = Self.healthSession()
        defer { session.invalidateAndCancel() }
        while Date() < deadline {
            refresh()
            if await Self.healthCheck(runtimeURL, session: session) {
                return true
            }
            try? await Task.sleep(for: .milliseconds(250))
        }
        refresh()
        return false
    }

    private func registerMissingAgents(initialErrors: [String] = []) {
        var errors = initialErrors
        for service in [runtimeAgent, notifierAgent] {
            let status = Self.map(service.status)
            guard status == .notRegistered || status == .notFound else {
                continue
            }
            do {
                try service.register()
            } catch {
                errors.append(error.localizedDescription)
            }
        }
        errorMessage = errors.first
        refresh()
    }

    private func register() {
        registerMissingAgents()
    }

    private static func map(_ status: SMAppService.Status) -> Status {
        switch status {
        case .enabled:
            .enabled
        case .requiresApproval:
            .requiresApproval
        case .notRegistered:
            .notRegistered
        case .notFound:
            .notFound
        @unknown default:
            .unavailable
        }
    }

    private static func combined(_ statuses: [Status]) -> Status {
        if statuses.contains(.notFound) {
            return .notFound
        }
        if statuses.contains(.requiresApproval) {
            return .requiresApproval
        }
        if statuses.contains(.unavailable) {
            return .unavailable
        }
        if statuses.allSatisfy({ $0 == .enabled }) {
            return .enabled
        }
        return .notRegistered
    }

    private static func healthCheck(
        _ runtimeURL: URL,
        session: URLSession
    ) async -> Bool {
        var request = URLRequest(
            url: runtimeURL.appendingPathComponent("v1").appendingPathComponent("health")
        )
        request.timeoutInterval = 1
        do {
            let (_, response) = try await session.data(for: request)
            return (response as? HTTPURLResponse)?.statusCode == 200
        } catch {
            return false
        }
    }

    private static func healthSession() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.waitsForConnectivity = false
        configuration.timeoutIntervalForRequest = 1
        configuration.timeoutIntervalForResource = 2
        return URLSession(configuration: configuration)
    }

    private enum PreferenceKey {
        static let runtimeAgentPlist = "com.else.desktop.runtime.plist"
        static let notifierAgentPlist = "com.else.desktop.notifier.plist"
        static let enabled = "runtimeHelperDesired"
    }
}
