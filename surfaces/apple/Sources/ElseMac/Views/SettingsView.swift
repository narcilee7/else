import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct SettingsView: View {
    @EnvironmentObject private var store: AppStore
    @EnvironmentObject private var runtimeService: RuntimeServiceManager
    @State private var launchAtLogin = LaunchAtLoginManager.isEnabled
    @State private var launchAtLoginError: String?
    @State private var isChangingRuntimeService = false
    @State private var isExportingData = false
    @State private var exportStatus: String?
    @State private var pendingRetentionDays: UInt64?
    @State private var isConfirmingRetention = false
    @State private var isExportingDiagnostics = false
    @State private var isConfirmingLeaseCleanup = false
    @State private var isCleaningLeases = false
    @State private var leaseCleanupStatus: String?

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

            Section("后台 Runtime") {
                MetricRow(
                    label: "Runtime",
                    value: runtimeService.status.label
                )
                MetricRow(
                    label: "通知",
                    value: runtimeService.notifierStatus.label
                )
                MetricRow(
                    label: "Token",
                    value: runtimeService.tokenConfigured ? "已配置" : "未配置"
                )
                MetricRow(
                    label: "地址",
                    value: runtimeService.runtimeURL.absoluteString
                )

                HStack {
                    Button("启动") {
                        runtimeService.start()
                    }
                    .disabled(
                        isChangingRuntimeService
                            || runtimeService.allAgentsEnabled
                    )

                    Button("停止") {
                        isChangingRuntimeService = true
                        Task {
                            await runtimeService.stop()
                            isChangingRuntimeService = false
                        }
                    }
                    .disabled(
                        isChangingRuntimeService
                            || !runtimeService.hasRegisteredAgent
                    )

                    Button("重启") {
                        isChangingRuntimeService = true
                        Task {
                            await runtimeService.restart()
                            isChangingRuntimeService = false
                        }
                    }
                    .disabled(isChangingRuntimeService)

                    Spacer()

                    Button("重新连接") {
                        Task {
                            if !store.isDemo {
                                _ = await runtimeService.waitUntilReachable()
                            }
                            await store.refresh()
                        }
                    }
                    .disabled(store.isRefreshing)

                    Button("打开数据目录") {
                        runtimeService.openRuntimeDirectory()
                    }
                }

                if runtimeService.status == .requiresApproval {
                    Button("打开系统登录项设置") {
                        runtimeService.openSystemApprovalSettings()
                    }
                }

                if runtimeService.status == .notFound {
                    Text("Developer ID 签名并安装后可启用独立后台服务。")
                        .font(ElseType.caption)
                        .foregroundStyle(ElseColor.textSecondary)
                }

                if let message = runtimeService.errorMessage {
                    Text(message)
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

            Section("数据") {
                Button {
                    Task { await exportData() }
                } label: {
                    Label(
                        isExportingData ? "正在导出" : "导出 Runtime 数据",
                        systemImage: "square.and.arrow.up"
                    )
                }
                .disabled(isExportingData)

                Button {
                    Task { await exportDiagnostics() }
                } label: {
                    Label(
                        isExportingDiagnostics ? "正在生成诊断" : "导出诊断报告",
                        systemImage: "stethoscope"
                    )
                }
                .disabled(isExportingDiagnostics)

                Button {
                    isConfirmingLeaseCleanup = true
                } label: {
                    Label(
                        isCleaningLeases ? "正在清理" : "清理孤儿 Lease",
                        systemImage: "wrench.and.screwdriver"
                    )
                }
                .disabled(isCleaningLeases)
                .confirmationDialog(
                    "清理孤儿 Lease？",
                    isPresented: $isConfirmingLeaseCleanup,
                    titleVisibility: .visible
                ) {
                    Button("清理孤儿 Lease", role: .destructive) {
                        Task { await cleanupLeases() }
                    }
                    Button("取消", role: .cancel) {}
                } message: {
                    Text("只会回收 Run 已结束、Scope 已终态或 Generation 已失效的 Lease；活跃 Run 不会受影响。")
                }

                Text("导出 Event Ledger、Intent、工件内容与来源血缘；凭据和私有 Harness 状态不会写入导出包。")
                    .font(ElseType.caption)
                    .foregroundStyle(ElseColor.textSecondary)

                Picker(
                    "终态 Intent 保留",
                    selection: Binding(
                        get: { store.retentionDays },
                        set: { days in
                            if let days {
                                pendingRetentionDays = days
                                isConfirmingRetention = true
                            } else {
                                Task { await store.setRetentionDays(nil) }
                            }
                        }
                    )
                ) {
                    Text("永不自动删除").tag(UInt64?.none)
                    Text("7 天").tag(UInt64?.some(7))
                    Text("30 天").tag(UInt64?.some(30))
                    Text("90 天").tag(UInt64?.some(90))
                }

                Text("自动清理只作用于 Completed / Failed / Cancelled Intent；运行中、待决策与等待外部状态不会自动删除。")
                    .font(ElseType.caption)
                    .foregroundStyle(ElseColor.textSecondary)

                if let exportStatus {
                    Text(exportStatus)
                        .font(ElseType.caption)
                        .foregroundStyle(ElseColor.textSecondary)
                        .textSelection(.enabled)
                }

                if let leaseCleanupStatus {
                    Text(leaseCleanupStatus)
                        .font(ElseType.caption)
                        .foregroundStyle(ElseColor.textSecondary)
                }
            }
        }
        .formStyle(.grouped)
        .padding()
        .frame(width: 500, height: 650)
        .onAppear {
            runtimeService.refresh()
        }
        .confirmationDialog(
            "启用自动删除？",
            isPresented: $isConfirmingRetention,
            titleVisibility: .visible
        ) {
            if let pendingRetentionDays {
                Button("保留 \(pendingRetentionDays) 天后删除", role: .destructive) {
                    Task {
                        await store.setRetentionDays(pendingRetentionDays)
                    }
                }
            }
            Button("取消", role: .cancel) {}
        } message: {
            Text("超过保留期的终态 Intent 会被永久删除，包括事件、关联凭据与 Scope Workspace。共享 CAS 工件不会盲目删除。")
        }
    }

    private func exportData() async {
        isExportingData = true
        exportStatus = nil
        defer { isExportingData = false }
        guard let data = await store.exportRuntimeData() else {
            return
        }
        save(data, prefix: "else-export")
    }

    private func exportDiagnostics() async {
        isExportingDiagnostics = true
        exportStatus = nil
        defer { isExportingDiagnostics = false }
        guard let data = await store.exportDiagnostics() else {
            return
        }
        save(data, prefix: "else-diagnostics")
    }

    private func cleanupLeases() async {
        isCleaningLeases = true
        leaseCleanupStatus = nil
        defer { isCleaningLeases = false }
        guard let report = await store.cleanupOrphanLeases() else {
            leaseCleanupStatus = "清理失败"
            return
        }
        leaseCleanupStatus =
            "清理 \(report.revokedLeases) 个 Lease，跳过 \(report.skippedActiveLeases) 个活跃 Lease"
    }

    private func save(_ data: Data, prefix: String) {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.json]
        panel.nameFieldStringValue =
            "\(prefix)-\(Date.now.formatted(.iso8601.year().month().day())).json"
        panel.isExtensionHidden = false
        panel.canCreateDirectories = true
        guard panel.runModal() == .OK, let url = panel.url else {
            return
        }
        do {
            try data.write(to: url, options: .atomic)
            exportStatus = "已导出到 \(url.path)"
        } catch {
            exportStatus = "导出失败：\(error.localizedDescription)"
        }
    }
}
