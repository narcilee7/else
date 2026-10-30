import ElseClientCore
import SwiftUI

struct IntentListView: View {
    @EnvironmentObject private var store: AppStore

    private var visibleSnapshots: [IntentSnapshot] {
        switch store.section {
        case .today: store.today
        case .decisions: store.decisions.map(\.snapshot)
        case .results: store.results
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()

            if store.connection == .connecting && store.snapshots.isEmpty {
                ProgressView("正在读取 Runtime")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if isEmpty {
                EmptySectionView(
                    section: store.section,
                    connection: store.connection
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    LazyVStack(spacing: 0) {
                        if store.section == .decisions {
                            ForEach(store.decisions, id: \.request.id) { item in
                                DecisionSummaryRow(item: item)
                                    .onTapGesture {
                                        store.selectedIntentID = item.snapshot.id
                                    }
                            }
                        } else {
                            ForEach(visibleSnapshots) { snapshot in
                                IntentRow(
                                    snapshot: snapshot,
                                    isMutating: store.mutatingIntentIDs.contains(snapshot.id)
                                )
                                .onTapGesture {
                                    store.selectedIntentID = snapshot.id
                                }
                                .background(
                                    store.selectedIntentID == snapshot.id
                                        ? ElseColor.selection
                                        : Color.clear,
                                    in: Rectangle()
                                )
                                .contextMenu {
                                    if !snapshot.isTerminal {
                                        Button("继续执行") {
                                            Task { await store.run(snapshot) }
                                        }
                                        Button("取消 Intent", role: .destructive) {
                                            Task { await store.abort(snapshot) }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                .background(ElseColor.surface)
            }
        }
        .navigationTitle(store.section.title)
    }

    private var isEmpty: Bool {
        switch store.section {
        case .today: store.today.isEmpty
        case .decisions: store.decisions.isEmpty
        case .results: store.results.isEmpty
        }
    }

    private var header: some View {
        HStack(alignment: .center) {
            VStack(alignment: .leading, spacing: 2) {
                Text(store.section.title)
                    .font(ElseType.pageTitle)
                    .foregroundStyle(ElseColor.textPrimary)
                Text(subtitle)
                    .font(ElseType.caption)
                    .foregroundStyle(ElseColor.textSecondary)
            }

            Spacer()

            Button {
                store.isCreatingIntent = true
            } label: {
                Image(systemName: "plus")
            }
            .buttonStyle(.borderless)
            .foregroundStyle(ElseColor.textSecondary)
            .help("新建 Intent")

            Button {
                Task { await store.refresh() }
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .buttonStyle(.borderless)
            .foregroundStyle(ElseColor.textSecondary)
            .disabled(store.isRefreshing)
            .help("刷新")
        }
        .padding(.horizontal, ElseSpacing.x4)
        .padding(.vertical, ElseSpacing.x3)
    }

    private var subtitle: String {
        switch store.section {
        case .today: "\(store.today.count) 个进行中"
        case .decisions: store.decisionCount == 0 ? "无需你介入" : "\(store.decisionCount) 个请求"
        case .results: "\(store.results.count) 个已收敛"
        }
    }
}

private struct IntentRow: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let snapshot: IntentSnapshot
    let isMutating: Bool

    var body: some View {
        HStack(alignment: .top, spacing: ElseSpacing.x3) {
            StatusRail(state: snapshot.state, height: 42)

            VStack(alignment: .leading, spacing: ElseSpacing.x2) {
                HStack(alignment: .top, spacing: ElseSpacing.x2) {
                    Text(snapshot.spec.outcome)
                        .font(ElseType.callout)
                        .foregroundStyle(ElseColor.textPrimary)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)

                    Spacer(minLength: ElseSpacing.x1)

                    if isMutating {
                        ProgressView()
                            .controlSize(.mini)
                    } else {
                        StateBadge(state: snapshot.state)
                    }
                }

                HStack(spacing: ElseSpacing.x3) {
                    if let project = snapshot.spec.project {
                        Label(project, systemImage: "folder")
                    }
                    Label("R\(snapshot.spec.revision)", systemImage: "arrow.triangle.branch")
                    if !snapshot.decisions.isEmpty {
                        Label("\(snapshot.decisions.count)", systemImage: "hand.raised")
                            .foregroundStyle(ElseColor.signal)
                    }
                }
                .font(ElseType.caption)
                .foregroundStyle(ElseColor.textSecondary)
            }
        }
        .padding(.horizontal, ElseSpacing.x4)
        .padding(.vertical, ElseSpacing.x3)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .overlay(alignment: .bottom) {
            Rectangle()
                .fill(ElseColor.lineSubtle)
                .frame(height: 1)
                .padding(.leading, ElseSpacing.x8)
        }
        .animation(reduceMotion ? nil : ElseMotion.quick, value: snapshot.state)
        .accessibilityElement(children: .combine)
    }
}

private struct DecisionSummaryRow: View {
    let item: (snapshot: IntentSnapshot, request: AttentionRequest)

    var body: some View {
        SignalBand {
            VStack(alignment: .leading, spacing: ElseSpacing.x2) {
                HStack {
                    Label(item.request.kind.label, systemImage: "hand.raised.fill")
                        .font(ElseType.micro)
                        .foregroundStyle(ElseColor.signal)
                    Spacer()
                    Text("影响 \(item.request.expectedLoss.label)")
                        .font(ElseType.micro)
                        .foregroundStyle(lossTint)
                }

                Text(item.request.summary)
                    .font(ElseType.callout)
                    .foregroundStyle(ElseColor.textPrimary)
                    .lineLimit(3)
                    .multilineTextAlignment(.leading)

                Text(item.snapshot.spec.outcome)
                    .font(ElseType.caption)
                    .foregroundStyle(ElseColor.textSecondary)
                    .lineLimit(1)
            }
        }
        .padding(ElseSpacing.x4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
    }

    private var lossTint: Color {
        item.request.expectedLoss >= .high ? ElseColor.danger : ElseColor.textSecondary
    }
}

private struct EmptySectionView: View {
    let section: AppSection
    let connection: RuntimeConnection

    var body: some View {
        if case let .failed(message) = connection {
            VStack(spacing: 12) {
                Image(systemName: "network.slash")
                    .font(.system(size: 24))
                    .foregroundStyle(ElseColor.textSecondary)
                Text("Runtime 离线")
                    .font(ElseType.section)
                    .foregroundStyle(ElseColor.textPrimary)
                Text(message)
                    .font(ElseType.caption)
                    .foregroundStyle(ElseColor.textSecondary)
                    .multilineTextAlignment(.center)
                    .lineLimit(3)
                    .frame(maxWidth: 260)
            }
            .padding(24)
        } else {
            QuietEmptyState(
                title: emptyTitle,
                description: emptyDescription
            )
        }
    }

    private var emptyTitle: String {
        switch section {
        case .today: "今天没有进行中的 Intent"
        case .decisions: "目前无需决策"
        case .results: "还没有收敛的结果"
        }
    }

    private var emptyDescription: String {
        switch section {
        case .today: "提交一个目标，让 Runtime 处理其余部分。"
        case .decisions: "只有真正需要你的判断时，请求才会出现在这里。"
        case .results: "完成的 Intent 会带着证据和资源账回到这里。"
        }
    }
}
