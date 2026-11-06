import Foundation

public struct ID<Tag: Sendable>: RawRepresentable, Codable, Hashable, Comparable, Sendable,
    CustomStringConvertible
{
    public let rawValue: String

    public init(rawValue: String) {
        self.rawValue = rawValue
    }

    public init(_ rawValue: String) {
        self.rawValue = rawValue
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        rawValue = try container.decode(String.self)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(rawValue)
    }

    public static func < (lhs: Self, rhs: Self) -> Bool {
        lhs.rawValue < rhs.rawValue
    }

    public var description: String { rawValue }
}

public enum ThreadTag: Sendable {}
public enum AttentionTag: Sendable {}
public enum ArtifactTag: Sendable {}
public enum RunTag: Sendable {}
public enum WorldItemTag: Sendable {}
public enum WorldSignalTag: Sendable {}
public enum ConnectorTag: Sendable {}
public enum ConnectorActionTag: Sendable {}
public enum ToolCallTag: Sendable {}
public enum AttentionWindowTag: Sendable {}
public enum TimerTag: Sendable {}

public typealias ThreadID = ID<ThreadTag>
public typealias AttentionID = ID<AttentionTag>
public typealias ArtifactID = ID<ArtifactTag>
public typealias RunID = ID<RunTag>
public typealias WorldItemID = ID<WorldItemTag>
public typealias WorldSignalID = ID<WorldSignalTag>
public typealias ConnectorID = ID<ConnectorTag>
public typealias ConnectorActionID = ID<ConnectorActionTag>
public typealias ToolCallID = ID<ToolCallTag>
public typealias AttentionWindowID = ID<AttentionWindowTag>
public typealias TimerID = ID<TimerTag>
