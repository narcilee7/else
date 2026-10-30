import ElseClientCore
import SwiftUI

extension IntentState {
    var label: String {
        switch self {
        case .active: "执行中"
        case .waitingExternal: "等待外部"
        case .needsDecision: "待决策"
        case .paused: "已暂停"
        case .completed: "已完成"
        case .cancelled: "已取消"
        case .failed: "失败"
        }
    }

    var systemImage: String {
        switch self {
        case .active: "bolt"
        case .waitingExternal: "clock"
        case .needsDecision: "hand.raised"
        case .paused: "pause"
        case .completed: "checkmark"
        case .cancelled: "xmark"
        case .failed: "exclamationmark"
        }
    }

    var tint: Color {
        switch self {
        case .active: ElseColor.textPrimary
        case .waitingExternal: ElseColor.info
        case .needsDecision: ElseColor.signal
        case .paused: ElseColor.textTertiary
        case .completed: ElseColor.success
        case .cancelled: ElseColor.textTertiary
        case .failed: ElseColor.danger
        }
    }

    var railDash: [CGFloat] {
        switch self {
        case .waitingExternal: [3, 4]
        case .paused: [2, 5]
        case .completed: [18, 4, 8]
        case .cancelled: [6, 5]
        default: []
        }
    }
}

extension AttentionKind {
    var label: String {
        switch self {
        case .judgment: "判断"
        case .authorization: "授权"
        case .missingInfo: "补充信息"
        case .takeover: "需要接管"
        }
    }
}

extension ExpectedLoss {
    var label: String {
        switch self {
        case .low: "低"
        case .medium: "中"
        case .high: "高"
        case .critical: "紧急"
        }
    }
}

struct StateBadge: View {
    let state: IntentState

    var body: some View {
        Label(state.label, systemImage: state.systemImage)
            .font(ElseType.micro)
            .foregroundStyle(state.tint)
            .contentTransition(.symbolEffect(.replace))
    }
}

struct SectionLabel: View {
    let title: String

    var body: some View {
        Text(title)
            .font(ElseType.micro)
            .tracking(0.4)
            .foregroundStyle(ElseColor.textSecondary)
    }
}

struct MetricRow: View {
    let label: String
    let value: String

    var body: some View {
        HStack {
            Text(label)
                .foregroundStyle(ElseColor.textSecondary)
            Spacer()
            Text(value)
                .font(ElseType.mono)
                .monospacedDigit()
        }
        .font(ElseType.body)
    }
}

struct StatusRail: View {
    let state: IntentState
    var height: CGFloat = 42

    var body: some View {
        Canvas { context, size in
            var path = Path()
            path.move(to: CGPoint(x: size.width / 2, y: 0))
            path.addLine(to: CGPoint(x: size.width / 2, y: size.height))
            context.stroke(
                path,
                with: .color(state.tint),
                style: StrokeStyle(
                    lineWidth: 2,
                    lineCap: .round,
                    dash: state.railDash
                )
            )
        }
        .frame(width: 2, height: height)
        .accessibilityHidden(true)
    }
}

struct SignalBand<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        HStack(spacing: 0) {
            Rectangle()
                .fill(ElseColor.signal)
                .frame(width: 2)

            content
                .padding(.horizontal, ElseSpacing.x4)
                .padding(.vertical, ElseSpacing.x3)
        }
        .background(
            ElseColor.signalSoft,
            in: RoundedRectangle(cornerRadius: ElseRadius.floating)
        )
        .clipShape(RoundedRectangle(cornerRadius: ElseRadius.floating))
    }
}

struct HairlineProgress: View {
    let value: Double
    let tint: Color

    private var clampedValue: Double {
        min(max(value, 0), 1)
    }

    var body: some View {
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                Capsule()
                    .fill(ElseColor.lineSubtle)

                Capsule()
                    .fill(tint)
                    .frame(width: proxy.size.width * clampedValue)
            }
        }
        .frame(height: 2)
        .accessibilityValue(
            "\(Int((clampedValue * 100).rounded()))%"
        )
    }
}

struct QuietEmptyState: View {
    let title: String
    let description: String

    var body: some View {
        VStack(spacing: ElseSpacing.x3) {
            Rectangle()
                .fill(ElseColor.lineStrong)
                .frame(width: 28, height: 1)

            Text(title)
                .font(ElseType.section)
                .foregroundStyle(ElseColor.textPrimary)

            Text(description)
                .font(ElseType.caption)
                .foregroundStyle(ElseColor.textSecondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 280)
        }
        .padding(ElseSpacing.x6)
    }
}

extension UInt64 {
    var compactFormatted: String {
        if self >= 1_000_000 {
            return String(format: "%.1fM", Double(self) / 1_000_000)
        }
        if self >= 1_000 {
            return String(format: "%.1fk", Double(self) / 1_000)
        }
        return formatted()
    }
}
