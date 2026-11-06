import AppKit
import ElseClientCore
import Foundation
import UserNotifications

extension Notification.Name {
    static let elseOpenThread = Notification.Name("else.openThread")
    static let elseOpenRoute = Notification.Name("else.openRoute")
    static let elseQuickThread = Notification.Name("else.quickThread")
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
        threadID: ThreadID,
        request: AttentionRequest
    ) async {
        let content = UNMutableNotificationContent()
        content.title = request.kind.notificationTitle
        content.body = request.summary
        content.sound = .default
        content.categoryIdentifier = ElseNotificationContract.decisionCategory
        content.userInfo = [
            ElseNotificationContract.threadIDKey: threadID.rawValue,
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
        guard let rawThreadID = userInfo[ElseNotificationContract.threadIDKey] as? String
        else {
            return
        }
        await MainActor.run {
            NSApplication.shared.activate(ignoringOtherApps: true)
            Foundation.NotificationCenter.default.post(
                name: .elseOpenThread,
                object: ThreadID(rawThreadID)
            )
        }
    }
}
