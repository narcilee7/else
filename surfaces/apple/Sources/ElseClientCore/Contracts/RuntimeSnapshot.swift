import Foundation

public struct ThreadSnapshot: Codable, Equatable, Identifiable, Sendable {
    public let spec: ThreadSpec
    public let state: ThreadState
    public let decisions: [AttentionRequest]
    public let artifacts: [Artifact]
    public let resourceBudget: ResourceBudget
    public let resourceUsage: ResourceUsage
    public let cursor: UInt64

    public var id: ThreadID { spec.id }
    public var isTerminal: Bool {
        switch state {
        case .completed, .cancelled, .failed: true
        default: false
        }
    }

    public init(
        spec: ThreadSpec,
        state: ThreadState,
        decisions: [AttentionRequest],
        artifacts: [Artifact],
        resourceBudget: ResourceBudget,
        resourceUsage: ResourceUsage,
        cursor: UInt64
    ) {
        self.spec = spec
        self.state = state
        self.decisions = decisions
        self.artifacts = artifacts
        self.resourceBudget = resourceBudget
        self.resourceUsage = resourceUsage
        self.cursor = cursor
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        spec = try container.decode(ThreadSpec.self, forKey: .spec)
        state = try container.decodeIfPresent(ThreadState.self, forKey: .state) ?? .active
        decisions = try container.decodeIfPresent(
            [AttentionRequest].self,
            forKey: .decisions
        ) ?? []
        artifacts = try container.decodeIfPresent([Artifact].self, forKey: .artifacts) ?? []
        resourceBudget = try container.decode(
            ResourceBudget.self,
            forKey: .resourceBudget
        )
        resourceUsage = try container.decode(
            ResourceUsage.self,
            forKey: .resourceUsage
        )
        cursor = try container.decodeIfPresent(UInt64.self, forKey: .cursor) ?? 0
    }

    private enum CodingKeys: String, CodingKey {
        case spec
        case state
        case decisions
        case artifacts
        case resourceBudget = "resource_budget"
        case resourceUsage = "resource_usage"
        case cursor
    }
}

public struct CreateThreadRequest: Codable, Equatable, Sendable {
    public let outcome: String
    public let project: String?

    public init(outcome: String, project: String?) {
        self.outcome = outcome
        self.project = project
    }
}

public struct ResolveDecisionRequest: Codable, Equatable, Sendable {
    public let choice: DecisionChoice

    public init(choice: DecisionChoice) {
        self.choice = choice
    }
}

public struct AbortThreadRequest: Codable, Equatable, Sendable {
    public let reason: String

    public init(reason: String) {
        self.reason = reason
    }
}
