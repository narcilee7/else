import Foundation

public enum RuntimeCapabilityKind: String, Codable, Sendable {
    case builtinTool = "builtin_tool"
    case mcpTool = "mcp_tool"
    case skill
    case externalAction = "external_action"
}

public struct RuntimeCapabilityManifest: Codable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let description: String
    public let kind: RuntimeCapabilityKind
    public let effect: SideEffectKind
    public let idempotencyRequired: Bool
    public let inputSchema: JSONValue

    public init(
        id: String,
        name: String,
        description: String,
        kind: RuntimeCapabilityKind,
        effect: SideEffectKind,
        idempotencyRequired: Bool,
        inputSchema: JSONValue
    ) {
        self.id = id
        self.name = name
        self.description = description
        self.kind = kind
        self.effect = effect
        self.idempotencyRequired = idempotencyRequired
        self.inputSchema = inputSchema
    }

    private enum CodingKeys: String, CodingKey {
        case id
        case name
        case description
        case kind
        case effect
        case idempotencyRequired = "idempotency_required"
        case inputSchema = "input_schema"
    }
}

public struct RuntimeExternalActionBinding: Codable, Equatable, Sendable {
    public let connectorID: ConnectorID
    public let operation: String

    private enum CodingKeys: String, CodingKey {
        case connectorID = "connector_id"
        case operation
    }
}

public struct RuntimeCapabilityAdapter: Codable, Equatable, Sendable {
    public let kind: String
    public let value: RuntimeExternalActionBinding?
}

public struct RuntimeCapabilityRegistration: Codable, Equatable, Sendable {
    public let manifest: RuntimeCapabilityManifest
    public let adapter: RuntimeCapabilityAdapter

    public var externalAction: RuntimeExternalActionBinding? {
        guard adapter.kind == "external_action" else {
            return nil
        }
        return adapter.value
    }
}

public struct ConnectorActionFieldOption: Equatable, Sendable {
    public let value: JSONValue
    public let label: String
}

public struct ConnectorActionFieldSchema: Equatable, Sendable, Identifiable {
    public let path: String
    public let title: String
    public let kind: String
    public let required: Bool
    public let options: [ConnectorActionFieldOption]

    public var id: String { path }
}

public struct ConnectorActionSchema: Equatable, Sendable {
    public let title: String
    public let targetLabel: String
    public let previewSupported: Bool
    public let fields: [ConnectorActionFieldSchema]
}

public extension JSONValue {
    var connectorActionSchema: ConnectorActionSchema? {
        guard let object = objectValue,
              let title = object["title"]?.stringValue,
              let targetLabel = object["target_label"]?.stringValue,
              let fieldsValue = object["fields"]?.arrayValue
        else {
            return nil
        }
        let fields = fieldsValue.compactMap { field -> ConnectorActionFieldSchema? in
            guard let value = field.objectValue,
                  let path = value["path"]?.stringValue,
                  let title = value["title"]?.stringValue,
                  let kind = value["kind"]?.stringValue
            else {
                return nil
            }
            let options = value["options"]?.arrayValue?.compactMap {
                option -> ConnectorActionFieldOption? in
                guard let object = option.objectValue,
                      let value = object["value"],
                      let label = object["label"]?.stringValue
                else {
                    return nil
                }
                return ConnectorActionFieldOption(value: value, label: label)
            } ?? []
            return ConnectorActionFieldSchema(
                path: path,
                title: title,
                kind: kind,
                required: value["required"]?.boolValue ?? false,
                options: options
            )
        }
        return ConnectorActionSchema(
            title: title,
            targetLabel: targetLabel,
            previewSupported: object["preview_supported"]?.boolValue ?? false,
            fields: fields
        )
    }

    var stringValue: String? {
        if case let .string(value) = self {
            return value
        }
        return nil
    }

    var boolValue: Bool? {
        if case let .bool(value) = self {
            return value
        }
        return nil
    }

    var arrayValue: [JSONValue]? {
        if case let .array(value) = self {
            return value
        }
        return nil
    }
}
