import Foundation

public enum ConnectorAuthMode: String, Codable, Sendable {
    case none
    case apiToken = "api_token"
    case oauth
}

public enum ConnectorSyncMode: String, Codable, Sendable {
    case pull
    case webhook
    case hybrid
}

public enum ConnectorHealth: String, Codable, Sendable {
    case healthy
    case degraded
    case error
}

public struct ConnectorManifest: Codable, Equatable, Identifiable, Sendable {
    public let adapter: String
    public let provider: String
    public let authMode: ConnectorAuthMode
    public let scopes: [String]
    public let readCapabilities: [String]
    public let writeCapabilities: [String]
    public let syncModes: [ConnectorSyncMode]
    public let dataSensitivity: String
    public let retentionPolicy: String
    public let health: ConnectorHealth

    public var id: String { adapter }

    public init(
        adapter: String,
        provider: String,
        authMode: ConnectorAuthMode,
        scopes: [String],
        readCapabilities: [String],
        writeCapabilities: [String],
        syncModes: [ConnectorSyncMode],
        dataSensitivity: String,
        retentionPolicy: String,
        health: ConnectorHealth
    ) {
        self.adapter = adapter
        self.provider = provider
        self.authMode = authMode
        self.scopes = scopes
        self.readCapabilities = readCapabilities
        self.writeCapabilities = writeCapabilities
        self.syncModes = syncModes
        self.dataSensitivity = dataSensitivity
        self.retentionPolicy = retentionPolicy
        self.health = health
    }

    private enum CodingKeys: String, CodingKey {
        case adapter
        case provider
        case authMode = "auth_mode"
        case scopes
        case readCapabilities = "read_capabilities"
        case writeCapabilities = "write_capabilities"
        case syncModes = "sync_modes"
        case dataSensitivity = "data_sensitivity"
        case retentionPolicy = "retention_policy"
        case health
    }
}

public struct ConnectorCredentialMetadata: Codable, Equatable, Sendable {
    public let reference: String
    public let authMode: ConnectorAuthMode
    public let scopes: [String]
    public let expiresAt: Date?
    public let configuredAt: Date
    public let revokedAt: Date?

    public init(
        reference: String,
        authMode: ConnectorAuthMode,
        scopes: [String],
        expiresAt: Date?,
        configuredAt: Date,
        revokedAt: Date?
    ) {
        self.reference = reference
        self.authMode = authMode
        self.scopes = scopes
        self.expiresAt = expiresAt
        self.configuredAt = configuredAt
        self.revokedAt = revokedAt
    }

    private enum CodingKeys: String, CodingKey {
        case reference
        case authMode = "auth_mode"
        case scopes
        case expiresAt = "expires_at"
        case configuredAt = "configured_at"
        case revokedAt = "revoked_at"
    }
}

public struct ConnectorRegistration: Codable, Equatable, Identifiable, Sendable {
    public let id: ConnectorID
    public let name: String
    public let adapter: String
    public let source: String
    public let enabled: Bool
    public let manifest: ConnectorManifest
    public let syncIntervalSeconds: UInt64
    public let registeredAt: Date?
    public let credential: ConnectorCredentialMetadata?

    public init(
        id: ConnectorID,
        name: String,
        adapter: String,
        source: String,
        enabled: Bool,
        manifest: ConnectorManifest,
        syncIntervalSeconds: UInt64,
        registeredAt: Date?,
        credential: ConnectorCredentialMetadata?
    ) {
        self.id = id
        self.name = name
        self.adapter = adapter
        self.source = source
        self.enabled = enabled
        self.manifest = manifest
        self.syncIntervalSeconds = syncIntervalSeconds
        self.registeredAt = registeredAt
        self.credential = credential
    }

    private enum CodingKeys: String, CodingKey {
        case id
        case name
        case adapter
        case source
        case enabled
        case manifest
        case syncIntervalSeconds = "sync_interval_seconds"
        case registeredAt = "registered_at"
        case credential
    }
}

public struct OAuthRefreshConfigInput: Codable, Equatable, Sendable {
    public let tokenURL: String
    public let clientID: String
    public let clientSecret: String?

    public init(tokenURL: String, clientID: String, clientSecret: String? = nil) {
        self.tokenURL = tokenURL
        self.clientID = clientID
        self.clientSecret = clientSecret
    }

