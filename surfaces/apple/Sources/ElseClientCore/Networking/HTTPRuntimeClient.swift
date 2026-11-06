import Foundation

public struct HTTPRuntimeClient: RuntimeClient {
    private let baseURL: URL
    private let token: String?
    private let tokenProvider: (@Sendable () -> String?)?
    private let session: URLSession
    private let revisions = RevisionStore()

    public init(
        baseURL: URL,
        token: String? = nil,
        tokenProvider: (@Sendable () -> String?)? = nil,
        session: URLSession = .shared
    ) {
        self.baseURL = baseURL
        self.token = token
        self.tokenProvider = tokenProvider
        self.session = session
    }

    public func listThreads() async throws -> [ThreadSnapshot] {
        try await send(method: "GET", path: ["v1", "threads"])
    }

    public func thread(id: ThreadID) async throws -> ThreadSnapshot {
        try await send(method: "GET", path: ["v1", "threads", id.rawValue])
    }

    public func deleteThread(id: ThreadID) async throws -> ThreadDeletionReport {
        try await send(
            method: "DELETE",
            path: ["v1", "threads", id.rawValue],
            threadID: id
        )
    }

    public func retryFailedThread(id: ThreadID) async throws -> ThreadSnapshot {
        let response: ScopeRetryResponse = try await send(
            method: "POST",
            path: ["v1", "threads", id.rawValue, "repair", "retry"],
            threadID: id
        )
        return response.thread
    }

    public func createThread(outcome: String, project: String?) async throws -> ThreadSnapshot {
        try await send(
            method: "POST",
            path: ["v1", "threads"],
            body: CreateThreadRequest(
                outcome: outcome,
                project: project?.trimmingCharacters(in: .whitespacesAndNewlines)
                    .nilIfEmpty
            )
        )
    }

    public func runThread(id: ThreadID) async throws -> ThreadSnapshot {
        try await send(
            method: "POST",
            path: ["v1", "threads", id.rawValue, "run"],
            threadID: id
        )
    }

    public func resolveDecision(
        threadID: ThreadID,
        decisionID: AttentionID,
        choice: DecisionChoice
    ) async throws -> ThreadSnapshot {
        try await send(
            method: "POST",
            path: [
                "v1",
                "threads",
                threadID.rawValue,
                "decisions",
                decisionID.rawValue,
                "resolve",
            ],
            body: ResolveDecisionRequest(choice: choice),
            threadID: threadID
        )
    }

    public func abortThread(id: ThreadID, reason: String) async throws -> ThreadSnapshot {
        try await send(
            method: "POST",
            path: ["v1", "threads", id.rawValue, "abort"],
            body: AbortThreadRequest(reason: reason),
            threadID: id
        )
    }

    public func personalBrief() async throws -> PersonalBrief {
        try await send(method: "GET", path: ["v1", "world", "brief"])
    }

    public func resolveWorldItem(
        threadID: ThreadID,
        itemID: WorldItemID,
        kind: WorldItemResolutionKind
    ) async throws -> WorldItem {
        try await send(
            method: "POST",
            path: ["v1", "threads", threadID.rawValue, "world", "resolve"],
            body: ResolveWorldItemRequest(itemID: itemID, kind: kind),
            threadID: threadID
        )
    }

    public func connectorActions(threadID: ThreadID) async throws -> [ConnectorAction] {
        try await send(
            method: "GET",
            path: ["v1", "threads", threadID.rawValue, "actions"]
        )
    }

    public func executeConnectorAction(
        threadID: ThreadID,
        actionID: ConnectorActionID
    ) async throws -> ConnectorAction {
        try await actionCommand("execute", threadID: threadID, actionID: actionID)
    }

    public func verifyConnectorAction(
        threadID: ThreadID,
        actionID: ConnectorActionID
    ) async throws -> ConnectorAction {
        try await actionCommand("verify", threadID: threadID, actionID: actionID)
    }

    public func compensateConnectorAction(
        threadID: ThreadID,
        actionID: ConnectorActionID
    ) async throws -> ConnectorAction {
        try await actionCommand("compensate", threadID: threadID, actionID: actionID)
    }

    public func proposeConnectorAction(
        threadID: ThreadID,
        request: ProposeConnectorActionRequest
    ) async throws -> ConnectorAction {
        try await send(
            method: "POST",
            path: ["v1", "threads", threadID.rawValue, "actions"],
            body: request,
            threadID: threadID
        )
    }

