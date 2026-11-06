import AppKit
import ElseClientCore
import Foundation
import UserNotifications

@main
struct ElseNotifierMain {
    @MainActor
    static func main() async {
        let configuration = ClientConfiguration.fromEnvironment()
        if CommandLine.arguments.contains("--check") {
            print("bundle: \(Bundle.main.bundleIdentifier ?? "none")")
            print("bundle_url: \(Bundle.main.bundleURL.path)")
            print("token: \(configuration.currentRuntimeToken() != nil ? "configured" : "missing")")
            return
        }

        let runtime = BackgroundNotificationRuntime(configuration: configuration)
        if CommandLine.arguments.contains("--once") {
            await runtime.printDiagnostics()
            return
        }
        await runtime.run()
    }
}

@MainActor
private final class BackgroundNotificationRuntime: NSObject, UNUserNotificationCenterDelegate {
    private let configuration: ClientConfiguration
    private let center = UNUserNotificationCenter.current()
    private let defaults: UserDefaults
    private let client: HTTPRuntimeClient
    private let state: NotificationStateStore

    init(
        configuration: ClientConfiguration,
        defaults: UserDefaults = .standard
    ) {
        self.configuration = configuration
        self.defaults = defaults
        state = NotificationStateStore(
            url: configuration.runtimeDirectory.appendingPathComponent(
                "notifier-state.json"
            )
        )
        let sessionConfiguration = URLSessionConfiguration.ephemeral
        sessionConfiguration.waitsForConnectivity = false
        sessionConfiguration.timeoutIntervalForRequest = 8
        sessionConfiguration.timeoutIntervalForResource = 20
        client = HTTPRuntimeClient(
            baseURL: configuration.runtimeURL,
            tokenProvider: {
                configuration.currentRuntimeToken()
            },
            session: URLSession(configuration: sessionConfiguration)
        )
        super.init()
        center.delegate = self
        center.setNotificationCategories([
            UNNotificationCategory(
                identifier: ElseNotificationContract.decisionCategory,
                actions: [
                    UNNotificationAction(
                        identifier: ElseNotificationContract.openAction,
                        title: "查看",
                        options: [.foreground]
                    ),
                ],
                intentIdentifiers: [],
                options: []
            ),
        ])
    }

    func run() async {
        while !Task.isCancelled {
            await poll()
            let interval = max(
                15,
                defaults.object(
                    forKey: ElsePreferenceKey.backgroundRefreshInterval
                ) as? TimeInterval ?? 30
            )
            try? await Task.sleep(for: .seconds(interval))
        }
    }

    func printDiagnostics() async {
        let notificationsEnabled = defaults.bool(
            forKey: ElsePreferenceKey.notificationsEnabled
        )
        let settings = await center.notificationSettings()
        let decisions = (try? await client.listThreads())?.flatMap { snapshot in
            snapshot.decisions.map { (snapshot: snapshot, request: $0) }
        } ?? []
        print("bundle: \(Bundle.main.bundleIdentifier ?? "none")")
        print("bundle_url: \(Bundle.main.bundleURL.path)")
        print("token: \(configuration.currentRuntimeToken() != nil ? "configured" : "missing")")
        print("notifications_enabled: \(notificationsEnabled)")
        print("authorization: \(settings.authorizationStatus.rawValue)")
        print("main_app_running: \(mainAppIsRunning)")
        print("decisions: \(decisions.count)")
    }

    private func poll() async {
        guard let snapshots = try? await client.listThreads() else {
            return
        }
        let decisions = snapshots.flatMap { snapshot in
            snapshot.decisions.map { (snapshot: snapshot, request: $0) }
        }
        let identifiers = decisions.map(\.request.id.rawValue)
        let newIDs = await state.observe(identifiers)
        guard !newIDs.isEmpty,
              !mainAppIsRunning,
              defaults.bool(forKey: ElsePreferenceKey.notificationsEnabled)
        else {
            return
        }

        let settings = await center.notificationSettings()
        guard settings.authorizationStatus == .authorized
            || settings.authorizationStatus == .provisional
        else {
            return
        }

        for item in decisions
        where newIDs.contains(item.request.id.rawValue) {
            await post(item.snapshot.id, request: item.request)
        }
    }

    private func post(
        _ threadID: ThreadID,
        request: AttentionRequest
    ) async {
        let content = UNMutableNotificationContent()
        content.title = request.kind.notificationTitle
        content.body = request.summary
        content.sound = .default
        content.categoryIdentifier = ElseNotificationContract.decisionCategory
        content.userInfo = [
            ElseNotificationContract.threadIDKey: threadID.rawValue,
        ]
        try? await center.add(
            UNNotificationRequest(
                identifier: ElseNotificationContract.decisionIdentifier(request.id),
                content: content,
                trigger: nil
            )
        )
    }

    private var mainAppIsRunning: Bool {
        NSWorkspace.shared.runningApplications.contains { application in
            application.bundleIdentifier == Bundle.main.bundleIdentifier
                && application.executableURL?.lastPathComponent == "ElseMac"
                && application.processIdentifier != ProcessInfo.processInfo.processIdentifier
        }
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .sound, .list]
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse
    ) async {
        let userInfo = response.notification.request.content.userInfo
        guard let rawThreadID = userInfo[ElseNotificationContract.threadIDKey] as? String
        else {
            return
        }
        await MainActor.run {
            let configuration = NSWorkspace.OpenConfiguration()
            configuration.activates = true
            configuration.arguments = ["--open-thread", rawThreadID]
            NSWorkspace.shared.openApplication(
                at: Bundle.main.bundleURL,
                configuration: configuration
            )
        }
    }
}

private actor NotificationStateStore {
    private struct State: Codable {
        var hasBaseline = false
        var seenDecisionIDs: [String] = []
    }

    private let url: URL
    private var state: State

    init(url: URL) {
        self.url = url
        if let data = try? Data(contentsOf: url),
           let decoded = try? JSONDecoder().decode(State.self, from: data) {
            state = decoded
        } else {
            state = State()
        }
    }

    func observe(_ identifiers: [String]) -> Set<String> {
        let current = Set(identifiers)
        let newIDs = state.hasBaseline
            ? current.subtracting(state.seenDecisionIDs)
            : []
        if state.hasBaseline,
           current == Set(state.seenDecisionIDs) {
            return newIDs
        }
        state.hasBaseline = true
        state.seenDecisionIDs = identifiers.sorted()
        persist()
        return newIDs
    }

    private func persist() {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        guard let data = try? encoder.encode(state) else {
            return
        }
        try? FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try? data.write(to: url, options: .atomic)
        try? FileManager.default.setAttributes(
            [.posixPermissions: 0o600],
            ofItemAtPath: url.path
        )
    }
}
