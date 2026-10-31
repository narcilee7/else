import Foundation
import Testing
@testable import ElseClientCore

@Suite("Capability contract")
struct CapabilityContractTests {
    @Test
    func decodesConnectorActionSchemaFromCapabilityManifest() throws {
        let registration = try RuntimeJSON.decoder().decode(
            RuntimeCapabilityRegistration.self,
            from: Data(Self.externalActionJSON.utf8)
        )
        let binding = try #require(registration.externalAction)
        let schema = try #require(
            registration.manifest.inputSchema.connectorActionSchema
        )

        #expect(binding.connectorID == ConnectorID("connector-1"))
        #expect(binding.operation == "calendar.event.update")
        #expect(schema.title == "更新日历事件")
        #expect(schema.targetLabel == "Event ID")
        #expect(schema.previewSupported)
        #expect(schema.fields.count == 2)
        #expect(schema.fields[0].path == "changes.summary")
        #expect(schema.fields[1].options.count == 2)
    }

    private static let externalActionJSON = """
    {
      "manifest": {
        "id": "connector:connector-1:calendar.event.update",
        "name": "calendar.event.update",
        "description": "calendar write capability via Tool Gateway",
        "kind": "external_action",
        "effect": "unknown",
        "idempotency_required": true,
        "input_schema": {
          "type": "object",
          "title": "更新日历事件",
          "target_label": "Event ID",
          "preview_supported": true,
          "fields": [
            {"path":"changes.summary","title":"标题","kind":"text"},
            {
              "path":"changes.status",
              "title":"状态",
              "kind":"enum",
              "options":[
                {"value":"confirmed","label":"已确认"},
                {"value":"cancelled","label":"已取消"}
              ]
            }
          ]
        }
      },
      "adapter": {
        "kind": "external_action",
        "value": {
          "connector_id": "connector-1",
          "operation": "calendar.event.update"
        }
      }
    }
    """
}
