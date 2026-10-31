import Foundation

public protocol RuntimeClient: Sendable {
    func listIntents() async throws -> [IntentSnapshot]
    func intent(id: IntentID) async throws -> IntentSnapshot
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
    func attentionWindows() async throws -> [AttentionWindowRecord]
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
