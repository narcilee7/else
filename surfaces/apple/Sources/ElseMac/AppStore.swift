import ElseClientCore
import Foundation

enum AppSection: String, CaseIterable, Hashable, Identifiable {
    case today
    case world
    case connectors
    case decisions
    case results

    var id: Self { self }

    var title: String {
        switch self {
        case .today: "今天"
        case .world: "世界"
        case .connectors: "连接"
        case .decisions: "待决策"
        case .results: "结果"
        }
    }

    var systemImage: String {
        switch self {
        case .today: "sun.max"
        case .world: "circle.hexagongrid"
        case .connectors: "link"
        case .decisions: "hand.raised"
        case .results: "checkmark.seal"
        }
    }
}

enum RuntimeConnection: Equatable {
    case idle
    case connecting
    case connected
    case failed(String)

    var label: String {
        switch self {
        case .idle: "未连接"
        case .connecting: "连接中"
        case .connected: "Runtime 已连接"
        case .failed: "Runtime 离线"
        }
    }
}

@MainActor
final class AppStore: ObservableObject {
    @Published private(set) var snapshots: [ThreadSnapshot] = []
    @Published private(set) var brief: PersonalBrief?
    @Published private(set) var actionsByThread: [ThreadID: [ConnectorAction]] = [:]
    @Published private(set) var attentionWindows: [AttentionWindowRecord] = []
    @Published private(set) var projectWorlds: [ProjectWorld] = []
    @Published private(set) var identityWorlds: [IdentityWorld] = []
    @Published private(set) var connectorManifests: [ConnectorManifest] = []
    @Published private(set) var connectorsByThread: [ThreadID: [ConnectorRegistration]] = [:]
    @Published private(set) var capabilitiesByThread:
        [ThreadID: [RuntimeCapabilityRegistration]] = [:]
    @Published private(set) var connectorHealth: [ConnectorID: ConnectorHealthReport] = [:]
    @Published private(set) var resolvingWorldItemIDs: Set<WorldItemID> = []
    @Published private(set) var mutatingActionIDs: Set<ConnectorActionID> = []
    @Published private(set) var authenticatingConnectorIDs: Set<ConnectorID> = []
    @Published private(set) var notificationsEnabled: Bool
    @Published private(set) var backgroundRefreshEnabled: Bool
    @Published private(set) var backgroundRefreshInterval: TimeInterval
    @Published private(set) var retentionDays: UInt64?
    @Published private(set) var connection: RuntimeConnection = .idle
    @Published var section: AppSection = .today
    @Published var selectedThreadID: ThreadID?
    @Published var isCreatingThread = false
    @Published private(set) var isRefreshing = false
    @Published private(set) var lastRefreshedAt: Date?
    @Published private(set) var mutatingThreadIDs: Set<ThreadID> = []
    @Published var operationError: String?

    private let client: any RuntimeClient
    private let configuration: ClientConfiguration
    private let runtimeService: RuntimeServiceManager
    private let defaults: UserDefaults
    private var didLoad = false
    private var backgroundTask: Task<Void, Never>?
    private var seenDecisionIDs: Set<AttentionID>?

    init(
        client: any RuntimeClient,
        configuration: ClientConfiguration,
        runtimeService: RuntimeServiceManager,
        defaults: UserDefaults = .standard
    ) {
        self.client = client
        self.configuration = configuration
        self.runtimeService = runtimeService
        self.defaults = defaults
        notificationsEnabled = defaults.bool(forKey: PreferenceKey.notifications)
        backgroundRefreshEnabled = defaults.object(
            forKey: PreferenceKey.backgroundRefresh
        ) as? Bool ?? true
        backgroundRefreshInterval = defaults.object(
            forKey: PreferenceKey.refreshInterval
        ) as? TimeInterval ?? 30
        retentionDays = nil
        if let threadID = Self.launchThreadID {
            selectedThreadID = ThreadID(threadID)
            section = .decisions
        }
    }

    var runtimeURL: URL { configuration.runtimeURL }
    var isDemo: Bool { configuration.usesDemoRuntime }

    var today: [ThreadSnapshot] {
        snapshots.filter { !$0.isTerminal }
    }

    var results: [ThreadSnapshot] {
        snapshots.filter(\.isTerminal)
    }

    var decisions: [(snapshot: ThreadSnapshot, request: AttentionRequest)] {
        snapshots.flatMap { snapshot in
            snapshot.decisions.map { (snapshot, $0) }
        }
    }

    var decisionCount: Int {
        decisions.count
    }

