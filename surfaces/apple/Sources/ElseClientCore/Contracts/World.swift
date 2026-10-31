import Foundation

public enum WorldObjectKind: String, Codable, Sendable {
    case calendarEvent = "calendar_event"
    case workItem = "work_item"
    case milestone
    case message
    case document
    case other
}

public enum WorldItemKind: String, Codable, Sendable {
    case commitment
    case waitingExternal = "waiting_external"
    case actionItem = "action_item"
    case risk
}

public enum WorldItemState: String, Codable, Sendable {
    case open
    case waitingExternal = "waiting_external"
    case overdue
    case atRisk = "at_risk"
    case completed
    case dismissed
    case superseded

    public var isResolved: Bool {
        self == .completed || self == .dismissed || self == .superseded
    }
}

public enum WorldItemResolutionKind: String, Codable, Sendable {
    case completed
    case dismissed
}

public enum CorrelationKind: String, Codable, CaseIterable, Hashable, Sendable {
    case project
    case contact
    case customer
    case thread
    case milestone
}

public enum WorldCorrelationCorrectionKind: String, Codable, Hashable, Sendable {
    case merge
    case split
    case rename
}

public struct ExternalEntityRef: Codable, Equatable, Sendable {
    public let provider: String
    public let tenant: String?
    public let account: String?
    public let entityKind: String
    public let externalID: String
    public let canonicalURL: URL?
    public let sourceVersion: String?

    public init(
        provider: String,
        tenant: String? = nil,
        account: String? = nil,
        entityKind: String,
        externalID: String,
        canonicalURL: URL? = nil,
        sourceVersion: String? = nil
    ) {
        self.provider = provider
        self.tenant = tenant
        self.account = account
        self.entityKind = entityKind
        self.externalID = externalID
        self.canonicalURL = canonicalURL
        self.sourceVersion = sourceVersion
    }

    private enum CodingKeys: String, CodingKey {
        case provider
        case tenant
        case account
        case entityKind = "entity_kind"
        case externalID = "external_id"
        case canonicalURL = "canonical_url"
        case sourceVersion = "source_version"
    }
}

public struct CorrelationHint: Codable, Equatable, Sendable {
    public let kind: String
    public let key: String
    public let label: String

    public init(kind: String, key: String, label: String) {
        self.kind = kind
        self.key = key
        self.label = label
    }
}

public struct WorldItem: Codable, Equatable, Identifiable, Sendable {
    public let id: WorldItemID
    public let entityRef: ExternalEntityRef
    public let objectKind: WorldObjectKind
    public let kind: WorldItemKind
    public let state: WorldItemState
    public let title: String
    public let summary: String
    public let deadline: Date?
    public let priority: Int
    public let priorityReason: String
    public let signalIDs: [WorldSignalID]
    public let connectorIDs: [ConnectorID]
    public let projects: [CorrelationHint]
    public let identities: [CorrelationHint]
    public let firstObservedAt: Date
    public let lastObservedAt: Date

