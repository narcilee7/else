import AppIntents
import ElseClientCore
import Foundation

struct SubmitElseThread: AppIntent {
    static var title: LocalizedStringResource = "提交 else Thread"
    static var description = IntentDescription(
        "将一个新目标提交到本地 else Runtime。"
    )
    static var openAppWhenRun = false

    @Parameter(title: "目标")
    var outcome: String

    @Parameter(title: "项目")
    var project: String?

    static var parameterSummary: some ParameterSummary {
        Summary("向 else 提交 \(\.$outcome)") {
            \.$project
        }
    }

    func perform() async throws -> some IntentResult & ProvidesDialog {
        let outcome = outcome.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !outcome.isEmpty else {
            throw ElseAppIntentError.emptyOutcome
        }
        let snapshot = try await ElseThreadRuntime.shared.createThread(
            outcome: outcome,
            project: project
        )
        return .result(
            dialog:
                "已提交：\(snapshot.spec.outcome)。Thread ID：\(snapshot.id.rawValue)"
        )
    }
}

struct CheckElseBrief: AppIntent {
    static var title: LocalizedStringResource = "查看 else 今天"
    static var description = IntentDescription(
        "读取 else Personal Brief 的待处理数量。"
    )
    static var openAppWhenRun = false

    func perform() async throws -> some IntentResult & ProvidesDialog {
        let brief = try await ElseThreadRuntime.shared.personalBrief()
        return .result(
            dialog:
                "\(brief.outstandingCount) 件外部事项待处理。"
        )
    }
}

struct ElseAppShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: SubmitElseThread(),
            phrases: [
                "在 \(.applicationName) 提交目标",
                "让 \(.applicationName) 处理这个目标",
            ],
            shortTitle: "提交 Thread",
            systemImageName: "plus.circle"
        )
        AppShortcut(
            intent: CheckElseBrief(),
            phrases: [
                "查看 \(.applicationName) 今天的待处理事项",
            ],
            shortTitle: "查看今天",
            systemImageName: "sun.max"
        )
    }
}

private actor ElseThreadRuntime {
    static let shared = ElseThreadRuntime()

    private let configuration = ClientConfiguration.fromEnvironment()
    private lazy var client = HTTPRuntimeClient(
        baseURL: configuration.runtimeURL,
        tokenProvider: {
            self.configuration.currentRuntimeToken()
        }
    )

    func createThread(
        outcome: String,
        project: String?
    ) async throws -> ThreadSnapshot {
        do {
            return try await client.createThread(
                outcome: outcome,
                project: project
            )
        } catch {
            throw ElseAppIntentError.runtime(error.localizedDescription)
        }
    }

    func personalBrief() async throws -> PersonalBrief {
        do {
            return try await client.personalBrief()
        } catch {
            throw ElseAppIntentError.runtime(error.localizedDescription)
        }
    }
}

private enum ElseAppIntentError: LocalizedError {
    case emptyOutcome
    case runtime(String)

    var errorDescription: String? {
        switch self {
        case .emptyOutcome:
            "目标不能为空"
        case let .runtime(message):
            "else Runtime 操作失败：\(message)"
        }
    }
}
