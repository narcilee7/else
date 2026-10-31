import ElseClientCore
import SwiftUI

struct IntentDetailView: View {
    @EnvironmentObject private var store: AppStore
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var isProposingAction = false
    @State private var isConfirmingDelete = false
    @State private var isConfirmingRetry = false
    let snapshot: IntentSnapshot?

    var body: some View {
        Group {
            if let snapshot {
                ScrollView {
                    VStack(alignment: .leading, spacing: 22) {
                        detailHeader(snapshot)

                        if !snapshot.decisions.isEmpty {
                            decisions(snapshot)
                        }

                        if !store.actionsByIntent[snapshot.id, default: []].isEmpty
                            || !store.connectorsByIntent[snapshot.id, default: []].isEmpty
                        {
                            connectorActions(snapshot)
                        }

                        acceptance(snapshot)
                        boundaries(snapshot)
                        artifacts(snapshot)
                        resources(snapshot)
                    }
                    .padding(28)
                    .frame(maxWidth: 820, alignment: .leading)
                }
                .background(ElseColor.canvas)
            } else {
                QuietEmptyState(
                    title: "选择一个 Intent",
                    description: "目标、决策、证据与资源账会显示在这里。"
                )
            }
        }
        .navigationTitle(snapshot?.spec.outcome ?? "Intent")
        .toolbar {
            if let snapshot {
                ToolbarItemGroup {
                    if !snapshot.isTerminal {
                        Button {
                            Task { await store.run(snapshot) }
                        } label: {
                            Label("继续执行", systemImage: "play")
                        }
                        .disabled(store.mutatingIntentIDs.contains(snapshot.id))

                        Button(role: .destructive) {
                            Task { await store.abort(snapshot) }
                        } label: {
                            Label("取消", systemImage: "stop")
                        }
                        .disabled(store.mutatingIntentIDs.contains(snapshot.id))
                    }

                    if snapshot.state == .failed {
                        Button {
                            isConfirmingRetry = true
                        } label: {
                            Label("修复并重试", systemImage: "arrow.clockwise.circle")
                        }
                        .disabled(store.mutatingIntentIDs.contains(snapshot.id))
                        .confirmationDialog(
                            "重试失败的 Scope？",
                            isPresented: $isConfirmingRetry,
                            titleVisibility: .visible
                        ) {
                            Button("修复并重试") {
                                Task { await store.retryFailed(snapshot) }
                            }
                            Button("取消", role: .cancel) {}
                        } message: {
                            Text("会追加 ScopeRetryApproved 事件、重置失败计数并以新 Generation 重新执行。仍可能再次失败。")
                        }
                    }

                    Button(role: .destructive) {
                        isConfirmingDelete = true
                    } label: {
                        Label("永久删除", systemImage: "trash")
                    }
                    .disabled(store.mutatingIntentIDs.contains(snapshot.id))
                    .help("永久删除 Intent、事件、关联凭据与 Scope Workspace")
                }
            }
        }
        .animation(reduceMotion ? nil : ElseMotion.emphasis, value: snapshot?.state)
        .sheet(isPresented: $isProposingAction) {
            if let snapshot {
                ActionProposalSheet(intentID: snapshot.id)
                    .environmentObject(store)
            }
        }
        .confirmationDialog(
            "永久删除这个 Intent？",
            isPresented: $isConfirmingDelete,
            titleVisibility: .visible
        ) {
            if let snapshot {
                Button("永久删除", role: .destructive) {
                    Task { await store.deleteIntent(snapshot) }
                }
            }
            Button("取消", role: .cancel) {}
        } message: {
            Text("将删除事件、关联凭据与 Scope Workspace。共享 CAS 工件不会盲目删除；此操作无法撤销。")
        }
    }

    private func detailHeader(_ snapshot: IntentSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 9) {
                StateBadge(state: snapshot.state)
                if let project = snapshot.spec.project {
                    Label(project, systemImage: "folder")
                        .font(ElseType.caption)
                        .foregroundStyle(ElseColor.textSecondary)
                }
                Text("revision \(snapshot.spec.revision)")
                    .font(ElseType.mono)
                    .foregroundStyle(ElseColor.textTertiary)
                    .monospacedDigit()
                Spacer()
            }

