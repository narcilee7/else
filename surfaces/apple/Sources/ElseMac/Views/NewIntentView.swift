import SwiftUI

struct NewIntentView: View {
    @EnvironmentObject private var store: AppStore
    @Environment(\.dismiss) private var dismiss

    @State private var outcome = ""
    @State private var project = ""
    @State private var isSubmitting = false

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text("新建 Intent")
                        .font(ElseType.pageTitle)
                        .foregroundStyle(ElseColor.textPrimary)
                    Text("描述希望现实发生的变化。")
                        .font(ElseType.caption)
                        .foregroundStyle(ElseColor.textSecondary)
                }
                Spacer()
                Button {
                    dismiss()
                } label: {
                    Image(systemName: "xmark")
                }
                .buttonStyle(.borderless)
                .foregroundStyle(ElseColor.textSecondary)
                .help("关闭")
            }

            VStack(alignment: .leading, spacing: 7) {
                Text("目标")
                    .font(ElseType.micro)
                    .foregroundStyle(ElseColor.textSecondary)
                ZStack(alignment: .topLeading) {
                    if outcome.isEmpty {
                        Text("例如：整理本周客户反馈并形成可评审的摘要")
                            .font(ElseType.body)
                            .foregroundStyle(ElseColor.textTertiary)
                            .padding(.horizontal, 5)
                            .padding(.vertical, 7)
                            .allowsHitTesting(false)
                    }
                    TextEditor(text: $outcome)
                        .font(ElseType.body)
                        .foregroundStyle(ElseColor.textPrimary)
                        .scrollContentBackground(.hidden)
                        .padding(0)
                }
                .frame(minHeight: 112)
                .overlay(alignment: .bottom) {
                    Rectangle()
                        .fill(ElseColor.lineStrong)
                        .frame(height: 1)
                }
            }

            VStack(alignment: .leading, spacing: 7) {
                Text("项目")
                    .font(ElseType.micro)
                    .foregroundStyle(ElseColor.textSecondary)
                TextField("可选，用于限定记忆作用域", text: $project)
                    .textFieldStyle(.plain)
                    .font(ElseType.body)
                    .padding(.vertical, 6)
                    .overlay(alignment: .bottom) {
                        Rectangle()
                            .fill(ElseColor.lineSubtle)
                            .frame(height: 1)
                    }
            }

            HStack {
                if store.isDemo {
                    Label("Demo 数据", systemImage: "testtube.2")
                        .font(ElseType.caption)
                        .foregroundStyle(ElseColor.textSecondary)
                }
                Spacer()
                Button("取消") {
                    dismiss()
                }
                .keyboardShortcut(.cancelAction)

                Button("提交") {
                    submit()
                }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.borderedProminent)
                .tint(ElseColor.action)
                .disabled(trimmedOutcome.isEmpty || isSubmitting)
            }
        }
        .padding(24)
        .frame(width: 540)
        .background(ElseColor.elevated)
    }

    private var trimmedOutcome: String {
        outcome.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func submit() {
        isSubmitting = true
        let projectValue = project.trimmingCharacters(in: .whitespacesAndNewlines)
        Task {
            await store.createIntent(
                outcome: trimmedOutcome,
                project: projectValue.isEmpty ? nil : projectValue
            )
            isSubmitting = false
        }
    }
}
