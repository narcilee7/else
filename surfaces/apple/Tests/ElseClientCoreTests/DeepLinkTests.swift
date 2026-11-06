import Foundation
import Testing
@testable import ElseClientCore

@Suite("Deep links")
struct DeepLinkTests {
    @Test
    func parsesSectionsAndThreadRoutes() throws {
        #expect(
            ElseDeepLink(url: URL(string: "else://today")!) == .today
        )
        #expect(
            ElseDeepLink(url: URL(string: "else://new-thread")!)
                == .newThread
        )
        #expect(
            ElseDeepLink(
                url: URL(string: "else://thread/73f66f45-d08e-414b-a2f6-24034eaf12ab")!
            )
                == .thread(
                    ThreadID("73f66f45-d08e-414b-a2f6-24034eaf12ab")
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
            ElseDeepLink(url: URL(string: "else://thread")!) == nil
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
            .newThread,
            .thread(ThreadID("thread-1")),
        ]
        for route in routes {
            let url = try #require(route.url)
            #expect(ElseDeepLink(url: url) == route)
        }
    }
}