    var briefOutstandingCount: Int {
        brief?.outstandingCount ?? 0
    }

    var selectedSnapshot: ThreadSnapshot? {
        guard let selectedThreadID else {
            return snapshots.first ?? nil
        }
        return snapshots.first { $0.id == selectedThreadID }
    }

    func load() async {
        guard !didLoad else { return }
        didLoad = true
        if !configuration.usesDemoRuntime {
            runtimeService.ensureEnabledIfNeeded()
            if runtimeService.status != .notFound,
               runtimeService.status != .unavailable {
                _ = await runtimeService.waitUntilReachable()
            }
        }
        await refresh()
        restartBackgroundTask()
    }

    func refresh() async {
        guard !isRefreshing else { return }
        isRefreshing = true
        if snapshots.isEmpty {
            connection = .connecting
        }
        defer { isRefreshing = false }

        do {
            snapshots = try await client.listThreads()
            connection = .connected
            lastRefreshedAt = Date()
            if selectedThreadID == nil {
                selectedThreadID = snapshots.first?.id
            }
            var actions: [ThreadID: [ConnectorAction]] = [:]
            var capabilities: [ThreadID: [RuntimeCapabilityRegistration]] = [:]
            for snapshot in snapshots {
                if let threadActions = try? await client.connectorActions(
                    threadID: snapshot.id
                ), !threadActions.isEmpty {
                    actions[snapshot.id] = threadActions
                }
                if let threadCapabilities = try? await client.capabilities(
                    threadID: snapshot.id
                ), !threadCapabilities.isEmpty {
                    capabilities[snapshot.id] = threadCapabilities
                }
            }
            actionsByThread = actions
            capabilitiesByThread = capabilities
            if let settings = try? await client.runtimeSettings() {
                retentionDays = settings.retentionDays
            }
        } catch {
            connection = .failed(error.localizedDescription)
            operationError = error.localizedDescription
        }
        do {
            brief = try await client.personalBrief()
        } catch {
            if connection == .connected {
                operationError = error.localizedDescription
            }
        }
        if let windows = try? await client.attentionWindows() {
            attentionWindows = windows
        }
        projectWorlds = (try? await client.projectWorlds(includeResolved: false)) ?? []
        identityWorlds = (try? await client.identityWorlds(includeResolved: false)) ?? []
        await refreshConnectors()
        deliverNewDecisionNotifications()
        NotificationCoordinator.shared.updateBadge(decisionCount: decisionCount)
    }

    func createThread(outcome: String, project: String?) async {
        do {
            let snapshot = try await client.createThread(outcome: outcome, project: project)
            upsert(snapshot)
            isCreatingThread = false
            section = .today
            selectedThreadID = snapshot.id
            await run(snapshot)
        } catch {
            operationError = error.localizedDescription
        }
    }

    func run(_ snapshot: ThreadSnapshot) async {
        await mutate(snapshot.id) {
            try await self.client.runThread(id: snapshot.id)
        }
    }

    func retryFailed(_ snapshot: ThreadSnapshot) async {
        await mutate(snapshot.id) {
            try await self.client.retryFailedThread(id: snapshot.id)
        }
    }

    func resolve(
        snapshot: ThreadSnapshot,
        request: AttentionRequest,
        choice: DecisionChoice
    ) async {
        await mutate(snapshot.id) {
            try await self.client.resolveDecision(
                threadID: snapshot.id,
                decisionID: request.id,
                choice: choice
            )
        }
    }

    func abort(_ snapshot: ThreadSnapshot) async {
        await mutate(snapshot.id) {
            try await self.client.abortThread(
                id: snapshot.id,
                reason: "用户从 macOS 客户端取消"
            )
        }
    }

    func deleteThread(_ snapshot: ThreadSnapshot) async {
        guard !mutatingThreadIDs.contains(snapshot.id) else {
            return
        }
        mutatingThreadIDs.insert(snapshot.id)
        defer { mutatingThreadIDs.remove(snapshot.id) }
        do {
            let report = try await client.deleteThread(id: snapshot.id)
            snapshots.removeAll { $0.id == snapshot.id }
            actionsByThread[snapshot.id] = nil
            connectorsByThread[snapshot.id] = nil
            capabilitiesByThread[snapshot.id] = nil
            if selectedThreadID == snapshot.id {
                selectedThreadID = snapshots.first?.id
            }
            if !report.cleanupErrors.isEmpty {
                operationError = report.cleanupErrors.joined(separator: "\n")
            }
            await refresh()
        } catch {
            operationError = error.localizedDescription
        }
    }

