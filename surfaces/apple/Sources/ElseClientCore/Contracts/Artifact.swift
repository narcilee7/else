import Foundation

public enum ArtifactKind: String, Codable, Sendable {
    case fileTree = "file_tree"
    case document
    case structuredRecord = "structured_record"
    case externalHandle = "external_handle"
}

public struct Artifact: Codable, Equatable, Identifiable, Sendable {
    public let id: ArtifactID
    public let kind: ArtifactKind
    public let locator: String
    public let manifest: String?
    public let producedBy: RunID
    public let note: String

    public init(
        id: ArtifactID,
        kind: ArtifactKind,
        locator: String,
        manifest: String?,
        producedBy: RunID,
        note: String
    ) {
        self.id = id
        self.kind = kind
        self.locator = locator
        self.manifest = manifest
        self.producedBy = producedBy
        self.note = note
    }

    private enum CodingKeys: String, CodingKey {
        case id
        case kind
        case locator
        case manifest
        case producedBy = "produced_by"
        case note
    }
}

public struct OutcomeEvidence: Codable, Equatable, Sendable {
    public let criterion: String
    public let artifactID: ArtifactID?
    public let note: String

    public init(criterion: String, artifactID: ArtifactID?, note: String) {
        self.criterion = criterion
        self.artifactID = artifactID
        self.note = note
    }

    private enum CodingKeys: String, CodingKey {
        case criterion
        case artifactID = "artifact_id"
        case note
    }
}
