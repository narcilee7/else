import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var store: AppStore

    var body: some View {
        Form {
            Section("Daily Brief") {
                Toggle(
                    "系统通知",
                    isOn: Binding(
                        get: { store.notificationsEnabled },
                        set: { enabled in
                            Task { await store.setNotificationsEnabled(enabled) }
                        }
                    )
                )

                Toggle(
                    "后台刷新",
                    isOn: Binding(
                        get: { store.backgroundRefreshEnabled },
                        set: { store.setBackgroundRefreshEnabled($0) }
                    )
                )

                Picker(
                    "刷新间隔",
                    selection: Binding(
                        get: { store.backgroundRefreshInterval },
                        set: { store.setBackgroundRefreshInterval($0) }
                    )
                ) {
                    Text("15 秒").tag(15.0)
                    Text("30 秒").tag(30.0)
                    Text("60 秒").tag(60.0)
                }
                .disabled(!store.backgroundRefreshEnabled)
            }
        }
        .formStyle(.grouped)
        .padding()
        .frame(width: 420, height: 230)
    }
}
