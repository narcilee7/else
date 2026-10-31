import ElseClientCore
import Foundation

enum AppSection: String, CaseIterable, Hashable, Identifiable {
    case today
    case decisions
    case results

    var id: Self { self }

    var title: String {
        switch self {
        case .today: "今天"
        case .decisions: "待决策"
        case .results: "结果"
        }
    }

    var systemImage: String {
        switch self {
        case .today: "sun.max"
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
    @Published private(set) var snapshots: [IntentSnapshot] = []
    @Published private(set) var brief: PersonalBrief?
    @Published private(set) var actionsByIntent: [IntentID: [ConnectorAction]] = [:]
    @Published private(set) var attentionWindows: [AttentionWindowRecord] = []
    @Published private(set) var resolvingWorldItemIDs: Set<WorldItemID> = []
    @Published private(set) var mutatingActionIDs: Set<ConnectorActionID> = []
    @Published private(set) var connection: RuntimeConnection = .idle
    @Published var section: AppSection = .today
    @Published var selectedIntentID: IntentID?
    @Published var isCreatingIntent = false
    @Published private(set) var isRefreshing = false
    @Published private(set) var mutatingIntentIDs: Set<IntentID> = []
    @Published var operationError: String?

    private let client: any RuntimeClient
    private let configuration: ClientConfiguration
    private var didLoad = false

    init(client: any RuntimeClient, configuration: ClientConfiguration) {
        self.client = client
        self.configuration = configuration
    }

    var runtimeURL: URL { configuration.runtimeURL }
    var isDemo: Bool { configuration.usesDemoRuntime }

    var today: [IntentSnapshot] {
        snapshots.filter { !$0.isTerminal }
    }

    var results: [IntentSnapshot] {
        snapshots.filter(\.isTerminal)
    }

    var decisions: [(snapshot: IntentSnapshot, request: AttentionRequest)] {
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

    var selectedSnapshot: IntentSnapshot? {
        guard let selectedIntentID else {
            return snapshots.first ?? nil
        }
        return snapshots.first { $0.id == selectedIntentID }
    }

    func load() async {
        guard !didLoad else { return }
        didLoad = true
        await refresh()
    }

    func refresh() async {
        guard !isRefreshing else { return }
        isRefreshing = true
        if snapshots.isEmpty {
            connection = .connecting
        }
        defer { isRefreshing = false }

        do {
            snapshots = try await client.listIntents()
            connection = .connected
            if selectedIntentID == nil {
                selectedIntentID = snapshots.first?.id
            }
            var actions: [IntentID: [ConnectorAction]] = [:]
            for snapshot in snapshots {
                if let intentActions = try? await client.connectorActions(
                    intentID: snapshot.id
                ), !intentActions.isEmpty {
                    actions[snapshot.id] = intentActions
                }
            }
            actionsByIntent = actions
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
    }

    func createIntent(outcome: String, project: String?) async {
        do {
            let snapshot = try await client.createIntent(outcome: outcome, project: project)
            upsert(snapshot)
            isCreatingIntent = false
            section = .today
            selectedIntentID = snapshot.id
            await run(snapshot)
        } catch {
            operationError = error.localizedDescription
        }
    }

    func run(_ snapshot: IntentSnapshot) async {
        await mutate(snapshot.id) {
            try await self.client.runIntent(id: snapshot.id)
        }
    }

    func resolve(
        snapshot: IntentSnapshot,
        request: AttentionRequest,
        choice: DecisionChoice
    ) async {
        await mutate(snapshot.id) {
            try await self.client.resolveDecision(
                intentID: snapshot.id,
                decisionID: request.id,
                choice: choice
            )
        }
    }

    func abort(_ snapshot: IntentSnapshot) async {
        await mutate(snapshot.id) {
            try await self.client.abortIntent(
                id: snapshot.id,
                reason: "用户从 macOS 客户端取消"
            )
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
                intentID: record.intentID,
                itemID: record.item.id,
                kind: kind
            )
            brief = brief?.removing(itemID: record.item.id, generatedAt: Date())
        } catch {
            operationError = error.localizedDescription
        }
    }

    func executeConnectorAction(
        intentID: IntentID,
        action: ConnectorAction
    ) async {
        await mutateAction(intentID: intentID, actionID: action.id) {
            try await self.client.executeConnectorAction(
                intentID: intentID,
                actionID: action.id
            )
        }
    }

    func verifyConnectorAction(
        intentID: IntentID,
        action: ConnectorAction
    ) async {
        await mutateAction(intentID: intentID, actionID: action.id) {
            try await self.client.verifyConnectorAction(
                intentID: intentID,
                actionID: action.id
            )
        }
    }

    func compensateConnectorAction(
        intentID: IntentID,
        action: ConnectorAction
    ) async {
        await mutateAction(intentID: intentID, actionID: action.id) {
            try await self.client.compensateConnectorAction(
                intentID: intentID,
                actionID: action.id
            )
        }
    }

    func clearError() {
        operationError = nil
    }

    private func mutate(
        _ intentID: IntentID,
        operation: @escaping () async throws -> IntentSnapshot
    ) async {
        mutatingIntentIDs.insert(intentID)
        defer { mutatingIntentIDs.remove(intentID) }

        do {
            upsert(try await operation())
            connection = .connected
        } catch {
            operationError = error.localizedDescription
        }
    }

    private func upsert(_ snapshot: IntentSnapshot) {
        if let index = snapshots.firstIndex(where: { $0.id == snapshot.id }) {
            snapshots[index] = snapshot
        } else {
            snapshots.insert(snapshot, at: 0)
        }
    }

    private func mutateAction(
        intentID: IntentID,
        actionID: ConnectorActionID,
        operation: @escaping () async throws -> ConnectorAction
    ) async {
        mutatingActionIDs.insert(actionID)
        defer { mutatingActionIDs.remove(actionID) }

        do {
            let action = try await operation()
            upsertAction(action, intentID: intentID)
            connection = .connected
        } catch {
            operationError = error.localizedDescription
            if let runtimeError = error as? RuntimeClientError,
                case .http(status: 409, message: _) = runtimeError
            {
                await refresh()
            } else {
                if let actions = try? await client.connectorActions(intentID: intentID) {
                    actionsByIntent[intentID] = actions
                }
            }
        }
    }

    private func upsertAction(_ action: ConnectorAction, intentID: IntentID) {
        var actions = actionsByIntent[intentID] ?? []
        if let index = actions.firstIndex(where: { $0.id == action.id }) {
            actions[index] = action
        } else {
            actions.insert(action, at: 0)
        }
        actionsByIntent[intentID] = actions
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
