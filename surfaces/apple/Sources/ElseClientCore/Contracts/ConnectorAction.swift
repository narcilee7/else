import Foundation

public enum JSONValue: Codable, Equatable, Sendable {
    case object([String: JSONValue])
    case array([JSONValue])
    case string(String)
    case number(Double)
    case bool(Bool)
    case null

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
        } else {
            self = .object(try container.decode([String: JSONValue].self))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case let .object(value): try container.encode(value)
        case let .array(value): try container.encode(value)
        case let .string(value): try container.encode(value)
        case let .number(value): try container.encode(value)
        case let .bool(value): try container.encode(value)
        case .null: try container.encodeNil()
        }
    }

    public static func parse(_ text: String) throws -> JSONValue {
        try JSONDecoder().decode(JSONValue.self, from: Data(text.utf8))
    }
}

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

public struct ConnectorActionPreview: Codable, Equatable, Sendable {
    public let connectorID: ConnectorID
    public let capability: String
    public let target: String
    public let current: JSONValue
    public let proposed: JSONValue
    public let checkedAt: Date

    public init(
        connectorID: ConnectorID,
        capability: String,
        target: String,
        current: JSONValue,
        proposed: JSONValue,
        checkedAt: Date
    ) {
        self.connectorID = connectorID
        self.capability = capability
        self.target = target
        self.current = current
        self.proposed = proposed
        self.checkedAt = checkedAt
    }

    private enum CodingKeys: String, CodingKey {
        case connectorID = "connector_id"
        case capability
        case target
        case current
        case proposed
        case checkedAt = "checked_at"
    }
}

public struct ConnectorActionFieldDiff: Identifiable, Equatable, Sendable {
    public let path: String
    public let current: JSONValue
    public let proposed: JSONValue

    public var id: String { path }
}

public enum ConnectorActionInputDiff {
    public static func changes(
        current: JSONValue,
        proposed: JSONValue
    ) -> [ConnectorActionFieldDiff] {
        let currentFields = flattened(current)
        let proposedFields = flattened(proposed)
        let paths = proposedFields.keys.sorted()
        return paths.compactMap { path in
            let before = currentFields[path] ?? .null
            let after = proposedFields[path] ?? .null
            guard before != after else {
                return nil
            }
            return ConnectorActionFieldDiff(
                path: path,
                current: before,
                proposed: after
            )
        }
    }

    public static func display(_ value: JSONValue) -> String {
        switch value {
        case let .string(value): value
        case let .number(value):
            value.rounded() == value
                ? String(Int(value))
                : String(value)
        case let .bool(value): value ? "true" : "false"
        case .null: "未设置"
        case let .object(value):
            encode(.object(value))
        case let .array(value):
            encode(.array(value))
        }
    }

    private static func flattened(
        _ value: JSONValue,
        prefix: String = ""
    ) -> [String: JSONValue] {
        switch value {
        case let .object(fields):
            return fields.reduce(into: [:]) { result, item in
                let path = prefix.isEmpty ? item.key : "\(prefix).\(item.key)"
                result.merge(flattened(item.value, prefix: path)) { _, new in new }
            }
        case let .array(values):
            return values.enumerated().reduce(into: [:]) { result, item in
                let path = prefix.isEmpty ? "[\(item.offset)]" : "\(prefix)[\(item.offset)]"
                result.merge(flattened(item.element, prefix: path)) { _, new in new }
            }
        default:
            return [prefix: value]
        }
    }

    private static func encode(_ value: JSONValue) -> String {
        guard let data = try? JSONEncoder().encode(value),
              let string = String(data: data, encoding: .utf8)
        else {
            return "{}"
        }
        return string
    }
}

public struct ProposeConnectorActionRequest: Codable, Equatable, Sendable {
    public let connectorID: ConnectorID
    public let capability: String
    public let target: String
    public let input: JSONValue
    public let idempotencyKey: String?

    public init(
        connectorID: ConnectorID,
        capability: String,
        target: String,
        input: JSONValue,
        idempotencyKey: String? = nil
    ) {
        self.connectorID = connectorID
        self.capability = capability
        self.target = target
        self.input = input
        self.idempotencyKey = idempotencyKey
    }

    private enum CodingKeys: String, CodingKey {
        case connectorID = "connector_id"
        case capability
        case target
        case input
        case idempotencyKey = "idempotency_key"
    }
}

public struct PreviewConnectorActionRequest: Codable, Equatable, Sendable {
    public let capability: String
    public let target: String
    public let input: JSONValue

    public init(capability: String, target: String, input: JSONValue) {
        self.capability = capability
        self.target = target
        self.input = input
    }
}
