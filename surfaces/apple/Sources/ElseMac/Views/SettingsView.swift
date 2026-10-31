import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var store: AppStore
    @State private var launchAtLogin = LaunchAtLoginManager.isEnabled
    @State private var launchAtLoginError: String?

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

            Section("常驻") {
                Toggle(
                    "登录时启动",
                    isOn: Binding(
                        get: { launchAtLogin },
                        set: { enabled in
                            do {
                                try LaunchAtLoginManager.setEnabled(enabled)
                                launchAtLogin = enabled
                                launchAtLoginError = nil
                            } catch {
                                launchAtLogin = LaunchAtLoginManager.isEnabled
                                launchAtLoginError = error.localizedDescription
                            }
                        }
                    )
                )
                if let launchAtLoginError {
                    Text(launchAtLoginError)
                        .font(ElseType.caption)
                        .foregroundStyle(ElseColor.danger)
                }
            }

            Section("Runtime") {
                MetricRow(label: "连接", value: store.connection.label)
                MetricRow(
                    label: "最近刷新",
                    value:
                        store.lastRefreshedAt?.formatted(
                            date: .omitted,
                            time: .standard
                        ) ?? "从未"
                )
                MetricRow(
                    label: "事件水位",
                    value: store.snapshots.map(\.cursor).max()?.formatted() ?? "0"
                )
                MetricRow(
                    label: "连接器",
                    value: "\(store.connectorsByIntent.values.reduce(0) { $0 + $1.count })"
                )
                MetricRow(
                    label: "健康",
                    value:
                        "\(store.connectorHealth.values.filter { $0.health == .healthy }.count) / \(store.connectorHealth.count)"
                )
            }
        }
        .formStyle(.grouped)
        .padding()
        .frame(width: 440, height: 470)
    }
}
