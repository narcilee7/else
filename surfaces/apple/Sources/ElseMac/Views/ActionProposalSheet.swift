import ElseClientCore
import SwiftUI

struct ActionProposalSheet: View {
    @EnvironmentObject private var store: AppStore
    @Environment(\.dismiss) private var dismiss
    let intentID: IntentID

    @State private var connectorID: ConnectorID?
    @State private var capability = ""
    @State private var target = ""
    @State private var editorMode: ActionEditorMode = .form
    @State private var form = ActionFormState()
    @State private var jsonInput = "{}"
    @State private var idempotencyKey = ""
    @State private var isSaving = false
    @State private var errorMessage: String?
    @State private var preview: ConnectorActionPreview?
    @State private var isPreviewing = false
    @State private var previewError: String?

    private var connectors: [ConnectorRegistration] {
        store.connectorsByIntent[intentID, default: []]
            .filter { !$0.manifest.writeCapabilities.isEmpty }
    }

    private var capabilities: [String] {
        connectors.first(where: { $0.id == connectorID })?
            .manifest.writeCapabilities ?? []
    }

    private var hasStaticStructuredForm: Bool {
        [
            "calendar.event.update",
            "linear.issue.update",
            "linear.issue.comment",
            "notion.page.update",
            "notion.database.update",
        ].contains(capability)
    }

    private var supportsStructuredForm: Bool {
        hasStaticStructuredForm || actionSchema?.fields.isEmpty == false
    }

    private var actionSchema: ConnectorActionSchema? {
        guard let connectorID else {
            return nil
        }
        return store.capabilitiesByIntent[intentID]?
            .first(where: { registration in
                registration.externalAction?.connectorID == connectorID
                    && registration.externalAction?.operation == capability
            })?
            .manifest.inputSchema.connectorActionSchema
    }