    public func capabilities(
        threadID: ThreadID
    ) async throws -> [RuntimeCapabilityRegistration] {
        try await send(
            method: "GET",
            path: ["v1", "threads", threadID.rawValue, "capabilities"]
        )
    }

    public func exportRuntimeData() async throws -> JSONValue {
        try await send(method: "GET", path: ["v1", "export"])
    }

    public func diagnostics() async throws -> JSONValue {
        try await send(method: "GET", path: ["v1", "diagnostics"])
    }

    public func cleanupOrphanLeases() async throws -> LeaseCleanupReport {
        try await send(
            method: "POST",
            path: ["v1", "diagnostics", "leases", "cleanup"]
        )
    }

    public func runtimeSettings() async throws -> RuntimeSettings {
        try await send(method: "GET", path: ["v1", "settings"])
    }

    public func updateRuntimeSettings(
        _ settings: RuntimeSettings
    ) async throws -> RuntimeSettings {
        try await send(
            method: "POST",
            path: ["v1", "settings"],
            body: settings
        )
    }

    public func previewConnectorAction(
        threadID: ThreadID,
        connectorID: ConnectorID,
        request: PreviewConnectorActionRequest
    ) async throws -> ConnectorActionPreview {
        try await send(
            method: "POST",
            path: [
                "v1",
                "threads",
                threadID.rawValue,
                "connectors",
                connectorID.rawValue,
                "actions",
                "preview",
            ],
            body: request,
            threadID: threadID
        )
    }

    private func actionCommand(
        _ command: String,
        threadID: ThreadID,
        actionID: ConnectorActionID
    ) async throws -> ConnectorAction {
        try await send(
            method: "POST",
            path: [
                "v1",
                "threads",
                threadID.rawValue,
                "actions",
                actionID.rawValue,
                command,
            ],
            threadID: threadID
        )
    }

    public func attentionWindows() async throws -> [AttentionWindowRecord] {
        try await send(method: "GET", path: ["v1", "attention"])
    }

    public func projectWorlds(includeResolved: Bool) async throws -> [ProjectWorld] {
        try await send(
            method: "GET",
            path: ["v1", "projects"],
            queryItems: [
                URLQueryItem(
                    name: "include_resolved",
                    value: includeResolved ? "true" : "false"
                ),
            ]
        )
    }

    public func identityWorlds(includeResolved: Bool) async throws -> [IdentityWorld] {
        try await send(
            method: "GET",
            path: ["v1", "identities"],
            queryItems: [
                URLQueryItem(
                    name: "include_resolved",
                    value: includeResolved ? "true" : "false"
                ),
            ]
        )
    }

    public func correctWorldCorrelation(
        threadID: ThreadID,
        correction: WorldCorrelationCorrection
    ) async throws -> WorldCorrelationCorrection {
        try await send(
            method: "POST",
            path: ["v1", "threads", threadID.rawValue, "world", "corrections"],
            body: correction,
            threadID: threadID
        )
    }

    public func connectorAdapters() async throws -> [ConnectorManifest] {
        try await send(method: "GET", path: ["v1", "connector-adapters"])
    }

    public func connectors(threadID: ThreadID) async throws -> [ConnectorRegistration] {
        try await send(
            method: "GET",
            path: ["v1", "threads", threadID.rawValue, "connectors"]
        )
    }

    public func connectorHealth(
        threadID: ThreadID,
        connectorID: ConnectorID
    ) async throws -> ConnectorHealthReport {
        try await send(
            method: "GET",
            path: [
                "v1",
                "threads",
                threadID.rawValue,
                "connectors",
                connectorID.rawValue,
                "health",
            ]
        )
    }

    public func attachConnectorCredential(
        threadID: ThreadID,
        connectorID: ConnectorID,
        credential: ConnectorCredentialInput
    ) async throws -> ConnectorCredentialMetadata {
        try await send(
            method: "POST",
            path: [
                "v1",
                "threads",
                threadID.rawValue,
                "connectors",
                connectorID.rawValue,
                "credential",
            ],
            body: credential,
            threadID: threadID
        )
    }

    public func revokeConnectorCredential(
        threadID: ThreadID,
        connectorID: ConnectorID
    ) async throws -> ConnectorCredentialMetadata {
        try await send(
            method: "DELETE",
            path: [
                "v1",
                "threads",
                threadID.rawValue,
                "connectors",
                connectorID.rawValue,
                "credential",
            ],
            threadID: threadID
        )
    }