    func resolveWorldItem(
        _ record: WorldItemRecord,
        kind: WorldItemResolutionKind
    ) async {
        resolvingWorldItemIDs.insert(record.item.id)
        defer { resolvingWorldItemIDs.remove(record.item.id) }
        do {
            _ = try await client.resolveWorldItem(
                threadID: record.threadID,
                itemID: record.item.id,
                kind: kind
            )
            brief = brief?.removing(itemID: record.item.id, generatedAt: Date())
        } catch {
            operationError = error.localizedDescription
        }
    }

    func executeConnectorAction(
        threadID: ThreadID,
        action: ConnectorAction
    ) async {
        await mutateAction(threadID: threadID, actionID: action.id) {
            try await self.client.executeConnectorAction(
                threadID: threadID,
                actionID: action.id
            )
        }
    }

    func verifyConnectorAction(
        threadID: ThreadID,
        action: ConnectorAction
    ) async {
        await mutateAction(threadID: threadID, actionID: action.id) {
            try await self.client.verifyConnectorAction(
                threadID: threadID,
                actionID: action.id
            )
        }
    }

    func compensateConnectorAction(
        threadID: ThreadID,
        action: ConnectorAction
    ) async {
        await mutateAction(threadID: threadID, actionID: action.id) {
            try await self.client.compensateConnectorAction(
                threadID: threadID,
                actionID: action.id
            )
        }
    }

    func proposeConnectorAction(
        threadID: ThreadID,
        request: ProposeConnectorActionRequest
    ) async -> ConnectorAction? {
        do {
            let action = try await client.proposeConnectorAction(
                threadID: threadID,
                request: request
            )
            upsertAction(action, threadID: threadID)
            return action
        } catch {
            operationError = error.localizedDescription
            return nil
        }
    }

    func previewConnectorAction(
        threadID: ThreadID,
        connectorID: ConnectorID,
        request: PreviewConnectorActionRequest
    ) async -> ConnectorActionPreview? {
        do {
            return try await client.previewConnectorAction(
                threadID: threadID,
                connectorID: connectorID,
                request: request
            )
        } catch {
            operationError = error.localizedDescription
            return nil
        }
    }

    func exportRuntimeData() async -> Data? {
        do {
            let export = try await client.exportRuntimeData()
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            return try encoder.encode(export)
        } catch {
            operationError = error.localizedDescription
            return nil
        }
    }

    func exportDiagnostics() async -> Data? {
        do {
            let diagnostics = try await client.diagnostics()
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            return try encoder.encode(diagnostics)
        } catch {
            operationError = error.localizedDescription
            return nil
        }
    }

    func cleanupOrphanLeases() async -> LeaseCleanupReport? {
        do {
            return try await client.cleanupOrphanLeases()
        } catch {
            operationError = error.localizedDescription
            return nil
        }
    }

    func clearError() {
        operationError = nil
    }

    func open(_ deepLink: ElseDeepLink) {
        switch deepLink {
        case .today:
            section = .today
        case .world:
            section = .world
        case .connectors:
            section = .connectors
        case .decisions:
            section = .decisions
        case .results:
            section = .results
        case .newThread:
            section = .today
            isCreatingThread = true
        case let .thread(threadID):
            selectedThreadID = threadID
            section = .decisions
        }
    }

    func setNotificationsEnabled(_ enabled: Bool) async {
        if enabled {
            let granted = await NotificationCoordinator.shared.requestAuthorization()
            guard granted else {
                notificationsEnabled = false
                defaults.set(false, forKey: PreferenceKey.notifications)
                operationError = "系统通知权限未开启，请在系统设置中允许 else 发送通知。"
                return
            }
        }
        notificationsEnabled = enabled
        defaults.set(enabled, forKey: PreferenceKey.notifications)
        if enabled {
            seenDecisionIDs = Set(decisions.map(\.request.id))
        }
    }

    func setBackgroundRefreshEnabled(_ enabled: Bool) {
        backgroundRefreshEnabled = enabled
        defaults.set(enabled, forKey: PreferenceKey.backgroundRefresh)
        restartBackgroundTask()
    }

    func setBackgroundRefreshInterval(_ interval: TimeInterval) {
        backgroundRefreshInterval = interval
        defaults.set(interval, forKey: PreferenceKey.refreshInterval)
        restartBackgroundTask()
    }

    func setRetentionDays(_ days: UInt64?) async {
        do {
            let settings = try await client.updateRuntimeSettings(
                RuntimeSettings(retentionDays: days)
            )
            retentionDays = settings.retentionDays
        } catch {
            operationError = error.localizedDescription
        }
    }

