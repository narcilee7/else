import AppKit
import ElseClientCore
import Foundation
import UserNotifications

extension Notification.Name {
    static let elseOpenIntent = Notification.Name("else.openIntent")
}

@MainActor
final class NotificationCoordinator: NSObject, UNUserNotificationCenterDelegate {
    static let shared = NotificationCoordinator()

    private let center = UNUserNotificationCenter.current()
    private static let decisionCategory = "ELSE_DECISION"
    private static let openAction = "ELSE_OPEN"

    func configure() {
        center.delegate = self
        center.setNotificationCategories([
            UNNotificationCategory(
                identifier: Self.decisionCategory,
                actions: [
                    UNNotificationAction(
                        identifier: Self.openAction,
                        title: "查看",
                        options: [.foreground]
                    ),
                ],
                intentIdentifiers: [],
                options: []
            ),
        ])
    }

    func requestAuthorization() async -> Bool {
        do {
            return try await center.requestAuthorization(options: [.alert, .sound, .badge])
        } catch {
            return false
        }
    }

    func postDecisionNotification(
        intentID: IntentID,
        request: AttentionRequest
    ) async {
        let content = UNMutableNotificationContent()
        content.title = request.kind.notificationTitle
        content.body = request.summary
        content.sound = .default
        content.categoryIdentifier = Self.decisionCategory
        content.userInfo = ["intent_id": intentID.rawValue]

        let notification = UNNotificationRequest(
            identifier: "decision:\(request.id.rawValue)",
            content: content,
            trigger: nil
        )
        try? await center.add(notification)
    }

    func updateBadge(decisionCount: Int) {
        NSApplication.shared.dockTile.badgeLabel =
            decisionCount > 0 ? "\(decisionCount)" : nil
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .sound, .list]
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse
    ) async {
        let userInfo = response.notification.request.content.userInfo
        guard let rawIntentID = userInfo["intent_id"] as? String else {
            return
        }
        await MainActor.run {
            NSApplication.shared.activate(ignoringOtherApps: true)
            Foundation.NotificationCenter.default.post(
                name: .elseOpenIntent,
                object: IntentID(rawIntentID)
            )
        }
    }
}

private extension AttentionKind {
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
