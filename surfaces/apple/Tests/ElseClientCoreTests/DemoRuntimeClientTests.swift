import Foundation
import Testing
@testable import ElseClientCore

@Suite("Demo runtime")
struct DemoRuntimeClientTests {
    @Test
    func supportsThePrimaryDecisionLoop() async throws {
        let client = DemoRuntimeClient()
        let initial = try await client.listIntents()
        let waiting = try #require(
            initial.first { $0.state == .needsDecision }
        )
        let decision = try #require(waiting.decisions.first)

        let updated = try await client.resolveDecision(
            intentID: waiting.id,
            decisionID: decision.id,
            choice: .option("approve")
        )

        #expect(updated.decisions.isEmpty)
        #expect(updated.state == .completed)
        #expect(!updated.artifacts.isEmpty)
    }

    @Test
    func exposesAndResolvesDailyBriefItems() async throws {
        let client = DemoRuntimeClient()
        let brief = try await client.personalBrief()
        let record = try #require(brief.overdue.first)

        #expect(brief.outstandingCount == 2)
        let resolved = try await client.resolveWorldItem(
            intentID: record.intentID,
            itemID: record.item.id,
            kind: .completed
        )
        #expect(resolved.state == .completed)

        let updated = try await client.personalBrief()
        #expect(updated.overdue.isEmpty)
        #expect(updated.outstandingCount == 1)
    }

    @Test
    func exposesAndCompensatesCalendarAction() async throws {
        let client = DemoRuntimeClient()
        let intent = try #require(
            try await client.listIntents().first { $0.state == .active }
        )
        let action = try #require(
            try await client.connectorActions(intentID: intent.id).first
        )
        #expect(action.status == .applied)
        #expect(action.capability == "calendar.event.update")

        let compensated = try await client.compensateConnectorAction(
            intentID: intent.id,
            actionID: action.id
        )
        #expect(compensated.status == .compensated)
        #expect(compensated.outcome?.effect == .compensated)
    }

    @Test
    func exposesAttentionBatchForDecision() async throws {
        let client = DemoRuntimeClient()
        let windows = try await client.attentionWindows()
        let window = try #require(windows.first)

        #expect(window.window.kind == .batch)
        #expect(!window.requests.isEmpty)
        #expect(window.window.deliveredAt != nil)
    }

    @Test
    func configurationReadsLoopbackAndDemoFlag() {
        let configuration = ClientConfiguration.fromEnvironment([
            "ELSE_RUNTIME_URL": "http://127.0.0.1:9999",
            "ELSE_DEMO": "1",
        ])

        #expect(configuration.runtimeURL.port == 9_999)
        #expect(configuration.usesDemoRuntime)
    }
}
