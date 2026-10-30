import AppKit
import ElseClientCore
import SwiftUI

@main
struct ElseMacApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var store: AppStore

    init() {
        let configuration = AppDependencies.configuration()
        _store = StateObject(
            wrappedValue: AppStore(
                client: AppDependencies.client(for: configuration),
                configuration: configuration
            )
        )
    }

    var body: some Scene {
        Window("else", id: "main") {
            AppShellView()
                .environmentObject(store)
                .frame(minWidth: 960, minHeight: 620)
        }
        .defaultSize(width: 1_180, height: 760)
        .commands {
            CommandGroup(after: .newItem) {
                Button("新建 Intent") {
                    store.isCreatingIntent = true
                }
                .keyboardShortcut("n", modifiers: [.command])

                Button("快速新建 Intent") {
                    store.isCreatingIntent = true
                }
                .keyboardShortcut("k", modifiers: [.command])
            }
            CommandMenu("Runtime") {
                Button("刷新") {
                    Task { await store.refresh() }
                }
                .keyboardShortcut("r", modifiers: [.command])
            }
        }
    }
}

private final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApplication.shared.setActivationPolicy(.regular)
        NSApplication.shared.activate(ignoringOtherApps: true)
    }
}