            Text(snapshot.spec.outcome)
                .font(ElseType.display)
                .foregroundStyle(ElseColor.textPrimary)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)

            if let deadline = snapshot.spec.deadline {
                Label(
                    deadline.formatted(date: .abbreviated, time: .shortened),
                    systemImage: "calendar"
                )
                .font(ElseType.caption)
                .foregroundStyle(ElseColor.textSecondary)
            }
        }
    }

    private func decisions(_ snapshot: IntentSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionLabel(title: "需要你")
            ForEach(snapshot.decisions) { request in
                DecisionBand(snapshot: snapshot, request: request)
                    .transition(
                        reduceMotion
                            ? .opacity
                            : .opacity.combined(with: .offset(y: -5))
                    )
            }
        }
        .animation(reduceMotion ? nil : ElseMotion.emphasis, value: snapshot.decisions.map(\.id))
    }

    private func acceptance(_ snapshot: IntentSnapshot) -> some View {
        DetailSection(title: "验收", systemImage: "checkmark.seal") {
            VStack(alignment: .leading, spacing: 8) {
                ForEach(Array(snapshot.spec.doneWhen.enumerated()), id: \.offset) { index, item in
                    HStack(alignment: .top, spacing: 9) {
                        Text(String(format: "%02d", index + 1))
                            .font(ElseType.mono)
                            .foregroundStyle(ElseColor.textTertiary)
                            .frame(width: 22, alignment: .leading)
                        Text(item.description)
                            .font(ElseType.body)
                            .foregroundStyle(ElseColor.textPrimary)
                            .textSelection(.enabled)
                    }
                }
            }
        }
    }

    private func connectorActions(_ snapshot: IntentSnapshot) -> some View {
        DetailSection(title: "外部动作", systemImage: "arrow.up.right.square") {
            VStack(spacing: 0) {
                HStack {
                    Spacer()
                    Button {
                        isProposingAction = true
                    } label: {
                        Label("新建动作", systemImage: "plus")
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.small)
                    .disabled(store.connectorsByIntent[snapshot.id, default: []].isEmpty)
                }
                .padding(.bottom, ElseSpacing.x2)

                if store.actionsByIntent[snapshot.id, default: []].isEmpty {
                    QuietEmptyState(
                        title: "还没有外部动作",
                        description: "为连接器创建一个受控写入提案。"
                    )
                }
                ForEach(store.actionsByIntent[snapshot.id, default: []]) { action in
                    ConnectorActionRow(intentID: snapshot.id, action: action)
                    if action.id != store.actionsByIntent[snapshot.id]?.last?.id {
                        Divider()
                    }
                }
            }
        }
    }

    private func boundaries(_ snapshot: IntentSnapshot) -> some View {
        DetailSection(title: "边界", systemImage: "lock.shield") {
            VStack(alignment: .leading, spacing: 10) {
                if snapshot.spec.constraints.isEmpty && snapshot.spec.authority.forbidden.isEmpty {
                    Text("未设置额外约束")
                        .font(ElseType.body)
                        .foregroundStyle(ElseColor.textSecondary)
                }

                ForEach(snapshot.spec.constraints, id: \.self) { constraint in
                    Label(constraint, systemImage: "minus.circle")
                        .font(ElseType.body)
                        .foregroundStyle(ElseColor.textPrimary)
                }

                ForEach(snapshot.spec.authority.forbidden, id: \.self) { rule in
                    Label(rule, systemImage: "xmark.shield")
                        .font(ElseType.body)
                        .foregroundStyle(ElseColor.danger)
                }

                Divider()

                HStack {
                    Text("自主副作用上限")
                        .foregroundStyle(ElseColor.textSecondary)
                    Spacer()
                    Text(snapshot.spec.authority.maxAutonomousEffect.rawValue)
                        .font(ElseType.mono)
                        .foregroundStyle(ElseColor.textPrimary)
                }
                .font(ElseType.body)
            }
        }
    }

    @ViewBuilder
    private func artifacts(_ snapshot: IntentSnapshot) -> some View {
        if !snapshot.artifacts.isEmpty {
            DetailSection(title: "产物", systemImage: "shippingbox") {
                VStack(spacing: 0) {
                    ForEach(snapshot.artifacts) { artifact in
                        HStack(alignment: .top, spacing: 10) {
                            Image(systemName: artifactIcon(artifact.kind))
                                .foregroundStyle(ElseColor.textSecondary)
                                .frame(width: 18)
                            VStack(alignment: .leading, spacing: 3) {
                                Text(artifact.locator)
                                    .font(ElseType.mono)
                                    .foregroundStyle(ElseColor.textPrimary)
                                    .textSelection(.enabled)
                                Text(artifact.note)
                                    .font(ElseType.caption)
                                    .foregroundStyle(ElseColor.textSecondary)
                            }
                            Spacer()
                            Text(artifact.id.rawValue.prefix(8))
                                .font(ElseType.mono)
                                .foregroundStyle(ElseColor.textTertiary)
                        }
                        .padding(.vertical, 9)

                        if artifact.id != snapshot.artifacts.last?.id {
                            Divider()
                        }
                    }
                }
            }
        }
    }

    private func resources(_ snapshot: IntentSnapshot) -> some View {
        DetailSection(title: "资源账", systemImage: "gauge.with.dots.needle.33percent") {
            VStack(spacing: 10) {
                MetricRow(
                    label: "模型 Token",
                    value:
                        "\(snapshot.resourceUsage.modelTokens.compactFormatted) / \(snapshot.resourceBudget.maxModelTokens.compactFormatted)"
                )
                MetricRow(
                    label: "模型费用",
                    value:
                        "$\(cents(snapshot.resourceUsage.modelSpendUSDMicros)) / $\(cents(snapshot.resourceBudget.maxModelSpendUSDMicros))"
                )
                MetricRow(
                    label: "工具调用",
                    value:
                        "\(snapshot.resourceUsage.toolCalls.formatted()) / \(snapshot.resourceBudget.maxToolCalls.formatted())"
                )
                MetricRow(
                    label: "Sandbox",
                    value:
                        "\(duration(snapshot.resourceUsage.sandboxSeconds)) / \(duration(snapshot.resourceBudget.maxSandboxSeconds))"
                )

                HairlineProgress(
                    value: min(
                        Double(snapshot.resourceUsage.modelTokens)
                            / Double(max(snapshot.resourceBudget.maxModelTokens, 1)),
                        1
                    ),
                    tint: snapshot.state.tint
                )
            }
        }
    }

    private func artifactIcon(_ kind: ArtifactKind) -> String {
        switch kind {
        case .fileTree: "folder"
        case .document: "doc.text"
        case .structuredRecord: "tablecells"
        case .externalHandle: "link"
        }
    }

    private func cents(_ micros: UInt64) -> String {
        String(format: "%.2f", Double(micros) / 1_000_000)
    }

    private func duration(_ seconds: UInt64) -> String {
        Duration.seconds(seconds).formatted(.units(allowed: [.hours, .minutes], width: .narrow))
    }
}

