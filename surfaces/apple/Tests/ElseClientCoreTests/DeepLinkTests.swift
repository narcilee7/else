import Foundation
import Testing
@testable import ElseClientCore

@Suite("Deep links")
struct DeepLinkTests {
    @Test
    func parsesSectionsAndIntentRoutes() throws {
        #expect(
            ElseDeepLink(url: URL(string: "else://today")!) == .today
        )
        #expect(
            ElseDeepLink(url: URL(string: "else://new-intent")!)
                == .newIntent
        )
        #expect(
            ElseDeepLink(
                url: URL(string: "else://intent/73f66f45-d08e-414b-a2f6-24034eaf12ab")!
            )
                == .intent(
                    IntentID("73f66f45-d08e-414b-a2f6-24034eaf12ab")
                )
        )
    }

    @Test
    func rejectsUnknownSchemesAndRoutes() {
        #expect(
            ElseDeepLink(url: URL(string: "https://today")!) == nil
        )
        #expect(
            ElseDeepLink(url: URL(string: "else://unknown")!) == nil
        )
        #expect(
            ElseDeepLink(url: URL(string: "else://intent")!) == nil
        )
    }

    @Test
    func routesRoundTripThroughURLs() throws {
        let routes: [ElseDeepLink] = [
            .today,
            .world,
            .connectors,
            .decisions,
            .results,
            .newIntent,
            .intent(IntentID("intent-1")),
        ]
        for route in routes {
            let url = try #require(route.url)
            #expect(ElseDeepLink(url: url) == route)
        }
    }
}