    public func registerConnector(
        threadID: ThreadID,
        request: RegisterConnectorRequest
    ) async throws -> ConnectorRegistration {
        try await send(
            method: "POST",
            path: ["v1", "threads", threadID.rawValue, "connectors"],
            body: request,
            threadID: threadID
        )
    }

    public func syncConnector(
        threadID: ThreadID,
        connectorID: ConnectorID
    ) async throws -> ConnectorSyncResult {
        try await send(
            method: "POST",
            path: [
                "v1",
                "threads",
                threadID.rawValue,
                "connectors",
                connectorID.rawValue,
                "sync",
            ],
            threadID: threadID
        )
    }

    public func repairConnector(
        threadID: ThreadID,
        connectorID: ConnectorID
    ) async throws -> ConnectorRepairReport {
        try await send(
            method: "POST",
            path: [
                "v1",
                "threads",
                threadID.rawValue,
                "connectors",
                connectorID.rawValue,
                "repair",
            ],
            threadID: threadID
        )
    }

    private func send<Response: Decodable>(
        method: String,
        path: [String],
        threadID: ThreadID? = nil,
        queryItems: [URLQueryItem] = []
    ) async throws -> Response {
        try await send(
            method: method,
            path: path,
            body: Optional<EmptyBody>.none,
            threadID: threadID,
            queryItems: queryItems
        )
    }

    private func send<Response: Decodable, Body: Encodable>(
        method: String,
        path: [String],
        body: Body?,
        threadID: ThreadID? = nil,
        queryItems: [URLQueryItem] = []
    ) async throws -> Response {
        let base = path.reduce(baseURL) { partial, component in
            partial.appendingPathComponent(component)
        }
        var components = URLComponents(url: base, resolvingAgainstBaseURL: false)
        components?.queryItems = queryItems.isEmpty ? nil : queryItems
        guard let url = components?.url else {
            throw RuntimeClientError.invalidResponse
        }
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.timeoutInterval = 20
        let resolvedToken = tokenProvider.map { $0() } ?? token
        if let resolvedToken, !resolvedToken.isEmpty {
            request.setValue(
                "Bearer \(resolvedToken)",
                forHTTPHeaderField: "Authorization"
            )
        }
        if method == "POST" || method == "DELETE" {
            request.setValue(UUID().uuidString, forHTTPHeaderField: "Idempotency-Key")
        }
        if let threadID, let revision = await revisions.revision(for: threadID) {
            request.setValue("\(revision)", forHTTPHeaderField: "If-Match")
        }

        if let body {
            request.httpBody = try RuntimeJSON.encoder().encode(body)
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }

        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await session.data(for: request)
        } catch {
            throw RuntimeClientError.transport(error.localizedDescription)
        }

        guard let httpResponse = response as? HTTPURLResponse else {
            throw RuntimeClientError.invalidResponse
        }
        guard (200..<300).contains(httpResponse.statusCode) else {
            let message = try? RuntimeJSON.decoder().decode(
                RuntimeErrorEnvelope.self,
                from: data
            )
            throw RuntimeClientError.http(
                status: httpResponse.statusCode,
                message: message?.message ?? message?.error
            )
        }

        do {
            let decoded = try RuntimeJSON.decoder().decode(Response.self, from: data)
            if let snapshot = decoded as? ThreadSnapshot {
                await revisions.record(snapshot.spec.revision, for: snapshot.spec.id)
            }
            return decoded
        } catch {
            throw RuntimeClientError.decoding(error.localizedDescription)
        }
    }
}

private actor RevisionStore {
    private var revisions: [ThreadID: UInt64] = [:]

    func revision(for id: ThreadID) -> UInt64? {
        revisions[id]
    }

    func record(_ revision: UInt64, for id: ThreadID) {
        revisions[id] = revision
    }
}

private struct EmptyBody: Encodable {}

private struct RuntimeErrorEnvelope: Decodable {
    let error: String?
    let message: String?
}

private struct ScopeRetryResponse: Decodable {
    let retriedScopes: [String]
    let thread: ThreadSnapshot

    private enum CodingKeys: String, CodingKey {
        case retriedScopes = "retried_scopes"
        case thread
    }
}

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }
}
