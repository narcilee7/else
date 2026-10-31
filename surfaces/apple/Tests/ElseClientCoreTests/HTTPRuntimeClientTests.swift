import Foundation
import Testing
@testable import ElseClientCore

@Suite("HTTP runtime client", .serialized)
struct HTTPRuntimeClientTests {
    @Test
    func listsIntentsFromVersionedEndpoint() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == "/v1/intents")
            return (
                HTTPURLResponse(
                    url: try #require(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data("[]".utf8)
            )
        }

        let snapshots = try await client.listIntents()
        #expect(snapshots.isEmpty)
    }

    @Test
    func createsIntentWithOptionalProject() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            #expect(request.httpMethod == "POST")
            #expect(request.url?.path == "/v1/intents")
            let body = try #require(requestBodyData(request))
            let payload = try JSONSerialization.jsonObject(with: body) as? [String: Any]
            #expect(payload?["outcome"] as? String == "整理反馈")
            #expect(payload?["project"] as? String == "research")

            return (
                HTTPURLResponse(
                    url: try #require(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: nil
                )!,
                Data(Self.minimalSnapshotJSON.utf8)
            )
        }

        let snapshot = try await client.createIntent(
            outcome: "整理反馈",
            project: "research"
        )
        #expect(snapshot.spec.outcome == "整理反馈")
    }

    @Test
    func mapsRuntimeErrorEnvelope() async {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 409,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data(#"{"error":"conflict","message":"Decision already resolved"}"#.utf8)
            )
        }

        do {
            _ = try await client.listIntents()
            Issue.record("Expected HTTP error")
        } catch let RuntimeClientError.http(status, message) {
            #expect(status == 409)
            #expect(message == "Decision already resolved")
        } catch {
            Issue.record("Unexpected error: \(error)")
        }
    }

    @Test
    func loadsPersonalBriefFromWorldEndpoint() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == "/v1/world/brief")
            return (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data(Self.emptyBriefJSON.utf8)
            )
        }

        let brief = try await client.personalBrief()
        #expect(brief.outstandingCount == 0)
    }

    @Test
    func listsIntentConnectorActions() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == "/v1/intents/intent-1/actions")
            return (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data("[]".utf8)
            )
        }

        let actions = try await client.connectorActions(
            intentID: IntentID("intent-1")
        )
        #expect(actions.isEmpty)
    }

    @Test
    func loadsAttentionWindows() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == "/v1/attention")
            return (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data(Self.attentionWindowJSON.utf8)
            )
        }

        let windows = try await client.attentionWindows()
        #expect(windows.count == 1)
        #expect(windows.first?.window.kind == .batch)
    }

    @Test
    func loadsProjectAndIdentityWorlds() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(
                request.url?.path == "/v1/projects"
                    || request.url?.path == "/v1/identities"
            )
            return (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data("[]".utf8)
            )
        }

        let projects = try await client.projectWorlds(includeResolved: false)
        let identities = try await client.identityWorlds(includeResolved: false)
        #expect(projects.isEmpty)
        #expect(identities.isEmpty)
    }

    @Test
    func loadsConnectorAdapters() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == "/v1/connector-adapters")
            return (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data("[]".utf8)
            )
        }

        let adapters = try await client.connectorAdapters()
        #expect(adapters.isEmpty)
    }

    @Test
    func proposesConnectorActionWithJSONInput() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            #expect(request.httpMethod == "POST")
            #expect(request.url?.path == "/v1/intents/intent-1/actions")
            let body = try #require(requestBodyData(request))
            let payload = try JSONSerialization.jsonObject(with: body) as? [String: Any]
            #expect(payload?["connector_id"] as? String == "connector-1")
            #expect(payload?["capability"] as? String == "calendar.event.update")
            return (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data(Self.actionJSON.utf8)
            )
        }

        let action = try await client.proposeConnectorAction(
            intentID: IntentID("intent-1"),
            request: ProposeConnectorActionRequest(
                connectorID: ConnectorID("connector-1"),
                capability: "calendar.event.update",
                target: "event-1",
                input: .object(["changes": .object(["summary": .string("new")])])
            )
        )
        #expect(action.target == "event-1")
    }

    private func makeSession() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [StubURLProtocol.self]
        return URLSession(configuration: configuration)
    }

    private static let minimalSnapshotJSON = """
    {
      "spec": {
        "id": "C1DA6F1D-1CE1-44C6-AC0C-14A78CFB6E8D",
        "revision": 1,
        "outcome": "整理反馈",
        "done_when": [],
        "constraints": [],
        "authority": {
          "max_autonomous_effect": "read_only",
          "forbidden": []
        },
        "attention_policy": {
          "allow_interrupt": true,
          "batch_interval_minutes": 30
        },
        "deadline": null,
        "assumptions": [],
        "project": "research"
      },
      "state": "active",
      "decisions": [],
      "artifacts": [],
      "resource_budget": {
        "max_model_tokens": 1000000,
        "max_model_spend_usd_micros": 1000000,
        "max_tool_calls": 10000,
        "max_sandbox_seconds": 86400
      },
      "resource_usage": {
        "model_tokens": 0,
        "model_spend_usd_micros": 0,
        "tool_calls": 0,
        "sandbox_seconds": 0
      },
      "cursor": 2
    }
    """

    private static let emptyBriefJSON = """
    {
      "generated_at": "2026-10-31T06:00:00Z",
      "open_commitments": [],
      "waiting_external": [],
      "overdue": [],
      "action_items_due_soon": [],
      "risks": [],
      "upcoming_calendar": []
    }
    """

    private static let attentionWindowJSON = """
    [{
      "intent_id": "intent-1",
      "window": {
        "id": "window-1",
        "kind": "batch",
        "opened_at": "2026-10-31T05:00:00Z",
        "due_at": "2026-10-31T06:00:00Z",
        "timer_id": "timer-1",
        "closed_at": "2026-10-31T06:00:00Z",
        "delivered_at": "2026-10-31T06:00:00Z",
        "request_ids": ["attention-1"]
      },
      "requests": [{
        "id": "attention-1",
        "kind": "authorization",
        "summary": "允许执行？",
        "blocked_reason": "需要授权",
        "expected_loss": "high",
        "blocking": true,
        "reversible": true,
        "decide_by": null,
        "options": [],
        "default_option": null,
        "related_call_key": null
      }]
    }]
    """

    private static let actionJSON = """
    {
      "id": "action-1",
      "connector_id": "connector-1",
      "capability": "calendar.event.update",
      "target": "event-1",
      "input": {"changes":{"summary":"new"}},
      "effect": "unknown",
      "idempotency_key": "action-1",
      "status": "proposed",
      "proposed_at": "2026-10-31T06:00:00Z",
      "attention_id": null,
      "tool_call_id": null,
      "outcome": null,
      "last_error": null
    }
    """
}

private func requestBodyData(_ request: URLRequest) -> Data? {
    if let body = request.httpBody {
        return body
    }
    guard let stream = request.httpBodyStream else {
        return nil
    }

    stream.open()
    defer { stream.close() }

    var data = Data()
    var buffer = [UInt8](repeating: 0, count: 4_096)
    while true {
        let count = stream.read(&buffer, maxLength: buffer.count)
        if count <= 0 {
            break
        }
        data.append(buffer, count: count)
    }
    return data
}

private final class StubURLProtocol: URLProtocol {
    static var handler: ((URLRequest) throws -> (HTTPURLResponse, Data))?

    override class func canInit(with request: URLRequest) -> Bool {
        true
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        guard let handler = Self.handler else {
            client?.urlProtocol(
                self,
                didFailWithError: RuntimeClientError.invalidResponse
            )
            return
        }

        do {
            let (response, data) = try handler(request)
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
}