    public init(
        id: WorldItemID,
        entityRef: ExternalEntityRef,
        objectKind: WorldObjectKind,
        kind: WorldItemKind,
        state: WorldItemState,
        title: String,
        summary: String,
        deadline: Date? = nil,
        priority: Int = 0,
        priorityReason: String = "",
        signalIDs: [WorldSignalID] = [],
        connectorIDs: [ConnectorID] = [],
        projects: [CorrelationHint] = [],
        identities: [CorrelationHint] = [],
        firstObservedAt: Date,
        lastObservedAt: Date
    ) {
        self.id = id
        self.entityRef = entityRef
        self.objectKind = objectKind
        self.kind = kind
        self.state = state
        self.title = title
        self.summary = summary
        self.deadline = deadline
        self.priority = priority
        self.priorityReason = priorityReason
        self.signalIDs = signalIDs
        self.connectorIDs = connectorIDs
        self.projects = projects
        self.identities = identities
        self.firstObservedAt = firstObservedAt
        self.lastObservedAt = lastObservedAt
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(WorldItemID.self, forKey: .id)
        entityRef = try container.decode(ExternalEntityRef.self, forKey: .entityRef)
        objectKind = try container.decodeIfPresent(
            WorldObjectKind.self,
            forKey: .objectKind
        ) ?? .other
        kind = try container.decode(WorldItemKind.self, forKey: .kind)
        state = try container.decode(WorldItemState.self, forKey: .state)
        title = try container.decode(String.self, forKey: .title)
        summary = try container.decode(String.self, forKey: .summary)
        deadline = try container.decodeIfPresent(Date.self, forKey: .deadline)
        priority = try container.decodeIfPresent(Int.self, forKey: .priority) ?? 0
        priorityReason = try container.decodeIfPresent(
            String.self,
            forKey: .priorityReason
        ) ?? ""
        signalIDs = try container.decodeIfPresent(
            [WorldSignalID].self,
            forKey: .signalIDs
        ) ?? []
        connectorIDs = try container.decodeIfPresent(
            [ConnectorID].self,
            forKey: .connectorIDs
        ) ?? []
        projects = try container.decodeIfPresent(
            [CorrelationHint].self,
            forKey: .projects
        ) ?? []
        identities = try container.decodeIfPresent(
            [CorrelationHint].self,
            forKey: .identities
        ) ?? []
        firstObservedAt = try container.decode(Date.self, forKey: .firstObservedAt)
        lastObservedAt = try container.decode(Date.self, forKey: .lastObservedAt)
    }

    private enum CodingKeys: String, CodingKey {
        case id
        case entityRef = "entity_ref"
        case objectKind = "object_kind"
        case kind
        case state
        case title
        case summary
        case deadline
        case priority
        case priorityReason = "priority_reason"
        case signalIDs = "signal_ids"
        case connectorIDs = "connector_ids"
        case projects
        case identities
        case firstObservedAt = "first_observed_at"
        case lastObservedAt = "last_observed_at"
    }
}

public struct WorldItemRecord: Codable, Equatable, Sendable {
    public let intentID: IntentID
    public let item: WorldItem

    public init(intentID: IntentID, item: WorldItem) {
        self.intentID = intentID
        self.item = item
    }

    private enum CodingKeys: String, CodingKey {
        case intentID = "intent_id"
        case item
    }
}

public struct PersonalBrief: Codable, Equatable, Sendable {
    public let generatedAt: Date
    public let openCommitments: [WorldItemRecord]
    public let waitingExternal: [WorldItemRecord]
    public let overdue: [WorldItemRecord]
    public let actionItemsDueSoon: [WorldItemRecord]
    public let risks: [WorldItemRecord]
    public let upcomingCalendar: [WorldItemRecord]

    public init(
        generatedAt: Date,
        openCommitments: [WorldItemRecord] = [],
        waitingExternal: [WorldItemRecord] = [],
        overdue: [WorldItemRecord] = [],
        actionItemsDueSoon: [WorldItemRecord] = [],
        risks: [WorldItemRecord] = [],
        upcomingCalendar: [WorldItemRecord] = []
    ) {
        self.generatedAt = generatedAt
        self.openCommitments = openCommitments
        self.waitingExternal = waitingExternal
        self.overdue = overdue
        self.actionItemsDueSoon = actionItemsDueSoon
        self.risks = risks
        self.upcomingCalendar = upcomingCalendar
    }

    public var outstandingCount: Int {
        openCommitments.count
            + waitingExternal.count
            + overdue.count
            + actionItemsDueSoon.count
            + risks.count
    }

    private enum CodingKeys: String, CodingKey {
        case generatedAt = "generated_at"
        case openCommitments = "open_commitments"
        case waitingExternal = "waiting_external"
        case overdue
        case actionItemsDueSoon = "action_items_due_soon"
        case risks
        case upcomingCalendar = "upcoming_calendar"
    }
}

public struct ResolveWorldItemRequest: Codable, Equatable, Sendable {
    public let itemID: WorldItemID
    public let kind: WorldItemResolutionKind
    public let note: String?