    func correctWorldCorrelation(
        threadID: ThreadID,
        correction: WorldCorrelationCorrection
    ) async {
        do {
            _ = try await client.correctWorldCorrelation(
                threadID: threadID,
                correction: correction
            )
            projectWorlds = try await client.projectWorlds(includeResolved: false)
            identityWorlds = try await client.identityWorlds(includeResolved: false)
            brief = try await client.personalBrief()
        } catch {
            operationError = error.localizedDescription
        }
    }

    func registerConnector(
        threadID: ThreadID,
        request: RegisterConnectorRequest
    ) async -> ConnectorRegistration? {
        do {
            let connector = try await client.registerConnector(
                threadID: threadID,
                request: request
            )
            connectorsByThread[threadID, default: []].append(connector)
            if let health = try? await client.connectorHealth(
                threadID: threadID,
                connectorID: connector.id
            ) {
                connectorHealth[connector.id] = health
            }
            _ = try? await client.syncConnector(
                threadID: threadID,
                connectorID: connector.id
            )
            await refresh()
            return connector
        } catch {
            operationError = error.localizedDescription
            return nil
        }
    }

    func syncConnector(threadID: ThreadID, connectorID: ConnectorID) async {
        do {
            _ = try await client.syncConnector(
                threadID: threadID,
                connectorID: connectorID
            )
            if let health = try? await client.connectorHealth(
                threadID: threadID,
                connectorID: connectorID
            ) {
                connectorHealth[connectorID] = health
            }
        } catch {
            operationError = error.localizedDescription
        }
    }

    func repairConnector(threadID: ThreadID, connectorID: ConnectorID) async {
        do {
            let report = try await client.repairConnector(
                threadID: threadID,
                connectorID: connectorID
            )
            if let error = report.error {
                operationError = "Cursor 已重建，但重新同步失败：\(error)"
            }
            if let health = try? await client.connectorHealth(
                threadID: threadID,
                connectorID: connectorID
            ) {
                connectorHealth[connectorID] = health
            }
        } catch {
            operationError = error.localizedDescription
        }
    }

    func attachConnectorCredential(
        threadID: ThreadID,
        connectorID: ConnectorID,
        credential: ConnectorCredentialInput
    ) async -> Bool {
        authenticatingConnectorIDs.insert(connectorID)
        defer { authenticatingConnectorIDs.remove(connectorID) }
        do {
            let metadata = try await client.attachConnectorCredential(
                threadID: threadID,
                connectorID: connectorID,
                credential: credential
            )
            replaceConnectorCredential(
                threadID: threadID,
                connectorID: connectorID,
                credential: metadata
            )
            await refreshConnectorHealth(
                threadID: threadID,
                connectorID: connectorID
            )
            return true
        } catch {
            operationError = error.localizedDescription
            return false
        }
    }

    func revokeConnectorCredential(
        threadID: ThreadID,
        connectorID: ConnectorID
    ) async {
        authenticatingConnectorIDs.insert(connectorID)
        defer { authenticatingConnectorIDs.remove(connectorID) }
        do {
            let metadata = try await client.revokeConnectorCredential(
                threadID: threadID,
                connectorID: connectorID
            )
            replaceConnectorCredential(
                threadID: threadID,
                connectorID: connectorID,
                credential: metadata
            )
            await refreshConnectorHealth(
                threadID: threadID,
                connectorID: connectorID
            )
        } catch {
            operationError = error.localizedDescription
        }
    }

    private func replaceConnectorCredential(
        threadID: ThreadID,
        connectorID: ConnectorID,
        credential: ConnectorCredentialMetadata
    ) {
        guard let index = connectorsByThread[threadID]?.firstIndex(where: {
            $0.id == connectorID
        }) else {
            return
        }
        let old = connectorsByThread[threadID]![index]
        connectorsByThread[threadID]![index] = ConnectorRegistration(
            id: old.id,
            name: old.name,
            adapter: old.adapter,
            source: old.source,
            enabled: old.enabled,
            manifest: old.manifest,
            syncIntervalSeconds: old.syncIntervalSeconds,
            registeredAt: old.registeredAt,
            credential: credential
        )
    }

    private func refreshConnectorHealth(
        threadID: ThreadID,
        connectorID: ConnectorID
    ) async {
        if let health = try? await client.connectorHealth(
            threadID: threadID,
            connectorID: connectorID
        ) {
            connectorHealth[connectorID] = health
        }
    }

