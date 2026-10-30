import ElseClientCore
import SwiftUI

struct IntentDetailView: View {
    @EnvironmentObject private var store: AppStore
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
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
            if let snapshot, !snapshot.isTerminal {
                ToolbarItemGroup {
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
            }
        }
        .animation(reduceMotion ? nil : ElseMotion.emphasis, value: snapshot?.state)
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
