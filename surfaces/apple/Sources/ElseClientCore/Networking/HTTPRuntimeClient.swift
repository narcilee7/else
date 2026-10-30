import Foundation

public struct HTTPRuntimeClient: RuntimeClient {
    private let baseURL: URL
    private let session: URLSession

    public init(baseURL: URL, session: URLSession = .shared) {
        self.baseURL = baseURL
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
            path: ["v1", "intents", id.rawValue, "run"]
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
            body: ResolveDecisionRequest(choice: choice)
        )
    }

    public func abortIntent(id: IntentID, reason: String) async throws -> IntentSnapshot {
        try await send(
            method: "POST",
            path: ["v1", "intents", id.rawValue, "abort"],
            body: AbortIntentRequest(reason: reason)
        )
    }

    private func send<Response: Decodable>(
        method: String,
        path: [String]
    ) async throws -> Response {
        try await send(method: method, path: path, body: Optional<EmptyBody>.none)
    }

    private func send<Response: Decodable, Body: Encodable>(
        method: String,
        path: [String],
        body: Body?
    ) async throws -> Response {
        let url = path.reduce(baseURL) { partial, component in
            partial.appendingPathComponent(component)
        }
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.timeoutInterval = 20

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
            return try RuntimeJSON.decoder().decode(Response.self, from: data)
        } catch {
            throw RuntimeClientError.decoding(error.localizedDescription)
        }
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
