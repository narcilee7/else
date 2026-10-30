import Foundation
import Testing
@testable import ElseClientCore

@Suite("Runtime wire contract")
struct RuntimeContractTests {
    @Test
    func decodesIntentSnapshotUsingRuntimeWireFormat() throws {
        let data = Data(Self.snapshotJSON.utf8)
        let snapshot = try RuntimeJSON.decoder().decode(IntentSnapshot.self, from: data)

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

    private static let snapshotJSON = """
    {
      "spec": {
        "id": "C1DA6F1D-1CE1-44C6-AC0C-14A78CFB6E8D",
        "revision": 2,
        "outcome": "完成客户端基础设施",
        "done_when": [{"description": "预算内完成"}],
        "constraints": ["不绕过 API"],
        "authority": {
          "max_autonomous_effect": "reversible",
          "forbidden": ["不对外发送"]
        },
        "attention_policy": {
          "allow_interrupt": true,
          "batch_interval_minutes": 30
        },
        "deadline": "2026-10-30T12:00:00.123Z",
        "assumptions": [],
        "project": "else"
      },
      "state": "needs_decision",
      "decisions": [{
        "id": "DFD1A78E",
        "kind": "authorization",
        "summary": "允许执行？",
        "blocked_reason": "需要一次性授权",
        "expected_loss": "high",
        "blocking": true,
        "reversible": true,
        "decide_by": "2026-10-30T13:00:00Z",
        "options": [
          {"id": "approve", "label": "允许", "recommended": true}
        ],
        "default_option": "approve",
        "related_call_key": "shell:42"
      }],
      "artifacts": [],
      "resource_budget": {
        "max_model_tokens": 1000000,
        "max_model_spend_usd_micros": 1000000,
        "max_tool_calls": 10000,
        "max_sandbox_seconds": 86400
      },
      "resource_usage": {
        "model_tokens": 1234,
        "model_spend_usd_micros": 500,
        "tool_calls": 4,
        "sandbox_seconds": 32
      },
      "cursor": 17
    }
    """
}
