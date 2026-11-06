import Foundation

public struct RuntimeSettings: Codable, Equatable, Sendable {
    public let retentionDays: UInt64?

    public init(retentionDays: UInt64?) {
        self.retentionDays = retentionDays
    }

    private enum CodingKeys: String, CodingKey {
        case retentionDays = "retention_days"
    }
}

public struct LeaseCleanupReport: Codable, Equatable, Sendable {
    public let scannedThreads: Int
    public let orphanLeases: Int
    public let revokedLeases: Int
    public let sandboxFinishRecords: Int
    public let skippedActiveLeases: Int

    private enum CodingKeys: String, CodingKey {
        case scannedThreads = "scanned_threads"
        case orphanLeases = "orphan_leases"
        case revokedLeases = "revoked_leases"
        case sandboxFinishRecords = "sandbox_finish_records"
        case skippedActiveLeases = "skipped_active_leases"
    }
}
