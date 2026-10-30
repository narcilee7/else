import Foundation

public enum AttentionKind: String, Codable, Sendable {
    case judgment
    case authorization
    case missingInfo = "missing_info"
    case takeover
}

public enum ExpectedLoss: String, Codable, Comparable, Sendable {
    case low
    case medium
    case high
    case critical

    private var rank: Int {
        switch self {
        case .low: 0
        case .medium: 1
        case .high: 2
        case .critical: 3
        }
    }

    public static func < (lhs: Self, rhs: Self) -> Bool {
        lhs.rank < rhs.rank
    }
}

public struct DecisionOption: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let label: String
    public let recommended: Bool

    public init(id: String, label: String, recommended: Bool) {
        self.id = id
        self.label = label
        self.recommended = recommended
    }
}

public struct AttentionRequest: Codable, Equatable, Identifiable, Sendable {
    public let id: AttentionID
    public let kind: AttentionKind
    public let summary: String
    public let blockedReason: String
    public let expectedLoss: ExpectedLoss
    public let blocking: Bool
    public let reversible: Bool
    public let decideBy: Date?
    public let options: [DecisionOption]
    public let defaultOption: String?
    public let relatedCallKey: String?

    public init(
        id: AttentionID,
        kind: AttentionKind,
        summary: String,
        blockedReason: String,
        expectedLoss: ExpectedLoss,
        blocking: Bool,
        reversible: Bool,
        decideBy: Date? = nil,
        options: [DecisionOption] = [],
        defaultOption: String? = nil,
        relatedCallKey: String? = nil
    ) {
        self.id = id
        self.kind = kind
        self.summary = summary
        self.blockedReason = blockedReason
        self.expectedLoss = expectedLoss
        self.blocking = blocking
        self.reversible = reversible
        self.decideBy = decideBy
        self.options = options
        self.defaultOption = defaultOption
        self.relatedCallKey = relatedCallKey
    }

    private enum CodingKeys: String, CodingKey {
        case id
        case kind
        case summary
        case blockedReason = "blocked_reason"
        case expectedLoss = "expected_loss"
        case blocking
        case reversible
        case decideBy = "decide_by"
        case options
        case defaultOption = "default_option"
        case relatedCallKey = "related_call_key"
    }
}

public enum DecisionChoice: Equatable, Sendable {
    case option(String)
    case freeText(String)
    case dismissed
}

extension DecisionChoice: Codable {
    private enum CodingKeys: String, CodingKey {
        case kind
        case value
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(String.self, forKey: .kind) {
        case "option":
            self = .option(try container.decode(String.self, forKey: .value))
        case "free_text":
            self = .freeText(try container.decode(String.self, forKey: .value))
        case "dismissed":
            self = .dismissed
        case let kind:
            throw DecodingError.dataCorruptedError(
                forKey: .kind,
                in: container,
                debugDescription: "Unknown decision choice kind: \(kind)"
            )
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .option(value):
            try container.encode("option", forKey: .kind)
            try container.encode(value, forKey: .value)
        case let .freeText(value):
            try container.encode("free_text", forKey: .kind)
            try container.encode(value, forKey: .value)
        case .dismissed:
            try container.encode("dismissed", forKey: .kind)
        }
    }
}
