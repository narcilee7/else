import ElseClientCore
import SwiftUI

private enum WorldMode: String, CaseIterable, Identifiable {
    case projects
    case identities

    var id: Self { self }
}

private struct WorldGroupPresentation: Identifiable {
    let kind: CorrelationKind
    let key: String
    let title: String
    let items: [WorldItemRecord]
    let openItems: Int
    let overdue: Int
    let nextDeadline: Date?

    var id: String { "\(kind.rawValue):\(key)" }
}

struct WorldView: View {
    @EnvironmentObject private var store: AppStore
    @State private var mode: WorldMode = .projects
    @State private var editor: WorldCorrectionEditor?

    private var groups: [WorldGroupPresentation] {
        switch mode {
        case .projects:
            store.projectWorlds.map {
                WorldGroupPresentation(
                    kind: .project,
                    key: $0.key,
                    title: $0.title,
                    items: $0.items,
                    openItems: $0.openItems,
                    overdue: $0.overdue,
                    nextDeadline: $0.nextDeadline
                )
            }
        case .identities:
            store.identityWorlds.map {
                WorldGroupPresentation(
                    kind: $0.kind,
                    key: $0.key,
                    title: $0.title,
                    items: $0.items,
                    openItems: $0.openItems,
                    overdue: $0.overdue,
                    nextDeadline: $0.nextDeadline
                )
            }
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()

            if groups.isEmpty {
                QuietEmptyState(
                    title: "还没有可聚合的\(mode == .projects ? "项目" : "关系")",
                    description: "连接外部来源后，跨 Provider 的 World 会出现在这里。"
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(groups) { group in
                            WorldGroupRow(
                                group: group,
                                onRename: {
                                    editor = .rename(group)
                                },
                                onMerge: {
                                    editor = .merge(
                                        source: group,
                                        candidates: groups.filter { $0.kind == group.kind }
                                    )
                                }
                            )
                        }
                    }
                }
                .background(ElseColor.surface)
            }
        }
        .navigationTitle("世界")
        .sheet(item: $editor) { editor in
            WorldCorrectionSheet(
                editor: editor,
                groups: groups
            )
            .environmentObject(store)
        }
    }

    private var header: some View {
        HStack(spacing: ElseSpacing.x3) {
            VStack(alignment: .leading, spacing: 2) {
                Text("世界")
                    .font(ElseType.pageTitle)
                    .foregroundStyle(ElseColor.textPrimary)
                Text("\(groups.count) 个聚合")
                    .font(ElseType.caption)
                    .foregroundStyle(ElseColor.textSecondary)
            }

            Spacer()

            Picker("", selection: $mode) {
                Text("项目").tag(WorldMode.projects)
                Text("关系").tag(WorldMode.identities)
            }
            .pickerStyle(.segmented)
            .frame(width: 140)

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
}

private struct WorldGroupRow: View {
    @EnvironmentObject private var store: AppStore
    let group: WorldGroupPresentation
    let onRename: () -> Void
    let onMerge: () -> Void
    @State private var expanded = false

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: ElseSpacing.x3) {
                Button {
                    expanded.toggle()
                } label: {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                        .frame(width: 14)
                }
                .buttonStyle(.borderless)
                .foregroundStyle(ElseColor.textTertiary)

                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: ElseSpacing.x2) {
                        Text(group.title)
                            .font(ElseType.callout)
                            .foregroundStyle(ElseColor.textPrimary)
                        Text(group.kind.label)
                            .font(ElseType.micro)
                            .foregroundStyle(ElseColor.textTertiary)
                    }
                    HStack(spacing: ElseSpacing.x3) {
                        Text("开放 \(group.openItems)")
                        if group.overdue > 0 {
                            Text("逾期 \(group.overdue)")
                                .foregroundStyle(ElseColor.danger)
                        }
                        Text("\(group.items.count) 项")
                    }
                    .font(ElseType.micro)
                    .foregroundStyle(ElseColor.textSecondary)

                    if let deadline = group.nextDeadline {
                        Label(
                            deadline.formatted(
                                .dateTime
                                    .month(.abbreviated)
                                    .day()
                                    .hour()
                                    .minute()
                            ),
                            systemImage: "clock"
                        )
                        .font(ElseType.micro)
                        .foregroundStyle(ElseColor.textSecondary)
                        .lineLimit(1)
                    }
                }

                Spacer()

                Menu {
                    Button("重命名…", systemImage: "pencil", action: onRename)
                    Button("合并…", systemImage: "arrow.triangle.merge", action: onMerge)
                } label: {
                    Image(systemName: "ellipsis")
                }
                .menuStyle(.borderlessButton)
                .help("修正关联")
            }
            .padding(.horizontal, ElseSpacing.x4)
            .padding(.vertical, ElseSpacing.x3)

            if expanded {
                ForEach(Array(group.items.enumerated()), id: \.offset) { _, record in
                    HStack(spacing: ElseSpacing.x2) {
                        Rectangle()
                            .fill(record.item.state.tint)
                            .frame(width: 2, height: 30)
                        Button {
                            store.selectedIntentID = record.intentID
                        } label: {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(record.item.title)
                                    .font(ElseType.caption)
                                    .foregroundStyle(ElseColor.textPrimary)
                                Text(record.item.entityRef.provider)
                                    .font(ElseType.micro)
                                    .foregroundStyle(ElseColor.textTertiary)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .buttonStyle(.plain)

                        if group.kind != .project {
                            Button {
                                Task {
                                    await store.correctWorldCorrelation(
                                        intentID: record.intentID,
                                        correction: WorldCorrelationCorrection(
                                            kind: .split,
                                            correlationKind: group.kind,
                                            sourceKeys: [group.key],
                                            itemID: record.item.id
                                        )
                                    )
                                }
                            } label: {
                                Image(systemName: "minus.circle")
                            }
                            .buttonStyle(.borderless)
                            .foregroundStyle(ElseColor.textSecondary)
                            .help("从此事项移除此关系")
                        }
                    }
                    .padding(.leading, ElseSpacing.x8)
                    .padding(.trailing, ElseSpacing.x4)
                    .padding(.vertical, 6)
                }
            }
        }
        .overlay(alignment: .bottom) {
            Rectangle()
                .fill(ElseColor.lineSubtle)
                .frame(height: 1)
        }
    }
}

