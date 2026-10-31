import Foundation

public struct HTTPRuntimeClient: RuntimeClient {
    private let baseURL: URL
    private let token: String?
    private let session: URLSession
    private let revisions = RevisionStore()

    public init(
        baseURL: URL,
        token: String? = nil,
        session: URLSession = .shared
    ) {
        self.baseURL = baseURL
        self.token = token
        self.session = session
    }

    public func listIntents() async throws -> [IntentSnapshot] {
        try await send(method: "GET", path: ["v1", "intents"])
    }

    public func intent(id: IntentID) async throws -> IntentSnapshot {
        try await send(method: "GET", path: ["v1", "intents", id.rawValue])
    }

    public func createIntent(outcome: String, project: String?) async throws -> IntentSnapshot {
        try await send(
            method: "POST",
            path: ["v1", "intents"],
            body: CreateIntentRequest(
                outcome: outcome,
                project: project?.trimmingCharacters(in: .whitespacesAndNewlines)
                    .nilIfEmpty
            )
        )
    }

    public func runIntent(id: IntentID) async throws -> IntentSnapshot {
        try await send(
            method: "POST",
            path: ["v1", "intents", id.rawValue, "run"],
            intentID: id
        )
    }

    public func resolveDecision(
        intentID: IntentID,
        decisionID: AttentionID,
        choice: DecisionChoice
    ) async throws -> IntentSnapshot {
        try await send(
            method: "POST",
            path: [
                "v1",
                "intents",
                intentID.rawValue,
                "decisions",
                decisionID.rawValue,
                "resolve",
            ],
            body: ResolveDecisionRequest(choice: choice),
            intentID: intentID
        )
    }

    public func abortIntent(id: IntentID, reason: String) async throws -> IntentSnapshot {
        try await send(
            method: "POST",
            path: ["v1", "intents", id.rawValue, "abort"],
            body: AbortIntentRequest(reason: reason),
            intentID: id
        )
    }

    public func personalBrief() async throws -> PersonalBrief {
        try await send(method: "GET", path: ["v1", "world", "brief"])
    }

    public func resolveWorldItem(
        intentID: IntentID,
        itemID: WorldItemID,
        kind: WorldItemResolutionKind
    ) async throws -> WorldItem {
        try await send(
            method: "POST",
            path: ["v1", "intents", intentID.rawValue, "world", "resolve"],
            body: ResolveWorldItemRequest(itemID: itemID, kind: kind),
            intentID: intentID
        )
    }

    public func connectorActions(intentID: IntentID) async throws -> [ConnectorAction] {
        try await send(
            method: "GET",
            path: ["v1", "intents", intentID.rawValue, "actions"]
        )
    }

    public func executeConnectorAction(
        intentID: IntentID,
        actionID: ConnectorActionID
    ) async throws -> ConnectorAction {
        try await actionCommand("execute", intentID: intentID, actionID: actionID)
    }

    public func verifyConnectorAction(
        intentID: IntentID,
        actionID: ConnectorActionID
    ) async throws -> ConnectorAction {
        try await actionCommand("verify", intentID: intentID, actionID: actionID)
    }

    public func compensateConnectorAction(
        intentID: IntentID,
        actionID: ConnectorActionID
    ) async throws -> ConnectorAction {
        try await actionCommand("compensate", intentID: intentID, actionID: actionID)
    }

    public func proposeConnectorAction(
        intentID: IntentID,
        request: ProposeConnectorActionRequest
    ) async throws -> ConnectorAction {
        try await send(
            method: "POST",
            path: ["v1", "intents", intentID.rawValue, "actions"],
            body: request,
            intentID: intentID
        )
    }

    private func actionCommand(
        _ command: String,
        intentID: IntentID,
        actionID: ConnectorActionID
    ) async throws -> ConnectorAction {
        try await send(
            method: "POST",
            path: [
                "v1",
                "intents",
                intentID.rawValue,
                "actions",
                actionID.rawValue,
                command,
            ],
            intentID: intentID
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
        intentID: IntentID,
        correction: WorldCorrelationCorrection
    ) async throws -> WorldCorrelationCorrection {
        try await send(
            method: "POST",
            path: ["v1", "intents", intentID.rawValue, "world", "corrections"],
            body: correction,
            intentID: intentID
        )
    }

    public func connectorAdapters() async throws -> [ConnectorManifest] {
        try await send(method: "GET", path: ["v1", "connector-adapters"])
    }

    public func connectors(intentID: IntentID) async throws -> [ConnectorRegistration] {
        try await send(
            method: "GET",
            path: ["v1", "intents", intentID.rawValue, "connectors"]
        )
    }

    public func connectorHealth(
        intentID: IntentID,
        connectorID: ConnectorID
    ) async throws -> ConnectorHealthReport {
        try await send(
            method: "GET",
            path: [
                "v1",
                "intents",
                intentID.rawValue,
                "connectors",
                connectorID.rawValue,
                "health",
            ]
        )
    }

    public func registerConnector(
        intentID: IntentID,
        request: RegisterConnectorRequest
    ) async throws -> ConnectorRegistration {
        try await send(
            method: "POST",
            path: ["v1", "intents", intentID.rawValue, "connectors"],
            body: request,
            intentID: intentID
        )
    }

    public func syncConnector(
        intentID: IntentID,
        connectorID: ConnectorID
    ) async throws -> ConnectorSyncResult {
        try await send(
            method: "POST",
            path: [
                "v1",
                "intents",
                intentID.rawValue,
                "connectors",
                connectorID.rawValue,
                "sync",
            ],
            intentID: intentID
        )
    }

    private func send<Response: Decodable>(
        method: String,
        path: [String],
        intentID: IntentID? = nil,
        queryItems: [URLQueryItem] = []
    ) async throws -> Response {
        try await send(
            method: method,
            path: path,
            body: Optional<EmptyBody>.none,
            intentID: intentID,
            queryItems: queryItems
        )
    }

    private func send<Response: Decodable, Body: Encodable>(
        method: String,
        path: [String],
        body: Body?,
        intentID: IntentID? = nil,
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
        if let token, !token.isEmpty {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        if method == "POST" {
            request.setValue(UUID().uuidString, forHTTPHeaderField: "Idempotency-Key")
        }
        if let intentID, let revision = await revisions.revision(for: intentID) {
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
            if let snapshot = decoded as? IntentSnapshot {
                await revisions.record(snapshot.spec.revision, for: snapshot.spec.id)
            }
            return decoded
        } catch {
            throw RuntimeClientError.decoding(error.localizedDescription)
        }
    }
}

private actor RevisionStore {
    private var revisions: [IntentID: UInt64] = [:]

    func revision(for id: IntentID) -> UInt64? {
        revisions[id]
    }

    func record(_ revision: UInt64, for id: IntentID) {
        revisions[id] = revision
    }
}

private struct EmptyBody: Encodable {}

private struct RuntimeErrorEnvelope: Decodable {
    let error: String?
    let message: String?
}

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }
}