    private enum CodingKeys: String, CodingKey {
        case tokenURL = "token_url"
        case clientID = "client_id"
        case clientSecret = "client_secret"
    }
}

public struct ConnectorCredentialInput: Codable, Equatable, Sendable {
    public let accessToken: String
    public let refreshToken: String?
    public let tokenType: String
    public let scopes: [String]
    public let expiresAt: Date?
    public let refresh: OAuthRefreshConfigInput?

    public init(
        accessToken: String,
        refreshToken: String? = nil,
        tokenType: String = "bearer",
        scopes: [String],
        expiresAt: Date? = nil,
        refresh: OAuthRefreshConfigInput? = nil
    ) {
        self.accessToken = accessToken
        self.refreshToken = refreshToken
        self.tokenType = tokenType
        self.scopes = scopes
        self.expiresAt = expiresAt
        self.refresh = refresh
    }

    private enum CodingKeys: String, CodingKey {
        case accessToken = "access_token"
        case refreshToken = "refresh_token"
        case tokenType = "token_type"
        case scopes
        case expiresAt = "expires_at"
        case refresh
    }
}

public struct RegisterConnectorRequest: Codable, Equatable, Sendable {
    public let name: String
    public let source: String
    public let adapter: String
    public let credential: ConnectorCredentialInput?
    public let syncIntervalSeconds: UInt64

    public init(
        name: String,
        source: String,
        adapter: String,
        credential: ConnectorCredentialInput?,
        syncIntervalSeconds: UInt64 = 300
    ) {
        self.name = name
        self.source = source
        self.adapter = adapter
        self.credential = credential
        self.syncIntervalSeconds = syncIntervalSeconds
    }

    private enum CodingKeys: String, CodingKey {
        case name
        case source
        case adapter
        case credential
        case syncIntervalSeconds = "sync_interval_seconds"
    }
}

public struct ConnectorHealthReport: Codable, Equatable, Sendable {
    public let connectorID: ConnectorID
    public let provider: String
    public let health: ConnectorHealth
    public let credentialConfigured: Bool
    public let credentialScopes: [String]
    public let lastAttemptAt: Date?
    public let lastSuccessAt: Date?
    public let consecutiveFailures: UInt32
    public let lastError: String?
    public let nextSyncAt: Date
    public let backfillInProgress: Bool
    public let pagesSynced: UInt64

    public init(
        connectorID: ConnectorID,
        provider: String,
        health: ConnectorHealth,
        credentialConfigured: Bool,
        credentialScopes: [String],
        lastAttemptAt: Date?,
        lastSuccessAt: Date?,
        consecutiveFailures: UInt32,
        lastError: String?,
        nextSyncAt: Date,
        backfillInProgress: Bool,
        pagesSynced: UInt64
    ) {
        self.connectorID = connectorID
        self.provider = provider
        self.health = health
        self.credentialConfigured = credentialConfigured
        self.credentialScopes = credentialScopes
        self.lastAttemptAt = lastAttemptAt
        self.lastSuccessAt = lastSuccessAt
        self.consecutiveFailures = consecutiveFailures
        self.lastError = lastError
        self.nextSyncAt = nextSyncAt
        self.backfillInProgress = backfillInProgress
        self.pagesSynced = pagesSynced
    }

    private enum CodingKeys: String, CodingKey {
        case connectorID = "connector_id"
        case provider
        case health
        case credentialConfigured = "credential_configured"
        case credentialScopes = "credential_scopes"
        case lastAttemptAt = "last_attempt_at"
        case lastSuccessAt = "last_success_at"
        case consecutiveFailures = "consecutive_failures"
        case lastError = "last_error"
        case nextSyncAt = "next_sync_at"
        case backfillInProgress = "backfill_in_progress"
        case pagesSynced = "pages_synced"
    }
}

public struct ConnectorRepairReport: Codable, Equatable, Sendable {
    public let connectorID: ConnectorID
    public let resetCursors: Int
    public let sync: ConnectorSyncResult?
    public let error: String?

    private enum CodingKeys: String, CodingKey {
        case connectorID = "connector_id"
        case resetCursors = "reset_cursors"
        case sync
        case error
    }
}