private struct DetailSection<Content: View>: View {
    let title: String
    let systemImage: String
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label(title, systemImage: systemImage)
                .font(ElseType.section)
                .foregroundStyle(ElseColor.textPrimary)
            content
                .padding(.leading, 1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.top, ElseSpacing.x4)
        .overlay(alignment: .top) {
            Rectangle()
                .fill(ElseColor.lineSubtle)
                .frame(height: 1)
        }
    }
}

private struct ConnectorActionRow: View {
    @EnvironmentObject private var store: AppStore
    let intentID: IntentID
    let action: ConnectorAction

    private var isMutating: Bool {
        store.mutatingActionIDs.contains(action.id)
    }

    var body: some View {
        HStack(alignment: .top, spacing: ElseSpacing.x3) {
            Rectangle()
                .fill(action.status.tint)
                .frame(width: 2)
                .padding(.vertical, 2)

            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text(action.status.label)
                        .font(ElseType.micro)
                        .foregroundStyle(action.status.tint)
                    Spacer()
                    Text(action.effect.rawValue)
                        .font(ElseType.mono)
                        .foregroundStyle(ElseColor.textTertiary)
                }

                Text(action.capability)
                    .font(ElseType.callout)
                    .foregroundStyle(ElseColor.textPrimary)
                Text(action.target)
                    .font(ElseType.mono)
                    .foregroundStyle(ElseColor.textSecondary)
                    .lineLimit(1)
                    .textSelection(.enabled)

                if let outcome = action.outcome {
                    Text("\(outcome.status.rawValue) · \(outcome.effect.rawValue)")
                        .font(ElseType.caption)
                        .foregroundStyle(ElseColor.textSecondary)
                }
                if let error = action.lastError {
                    Text(error)
                        .font(ElseType.caption)
                        .foregroundStyle(ElseColor.danger)
                        .lineLimit(2)
                }
            }

            Spacer(minLength: ElseSpacing.x2)

