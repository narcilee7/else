import Foundation

public protocol RuntimeClient: Sendable {
    func listThreads() async throws -> [ThreadSnapshot]
    func thread(id: ThreadID) async throws -> ThreadSnapshot
    func deleteThread(id: ThreadID) async throws -> ThreadDeletionReport
    func retryFailedThread(id: ThreadID) async throws -> ThreadSnapshot
    func createThread(outcome: String, project: String?) async throws -> ThreadSnapshot
    func runThread(id: ThreadID) async throws -> ThreadSnapshot
    func resolveDecision(
        threadID: ThreadID,
        decisionID: AttentionID,
        choice: DecisionChoice
    ) async throws -> ThreadSnapshot
    func abortThread(id: ThreadID, reason: String) async throws -> ThreadSnapshot
    func personalBrief() async throws -> PersonalBrief
    func resolveWorldItem(
        threadID: ThreadID,
        itemID: WorldItemID,
        kind: WorldItemResolutionKind
    ) async throws -> WorldItem
    func connectorActions(threadID: ThreadID) async throws -> [ConnectorAction]
    func executeConnectorAction(
        threadID: ThreadID,
        actionID: ConnectorActionID
    ) async throws -> ConnectorAction
    func verifyConnectorAction(
        threadID: ThreadID,
        actionID: ConnectorActionID
    ) async throws -> ConnectorAction
    func compensateConnectorAction(
        threadID: ThreadID,
        actionID: ConnectorActionID
    ) async throws -> ConnectorAction
    func proposeConnectorAction(
        threadID: ThreadID,
        request: ProposeConnectorActionRequest
    ) async throws -> ConnectorAction
    func capabilities(threadID: ThreadID) async throws -> [RuntimeCapabilityRegistration]
    func exportRuntimeData() async throws -> JSONValue
    func diagnostics() async throws -> JSONValue
    func cleanupOrphanLeases() async throws -> LeaseCleanupReport
    func runtimeSettings() async throws -> RuntimeSettings
    func updateRuntimeSettings(_ settings: RuntimeSettings) async throws -> RuntimeSettings
    func previewConnectorAction(
        threadID: ThreadID,
        connectorID: ConnectorID,
        request: PreviewConnectorActionRequest
    ) async throws -> ConnectorActionPreview
    func attentionWindows() async throws -> [AttentionWindowRecord]
    func projectWorlds(includeResolved: Bool) async throws -> [ProjectWorld]
    func identityWorlds(includeResolved: Bool) async throws -> [IdentityWorld]
    func correctWorldCorrelation(
        threadID: ThreadID,
        correction: WorldCorrelationCorrection
    ) async throws -> WorldCorrelationCorrection
    func connectorAdapters() async throws -> [ConnectorManifest]
    func connectors(threadID: ThreadID) async throws -> [ConnectorRegistration]
    func connectorHealth(
        threadID: ThreadID,
        connectorID: ConnectorID
    ) async throws -> ConnectorHealthReport
    func attachConnectorCredential(
        threadID: ThreadID,
        connectorID: ConnectorID,
        credential: ConnectorCredentialInput
    ) async throws -> ConnectorCredentialMetadata
    func revokeConnectorCredential(
        threadID: ThreadID,
        connectorID: ConnectorID
    ) async throws -> ConnectorCredentialMetadata
    func registerConnector(
        threadID: ThreadID,
        request: RegisterConnectorRequest
    ) async throws -> ConnectorRegistration
    func syncConnector(
        threadID: ThreadID,
        connectorID: ConnectorID
    ) async throws -> ConnectorSyncResult
    func repairConnector(
        threadID: ThreadID,
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
