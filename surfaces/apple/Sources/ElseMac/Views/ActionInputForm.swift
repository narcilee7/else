import ElseClientCore
import SwiftUI

struct ActionFormState {
    var calendarSummary = ""
    var calendarDescription = ""
    var calendarLocation = ""
    var calendarIncludesStart = false
    var calendarStart = Date().addingTimeInterval(3_600)
    var calendarIncludesEnd = false
    var calendarEnd = Date().addingTimeInterval(7_200)
    var calendarStatus = ""
    var calendarVisibility = ""

    var linearTitle = ""
    var linearDescription = ""
    var linearDueDate = ""
    var linearPriority: Int?
    var linearAssigneeID = ""
    var linearProjectID = ""
    var linearStateID = ""
    var linearMilestoneID = ""
    var linearComment = ""

    var notionTitleProperty = "Name"
    var notionTitle = ""
    var notionArchive: NotionArchiveSelection = .unchanged
    var notionIcon = ""
    var notionCoverURL = ""
    var notionDatabaseTitle = ""
    var notionDatabaseDescription = ""
    var dynamicText: [String: String] = [:]
    var dynamicEnum: [String: String] = [:]
    var dynamicBoolean: [String: String] = [:]
}

enum NotionArchiveSelection: String, CaseIterable {
    case unchanged
    case archive
    case restore

    var archivedValue: Bool? {
        switch self {
        case .unchanged: nil
        case .archive: true
        case .restore: false
        }
    }
}

struct ActionInputForm: View {
    let capability: String
    @Binding var state: ActionFormState

    @ViewBuilder
    var body: some View {
        switch capability {
        case "calendar.event.update":
            calendarFields
        case "linear.issue.update":
            linearIssueFields
        case "linear.issue.comment":
            linearCommentFields
        case "notion.page.update":
            notionPageFields
        case "notion.database.update":
            notionDatabaseFields
        default:
            EmptyView()
        }
    }

    private var calendarFields: some View {
        Group {
            TextField("标题", text: $state.calendarSummary)
            TextField("描述", text: $state.calendarDescription, axis: .vertical)
            TextField("地点", text: $state.calendarLocation)

            Toggle("修改开始时间", isOn: $state.calendarIncludesStart)
            if state.calendarIncludesStart {
                DatePicker(
                    "开始",
                    selection: $state.calendarStart,
                    displayedComponents: [.date, .hourAndMinute]
                )
            }

            Toggle("修改结束时间", isOn: $state.calendarIncludesEnd)
            if state.calendarIncludesEnd {
                DatePicker(
                    "结束",
                    selection: $state.calendarEnd,
                    displayedComponents: [.date, .hourAndMinute]
                )
            }

            Picker("状态", selection: $state.calendarStatus) {
                Text("不修改").tag("")
                Text("已确认").tag("confirmed")
                Text("暂定").tag("tentative")
                Text("已取消").tag("cancelled")
            }
            Picker("可见性", selection: $state.calendarVisibility) {
                Text("不修改").tag("")
                Text("默认").tag("default")
                Text("公开").tag("public")
                Text("私密").tag("private")
                Text("机密").tag("confidential")
            }
        }
    }

    private var linearIssueFields: some View {
        Group {
            TextField("标题", text: $state.linearTitle)
            TextField("描述", text: $state.linearDescription, axis: .vertical)
            TextField("截止日期（YYYY-MM-DD）", text: $state.linearDueDate)
            Picker("优先级", selection: $state.linearPriority) {
                Text("不修改").tag(Int?.none)
                Text("无").tag(Int?.some(0))
                Text("低").tag(Int?.some(1))
                Text("中").tag(Int?.some(2))
                Text("高").tag(Int?.some(3))
                Text("紧急").tag(Int?.some(4))
            }
            TextField("Assignee ID", text: $state.linearAssigneeID)
            TextField("Project ID", text: $state.linearProjectID)
            TextField("State ID", text: $state.linearStateID)
            TextField("Milestone ID", text: $state.linearMilestoneID)
        }
    }

    private var linearCommentFields: some View {
        TextField("评论内容", text: $state.linearComment, axis: .vertical)
            .lineLimit(4...10)
    }

    private var notionPageFields: some View {
        Group {
            TextField("标题属性名", text: $state.notionTitleProperty)
            TextField("标题", text: $state.notionTitle)
            Picker("归档状态", selection: $state.notionArchive) {
                Text("不修改").tag(NotionArchiveSelection.unchanged)
                Text("归档").tag(NotionArchiveSelection.archive)
                Text("恢复").tag(NotionArchiveSelection.restore)
            }
            TextField("Emoji 图标", text: $state.notionIcon)
            TextField("封面图片 URL", text: $state.notionCoverURL)
        }
    }

    private var notionDatabaseFields: some View {
        Group {
            TextField("数据库标题", text: $state.notionDatabaseTitle)
            TextField(
                "数据库描述",
                text: $state.notionDatabaseDescription,
                axis: .vertical
            )
            Picker("归档状态", selection: $state.notionArchive) {
                Text("不修改").tag(NotionArchiveSelection.unchanged)
                Text("归档").tag(NotionArchiveSelection.archive)
                Text("恢复").tag(NotionArchiveSelection.restore)
            }
        }
    }
}

