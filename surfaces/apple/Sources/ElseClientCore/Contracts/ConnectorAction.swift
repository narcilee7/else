import Foundation

public enum ConnectorActionStatus: String, Codable, Equatable, Sendable {
    case proposed
    case needsApproval = "needs_approval"
    case executing
    case applied
    case compensating
    case compensationExecuting = "compensation_executing"
    case compensated
    case compensationFailed = "compensation_failed"
    case failed
    case unknown
    case cancelled
}

public enum ToolStatus: String, Codable, Equatable, Sendable {
    case succeeded
    case failed
    case unknown
}

public enum EffectConclusion: String, Codable, Equatable, Sendable {
    case noEffect = "no_effect"
    case applied
    case compensated
    case unknown
}

public struct ToolOutcome: Codable, Equatable, Sendable {
    public let status: ToolStatus
    public let effect: EffectConclusion

    public init(status: ToolStatus, effect: EffectConclusion) {
        self.status = status
        self.effect = effect
    }
}

public struct ConnectorAction: Codable, Equatable, Identifiable, Sendable {
    public let id: ConnectorActionID
    public let connectorID: ConnectorID
    public let capability: String
    public let target: String
    public let effect: SideEffectKind
    public let idempotencyKey: String
    public let status: ConnectorActionStatus
    public let proposedAt: Date
    public let attentionID: AttentionID?
    public let toolCallID: ToolCallID?
    public let outcome: ToolOutcome?
    public let lastError: String?

    public init(
        id: ConnectorActionID,
        connectorID: ConnectorID,
        capability: String,
        target: String,
        effect: SideEffectKind,
        idempotencyKey: String,
        status: ConnectorActionStatus,
        proposedAt: Date,
        attentionID: AttentionID? = nil,
        toolCallID: ToolCallID? = nil,
        outcome: ToolOutcome? = nil,
        lastError: String? = nil
    ) {
        self.id = id
        self.connectorID = connectorID
        self.capability = capability
        self.target = target
        self.effect = effect
        self.idempotencyKey = idempotencyKey
        self.status = status
        self.proposedAt = proposedAt
        self.attentionID = attentionID
        self.toolCallID = toolCallID
        self.outcome = outcome
        self.lastError = lastError
    }

    private enum CodingKeys: String, CodingKey {
        case id
        case connectorID = "connector_id"
        case capability
        case target
        case effect
        case idempotencyKey = "idempotency_key"
        case status
        case proposedAt = "proposed_at"
        case attentionID = "attention_id"
        case toolCallID = "tool_call_id"
        case outcome
        case lastError = "last_error"
    }
}
