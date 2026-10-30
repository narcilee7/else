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
    func configurationReadsLoopbackAndDemoFlag() {
        let configuration = ClientConfiguration.fromEnvironment([
            "ELSE_RUNTIME_URL": "http://127.0.0.1:9999",
            "ELSE_DEMO": "1",
        ])

        #expect(configuration.runtimeURL.port == 9_999)
        #expect(configuration.usesDemoRuntime)
    }
}