enum ActionEditorMode: String, CaseIterable {
    case form
    case json
}

extension ActionFormState {
    func input(for capability: String, timeZone: TimeZone = .current) -> JSONValue {
        switch capability {
        case "calendar.event.update":
            ConnectorActionInputBuilder.calendarUpdate(
                summary: calendarSummary,
                description: calendarDescription,
                location: calendarLocation,
                start: calendarIncludesStart ? calendarStart : nil,
                end: calendarIncludesEnd ? calendarEnd : nil,
                timeZone: timeZone,
                status: calendarStatus,
                visibility: calendarVisibility
            )
        case "linear.issue.update":
            ConnectorActionInputBuilder.linearIssueUpdate(
                title: linearTitle,
                description: linearDescription,
                dueDate: linearDueDate,
                priority: linearPriority,
                assigneeID: linearAssigneeID,
                projectID: linearProjectID,
                stateID: linearStateID,
                milestoneID: linearMilestoneID
            )
        case "linear.issue.comment":
            ConnectorActionInputBuilder.linearComment(body: linearComment)
        case "notion.page.update":
            ConnectorActionInputBuilder.notionPageUpdate(
                titleProperty: notionTitleProperty,
                title: notionTitle,
                archived: notionArchive.archivedValue,
                icon: notionIcon,
                coverURL: notionCoverURL
            )
        case "notion.database.update":
            ConnectorActionInputBuilder.notionDatabaseUpdate(
                title: notionDatabaseTitle,
                description: notionDatabaseDescription,
                archived: notionArchive.archivedValue
            )
        default:
            .object([:])
        }
    }

    func dynamicInput(schema: ConnectorActionSchema) -> JSONValue {
        var values: [String: JSONValue] = [:]
        for field in schema.fields {
            guard let value = dynamicValue(field) else {
                continue
            }
            values[field.path] = value
        }
        return ConnectorActionInputBuilder.dynamicInput(values: values)
    }

    func dynamicValidationError(
        schema: ConnectorActionSchema
    ) -> String? {
        for field in schema.fields where field.required {
            guard let value = dynamicValue(field),
                  value != .string("")
            else {
                return "请填写 \(field.title)"
            }
        }
        for field in schema.fields where field.kind == "json" {
            let text = dynamicText[field.path, default: ""]
                .trimmingCharacters(in: .whitespacesAndNewlines)
            if !text.isEmpty, (try? JSONValue.parse(text)) == nil {
                return "\(field.title) 必须是有效 JSON"
            }
        }
        return nil
    }

    private func dynamicValue(
        _ field: ConnectorActionFieldSchema
    ) -> JSONValue? {
        switch field.kind {
        case "boolean":
            switch dynamicBoolean[field.path] ?? "" {
            case "true":
                return .bool(true)
            case "false":
                return .bool(false)
            default:
                return nil
            }
        case "enum":
            guard let tag = dynamicEnum[field.path],
                  let value = try? JSONValue.parse(tag)
            else {
                return nil
            }
            return value
        case "integer":
            let text = dynamicText[field.path, default: ""]
            guard let value = Int(text) else {
                return nil
            }
            return .number(Double(value))
        case "json":
            let text = dynamicText[field.path, default: ""]
                .trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty else {
                return nil
            }
            return try? JSONValue.parse(text)
        default:
            let text = dynamicText[field.path, default: ""]
                .trimmingCharacters(in: .whitespacesAndNewlines)
            return text.isEmpty ? nil : .string(text)
        }
    }
}

struct DynamicActionInputForm: View {
    let schema: ConnectorActionSchema
    @Binding var state: ActionFormState

    var body: some View {
        ForEach(schema.fields) { field in
            fieldView(field)
        }
    }

    @ViewBuilder
    private func fieldView(_ field: ConnectorActionFieldSchema) -> some View {
        switch field.kind {
        case "enum":
            Picker(field.title, selection: textBinding(field.path, in: \.dynamicEnum)) {
                if !field.required {
                    Text("不修改").tag("")
                }
                ForEach(field.options, id: \.label) { option in
                    Text(option.label)
                        .tag(optionTag(option.value))
                }
            }
        case "boolean":
            Picker(field.title, selection: textBinding(field.path, in: \.dynamicBoolean)) {
                Text("不修改").tag("")
                Text("是").tag("true")
                Text("否").tag("false")
            }
        case "multiline", "json":
            TextField(
                field.title,
                text: textBinding(field.path, in: \.dynamicText),
                axis: .vertical
            )
            .lineLimit(field.kind == "json" ? 3...8 : 2...6)
        default:
            TextField(
                field.title,
                text: textBinding(field.path, in: \.dynamicText)
            )
        }
    }

    private func textBinding(
        _ path: String,
        in keyPath: WritableKeyPath<ActionFormState, [String: String]>
    ) -> Binding<String> {
        Binding(
            get: { $state.wrappedValue[keyPath: keyPath][path, default: ""] },
            set: { $state.wrappedValue[keyPath: keyPath][path] = $0 }
        )
    }

    private func optionTag(_ value: JSONValue) -> String {
        guard let data = try? JSONEncoder().encode(value),
              let tag = String(data: data, encoding: .utf8)
        else {
            return "null"
        }
        return tag
    }
}
