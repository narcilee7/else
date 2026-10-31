import AppKit
import ElseClientCore
import Foundation
import UserNotifications

extension Notification.Name {
    static let elseOpenIntent = Notification.Name("else.openIntent")
    static let elseOpenRoute = Notification.Name("else.openRoute")
    static let elseQuickIntent = Notification.Name("else.quickIntent")
}

@MainActor
final class NotificationCoordinator: NSObject, UNUserNotificationCenterDelegate {
    static let shared = NotificationCoordinator()

    private let center = UNUserNotificationCenter.current()

    func configure() {
        center.delegate = self
        center.setNotificationCategories([
            UNNotificationCategory(
                identifier: ElseNotificationContract.decisionCategory,
                actions: [
                    UNNotificationAction(
                        identifier: ElseNotificationContract.openAction,
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
        content.categoryIdentifier = ElseNotificationContract.decisionCategory
        content.userInfo = [
            ElseNotificationContract.intentIDKey: intentID.rawValue,
        ]

        let notification = UNNotificationRequest(
            identifier: ElseNotificationContract.decisionIdentifier(request.id),
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
        guard let rawIntentID = userInfo[ElseNotificationContract.intentIDKey] as? String
        else {
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
