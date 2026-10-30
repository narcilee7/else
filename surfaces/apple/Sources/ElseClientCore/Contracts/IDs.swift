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

public enum IntentTag: Sendable {}
public enum AttentionTag: Sendable {}
public enum ArtifactTag: Sendable {}
public enum RunTag: Sendable {}

public typealias IntentID = ID<IntentTag>
public typealias AttentionID = ID<AttentionTag>
public typealias ArtifactID = ID<ArtifactTag>
public typealias RunID = ID<RunTag>
