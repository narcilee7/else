import ElseClientCore
import SwiftUI

struct DailyBriefView: View {
    @EnvironmentObject private var store: AppStore
    @State private var evidence: BriefEvidence?

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()

            if store.connection == .connecting && store.brief == nil {
                ProgressView("正在生成今日简报")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if store.brief != nil || !store.today.isEmpty {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        if let brief = store.brief {
                            ForEach(sections(for: brief)) { section in
                                BriefSectionView(
                                    section: section,
                                    onShowEvidence: {
                                        evidence = BriefEvidence(record: $0)
                                    }
                                )
                            }
                        }
                        activeIntents
                    }
                }
                .background(ElseColor.surface)
            } else if case let .failed(message) = store.connection {
                QuietEmptyState(
                    title: "Runtime 离线",
                    description: message
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                FirstUseEmptyState()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .navigationTitle("今天")
        .sheet(item: $evidence) { selection in
            WorldItemEvidenceView(record: selection.record)
        }
    }

    private var header: some View {
        HStack(alignment: .bottom, spacing: ElseSpacing.x3) {
            VStack(alignment: .leading, spacing: 3) {
                Text(Date.now.formatted(.dateTime.weekday(.wide).month().day()))
                    .font(ElseType.pageTitle)
                    .foregroundStyle(ElseColor.textPrimary)
                Text(summary)
                    .font(ElseType.caption)
                    .foregroundStyle(ElseColor.textSecondary)
            }

            Spacer()

            if store.isRefreshing {
                ProgressView()
                    .controlSize(.mini)
            } else if let generatedAt = store.brief?.generatedAt {
                Text(generatedAt.formatted(date: .omitted, time: .shortened))
                    .font(ElseType.mono)
                    .foregroundStyle(ElseColor.textTertiary)
                    .monospacedDigit()
            }

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

    private var summary: String {
        let outstanding = store.briefOutstandingCount
        let decisions = store.decisionCount
        if outstanding == 0 && decisions == 0 {
            return "没有待处理事项"
        }
        if decisions == 0 {
            return "\(outstanding) 件外部事项"
        }
        return "\(outstanding) 件外部事项 · \(decisions) 个待决策"
    }

    @ViewBuilder
    private var activeIntents: some View {
        if !store.today.isEmpty {
            VStack(alignment: .leading, spacing: 0) {
                SectionHeader(title: "进行中", count: store.today.count)

                ForEach(store.today) { snapshot in
                    Button {
                        store.selectedIntentID = snapshot.id
                    } label: {
                        HStack(alignment: .top, spacing: ElseSpacing.x3) {
                            StatusRail(state: snapshot.state, height: 38)
                            VStack(alignment: .leading, spacing: ElseSpacing.x2) {
                                Text(snapshot.spec.outcome)
                                    .font(ElseType.callout)
                                    .foregroundStyle(ElseColor.textPrimary)
                                    .lineLimit(2)
                                    .multilineTextAlignment(.leading)
                                HStack(spacing: ElseSpacing.x3) {
                                    StateBadge(state: snapshot.state)
                                    if let project = snapshot.spec.project {
                                        Label(project, systemImage: "folder")
                                    }
                                }
                                .font(ElseType.caption)
                                .foregroundStyle(ElseColor.textSecondary)
                            }
                            Spacer()
                        }
                        .padding(.horizontal, ElseSpacing.x4)
                        .padding(.vertical, ElseSpacing.x3)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .background(
                        store.selectedIntentID == snapshot.id
                            ? ElseColor.selection
                            : Color.clear
                    )
                    .overlay(alignment: .bottom) {
                        Rectangle()
                            .fill(ElseColor.lineSubtle)
                            .frame(height: 1)
                            .padding(.leading, ElseSpacing.x8)
                    }
                }
            }
        }
    }

    private func sections(for brief: PersonalBrief) -> [DailyBriefSection] {
        [
            DailyBriefSection(
                id: "overdue",
                title: "逾期",
                tint: ElseColor.danger,
                records: brief.overdue
            ),
            DailyBriefSection(
                id: "waiting",
                title: "等待外部",
                tint: ElseColor.info,
                records: brief.waitingExternal
            ),
            DailyBriefSection(
                id: "risk",
                title: "风险",
                tint: ElseColor.danger,
                records: brief.risks
            ),
            DailyBriefSection(
                id: "commitment",
                title: "承诺",
                tint: ElseColor.signal,
                records: brief.openCommitments
            ),
            DailyBriefSection(
                id: "actions",
                title: "接下来",
                tint: ElseColor.textPrimary,
                records: brief.actionItemsDueSoon
            ),
            DailyBriefSection(
                id: "calendar",
                title: "日历",
                tint: ElseColor.info,
                records: brief.upcomingCalendar
            ),
        ]
        .filter { !$0.records.isEmpty }
    }
}

private struct DailyBriefSection: Identifiable {
    let id: String
    let title: String
    let tint: Color
    let records: [WorldItemRecord]
}

private struct BriefSectionView: View {
    let section: DailyBriefSection
    let onShowEvidence: (WorldItemRecord) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SectionHeader(title: section.title, count: section.records.count)
            ForEach(section.records, id: \.item.id) { record in
                WorldItemRow(
                    record: record,
                    tint: section.tint,
                    onShowEvidence: { onShowEvidence(record) }
                )
            }
        }
    }
}

private struct WorldItemRow: View {
    @EnvironmentObject private var store: AppStore
    let record: WorldItemRecord
    let tint: Color
    let onShowEvidence: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: ElseSpacing.x3) {
            Rectangle()
                .fill(tint)
                .frame(width: 2)
                .padding(.vertical, 2)

            Button {
                store.selectedIntentID = record.intentID
            } label: {
                VStack(alignment: .leading, spacing: ElseSpacing.x2) {
                    Text(record.item.title)
                        .font(ElseType.callout)
                        .foregroundStyle(ElseColor.textPrimary)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)

                    Text(record.item.summary)
                        .font(ElseType.caption)
                        .foregroundStyle(ElseColor.textSecondary)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)

                    if !record.item.priorityReason.isEmpty {
                        Text(record.item.priorityReason)
                            .font(ElseType.micro)
                            .foregroundStyle(ElseColor.textSecondary)
                            .lineLimit(1)
                    }

                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: ElseSpacing.x3) {
                            priorityLabel
                            sourceLabel
                            projectLabel
                            deadlineLabel
                        }

                        VStack(alignment: .leading, spacing: 3) {
                            HStack(spacing: ElseSpacing.x3) {
                                priorityLabel
                                sourceLabel
                            }
                            HStack(spacing: ElseSpacing.x3) {
                                deadlineLabel
                                projectLabel
                            }
                        }
                    }
                    .font(ElseType.micro)
                    .foregroundStyle(ElseColor.textSecondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if store.resolvingWorldItemIDs.contains(record.item.id) {
                ProgressView()
                    .controlSize(.mini)
            } else {
                HStack(spacing: 6) {
                    Button(action: onShowEvidence) {
                        Image(systemName: "info.circle")
                    }
                    .help("查看来源证据")

                    Button {
                        Task {
                            await store.resolveWorldItem(record, kind: .completed)
                        }
                    } label: {
                        Image(systemName: "checkmark")
                    }
                    .help("标记完成")

                    Button {
                        Task {
                            await store.resolveWorldItem(record, kind: .dismissed)
                        }
                    } label: {
                        Image(systemName: "xmark")
                    }
                    .help("忽略")
                }
                .buttonStyle(.borderless)
                .foregroundStyle(ElseColor.textSecondary)
            }
        }
        .padding(.horizontal, ElseSpacing.x4)
        .padding(.vertical, ElseSpacing.x3)
        .overlay(alignment: .bottom) {
            Rectangle()
                .fill(ElseColor.lineSubtle)
                .frame(height: 1)
                .padding(.leading, ElseSpacing.x8)
        }
    }

    private var sourceIcon: String {
        switch record.item.entityRef.provider {
        case "calendar": "calendar"
        case "notion": "doc.text"
        case "linear": "checklist"
        default: "link"
        }
    }

    private var sourceLabel: some View {
        Label(record.item.entityRef.provider, systemImage: sourceIcon)
            .lineLimit(1)
    }

    private var priorityLabel: some View {
        Label("P\(record.item.priority)", systemImage: "exclamationmark.circle")
            .font(ElseType.micro)
            .foregroundStyle(
                record.item.priority >= 90
                    ? ElseColor.danger
                    : record.item.priority >= 70
                        ? ElseColor.signal
                        : ElseColor.textSecondary
            )
            .lineLimit(1)
    }

    @ViewBuilder
    private var projectLabel: some View {
        if let project = record.item.projects.first {
            Label(project.label, systemImage: "folder")
                .lineLimit(1)
        }
    }

    @ViewBuilder
    private var deadlineLabel: some View {
        if let deadline = record.item.deadline {
            Label(
                deadlineText(deadline),
                systemImage: "clock"
            )
            .lineLimit(1)
            .foregroundStyle(
                deadline < Date.now ? ElseColor.danger : ElseColor.textSecondary
            )
        }
    }

    private func deadlineText(_ date: Date) -> String {
        if Calendar.current.component(.year, from: date)
            == Calendar.current.component(.year, from: Date())
        {
            return date.formatted(
                .dateTime.month(.abbreviated).day().hour().minute()
            )
        }
        return date.formatted(date: .abbreviated, time: .shortened)
    }
}

