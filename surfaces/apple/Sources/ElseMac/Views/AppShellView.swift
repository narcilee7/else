import ElseClientCore
import SwiftUI

struct AppShellView: View {
    @EnvironmentObject private var store: AppStore

    var body: some View {
        NavigationSplitView {
            SidebarView()
                .navigationSplitViewColumnWidth(min: 190, ideal: 218, max: 250)
        } content: {
            Group {
                if store.section == .today {
                    DailyBriefView()
                } else if store.section == .world {
                    WorldView()
                } else if store.section == .connectors {
                    ConnectorsView()
                } else {
                    IntentListView()
                }
            }
            .navigationSplitViewColumnWidth(min: 330, ideal: 390, max: 480)
        } detail: {
            IntentDetailView(snapshot: store.selectedSnapshot)
        }
        .tint(ElseColor.action)
        .background(ElseColor.canvas)
        .task {
            await store.load()
        }
        .onReceive(
            NotificationCenter.default.publisher(for: .elseOpenIntent)
        ) { notification in
            guard let intentID = notification.object as? IntentID else {
                return
            }
            store.selectedIntentID = intentID
            store.section = .decisions
        }
        .sheet(isPresented: $store.isCreatingIntent) {
            NewIntentView()
                .environmentObject(store)
        }
        .alert(
            "Runtime 操作失败",
            isPresented: Binding(
                get: { store.operationError != nil },
                set: { if !$0 { store.clearError() } }
            )
        ) {
            Button("好") {
                store.clearError()
            }
        } message: {
            Text(store.operationError ?? "")
        }
    }
}

private struct SidebarView: View {
    @EnvironmentObject private var store: AppStore

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: ElseSpacing.x2) {
                Rectangle()
                    .fill(ElseColor.signal)
                    .frame(width: 2, height: 19)
                Text("else")
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(ElseColor.textPrimary)

                Spacer()
            }
            .padding(.horizontal, ElseSpacing.x4)
            .padding(.top, 14)
            .padding(.bottom, 10)

            List {
                Section {
                    ForEach(AppSection.allCases) { section in
                        Button {
                            store.section = section
                        } label: {
                            HStack(spacing: ElseSpacing.x2) {
                                Rectangle()
                                    .fill(
                                        store.section == section
                                            ? ElseColor.signal
                                            : Color.clear
                                    )
                                    .frame(width: 2, height: 17)

                                Label(section.title, systemImage: section.systemImage)
                                    .foregroundStyle(
                                        store.section == section
                                            ? ElseColor.textPrimary
                                            : ElseColor.textSecondary
                                    )

                                Spacer()

                                Text(badge(for: section), format: .number)
                                    .font(ElseType.mono)
                                    .foregroundStyle(
                                        store.section == section
                                            ? ElseColor.textSecondary
                                            : ElseColor.textTertiary
                                    )
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .listRowBackground(
                            store.section == section
                                ? ElseColor.selection
                                : Color.clear
                        )
                        .listRowSeparator(.hidden)
                        .accessibilityAddTraits(
                            store.section == section ? .isSelected : []
                        )
                    }
                }
            }
            .listStyle(.sidebar)
            .scrollContentBackground(.hidden)
            .tint(ElseColor.action)

            Divider()
                .overlay(ElseColor.lineSubtle)

            ConnectionStatusView(connection: store.connection, isDemo: store.isDemo)
                .padding(ElseSpacing.x3)
        }
        .background(ElseColor.surface)
    }

    private func badge(for section: AppSection) -> Int {
        switch section {
        case .today: store.today.count
        case .world: store.projectWorlds.count + store.identityWorlds.count
        case .connectors: store.connectorsByIntent.values.reduce(0) { $0 + $1.count }
        case .decisions: store.decisionCount
        case .results: store.results.count
        }
    }
}

private struct ConnectionStatusView: View {
    let connection: RuntimeConnection
    let isDemo: Bool

    private var tint: Color {
        switch connection {
        case .connected: isDemo ? ElseColor.signal : ElseColor.success
        case .connecting: ElseColor.info
        case .failed: ElseColor.danger
        case .idle: ElseColor.textTertiary
        }
    }

    var body: some View {
        HStack(spacing: 8) {
            Capsule()
                .fill(tint)
                .frame(width: 12, height: 2)

            VStack(alignment: .leading, spacing: 1) {
                Text(isDemo ? "Demo Runtime" : connection.label)
                    .font(ElseType.micro)
                    .foregroundStyle(ElseColor.textPrimary)
                if case .failed = connection {
                    Text("检查本地 daemon")
                        .font(ElseType.caption)
                        .foregroundStyle(ElseColor.textSecondary)
                }
            }

            Spacer()

            if case .connecting = connection {
                ProgressView()
                    .controlSize(.mini)
            }
        }
        .help(helpText)
    }

    private var helpText: String {
        switch connection {
        case .connected:
            isDemo ? "当前使用内置开发数据" : "本地 Runtime 连接正常"
        case .connecting:
            "正在连接 Runtime"
        case let .failed(message):
            message
        case .idle:
            "尚未连接 Runtime"
        }
    }
}
