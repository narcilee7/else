import Testing
@testable import ElseClientCore

@Suite("Notification contract")
struct NotificationContractTests {
    @Test
    func sharedPreferencesKeepTheirPersistedKeys() {
        #expect(ElsePreferenceKey.notificationsEnabled == "ElseNotificationsEnabled")
        #expect(
            ElsePreferenceKey.backgroundRefreshEnabled
                == "ElseBackgroundRefreshEnabled"
        )
        #expect(
            ElsePreferenceKey.backgroundRefreshInterval
                == "ElseBackgroundRefreshInterval"
        )
    }

    @Test
    func everyAttentionKindHasAUserFacingNotificationTitle() {
        let kinds: [AttentionKind] = [
            .judgment,
            .authorization,
            .missingInfo,
            .takeover,
            .verification,
        ]

        #expect(kinds.allSatisfy { !$0.notificationTitle.isEmpty })
    }
}