private struct BriefEvidence: Identifiable {
    let record: WorldItemRecord
    var id: WorldItemID { record.item.id }
}

private struct WorldItemEvidenceView: View {
    @Environment(\.dismiss) private var dismiss
    let record: WorldItemRecord

    var body: some View {
        VStack(alignment: .leading, spacing: ElseSpacing.x4) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(record.item.title)
                        .font(ElseType.pageTitle)
                        .foregroundStyle(ElseColor.textPrimary)
                    Text(record.item.summary)
                        .font(ElseType.body)
                        .foregroundStyle(ElseColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer()
                Button {
                    dismiss()
                } label: {
                    Image(systemName: "xmark")
                }
                .buttonStyle(.borderless)
                .help("关闭")
            }

            Divider()

            VStack(spacing: 10) {
                evidenceRow("来源", record.item.entityRef.provider)
                evidenceRow("对象", record.item.entityRef.entityKind)
                evidenceRow("外部 ID", record.item.entityRef.externalID)
                if let project = record.item.projects.first {
                    evidenceRow("项目", project.label)
                }
                if !record.item.identities.isEmpty {
                    evidenceRow(
                        "关系",
                        record.item.identities.map(\.label).joined(separator: " · ")
                    )
                }
                evidenceRow(
                    "首次观察",
                    record.item.firstObservedAt.formatted(
                        date: .abbreviated,
                        time: .shortened
                    )
                )
                evidenceRow(
                    "最近观察",
                    record.item.lastObservedAt.formatted(
                        date: .abbreviated,
                        time: .shortened
                    )
                )
                if let deadline = record.item.deadline {
                    evidenceRow(
                        "截止",
                        deadline.formatted(date: .abbreviated, time: .shortened)
                    )
                }
                evidenceRow(
                    "优先级",
                    "P\(record.item.priority) · \(record.item.priorityReason)"
                )
                evidenceRow("Signal", "\(record.item.signalIDs.count)")
                evidenceRow("Connector", "\(record.item.connectorIDs.count)")
            }

            if let url = record.item.entityRef.canonicalURL {
                Link(destination: url) {
                    Label("打开来源", systemImage: "arrow.up.right.square")
                }
                .buttonStyle(.borderedProminent)
                .tint(ElseColor.action)
            }

            Spacer()
        }
        .padding(ElseSpacing.x6)
        .frame(minWidth: 460, minHeight: 390, alignment: .topLeading)
        .background(ElseColor.canvas)
    }

    private func evidenceRow(_ label: String, _ value: String) -> some View {
        HStack(alignment: .top) {
            Text(label)
                .font(ElseType.caption)
                .foregroundStyle(ElseColor.textSecondary)
                .frame(width: 76, alignment: .leading)
            Text(value)
                .font(ElseType.mono)
                .foregroundStyle(ElseColor.textPrimary)
                .lineLimit(2)
                .textSelection(.enabled)
            Spacer()
        }
    }
}

