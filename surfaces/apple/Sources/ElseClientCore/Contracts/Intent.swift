import Foundation

public struct Criterion: Codable, Equatable, Sendable {
    public let description: String

    public init(description: String) {
        self.description = description
    }
}

public struct Assumption: Codable, Equatable, Sendable {
    public let text: String
    public let confirmed: Bool

    public init(text: String, confirmed: Bool) {
        self.text = text
        self.confirmed = confirmed
    }
}

public enum SideEffectKind: String, Codable, CaseIterable, Sendable {
    case readOnly = "read_only"
    case reversible
    case compensatable
    case irreversible
    case unknown
}

public struct Authority: Codable, Equatable, Sendable {
    public let maxAutonomousEffect: SideEffectKind
    public let forbidden: [String]

    public init(maxAutonomousEffect: SideEffectKind, forbidden: [String]) {
        self.maxAutonomousEffect = maxAutonomousEffect
        self.forbidden = forbidden
    }

    private enum CodingKeys: String, CodingKey {
        case maxAutonomousEffect = "max_autonomous_effect"
        case forbidden
    }
}

public struct AttentionPolicy: Codable, Equatable, Sendable {
    public let allowInterrupt: Bool
    public let batchIntervalMinutes: UInt32

    public init(allowInterrupt: Bool, batchIntervalMinutes: UInt32) {
        self.allowInterrupt = allowInterrupt
        self.batchIntervalMinutes = batchIntervalMinutes
    }

    private enum CodingKeys: String, CodingKey {
        case allowInterrupt = "allow_interrupt"
        case batchIntervalMinutes = "batch_interval_minutes"
    }
}

public struct IntentSpec: Codable, Equatable, Identifiable, Sendable {
    public let id: IntentID
    public let revision: UInt64
    public let outcome: String
    public let doneWhen: [Criterion]
    public let constraints: [String]
    public let authority: Authority
    public let attentionPolicy: AttentionPolicy
    public let deadline: Date?
    public let assumptions: [Assumption]
    public let project: String?

    public init(
        id: IntentID,
        revision: UInt64,
        outcome: String,
        doneWhen: [Criterion],
        constraints: [String],
        authority: Authority,
        attentionPolicy: AttentionPolicy,
        deadline: Date? = nil,
        assumptions: [Assumption] = [],
        project: String? = nil
    ) {
        self.id = id
        self.revision = revision
        self.outcome = outcome
        self.doneWhen = doneWhen
        self.constraints = constraints
        self.authority = authority
        self.attentionPolicy = attentionPolicy
        self.deadline = deadline
        self.assumptions = assumptions
        self.project = project
    }

    private enum CodingKeys: String, CodingKey {
        case id
        case revision
        case outcome
        case doneWhen = "done_when"
        case constraints
        case authority
        case attentionPolicy = "attention_policy"
        case deadline
        case assumptions
        case project
    }
}

public enum IntentState: String, Codable, Sendable {
    case active
    case waitingExternal = "waiting_external"
    case needsDecision = "needs_decision"
    case paused
    case completed
    case cancelled
    case failed
}
