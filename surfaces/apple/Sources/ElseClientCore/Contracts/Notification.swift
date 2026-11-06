import Foundation

public enum ElseNotificationContract {
    public static let decisionCategory = "ELSE_DECISION"
    public static let openAction = "ELSE_OPEN"
    public static let threadIDKey = "thread_id"

    public static func decisionIdentifier(_ id: AttentionID) -> String {
        "decision:\(id.rawValue)"
    }
}

public enum ElsePreferenceKey {
    public static let notificationsEnabled = "ElseNotificationsEnabled"
    public static let backgroundRefreshEnabled = "ElseBackgroundRefreshEnabled"
    public static let backgroundRefreshInterval = "ElseBackgroundRefreshInterval"
}

public extension AttentionKind {
    var notificationTitle: String {
        switch self {
        case .judgment: "需要你的判断"
        case .authorization: "需要一次性授权"
        case .missingInfo: "需要补充信息"
        case .takeover: "需要你接管"
        case .verification: "等待验收确认"
        }
    }
}