private struct SectionHeader: View {
    let title: String
    let count: Int

    var body: some View {
        HStack {
            Text(title)
                .font(ElseType.micro)
                .tracking(0.4)
                .foregroundStyle(ElseColor.textSecondary)
            Spacer()
            Text(count, format: .number)
                .font(ElseType.mono)
                .foregroundStyle(ElseColor.textTertiary)
        }
        .padding(.horizontal, ElseSpacing.x4)
        .padding(.top, ElseSpacing.x4)
        .padding(.bottom, ElseSpacing.x2)
    }
}

private struct FirstUseEmptyState: View {
    @EnvironmentObject private var store: AppStore

    var body: some View {
        VStack(spacing: ElseSpacing.x3) {
            Rectangle()
                .fill(ElseColor.lineStrong)
                .frame(width: 28, height: 1)
            Text(
                store.snapshots.isEmpty
                    ? "从第一个 Intent 开始"
                    : "今天没有需要你处理的外部事项"
            )
            .font(ElseType.section)
            .foregroundStyle(ElseColor.textPrimary)

            HStack(spacing: ElseSpacing.x2) {
                Button("新建 Intent") {
                    store.isCreatingIntent = true
                }
                .buttonStyle(.borderedProminent)
                .tint(ElseColor.action)

                Button("接入外部来源") {
                    store.section = .connectors
                }
                .buttonStyle(.bordered)
            }
        }
        .padding(ElseSpacing.x6)
    }
}

private extension PersonalBrief {
    var isEmpty: Bool {
        openCommitments.isEmpty
            && waitingExternal.isEmpty
            && overdue.isEmpty
            && actionItemsDueSoon.isEmpty
            && risks.isEmpty
            && upcomingCalendar.isEmpty
    }
}