private enum WorldCorrectionEditor: Identifiable {
    case rename(WorldGroupPresentation)
    case merge(
        source: WorldGroupPresentation,
        candidates: [WorldGroupPresentation]
    )

    var id: String {
        switch self {
        case let .rename(group): "rename:\(group.id)"
        case let .merge(source, _): "merge:\(source.id)"
        }
    }
}

private struct WorldCorrectionSheet: View {
    @EnvironmentObject private var store: AppStore
    @Environment(\.dismiss) private var dismiss
    let editor: WorldCorrectionEditor
    let groups: [WorldGroupPresentation]

    @State private var targetKey = ""
    @State private var label = ""

    var body: some View {
        VStack(alignment: .leading, spacing: ElseSpacing.x4) {
            Text(title)
                .font(ElseType.pageTitle)
                .foregroundStyle(ElseColor.textPrimary)

            switch editor {
            case let .rename(group):
                Text("当前：\(group.title)")
                    .font(ElseType.caption)
                    .foregroundStyle(ElseColor.textSecondary)
                TextField("显示名称", text: $label)
            case let .merge(_, candidates):
                Picker("合并到", selection: $targetKey) {
                    ForEach(candidates) { group in
                        Text(group.title).tag(group.key)
                    }
                }
                TextField("合并后名称（可选）", text: $label)
            }

            HStack {
                Spacer()
                Button("取消") {
                    dismiss()
                }
                Button("应用") {
                    Task {
                        await apply()
                        dismiss()
                    }
                }
                .buttonStyle(.borderedProminent)
                .tint(ElseColor.action)
                .disabled(!isValid)
            }
        }
        .padding(ElseSpacing.x6)
        .frame(width: 420)
        .background(ElseColor.canvas)
        .onAppear {
            switch editor {
            case let .rename(group):
                label = group.title
            case let .merge(source, candidates):
                targetKey = candidates.first(where: { $0.id != source.id })?.key ?? ""
            }
        }
    }

    private var title: String {
        switch editor {
        case .rename: "重命名关系"
        case .merge: "合并关系"
        }
    }

    private var isValid: Bool {
        switch editor {
        case let .rename(group):
            !label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && label != group.title
        case let .merge(source, _):
            !targetKey.isEmpty && targetKey != source.key
        }
    }

    private func apply() async {
        let correction: WorldCorrelationCorrection
        let intentID: IntentID
        switch editor {
        case let .rename(group):
            correction = WorldCorrelationCorrection(
                kind: .rename,
                correlationKind: group.kind,
                sourceKeys: [],
                targetKey: group.key,
                label: label.trimmingCharacters(in: .whitespacesAndNewlines)
            )
            intentID = group.items.first!.intentID
        case let .merge(source, _):
            correction = WorldCorrelationCorrection(
                kind: .merge,
                correlationKind: source.kind,
                sourceKeys: [source.key],
                targetKey: targetKey,
                label: label.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
            )
            intentID = source.items.first!.intentID
        }
        await store.correctWorldCorrelation(intentID: intentID, correction: correction)
    }
}

private extension CorrelationKind {
    var label: String {
        switch self {
        case .project: "项目"
        case .contact: "联系人"
        case .customer: "客户"
        case .thread: "线程"
        case .milestone: "里程碑"
        }
    }
}

private extension WorldItemState {
    var tint: Color {
        switch self {
        case .overdue, .atRisk: ElseColor.danger
        case .waitingExternal: ElseColor.info
        case .open: ElseColor.textPrimary
        case .completed: ElseColor.success
        case .dismissed, .superseded: ElseColor.textTertiary
        }
    }
}

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }
}