    public init(itemID: WorldItemID, kind: WorldItemResolutionKind, note: String? = nil) {
        self.itemID = itemID
        self.kind = kind
        self.note = note
    }

    private enum CodingKeys: String, CodingKey {
        case itemID = "item_id"
        case kind
        case note
    }
}

public struct ProjectWorld: Codable, Equatable, Identifiable, Sendable {
    public let key: String
    public let title: String
    public let items: [WorldItemRecord]
    public let intentIDs: [IntentID]
    public let connectorIDs: [ConnectorID]
    public let openItems: Int
    public let waitingExternal: Int
    public let overdue: Int
    public let nextDeadline: Date?

    public var id: String { key }

    public init(
        key: String,
        title: String,
        items: [WorldItemRecord],
        intentIDs: [IntentID],
        connectorIDs: [ConnectorID],
        openItems: Int,
        waitingExternal: Int,
        overdue: Int,
        nextDeadline: Date?
    ) {
        self.key = key
        self.title = title
        self.items = items
        self.intentIDs = intentIDs
        self.connectorIDs = connectorIDs
        self.openItems = openItems
        self.waitingExternal = waitingExternal
        self.overdue = overdue
        self.nextDeadline = nextDeadline
    }

    private enum CodingKeys: String, CodingKey {
        case key
        case title
        case items
        case intentIDs = "intent_ids"
        case connectorIDs = "connector_ids"
        case openItems = "open_items"
        case waitingExternal = "waiting_external"
        case overdue
        case nextDeadline = "next_deadline"
    }
}

public struct IdentityWorld: Codable, Equatable, Identifiable, Sendable {
    public let kind: CorrelationKind
    public let key: String
    public let title: String
    public let items: [WorldItemRecord]
    public let intentIDs: [IntentID]
    public let connectorIDs: [ConnectorID]
    public let openItems: Int
    public let overdue: Int
    public let nextDeadline: Date?

    public var id: String { "\(kind.rawValue):\(key)" }

    public init(
        kind: CorrelationKind,
        key: String,
        title: String,
        items: [WorldItemRecord],
        intentIDs: [IntentID],
        connectorIDs: [ConnectorID],
        openItems: Int,
        overdue: Int,
        nextDeadline: Date?
    ) {
        self.kind = kind
        self.key = key
        self.title = title
        self.items = items
        self.intentIDs = intentIDs
        self.connectorIDs = connectorIDs
        self.openItems = openItems
        self.overdue = overdue
        self.nextDeadline = nextDeadline
    }

    private enum CodingKeys: String, CodingKey {
        case kind
        case key
        case title
        case items
        case intentIDs = "intent_ids"
        case connectorIDs = "connector_ids"
        case openItems = "open_items"
        case overdue
        case nextDeadline = "next_deadline"
    }
}

public struct WorldCorrelationCorrection: Codable, Equatable, Sendable {
    public let id: String
    public let kind: WorldCorrelationCorrectionKind
    public let correlationKind: CorrelationKind
    public let sourceKeys: [String]
    public let targetKey: String?
    public let label: String?
    public let itemID: WorldItemID?
    public let correctedAt: Date

    public init(
        id: String = UUID().uuidString,
        kind: WorldCorrelationCorrectionKind,
        correlationKind: CorrelationKind,
        sourceKeys: [String],
        targetKey: String? = nil,
        label: String? = nil,
        itemID: WorldItemID? = nil,
        correctedAt: Date = Date()
    ) {
        self.id = id
        self.kind = kind
        self.correlationKind = correlationKind
        self.sourceKeys = sourceKeys
        self.targetKey = targetKey
        self.label = label
        self.itemID = itemID
        self.correctedAt = correctedAt
    }

    private enum CodingKeys: String, CodingKey {
        case id
        case kind
        case correlationKind = "correlation_kind"
        case sourceKeys = "source_keys"
        case targetKey = "target_key"
        case label
        case itemID = "item_id"
        case correctedAt = "corrected_at"
    }
}
