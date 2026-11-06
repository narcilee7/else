import AppKit
import ElseClientCore
import SwiftUI
import UserNotifications

@main
struct ElseMacApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var store: AppStore
    @StateObject private var runtimeService: RuntimeServiceManager

    init() {
        let configuration = AppDependencies.configuration()
        let runtimeService = RuntimeServiceManager(configuration: configuration)
        _runtimeService = StateObject(wrappedValue: runtimeService)
        _store = StateObject(
            wrappedValue: AppStore(
                client: AppDependencies.client(for: configuration),
                configuration: configuration,
                runtimeService: runtimeService
            )
        )
    }

    var body: some Scene {
        Window("else", id: "main") {
            AppShellView()
                .environmentObject(store)
                .frame(minWidth: 960, minHeight: 620)
        }
        .handlesExternalEvents(matching: ["*"])
        .defaultSize(width: 1_180, height: 760)
        .commands {
            CommandGroup(after: .newItem) {
                Button("新建 Thread") {
                    store.isCreatingThread = true
                }
                .keyboardShortcut("n", modifiers: [.command])

                Button("快速新建 Thread") {
                    store.isCreatingThread = true
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

        MenuBarExtra {
            MenuBarStatusView()
                .environmentObject(store)
        } label: {
            Image(
                systemName: store.decisionCount > 0
                    ? "hand.raised.fill"
                    : "sun.max"
            )
            .accessibilityLabel(
                store.decisionCount > 0
                    ? "\(store.decisionCount) 个待决策"
                    : "else"
            )
        }
        .menuBarExtraStyle(.menu)

        Settings {
            SettingsView()
                .environmentObject(store)
                .environmentObject(runtimeService)
        }
    }
}

private final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NotificationCoordinator.shared.configure()
        NSApplication.shared.servicesProvider = ElseServiceProvider.shared
        NSUpdateDynamicServices()
        NSApplication.shared.setActivationPolicy(.regular)
        NSApplication.shared.activate(ignoringOtherApps: true)
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls {
            guard let route = ElseDeepLink(url: url) else {
                continue
            }
            Foundation.NotificationCenter.default.post(
                name: .elseOpenRoute,
                object: route
            )
        }
        NSApplication.shared.activate(ignoringOtherApps: true)
    }
}

@MainActor
private final class ElseServiceProvider: NSObject {
    static let shared = ElseServiceProvider()

    @objc func submitSelectedText(
        _ pasteboard: NSPasteboard,
        userData: String,
        error: AutoreleasingUnsafeMutablePointer<NSString?>
    ) {
        guard let text = pasteboard.string(forType: .string),
              !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            error.pointee = "没有可提交的文本"
            return
        }
        Foundation.NotificationCenter.default.post(
            name: .elseQuickThread,
            object: text
        )
    }
}