            if isMutating {
                ProgressView()
                    .controlSize(.mini)
            } else {
                actionButtons
            }
        }
        .padding(.vertical, ElseSpacing.x3)
    }

    @ViewBuilder
    private var actionButtons: some View {
        HStack(spacing: 6) {
            if action.status.canExecute {
                Button {
                    Task {
                        await store.executeConnectorAction(
                            intentID: intentID,
                            action: action
                        )
                    }
                } label: {
                    Label("执行", systemImage: "play")
                }
                .help("执行外部动作；超出授权时会生成 Decision")
            }

            if action.status.canVerify {
                Button {
                    Task {
                        await store.verifyConnectorAction(
                            intentID: intentID,
                            action: action
                        )
                    }
                } label: {
                    Label("核验", systemImage: "arrow.clockwise")
                }
                .help("回源查询 Provider 的真实结果")
            }

            if action.status.canCompensate {
                Button {
                    Task {
                        await store.compensateConnectorAction(
                            intentID: intentID,
                            action: action
                        )
                    }
                } label: {
                    Label("补偿", systemImage: "arrow.uturn.backward")
                }
                .help("撤销已应用动作；需要重新授权")
            }
        }
        .buttonStyle(.bordered)
        .controlSize(.small)
    }
}

private struct DecisionBand: View {
    @EnvironmentObject private var store: AppStore
    let snapshot: IntentSnapshot
    let request: AttentionRequest

    var body: some View {
        SignalBand {
            VStack(alignment: .leading, spacing: ElseSpacing.x3) {
                decisionHeader
                decisionCopy
                decisionActions
            }
        }
    }

    private var decisionHeader: some View {
        HStack {
            Label(request.kind.label, systemImage: "hand.raised.fill")
                .font(ElseType.micro)
                .foregroundStyle(ElseColor.signal)
            Spacer()
            Text("影响 \(request.expectedLoss.label)")
                .font(ElseType.micro)
                .foregroundStyle(
                    request.expectedLoss >= .high
                        ? ElseColor.danger
                        : ElseColor.textSecondary
                )
            if request.blocking {
                Text("阻塞")
                    .font(ElseType.micro)
                    .foregroundStyle(ElseColor.textSecondary)
            }
        }
    }

    private var decisionCopy: some View {
        VStack(alignment: .leading, spacing: 7) {
            Text(request.summary)
                .font(ElseType.section)
                .foregroundStyle(ElseColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)

            Text(request.blockedReason)
                .font(ElseType.body)
                .foregroundStyle(ElseColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var decisionActions: some View {
        HStack(spacing: 8) {
            ForEach(request.options) { option in
                optionButton(option)
            }

            Spacer()

            Button("稍后", role: .cancel) {
                dismissRequest()
            }
            .buttonStyle(.plain)
            .foregroundStyle(ElseColor.textSecondary)
            .disabled(store.mutatingIntentIDs.contains(snapshot.id))
        }
    }

    @ViewBuilder
    private func optionButton(_ option: DecisionOption) -> some View {
        let button = Button(option.label) {
            choose(option)
        }
        .disabled(store.mutatingIntentIDs.contains(snapshot.id))

        if option.recommended {
            button
                .buttonStyle(.borderedProminent)
                .tint(ElseColor.signal)
        } else {
            button.buttonStyle(.bordered)
        }
    }

    private func choose(_ option: DecisionOption) {
        Task {
            await store.resolve(
                snapshot: snapshot,
                request: request,
                choice: .option(option.id)
            )
        }
    }

    private func dismissRequest() {
        Task {
            await store.resolve(
                snapshot: snapshot,
                request: request,
                choice: .dismissed
            )
        }
    }
}

private extension ConnectorActionStatus {
    var label: String {
        switch self {
        case .proposed: "待执行"
        case .needsApproval: "待授权"
        case .executing: "执行中"
        case .applied: "已应用"
        case .compensating: "补偿待授权"
        case .compensationExecuting: "补偿执行中"
        case .compensated: "已补偿"
        case .compensationFailed: "补偿失败"
        case .failed: "失败"
        case .unknown: "结果未知"
        case .cancelled: "已取消"
        }
    }

    var tint: Color {
        switch self {
        case .applied, .compensated: ElseColor.success
        case .needsApproval, .unknown: ElseColor.signal
        case .executing, .compensationExecuting: ElseColor.info
        case .failed, .compensationFailed: ElseColor.danger
        case .proposed, .compensating, .cancelled: ElseColor.textSecondary
        }
    }

    var canExecute: Bool {
        self == .proposed || self == .needsApproval
    }

    var canVerify: Bool {
        self == .executing || self == .unknown || self == .applied
    }

    var canCompensate: Bool {
        self == .applied || self == .compensating
    }
}
