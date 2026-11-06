import Foundation

public enum ElseDeepLink: Equatable, Sendable {
    case today
    case world
    case connectors
    case decisions
    case results
    case newThread
    case thread(ThreadID)

    public init?(url: URL) {
        guard url.scheme?.lowercased() == "else",
              let host = url.host?.lowercased()
        else {
            return nil
        }
        switch host {
        case "today":
            self = .today
        case "world":
            self = .world
        case "connectors":
            self = .connectors
        case "decisions":
            self = .decisions
        case "results":
            self = .results
        case "new-thread":
            self = .newThread
        case "thread":
            let components = url.pathComponents.filter { $0 != "/" }
            guard let value = components.first, !value.isEmpty else {
                return nil
            }
            self = .thread(ThreadID(value))
        default:
            return nil
        }
    }

    public var url: URL? {
        switch self {
        case .today:
            URL(string: "else://today")
        case .world:
            URL(string: "else://world")
        case .connectors:
            URL(string: "else://connectors")
        case .decisions:
            URL(string: "else://decisions")
        case .results:
            URL(string: "else://results")
        case .newThread:
            URL(string: "else://new-thread")
        case let .thread(id):
            URL(string: "else://thread/\(id.rawValue)")
        }
    }
}
