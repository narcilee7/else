import Foundation
import Testing
@testable import ElseClientCore

@Suite("Personal brief contract")
struct WorldContractTests {
    @Test
    func decodesPersonalBriefAndWorldItems() throws {
        let brief = try RuntimeJSON.decoder().decode(
            PersonalBrief.self,
            from: Data(Self.briefJSON.utf8)
        )

        #expect(brief.generatedAt.timeIntervalSince1970 > 0)
        #expect(brief.overdue.count == 1)
        #expect(brief.waitingExternal.count == 1)
        #expect(brief.upcomingCalendar.count == 1)
        #expect(brief.outstandingCount == 2)

        let overdue = try #require(brief.overdue.first)
        #expect(overdue.threadID.rawValue == "thread-1")
        #expect(overdue.item.kind == .actionItem)
        #expect(overdue.item.state == .overdue)
        #expect(overdue.item.objectKind == .workItem)
        #expect(overdue.item.priority == 95)
        #expect(overdue.item.priorityReason.contains("已逾期"))
        #expect(overdue.item.projects.first?.label == "else")
        let waiting = try #require(brief.waitingExternal.first)
        #expect(waiting.item.kind == .waitingExternal)
        #expect(waiting.item.entityRef.provider == "notion")
        #expect(waiting.item.identities.map(\.label) == ["Alice", "Acme"])

        let calendar = try #require(brief.upcomingCalendar.first)
        #expect(calendar.item.objectKind == .calendarEvent)
    }

    private static let briefJSON = """
    {
      "generated_at": "2026-10-31T06:00:00Z",
      "open_commitments": [],
      "waiting_external": [{
        "thread_id": "thread-1",
        "item": {
          "id": "world-waiting",
          "entity_ref": {
            "provider": "notion",
            "tenant": null,
            "account": null,
            "entity_kind": "notion_database_row",
            "external_id": "page-waiting",
            "canonical_url": "https://notion.so/page-waiting",
            "source_version": "1"
          },
          "object_kind": "work_item",
          "kind": "waiting_external",
          "state": "waiting_external",
          "title": "等待安全团队审批",
          "summary": "审批仍未返回",
          "deadline": "2026-11-01T09:00:00Z",
          "signal_ids": ["signal-waiting"],
          "connector_ids": ["connector-notion"],
          "projects": [{"kind":"project","key":"project:else","label":"else"}],
          "identities": [
            {"kind":"contact","key":"contact:alice","label":"Alice"},
            {"kind":"customer","key":"customer:acme","label":"Acme"}
          ],
          "first_observed_at": "2026-10-30T06:00:00Z",
          "last_observed_at": "2026-10-31T05:00:00Z"
        }
      }],
      "overdue": [{
        "thread_id": "thread-1",
        "item": {
          "id": "world-overdue",
          "entity_ref": {
            "provider": "linear",
            "tenant": null,
            "account": null,
            "entity_kind": "linear_issue",
            "external_id": "issue-1",
            "canonical_url": null,
            "source_version": "2"
          },
          "object_kind": "work_item",
          "kind": "action_item",
          "state": "overdue",
          "title": "补齐发布检查",
          "summary": "已超过截止时间",
          "deadline": "2026-10-30T09:00:00Z",
          "priority": 95,
          "priority_reason": "已逾期，需要立即收敛",
          "signal_ids": ["signal-overdue"],
          "connector_ids": ["connector-linear"],
          "projects": [{"kind":"project","key":"project:else","label":"else"}],
          "first_observed_at": "2026-10-29T06:00:00Z",
          "last_observed_at": "2026-10-31T05:00:00Z"
        }
      }],
      "action_items_due_soon": [],
      "risks": [],
      "upcoming_calendar": [{
        "thread_id": "thread-2",
        "item": {
          "id": "world-calendar",
          "entity_ref": {
            "provider": "calendar",
            "tenant": null,
            "account": null,
            "entity_kind": "calendar_event",
            "external_id": "event-1",
            "canonical_url": null,
            "source_version": "1"
          },
          "object_kind": "calendar_event",
          "kind": "action_item",
          "state": "open",
          "title": "产品评审",
          "summary": "准备最新构建",
          "deadline": "2026-10-31T07:00:00Z",
          "signal_ids": [],
          "connector_ids": [],
          "projects": [],
          "first_observed_at": "2026-10-30T06:00:00Z",
          "last_observed_at": "2026-10-31T05:00:00Z"
        }
      }]
    }
    """
}