    private func mutate(
        _ threadID: ThreadID,
        operation: @escaping () async throws -> ThreadSnapshot
    ) async {
        mutatingThreadIDs.insert(threadID)
        defer { mutatingThreadIDs.remove(threadID) }

        do {
            upsert(try await operation())
            connection = .connected
        } catch {
            operationError = error.localizedDescription
        }
    }

    private func upsert(_ snapshot: ThreadSnapshot) {
        if let index = snapshots.firstIndex(where: { $0.id == snapshot.id }) {
            snapshots[index] = snapshot
        } else {
            snapshots.insert(snapshot, at: 0)
        }
    }

    private func restartBackgroundTask() {
        backgroundTask?.cancel()
        backgroundTask = nil
        guard backgroundRefreshEnabled else {
            return
        }
        backgroundTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let interval = self?.backgroundRefreshInterval else {
                    return
                }
                try? await Task.sleep(
                    nanoseconds: UInt64(max(interval, 15) * 1_000_000_000)
                )
                guard !Task.isCancelled else {
                    return
                }
                await self?.refresh()
            }
        }
    }

    private func refreshConnectors() async {
        if let adapters = try? await client.connectorAdapters() {
            connectorManifests = adapters
        }
        var connectors: [ThreadID: [ConnectorRegistration]] = [:]
        var health: [ConnectorID: ConnectorHealthReport] = [:]
        for snapshot in snapshots {
            guard let items = try? await client.connectors(threadID: snapshot.id) else {
                continue
            }
            connectors[snapshot.id] = items
            for item in items {
                if let report = try? await client.connectorHealth(
                    threadID: snapshot.id,
                    connectorID: item.id
                ) {
                    health[item.id] = report
                }
            }
        }
        connectorsByThread = connectors
        connectorHealth = health
    }

    private func deliverNewDecisionNotifications() {
        let currentIDs = Set(decisions.map(\.request.id))
        defer { seenDecisionIDs = currentIDs }
        guard notificationsEnabled, let previousIDs = seenDecisionIDs else {
            return
        }
        for item in decisions where !previousIDs.contains(item.request.id) {
            Task {
                await NotificationCoordinator.shared.postDecisionNotification(
                    threadID: item.snapshot.id,
                    request: item.request
                )
            }
        }
    }

    private func mutateAction(
        threadID: ThreadID,
        actionID: ConnectorActionID,
        operation: @escaping () async throws -> ConnectorAction
    ) async {
        mutatingActionIDs.insert(actionID)
        defer { mutatingActionIDs.remove(actionID) }

        do {
            let action = try await operation()
            upsertAction(action, threadID: threadID)
            connection = .connected
        } catch {
            operationError = error.localizedDescription
            if let runtimeError = error as? RuntimeClientError,
                case .http(status: 409, message: _) = runtimeError
            {
                await refresh()
            } else {
                if let actions = try? await client.connectorActions(threadID: threadID) {
                    actionsByThread[threadID] = actions
                }
            }
        }
    }

    private func upsertAction(_ action: ConnectorAction, threadID: ThreadID) {
        var actions = actionsByThread[threadID] ?? []
        if let index = actions.firstIndex(where: { $0.id == action.id }) {
            actions[index] = action
        } else {
            actions.insert(action, at: 0)
        }
        actionsByThread[threadID] = actions
    }
}

private enum PreferenceKey {
    static let notifications = ElsePreferenceKey.notificationsEnabled
    static let backgroundRefresh = ElsePreferenceKey.backgroundRefreshEnabled
    static let refreshInterval = ElsePreferenceKey.backgroundRefreshInterval
}

private extension AppStore {
    static var launchThreadID: String? {
        let arguments = CommandLine.arguments
        guard let index = arguments.firstIndex(of: "--open-thread"),
              arguments.indices.contains(index + 1)
        else {
            return nil
        }
        return arguments[index + 1]
    }
}

private extension PersonalBrief {
    func removing(itemID: WorldItemID, generatedAt: Date) -> PersonalBrief {
        PersonalBrief(
            generatedAt: generatedAt,
            openCommitments: openCommitments.filter { $0.item.id != itemID },
            waitingExternal: waitingExternal.filter { $0.item.id != itemID },
            overdue: overdue.filter { $0.item.id != itemID },
            actionItemsDueSoon: actionItemsDueSoon.filter { $0.item.id != itemID },
            risks: risks.filter { $0.item.id != itemID },
            upcomingCalendar: upcomingCalendar.filter { $0.item.id != itemID }
        )
    }
}
