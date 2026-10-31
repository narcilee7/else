import AppKit
import SwiftUI

struct MenuBarStatusView: View {
    @EnvironmentObject private var store: AppStore
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                openMainWindow()
            } label: {
                Label("打开 else", systemImage: "macwindow")
            }
            .keyboardShortcut("o")

            Divider()

            Text("\(store.briefOutstandingCount) 件外部事项 · \(store.decisionCount) 个待决策")
                .font(.caption)
                .foregroundStyle(.secondary)

            if store.decisions.isEmpty {
                Text("暂无待决策")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(Array(store.decisions.prefix(4)), id: \.request.id) { item in
                    Button {
                        store.section = .decisions
                        store.selectedIntentID = item.snapshot.id
                        openMainWindow()
                    } label: {
                        Label(
                            item.request.summary,
                            systemImage: item.request.expectedLoss >= .high
                                ? "exclamationmark.circle"
                                : "circle"
                        )
                    }
                }
            }

            Divider()

            Button {
                Task { await store.refresh() }
            } label: {
                Label(
                    store.isRefreshing ? "正在刷新" : "刷新",
                    systemImage: "arrow.clockwise"
                )
            }
            .disabled(store.isRefreshing)

            Button {
                store.section = .today
                store.isCreatingIntent = true
                openMainWindow()
            } label: {
                Label("新建 Intent", systemImage: "plus")
            }
            .keyboardShortcut("n")
        }
    }

    private func openMainWindow() {
        openWindow(id: "main")
        NSApplication.shared.activate(ignoringOtherApps: true)
    }
}
