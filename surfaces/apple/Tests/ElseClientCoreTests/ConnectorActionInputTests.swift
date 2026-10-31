import Foundation
import Testing
@testable import ElseClientCore

@Suite("Connector action input")
struct ConnectorActionInputTests {
    @Test
    func buildsCalendarPatchWithStructuredTime() throws {
        let timeZone = try #require(TimeZone(identifier: "Asia/Shanghai"))
        let start = Date(timeIntervalSince1970: 1_793_400_000)
        let input = ConnectorActionInputBuilder.calendarUpdate(
            summary: "产品评审",
            description: "确认发布范围",
            location: "会议室",
            start: start,
            end: start.addingTimeInterval(3_600),
            timeZone: timeZone,
            status: "confirmed",
            visibility: "private"
        )
        let object = try encodedObject(input)
        let changes = try #require(object["changes"] as? [String: Any])
        let startValue = try #require(changes["start"] as? [String: Any])

        #expect(changes["summary"] as? String == "产品评审")
        #expect(changes["status"] as? String == "confirmed")
        #expect(startValue["timeZone"] as? String == "Asia/Shanghai")
        #expect(startValue["dateTime"] is String)
    }

    @Test
    func buildsLinearIssueAndCommentInputs() throws {
        let update = ConnectorActionInputBuilder.linearIssueUpdate(
            title: "Update API",
            description: "",
            dueDate: "2026-11-01",
            priority: 2,
            assigneeID: "user-1",
            projectID: "",
            stateID: "state-1",
            milestoneID: ""
        )
        let updated = try encodedObject(update)
        let changes = try #require(updated["changes"] as? [String: Any])
        #expect(changes["title"] as? String == "Update API")
        #expect(changes["priority"] as? Int == 2)
        #expect(changes["assigneeId"] as? String == "user-1")
        #expect(changes["description"] == nil)

        let comment = try encodedObject(
            ConnectorActionInputBuilder.linearComment(body: "Reviewed")
        )
        #expect(comment["body"] as? String == "Reviewed")
    }

    @Test
    func buildsNotionPagePropertiesAndMetadata() throws {
        let input = ConnectorActionInputBuilder.notionPageUpdate(
            titleProperty: "Name",
            title: "Launch brief",
            archived: false,
            icon: "🚀",
            coverURL: "https://example.com/cover.png"
        )
        let object = try encodedObject(input)
        let changes = try #require(object["changes"] as? [String: Any])
        let properties = try #require(changes["properties"] as? [String: Any])
        let name = try #require(properties["Name"] as? [String: Any])
        let icon = try #require(changes["icon"] as? [String: Any])
        let cover = try #require(changes["cover"] as? [String: Any])

        #expect(name["title"] is [[String: Any]])
        #expect(changes["archived"] as? Bool == false)
        #expect(icon["emoji"] as? String == "🚀")
        #expect(cover["type"] as? String == "external")
    }

    @Test
    func buildsNotionDatabaseRichText() throws {
        let input = ConnectorActionInputBuilder.notionDatabaseUpdate(
            title: "Roadmap",
            description: "H2",
            archived: nil
        )
        let object = try encodedObject(input)
        let changes = try #require(object["changes"] as? [String: Any])

        #expect(changes["title"] is [[String: Any]])
        #expect(changes["description"] is [[String: Any]])
        #expect(changes["archived"] == nil)
    }

    @Test
    func computesFieldLevelDiffWithoutShowingUnchangedCurrentFields() {
        let current: JSONValue = .object([
            "summary": .string("旧标题"),
            "location": .string("会议室 A"),
            "status": .string("tentative"),
        ])
        let proposed: JSONValue = .object([
            "summary": .string("新标题"),
            "status": .string("confirmed"),
        ])

        let diffs = ConnectorActionInputDiff.changes(
            current: current,
            proposed: proposed
        )

        #expect(diffs.map(\.path) == ["status", "summary"])
        #expect(
            ConnectorActionInputDiff.display(diffs[0].current) == "tentative"
        )
        #expect(
            ConnectorActionInputDiff.display(diffs[1].proposed) == "新标题"
        )
    }

    @Test
    func flattensNestedNotionPropertyDiff() {
        let current: JSONValue = .object([
            "properties": .object([
                "Name": .object([
                    "title": .array([
                        .object([
                            "type": .string("text"),
                            "text": .object([
                                "content": .string("旧页面"),
                            ]),
                        ]),
                    ]),
                ]),
            ]),
        ])
        let proposed: JSONValue = .object([
            "properties": .object([
                "Name": .object([
                    "title": .array([
                        .object([
                            "type": .string("text"),
                            "text": .object([
                                "content": .string("新页面"),
                            ]),
                        ]),
                    ]),
                ]),
            ]),
        ])

        let diffs = ConnectorActionInputDiff.changes(
            current: current,
            proposed: proposed
        )
        let title = diffs.first {
            $0.path == "properties.Name.title[0].text.content"
        }

        #expect(diffs.count == 1)
        #expect(title?.proposed == .string("新页面"))
    }

    @Test
    func buildsInputFromManifestFieldPaths() throws {
        let input = ConnectorActionInputBuilder.dynamicInput(values: [
            "changes.summary": .string("新标题"),
            "changes.description": .string("说明"),
        ])
        let object = try encodedObject(input)
        let changes = try #require(object["changes"] as? [String: Any])

        #expect(changes["summary"] as? String == "新标题")
        #expect(changes["description"] as? String == "说明")
    }

    private func encodedObject(_ value: JSONValue) throws -> [String: Any] {
        let data = try JSONEncoder().encode(value)
        return try #require(
            JSONSerialization.jsonObject(with: data) as? [String: Any]
        )
    }
}
