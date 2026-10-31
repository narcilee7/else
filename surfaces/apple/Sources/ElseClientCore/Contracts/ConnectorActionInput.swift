import Foundation

public enum ConnectorActionInputBuilder {
    public static func calendarUpdate(
        summary: String,
        description: String,
        location: String,
        start: Date?,
        end: Date?,
        timeZone: TimeZone,
        status: String,
        visibility: String
    ) -> JSONValue {
        var changes: [String: JSONValue] = [:]
        insert(summary, key: "summary", into: &changes)
        insert(description, key: "description", into: &changes)
        insert(location, key: "location", into: &changes)
        if let start {
            changes["start"] = calendarDate(start, timeZone: timeZone)
        }
        if let end {
            changes["end"] = calendarDate(end, timeZone: timeZone)
        }
        insert(status, key: "status", into: &changes)
        insert(visibility, key: "visibility", into: &changes)
        return .object(["changes": .object(changes)])
    }

    public static func linearIssueUpdate(
        title: String,
        description: String,
        dueDate: String,
        priority: Int?,
        assigneeID: String,
        projectID: String,
        stateID: String,
        milestoneID: String
    ) -> JSONValue {
        var changes: [String: JSONValue] = [:]
        insert(title, key: "title", into: &changes)
        insert(description, key: "description", into: &changes)
        insert(dueDate, key: "dueDate", into: &changes)
        if let priority {
            changes["priority"] = .number(Double(priority))
        }
        insert(assigneeID, key: "assigneeId", into: &changes)
        insert(projectID, key: "projectId", into: &changes)
        insert(stateID, key: "stateId", into: &changes)
        insert(milestoneID, key: "milestoneId", into: &changes)
        return .object(["changes": .object(changes)])
    }

    public static func linearComment(body: String) -> JSONValue {
        .object(["body": .string(body)])
    }

    public static func notionPageUpdate(
        titleProperty: String,
        title: String,
        archived: Bool?,
        icon: String,
        coverURL: String
    ) -> JSONValue {
        var changes: [String: JSONValue] = [:]
        if !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            let property = titleProperty.trimmingCharacters(in: .whitespacesAndNewlines)
            changes["properties"] = .object([
                (property.isEmpty ? "Name" : property): .object([
                    "title": .array([
                        .object([
                            "type": .string("text"),
                            "text": .object([
                                "content": .string(title),
                            ]),
                        ]),
                    ]),
                ]),
            ])
        }
        if let archived {
            changes["archived"] = .bool(archived)
        }
        if !icon.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            changes["icon"] = .object([
                "type": .string("emoji"),
                "emoji": .string(icon),
            ])
        }
        if !coverURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            changes["cover"] = .object([
                "type": .string("external"),
                "external": .object([
                    "url": .string(coverURL),
                ]),
            ])
        }
        return .object(["changes": .object(changes)])
    }

    public static func notionDatabaseUpdate(
        title: String,
        description: String,
        archived: Bool?
    ) -> JSONValue {
        var changes: [String: JSONValue] = [:]
        if !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            changes["title"] = notionRichText(title)
        }
        if !description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            changes["description"] = notionRichText(description)
        }
        if let archived {
            changes["archived"] = .bool(archived)
        }
        return .object(["changes": .object(changes)])
    }

    public static func dynamicInput(
        values: [String: JSONValue]
    ) -> JSONValue {
        var root: [String: JSONValue] = [:]
        for (path, value) in values.sorted(by: { $0.key < $1.key }) {
            insert(value, path: path.split(separator: ".").map(String.init), into: &root)
        }
        return .object(root)
    }

    private static func insert(
        _ value: String,
        key: String,
        into object: inout [String: JSONValue]
    ) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        object[key] = .string(trimmed)
    }

    private static func calendarDate(
        _ value: Date,
        timeZone: TimeZone
    ) -> JSONValue {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        formatter.timeZone = timeZone
        return .object([
            "dateTime": .string(formatter.string(from: value)),
            "timeZone": .string(timeZone.identifier),
        ])
    }

    private static func notionRichText(_ value: String) -> JSONValue {
        .array([
            .object([
                "type": .string("text"),
                "text": .object([
                    "content": .string(value),
                ]),
            ]),
        ])
    }

    private static func insert(
        _ value: JSONValue,
        path: [String],
        into object: inout [String: JSONValue]
    ) {
        guard let head = path.first else {
            return
        }
        if path.count == 1 {
            object[head] = value
            return
        }
        var child = object[head]?.objectValue ?? [:]
        insert(value, path: Array(path.dropFirst()), into: &child)
        object[head] = .object(child)
    }
}

public extension JSONValue {
    var isEmptyObject: Bool {
        if case let .object(value) = self {
            return value.isEmpty
        }
        return false
    }

    var objectValue: [String: JSONValue]? {
        if case let .object(value) = self {
            return value
        }
        return nil
    }
}
