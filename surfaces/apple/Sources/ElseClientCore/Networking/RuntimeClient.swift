import Foundation

public protocol RuntimeClient: Sendable {
    func listIntents() async throws -> [IntentSnapshot]
    func intent(id: IntentID) async throws -> IntentSnapshot
    func deleteIntent(id: IntentID) async throws -> IntentDeletionReport
    func retryFailedIntent(id: IntentID) async throws -> IntentSnapshot
    func createIntent(outcome: String, project: String?) async throws -> IntentSnapshot
    func runIntent(id: IntentID) async throws -> IntentSnapshot
    func resolveDecision(
        intentID: IntentID,
        decisionID: AttentionID,
        choice: DecisionChoice
    ) async throws -> IntentSnapshot
    func abortIntent(id: IntentID, reason: String) async throws -> IntentSnapshot
    func personalBrief() async throws -> PersonalBrief
    func resolveWorldItem(
        intentID: IntentID,
        itemID: WorldItemID,
        kind: WorldItemResolutionKind
    ) async throws -> WorldItem
    func connectorActions(intentID: IntentID) async throws -> [ConnectorAction]
    func executeConnectorAction(
        intentID: IntentID,
        actionID: ConnectorActionID
    ) async throws -> ConnectorAction
    func verifyConnectorAction(
        intentID: IntentID,
        actionID: ConnectorActionID
    ) async throws -> ConnectorAction
    func compensateConnectorAction(
        intentID: IntentID,
        actionID: ConnectorActionID
    ) async throws -> ConnectorAction
    func proposeConnectorAction(
        intentID: IntentID,
        request: ProposeConnectorActionRequest
    ) async throws -> ConnectorAction
    func capabilities(intentID: IntentID) async throws -> [RuntimeCapabilityRegistration]
    func exportRuntimeData() async throws -> JSONValue
    func diagnostics() async throws -> JSONValue
    func cleanupOrphanLeases() async throws -> LeaseCleanupReport
    func runtimeSettings() async throws -> RuntimeSettings
    func updateRuntimeSettings(_ settings: RuntimeSettings) async throws -> RuntimeSettings
    func previewConnectorAction(
        intentID: IntentID,
        connectorID: ConnectorID,
        request: PreviewConnectorActionRequest
    ) async throws -> ConnectorActionPreview
    func attentionWindows() async throws -> [AttentionWindowRecord]
    func projectWorlds(includeResolved: Bool) async throws -> [ProjectWorld]
    func identityWorlds(includeResolved: Bool) async throws -> [IdentityWorld]
    func correctWorldCorrelation(
        intentID: IntentID,
        correction: WorldCorrelationCorrection
    ) async throws -> WorldCorrelationCorrection
    func connectorAdapters() async throws -> [ConnectorManifest]
    func connectors(intentID: IntentID) async throws -> [ConnectorRegistration]
    func connectorHealth(
        intentID: IntentID,
        connectorID: ConnectorID
    ) async throws -> ConnectorHealthReport
    func attachConnectorCredential(
        intentID: IntentID,
        connectorID: ConnectorID,
        credential: ConnectorCredentialInput
    ) async throws -> ConnectorCredentialMetadata
    func revokeConnectorCredential(
        intentID: IntentID,
        connectorID: ConnectorID
    ) async throws -> ConnectorCredentialMetadata
    func registerConnector(
        intentID: IntentID,
        request: RegisterConnectorRequest
    ) async throws -> ConnectorRegistration
    func syncConnector(
        intentID: IntentID,
        connectorID: ConnectorID
    ) async throws -> ConnectorSyncResult
    func repairConnector(
        intentID: IntentID,
        connectorID: ConnectorID
    ) async throws -> ConnectorRepairReport
}

public struct ConnectorSyncResult: Codable, Equatable, Sendable {
    public let observed: UInt64
    public let inserted: UInt64
    public let skipped: UInt64
    public let candidatesProposed: UInt64
    public let autoAccepted: UInt64
    public let cursor: String?
    public let pagesSynced: UInt64
    public let hasMore: Bool
    public let backfillComplete: Bool
    public let hydrated: Bool

    public init(
        observed: UInt64,
        inserted: UInt64,
        skipped: UInt64,
        candidatesProposed: UInt64,
        autoAccepted: UInt64,
        cursor: String?,
        pagesSynced: UInt64,
        hasMore: Bool,
        backfillComplete: Bool,
        hydrated: Bool
    ) {
        self.observed = observed
        self.inserted = inserted
        self.skipped = skipped
        self.candidatesProposed = candidatesProposed
        self.autoAccepted = autoAccepted
        self.cursor = cursor
        self.pagesSynced = pagesSynced
        self.hasMore = hasMore
        self.backfillComplete = backfillComplete
        self.hydrated = hydrated
    }

    private enum CodingKeys: String, CodingKey {
        case observed
        case inserted
        case skipped
        case candidatesProposed = "candidates_proposed"
        case autoAccepted = "auto_accepted"
        case cursor
        case pagesSynced = "pages_synced"
        case hasMore = "has_more"
        case backfillComplete = "backfill_complete"
        case hydrated
    }
}

public enum RuntimeClientError: Error, LocalizedError, Sendable {
    case invalidResponse
    case http(status: Int, message: String?)
    case decoding(String)
    case transport(String)

    public var errorDescription: String? {
        switch self {
        case .invalidResponse:
            "Runtime 返回了无法识别的响应"
        case let .http(status, message):
            message ?? "Runtime 请求失败（HTTP \(status)）"
        case let .decoding(message):
            "Runtime 响应格式不兼容：\(message)"
        case let .transport(message):
            "无法连接 Runtime：\(message)"
        }
    }
}