    var body: some View {
        VStack(alignment: .leading, spacing: ElseSpacing.x4) {
            Text("新建外部动作")
                .font(ElseType.pageTitle)
                .foregroundStyle(ElseColor.textPrimary)

            Form {
                Section("动作") {
                    Picker("Connector", selection: $connectorID) {
                        ForEach(connectors) { connector in
                            Text(connector.name)
                                .tag(Optional(connector.id))
                        }
                    }
                    .onChange(of: connectorID) {
                        capability = capabilities.first ?? ""
                        preview = nil
                        updateEditorMode()
                    }

                    Picker("Capability", selection: $capability) {
                        ForEach(capabilities, id: \.self) { capability in
                            Text(capability).tag(capability)
                        }
                    }
                    .onChange(of: capability) {
                        preview = nil
                        updateEditorMode()
                    }

                    TextField(targetLabel, text: $target)
                        .onChange(of: target) {
                            preview = nil
                        }
                    TextField("幂等键（可选）", text: $idempotencyKey)
                }

                Section("参数") {
                    Picker("编辑方式", selection: $editorMode) {
                        if supportsStructuredForm {
                            Text("表单").tag(ActionEditorMode.form)
                        }
                        Text("高级 JSON").tag(ActionEditorMode.json)
                    }
                    .pickerStyle(.segmented)

                    if editorMode == .form, supportsStructuredForm {
                        if hasStaticStructuredForm {
                            ActionInputForm(capability: capability, state: $form)
                        } else if let actionSchema {
                            DynamicActionInputForm(
                                schema: actionSchema,
                                state: $form
                            )
                        }
                    } else {
                        TextEditor(text: $jsonInput)
                            .font(ElseType.mono)
                            .scrollContentBackground(.hidden)
                            .padding(ElseSpacing.x2)
                            .background(
                                ElseColor.surface,
                                in: RoundedRectangle(cornerRadius: ElseRadius.small)
                            )
                            .frame(minHeight: 130)
                            .onChange(of: jsonInput) {
                                preview = nil
                            }
                    }
                }

                if supportsStructuredForm {
                    Section("执行前预览") {
                        Text(inputPreview)
                            .font(ElseType.mono)
                            .foregroundStyle(ElseColor.textSecondary)
                            .textSelection(.enabled)
                            .lineLimit(6)

                        if let preview {
                            HStack {
                                Text(
                                    "已读取 \(preview.target) · \(preview.checkedAt.formatted(date: .omitted, time: .standard))"
                                )
                                .font(ElseType.micro)
                                .foregroundStyle(ElseColor.textTertiary)
                                Spacer()
                                Button {
                                    Task { await loadPreview() }
                                } label: {
                                    Image(systemName: "arrow.clockwise")
                                }
                                .buttonStyle(.borderless)
                                .help("刷新 Provider 当前值")
                                .disabled(isPreviewing || !canLoadPreview)
                            }

                            let diffs = previewDiffs
                            if diffs.isEmpty {
                                Text("没有字段变化")
                                    .font(ElseType.caption)
                                    .foregroundStyle(ElseColor.textSecondary)
                            } else {
                                ForEach(diffs) { diff in
                                    ActionDiffRow(diff: diff)
                                }
                            }
                        } else {
                            Button {
                                Task { await loadPreview() }
                            } label: {
                                Label(
                                    isPreviewing ? "读取中" : "读取当前值",
                                    systemImage: "arrow.triangle.2.circlepath"
                                )
                            }
                            .disabled(isPreviewing || !canLoadPreview)
                        }

                        if let previewError {
                            Text(previewError)
                                .font(ElseType.caption)
                                .foregroundStyle(ElseColor.danger)
                        }
                    }
                }
            }
            .formStyle(.grouped)

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
        .frame(width: 620, height: 680)
        .background(ElseColor.canvas)
        .onAppear {
            guard connectorID == nil else { return }
            connectorID = connectors.first?.id
            capability = capabilities.first ?? ""
            updateEditorMode()
        }
    }

    private var isValid: Bool {
        connectorID != nil
            && !capability.isEmpty
            && !target.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && validationError == nil
    }

    private var targetLabel: String {
        if let label = actionSchema?.targetLabel, !label.isEmpty {
            return label
        }
        return switch capability {
        case "calendar.event.update": "Event ID"
        case "linear.issue.update", "linear.issue.comment": "Issue ID"
        case "notion.page.update": "Page ID"
        case "notion.database.update": "Database ID"
        default: "目标对象 ID"
        }
    }

    private var inputPreview: String {
        if editorMode == .json {
            return jsonInput
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        guard let input = currentInput,
              let data = try? encoder.encode(input),
              let value = String(data: data, encoding: .utf8)
        else {
            return "{}"
        }
        return value
    }

    private var validationError: String? {
        if editorMode == .form, supportsStructuredForm {
            let input: JSONValue
            if hasStaticStructuredForm {
                input = form.input(for: capability)
            } else if let actionSchema {
                if let error = form.dynamicValidationError(schema: actionSchema) {
                    return error
                }
                input = form.dynamicInput(schema: actionSchema)
            } else {
                return "Capability 缺少输入 Schema"
            }
            if capability == "linear.issue.comment" {
                return form.linearComment.trimmingCharacters(
                    in: .whitespacesAndNewlines
                ).isEmpty ? "请输入评论内容" : nil
            }
            let changes = input.objectValue?["changes"]?.objectValue
            return changes?.isEmpty == true ? "至少修改一个字段" : nil
        }
        guard let parsed = try? JSONValue.parse(jsonInput),
              parsed.objectValue != nil
        else {
            return "参数必须是 JSON 对象"
        }
        return nil
    }

    private var currentInput: JSONValue? {
        if editorMode == .form, supportsStructuredForm {
            if hasStaticStructuredForm {
                return form.input(for: capability)
            }
            if let actionSchema {
                return form.dynamicInput(schema: actionSchema)
            }
        }
        return try? JSONValue.parse(jsonInput)
    }

    private var canLoadPreview: Bool {
        connectorID != nil
            && !target.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && currentInput?.objectValue != nil
            && (actionSchema?.previewSupported ?? true)
    }

    private var previewDiffs: [ConnectorActionFieldDiff] {
        guard let preview, let input = currentInput?.objectValue else {
            return []
        }
        let proposed = input["changes"]?.objectValue ?? input
        return ConnectorActionInputDiff.changes(
            current: preview.current,
            proposed: .object(proposed)
        )
    }

    private func save() async {
        guard let connectorID else { return }
        do {
            let parsedInput =
                if editorMode == .form, supportsStructuredForm {
                    hasStaticStructuredForm
                        ? form.input(for: capability)
                        : actionSchema.map { form.dynamicInput(schema: $0) } ?? .object([:])
                } else {
                    try JSONValue.parse(jsonInput)
                }
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

    private func updateEditorMode() {
        editorMode = supportsStructuredForm ? .form : .json
    }

    private func loadPreview() async {
        guard let connectorID,
              let input = currentInput,
              canLoadPreview
        else {
            return
        }
        isPreviewing = true
        previewError = nil
        defer { isPreviewing = false }
        preview = await store.previewConnectorAction(
            intentID: intentID,
            connectorID: connectorID,
            request: PreviewConnectorActionRequest(
                capability: capability,
                target: target.trimmingCharacters(in: .whitespacesAndNewlines),
                input: input
            )
        )
        if preview == nil {
            previewError = store.operationError ?? "无法读取 Provider 当前值"
        }
    }
}

private struct ActionDiffRow: View {
    let diff: ConnectorActionFieldDiff

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(diff.path)
                .font(ElseType.micro)
                .foregroundStyle(ElseColor.textTertiary)
            HStack(alignment: .top, spacing: ElseSpacing.x2) {
                Text(ConnectorActionInputDiff.display(diff.current))
                    .foregroundStyle(ElseColor.textSecondary)
                    .lineLimit(3)
                Image(systemName: "arrow.right")
                    .foregroundStyle(ElseColor.textTertiary)
                Text(ConnectorActionInputDiff.display(diff.proposed))
                    .foregroundStyle(ElseColor.textPrimary)
                    .lineLimit(3)
            }
            .font(ElseType.mono)
            .textSelection(.enabled)
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
