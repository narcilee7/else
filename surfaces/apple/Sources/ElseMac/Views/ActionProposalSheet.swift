import ElseClientCore
import SwiftUI

struct ActionProposalSheet: View {
    @EnvironmentObject private var store: AppStore
    @Environment(\.dismiss) private var dismiss
    let intentID: IntentID

    @State private var connectorID: ConnectorID?
    @State private var capability = ""
    @State private var target = ""
    @State private var input = "{}"
    @State private var idempotencyKey = ""
    @State private var isSaving = false
    @State private var errorMessage: String?

    private var connectors: [ConnectorRegistration] {
        store.connectorsByIntent[intentID, default: []]
            .filter { !$0.manifest.writeCapabilities.isEmpty }
    }

    private var capabilities: [String] {
        connectors.first(where: { $0.id == connectorID })?
            .manifest.writeCapabilities ?? []
    }

    var body: some View {
        VStack(alignment: .leading, spacing: ElseSpacing.x4) {
            Text("新建外部动作")
                .font(ElseType.pageTitle)
                .foregroundStyle(ElseColor.textPrimary)

            Form {
                Picker("Connector", selection: $connectorID) {
                    ForEach(connectors) { connector in
                        Text(connector.name)
                            .tag(Optional(connector.id))
                    }
                }
                .onChange(of: connectorID) {
                    capability = capabilities.first ?? ""
                }

                Picker("Capability", selection: $capability) {
                    ForEach(capabilities, id: \.self) { capability in
                        Text(capability).tag(capability)
                    }
                }

                TextField("目标对象 ID", text: $target)
                TextField("幂等键（可选）", text: $idempotencyKey)
            }
            .formStyle(.grouped)

            VStack(alignment: .leading, spacing: ElseSpacing.x2) {
                Text("参数")
                    .font(ElseType.micro)
                    .foregroundStyle(ElseColor.textSecondary)
                TextEditor(text: $input)
                    .font(ElseType.mono)
                    .scrollContentBackground(.hidden)
                    .padding(ElseSpacing.x2)
                    .background(
                        ElseColor.surface,
                        in: RoundedRectangle(cornerRadius: ElseRadius.small)
                    )
                    .frame(minHeight: 150)
            }

            if let errorMessage {
                Text(errorMessage)
                    .font(ElseType.caption)
                    .foregroundStyle(ElseColor.danger)
            }

            HStack {
                Spacer()
                Button("取消") {
                    dismiss()
                }
                Button(isSaving ? "创建中" : "创建提案") {
                    Task { await save() }
                }
                .buttonStyle(.borderedProminent)
                .tint(ElseColor.action)
                .disabled(!isValid || isSaving)
            }
        }
        .padding(ElseSpacing.x6)
        .frame(width: 560, height: 590)
        .background(ElseColor.canvas)
        .onAppear {
            guard connectorID == nil else { return }
            connectorID = connectors.first?.id
            capability = capabilities.first ?? ""
        }
    }

    private var isValid: Bool {
        connectorID != nil
            && !capability.isEmpty
            && !target.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func save() async {
        guard let connectorID else { return }
        do {
            let parsedInput = try JSONValue.parse(input)
            isSaving = true
            defer { isSaving = false }
            let action = await store.proposeConnectorAction(
                intentID: intentID,
                request: ProposeConnectorActionRequest(
                    connectorID: connectorID,
                    capability: capability,
                    target: target.trimmingCharacters(in: .whitespacesAndNewlines),
                    input: parsedInput,
                    idempotencyKey: idempotencyKey.nilIfEmpty
                )
            )
            if action == nil {
                errorMessage = store.operationError ?? "创建动作失败"
            } else {
                dismiss()
            }
        } catch {
            errorMessage = "参数必须是有效 JSON：\(error.localizedDescription)"
        }
    }
}

private extension String {
    var nilIfEmpty: String? {
        trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmptyValue
    }
}

private extension String {
    var nilIfEmptyValue: String? {
        self.isEmpty ? nil : self
    }
}
