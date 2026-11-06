import Foundation
import Testing
@testable import ElseClientCore

@Suite("HTTP runtime client", .serialized)
struct HTTPRuntimeClientTests {
    @Test
    func listsThreadsFromVersionedEndpoint() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == "/v1/threads")
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

        let snapshots = try await client.listThreads()
        #expect(snapshots.isEmpty)
    }

    @Test
    func readsBearerTokenForEachRequest() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            tokenProvider: { "rotated-token" },
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            #expect(
                request.value(forHTTPHeaderField: "Authorization")
                    == "Bearer rotated-token"
            )
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

        _ = try await client.listThreads()
    }

    @Test
    func createsThreadWithOptionalProject() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            #expect(request.httpMethod == "POST")
            #expect(request.url?.path == "/v1/threads")
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

        let snapshot = try await client.createThread(
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
            _ = try await client.listThreads()
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
    func listsThreadConnectorActions() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == "/v1/threads/thread-1/actions")
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
            threadID: ThreadID("thread-1")
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
            #expect(request.url?.path == "/v1/threads/thread-1/actions")
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
            threadID: ThreadID("thread-1"),
            request: ProposeConnectorActionRequest(
                connectorID: ConnectorID("connector-1"),
                capability: "calendar.event.update",
                target: "event-1",
                input: .object(["changes": .object(["summary": .string("new")])])
            )
        )
        #expect(action.target == "event-1")
    }

    @Test
    func previewsConnectorActionAgainstCurrentProviderState() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }
        let threadID = ThreadID("thread-1")
        let connectorID = ConnectorID("connector-1")

        StubURLProtocol.handler = { request in
            #expect(request.httpMethod == "POST")
            #expect(
                request.url?.path
                    == "/v1/threads/thread-1/connectors/connector-1/actions/preview"
            )
            let data = try #require(requestBodyData(request))
            let body = try JSONSerialization.jsonObject(with: data) as? [String: Any]
            #expect(body?["capability"] as? String == "calendar.event.update")
            #expect(body?["target"] as? String == "event-1")
            return (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data(Self.actionPreviewJSON.utf8)
            )
        }

        let preview = try await client.previewConnectorAction(
            threadID: threadID,
            connectorID: connectorID,
            request: PreviewConnectorActionRequest(
                capability: "calendar.event.update",
                target: "event-1",
                input: .object([
                    "changes": .object([
                        "summary": .string("新标题"),
                    ]),
                ])
            )
        )

        #expect(preview.connectorID == connectorID)
        #expect(preview.current.objectValue?["summary"] == .string("旧标题"))
        #expect(preview.proposed.objectValue?["summary"] == .string("新标题"))
    }

    @Test
    func loadsCapabilityManifestSchema() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }
        let threadID = ThreadID("thread-1")

        StubURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == "/v1/threads/thread-1/capabilities")
            return (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data(Self.capabilityJSON.utf8)
            )
        }

        let capabilities = try await client.capabilities(threadID: threadID)
        let capability = try #require(capabilities.first)
        #expect(
            capability.manifest.inputSchema.connectorActionSchema?.targetLabel
                == "Event ID"
        )
    }

    @Test
    func loadsRuntimeDataExport() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == "/v1/export")
            return (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data(
                    #"{"export_version":1,"protocol_version":"0.1.0","exported_at":"2026-10-31T08:00:00Z","threads":[]}"#
                        .utf8
                )
            )
        }

        let export = try await client.exportRuntimeData()
        #expect(export.objectValue?["export_version"] == .number(1))
        #expect(export.objectValue?["protocol_version"] == .string("0.1.0"))
    }

    @Test
    func deletesThreadWithIdempotencyAndRevision() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }
        let threadID = ThreadID("C1DA6F1D-1CE1-44C6-AC0C-14A78CFB6E8D")

        StubURLProtocol.handler = { request in
            if request.httpMethod == "GET" {
                return (
                    HTTPURLResponse(
                        url: request.url!,
                        statusCode: 200,
                        httpVersion: nil,
                        headerFields: ["Content-Type": "application/json"]
                    )!,
                    Data(Self.minimalSnapshotJSON.utf8)
                )
            }
            #expect(request.httpMethod == "DELETE")
            #expect(
                request.value(forHTTPHeaderField: "Idempotency-Key") != nil
            )
            #expect(request.value(forHTTPHeaderField: "If-Match") == "1")
            return (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data(Self.deletionReportJSON.utf8)
            )
        }

        _ = try await client.thread(id: threadID)
        let report = try await client.deleteThread(id: threadID)
        #expect(report.threadID == threadID)
        #expect(report.deletedEvents == 12)
        #expect(report.removedCredentials == 1)
        #expect(report.cleanupErrors.isEmpty)
    }

    @Test
    func readsAndUpdatesRuntimeRetentionSettings() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            #expect(request.url?.path == "/v1/settings")
            let value: String
            if request.httpMethod == "POST" {
                #expect(
                    request.value(forHTTPHeaderField: "Idempotency-Key") != nil
                )
                let data = try #require(requestBodyData(request))
                let body = try JSONSerialization.jsonObject(with: data)
                    as? [String: Any]
                #expect((body?["retention_days"] as? NSNumber)?.uint64Value == 7)
                value = #"{"retention_days":7}"#
            } else {
                value = #"{"retention_days":30}"#
            }
            return (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data(value.utf8)
            )
        }

        let settings = try await client.runtimeSettings()
        #expect(settings.retentionDays == 30)
        let updated = try await client.updateRuntimeSettings(
            RuntimeSettings(retentionDays: 7)
        )
        #expect(updated.retentionDays == 7)
    }

    @Test
    func loadsDiagnosticReport() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == "/v1/diagnostics")
            return (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data(
                    #"{"healthy":true,"database_integrity":"ok","event_count":10,"connectors":[]}"#.utf8
                )
            )
        }

        let diagnostics = try await client.diagnostics()
        #expect(diagnostics.objectValue?["healthy"] == .bool(true))
        #expect(diagnostics.objectValue?["database_integrity"] == .string("ok"))
    }

    @Test
    func repairsConnectorCursorAndSyncs() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }
        let threadID = ThreadID("C1DA6F1D-1CE1-44C6-AC0C-14A78CFB6E8D")
        let connectorID = ConnectorID("connector-1")

        StubURLProtocol.handler = { request in
            if request.httpMethod == "GET" {
                return (
                    HTTPURLResponse(
                        url: request.url!,
                        statusCode: 200,
                        httpVersion: nil,
                        headerFields: ["Content-Type": "application/json"]
                    )!,
                    Data(Self.minimalSnapshotJSON.utf8)
                )
            }
            #expect(request.httpMethod == "POST")
            #expect(
                request.url?.path
                    == "/v1/threads/C1DA6F1D-1CE1-44C6-AC0C-14A78CFB6E8D/connectors/connector-1/repair"
            )
            #expect(
                request.value(forHTTPHeaderField: "Idempotency-Key") != nil
            )
            #expect(request.value(forHTTPHeaderField: "If-Match") == "1")
            return (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data(Self.repairReportJSON.utf8)
            )
        }

        _ = try await client.thread(id: threadID)
        let report = try await client.repairConnector(
            threadID: threadID,
            connectorID: connectorID
        )
        #expect(report.connectorID == connectorID)
        #expect(report.resetCursors == 2)
        #expect(report.sync?.observed == 1)
        #expect(report.error == nil)
    }

    @Test
    func replacesAndRevokesConnectorCredential() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }
        let threadID = ThreadID("C1DA6F1D-1CE1-44C6-AC0C-14A78CFB6E8D")
        let connectorID = ConnectorID("connector-1")

        StubURLProtocol.handler = { request in
            if request.httpMethod == "GET" {
                return (
                    HTTPURLResponse(
                        url: request.url!,
                        statusCode: 200,
                        httpVersion: nil,
                        headerFields: ["Content-Type": "application/json"]
                    )!,
                    Data(Self.minimalSnapshotJSON.utf8)
                )
            }
            #expect(
                request.url?.path
                    == "/v1/threads/C1DA6F1D-1CE1-44C6-AC0C-14A78CFB6E8D/connectors/connector-1/credential"
            )
            #expect(
                request.value(forHTTPHeaderField: "Idempotency-Key") != nil
            )
            #expect(request.value(forHTTPHeaderField: "If-Match") == "1")
            let revoked = request.httpMethod == "DELETE"
            return (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data(Self.credentialMetadataJSON(revoked: revoked).utf8)
            )
        }

        _ = try await client.thread(id: threadID)
        let attached = try await client.attachConnectorCredential(
            threadID: threadID,
            connectorID: connectorID,
            credential: ConnectorCredentialInput(
                accessToken: "new-token",
                scopes: ["issues:read"]
            )
        )
        #expect(attached.revokedAt == nil)
        let revoked = try await client.revokeConnectorCredential(
            threadID: threadID,
            connectorID: connectorID
        )
        #expect(revoked.revokedAt != nil)
    }

    @Test
    func cleansOrphanLeases() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }

        StubURLProtocol.handler = { request in
            #expect(request.httpMethod == "POST")
            #expect(request.url?.path == "/v1/diagnostics/leases/cleanup")
            #expect(
                request.value(forHTTPHeaderField: "Idempotency-Key") != nil
            )
            return (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data(
                    #"{"scanned_threads":2,"orphan_leases":1,"revoked_leases":1,"sandbox_finish_records":1,"skipped_active_leases":1}"#
                        .utf8
                )
            )
        }

        let report = try await client.cleanupOrphanLeases()
        #expect(report.orphanLeases == 1)
        #expect(report.revokedLeases == 1)
        #expect(report.skippedActiveLeases == 1)
    }

    @Test
    func retriesFailedThreadScopes() async throws {
        let session = makeSession()
        let client = HTTPRuntimeClient(
            baseURL: URL(string: "http://127.0.0.1:4180")!,
            session: session
        )
        defer { StubURLProtocol.handler = nil }
        let threadID = ThreadID("C1DA6F1D-1CE1-44C6-AC0C-14A78CFB6E8D")

        StubURLProtocol.handler = { request in
            if request.httpMethod == "GET" {
                return (
                    HTTPURLResponse(
                        url: request.url!,
                        statusCode: 200,
                        httpVersion: nil,
                        headerFields: ["Content-Type": "application/json"]
                    )!,
                    Data(Self.minimalSnapshotJSON.utf8)
                )
            }
            #expect(request.httpMethod == "POST")
            #expect(
                request.url?.path
                    == "/v1/threads/C1DA6F1D-1CE1-44C6-AC0C-14A78CFB6E8D/repair/retry"
            )
            #expect(request.value(forHTTPHeaderField: "If-Match") == "1")
            return (
                HTTPURLResponse(
                    url: request.url!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!,
                Data(Self.retryResponseJSON.utf8)
            )
        }

        _ = try await client.thread(id: threadID)
        let snapshot = try await client.retryFailedThread(id: threadID)
        #expect(snapshot.id == threadID)
        #expect(snapshot.state == .active)
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

    private static let deletionReportJSON = """
    {
      "thread_id": "C1DA6F1D-1CE1-44C6-AC0C-14A78CFB6E8D",
      "deleted_events": 12,
      "removed_credentials": 1,
      "removed_workspace_paths": 2,
      "cleanup_errors": [],
      "retained_shared_artifacts": true
    }
    """

    private static let repairReportJSON = """
    {
      "connector_id": "connector-1",
      "reset_cursors": 2,
      "sync": {
        "observed": 1,
        "inserted": 1,
        "skipped": 0,
        "candidates_proposed": 0,
        "auto_accepted": 0,
        "cursor": "cursor-1",
        "pages_synced": 1,
        "has_more": false,
        "backfill_complete": true,
        "hydrated": false
      },
      "error": null
    }
    """

    private static let retryResponseJSON = """
    {
      "retried_scopes": ["scope-1"],
      "thread": \(minimalSnapshotJSON)
    }
    """

    private static func credentialMetadataJSON(revoked: Bool) -> String {
        """
        {
          "reference": "credential-1",
          "auth_mode": "api_token",
          "scopes": ["issues:read"],
          "expires_at": null,
          "configured_at": "2026-10-31T08:00:00Z",
          "revoked_at": \(revoked ? #""2026-10-31T09:00:00Z""# : "null")
        }
        """
    }

    private static let attentionWindowJSON = """
    [{
      "thread_id": "thread-1",
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

    private static let actionPreviewJSON = """
    {
      "connector_id": "connector-1",
      "capability": "calendar.event.update",
      "target": "event-1",
      "current": {"summary": "旧标题", "location": "会议室 A"},
      "proposed": {"summary": "新标题"},
      "checked_at": "2026-10-31T08:00:00Z"
    }
    """

    private static let capabilityJSON = """
    [{
      "manifest": {
        "id": "connector:connector-1:calendar.event.update",
        "name": "calendar.event.update",
        "description": "calendar write capability via Tool Gateway",
        "kind": "external_action",
        "effect": "unknown",
        "idempotency_required": true,
        "input_schema": {
          "title": "更新日历事件",
          "target_label": "Event ID",
          "preview_supported": true,
          "fields": [{"path":"changes.summary","title":"标题","kind":"text"}]
        }
      },
      "adapter": {
        "kind": "external_action",
        "value": {
          "connector_id": "connector-1",
          "operation": "calendar.event.update"
        }
      }
    }]
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
