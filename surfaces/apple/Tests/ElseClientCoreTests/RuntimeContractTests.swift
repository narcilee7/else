import Foundation
import Testing
@testable import ElseClientCore

@Suite("Runtime wire contract")
struct RuntimeContractTests {
    @Test
    func decodesThreadSnapshotUsingRuntimeWireFormat() throws {
        let data = try Self.sharedContractFixture()
        let snapshot = try RuntimeJSON.decoder().decode(ThreadSnapshot.self, from: data)

        #expect(snapshot.id.rawValue == "C1DA6F1D-1CE1-44C6-AC0C-14A78CFB6E8D")
        #expect(snapshot.spec.revision == 2)
        #expect(snapshot.spec.doneWhen.first?.description == "预算内完成")
        #expect(snapshot.spec.attentionPolicy.batchIntervalMinutes == 30)
        #expect(snapshot.state == .needsDecision)
        #expect(snapshot.decisions.first?.blockedReason == "需要一次性授权")
        #expect(snapshot.resourceBudget.maxModelTokens == 1_000_000)
        #expect(snapshot.resourceUsage.toolCalls == 4)
        #expect(snapshot.cursor == 17)
    }

    /// fixture 与 Rust 契约测试共用：`core/else-protocol/tests/contract.rs` 读同一份 JSON。
    /// Rust 侧改了线上形状，这里会先失败，而不是等到运行时解不出来。
    @Test
    func decodesSharedContractFixture() throws {
        let url = try #require(
            Bundle.module.url(
                forResource: "thread_snapshot",
                withExtension: "json",
                subdirectory: "Fixtures"
            ),
            "共享契约 fixture 缺失"
        )
        let data = try Data(contentsOf: url)
        let snapshot = try RuntimeJSON.decoder().decode(ThreadSnapshot.self, from: data)

        #expect(snapshot.spec.outcome == "完成客户端基础设施")
        #expect(snapshot.spec.constraints == ["不绕过 API"])
        #expect(snapshot.spec.assumptions.first?.text == "依赖可用")
        #expect(snapshot.decisions.first?.defaultOption == "approve")
        #expect(snapshot.decisions.first?.relatedCallKey == "shell:42")
        #expect(snapshot.spec.authority.forbidden == ["不对外发送"])
    }

    @Test
    func encodesDecisionChoiceUsingProtocolTaggedShape() throws {
        let data = try RuntimeJSON.encoder().encode(
            ResolveDecisionRequest(choice: .option("approve"))
        )
        let object = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let choice = try #require(object["choice"] as? [String: Any])

        #expect(choice["kind"] as? String == "option")
        #expect(choice["value"] as? String == "approve")
    }

    @Test
    func decodesVerificationAttentionKind() throws {
        let value = try RuntimeJSON.decoder().decode(
            AttentionKind.self,
            from: Data(#""verification""#.utf8)
        )
        #expect(value == .verification)
    }

    @Test
    func decodesDatesWithAndWithoutFractionalSeconds() throws {
        let decoder = RuntimeJSON.decoder()
        let withFraction = try decoder.decode(
            Date.self,
            from: Data(#""2026-10-30T12:34:56.789Z""#.utf8)
        )
        let withoutFraction = try decoder.decode(
            Date.self,
            from: Data(#""2026-10-30T12:34:56Z""#.utf8)
        )

        #expect(abs(withFraction.timeIntervalSince(withoutFraction) - 0.789) < 0.001)
    }

    @Test
    func decodesConnectorActionStateAndOutcome() throws {
        let action = try RuntimeJSON.decoder().decode(
            ConnectorAction.self,
            from: Data(Self.actionJSON.utf8)
        )
        #expect(action.status == .unknown)
        #expect(action.effect == .unknown)
        #expect(action.target == "event-1")
        #expect(action.outcome?.status == .unknown)
        #expect(action.outcome?.effect == .unknown)
    }

    private static func sharedContractFixture() throws -> Data {
        let url = try #require(
            Bundle.module.url(
                forResource: "thread_snapshot",
                withExtension: "json",
                subdirectory: "Fixtures"
            ),
            "共享契约 fixture 缺失"
        )
        return try Data(contentsOf: url)
    }

    private static let actionJSON = """
    {
      "id": "action-1",
      "connector_id": "connector-1",
      "capability": "calendar.event.update",
      "target": "event-1",
      "input": {"changes":{"summary":"new"}},
      "effect": "unknown",
      "idempotency_key": "calendar-event-1",
      "status": "unknown",
      "proposed_at": "2026-10-31T06:00:00Z",
      "attention_id": "attention-1",
      "tool_call_id": "tool-1",
      "outcome": {
        "status": "unknown",
        "output": {"before":{"summary":"old"}},
        "effect": "unknown"
      },
      "last_error": null
    }
    """
}
