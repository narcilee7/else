import Foundation

public actor DemoRuntimeClient: RuntimeClient {
    private var snapshots: [IntentSnapshot]

    public init() {
        snapshots = Self.samples
    }

    public func listIntents() async throws -> [IntentSnapshot] {
        snapshots
    }

    public func intent(id: IntentID) async throws -> IntentSnapshot {
        guard let snapshot = snapshots.first(where: { $0.id == id }) else {
            throw RuntimeClientError.http(status: 404, message: "Intent not found")
        }
        return snapshot
    }

    public func createIntent(outcome: String, project: String?) async throws -> IntentSnapshot {
        let snapshot = IntentSnapshot(
            spec: IntentSpec(
                id: IntentID(UUID().uuidString),
                revision: 1,
                outcome: outcome,
                doneWhen: [Criterion(description: "计划产物写入工作区且校验通过")],
                constraints: [],
                authority: Authority(
                    maxAutonomousEffect: .reversible,
                    forbidden: []
                ),
                attentionPolicy: AttentionPolicy(
                    allowInterrupt: true,
                    batchIntervalMinutes: 30
                ),
                assumptions: [
                    Assumption(
                        text: "假设：在当前工作区推进该目标（系统补全，可修正）",
                        confirmed: false
                    ),
                ],
                project: project
            ),
            state: .active,
            decisions: [],
            artifacts: [],
            resourceBudget: Self.defaultBudget,
            resourceUsage: .zero,
            cursor: 2
        )
        snapshots.insert(snapshot, at: 0)
        return snapshot
    }

    public func runIntent(id: IntentID) async throws -> IntentSnapshot {
        try update(id: id) { snapshot in
            snapshot.replacing(state: .active)
        }
    }

    public func resolveDecision(
        intentID: IntentID,
        decisionID: AttentionID,
        choice: DecisionChoice
    ) async throws -> IntentSnapshot {
        try update(id: intentID) { snapshot in
            let remaining = snapshot.decisions.filter { $0.id != decisionID }
            let nextState: IntentState
            switch choice {
            case .dismissed:
                nextState = .paused
            case .freeText, .option:
                nextState = remaining.isEmpty ? .completed : .needsDecision
            }
            return snapshot.replacing(
                state: nextState,
                decisions: remaining,
                artifacts: remaining.isEmpty
                    ? [
                        Artifact(
                            id: ArtifactID("demo-artifact-\(intentID.rawValue.prefix(8))"),
                            kind: .document,
                            locator: "store/result.md",
                            manifest: nil,
                            producedBy: RunID("demo-run"),
                            note: "验收结果与执行证据"
                        ),
                    ]
                    : snapshot.artifacts
            )
        }
    }

    public func abortIntent(id: IntentID, reason: String) async throws -> IntentSnapshot {
        _ = reason
        return try update(id: id) { snapshot in
            snapshot.replacing(state: .cancelled, decisions: [])
        }
    }

    private func update(
        id: IntentID,
        transform: (IntentSnapshot) -> IntentSnapshot
    ) throws -> IntentSnapshot {
        guard let index = snapshots.firstIndex(where: { $0.id == id }) else {
            throw RuntimeClientError.http(status: 404, message: "Intent not found")
        }
        snapshots[index] = transform(snapshots[index])
        return snapshots[index]
    }

    public static let samples: [IntentSnapshot] = {
        let budget = defaultBudget
        let now = Date()

        return [
            IntentSnapshot(
                spec: IntentSpec(
                    id: IntentID("9AA94E24-23DA-4D40-A417-07BBA3CA42A0"),
                    revision: 2,
                    outcome: "把 macOS 客户端的第一条 Intent 闭环跑通",
                    doneWhen: [
                        Criterion(description: "Intent 可提交并开始执行"),
                        Criterion(description: "决策结果可回写 Runtime"),
                    ],
                    constraints: ["不绕过 Runtime API", "默认不产生外部副作用"],
                    authority: Authority(
                        maxAutonomousEffect: .reversible,
                        forbidden: ["不对外发送任何内容"]
                    ),
                    attentionPolicy: AttentionPolicy(
                        allowInterrupt: true,
                        batchIntervalMinutes: 30
                    ),
                    assumptions: [
                        Assumption(text: "本机 Runtime 位于 127.0.0.1", confirmed: true),
                    ],
                    project: "else-client"
                ),
                state: .active,
                decisions: [],
                artifacts: [
                    Artifact(
                        id: ArtifactID("3CE3B47A"),
                        kind: .fileTree,
                        locator: "store/2cb48b9f",
                        manifest: "manifest.json",
                        producedBy: RunID("RUN-91A2"),
                        note: "客户端骨架与 API 契约"
                    ),
                ],
                resourceBudget: budget,
                resourceUsage: ResourceUsage(
                    modelTokens: 18_420,
                    modelSpendUSDMicros: 12_400,
                    toolCalls: 14,
                    sandboxSeconds: 286
                ),
                cursor: 48
            ),
            IntentSnapshot(
                spec: IntentSpec(
                    id: IntentID("D91E465C-09F8-49B1-8745-C45A67C00AE1"),
                    revision: 1,
                    outcome: "确认发布前安全回归清单",
                    doneWhen: [Criterion(description: "所有阻塞项都有明确结论")],
                    constraints: ["先验证危险路径"],
                    authority: Authority(
                        maxAutonomousEffect: .readOnly,
                        forbidden: ["不修改生产环境"]
                    ),
                    attentionPolicy: AttentionPolicy(
                        allowInterrupt: true,
                        batchIntervalMinutes: 30
                    ),
                    project: "else"
                ),
                state: .needsDecision,
                decisions: [
                    AttentionRequest(
                        id: AttentionID("BF1A57FE"),
                        kind: .authorization,
                        summary: "是否允许运行一次受控的恶意符号链接回归？",
                        blockedReason: "该用例需要创建临时符号链接，超出当前只读授权。",
                        expectedLoss: .high,
                        blocking: true,
                        reversible: true,
                        decideBy: now.addingTimeInterval(3_600),
                        options: [
                            DecisionOption(
                                id: "approve",
                                label: "允许本次回归",
                                recommended: true
                            ),
                            DecisionOption(
                                id: "reject",
                                label: "跳过该用例",
                                recommended: false
                            ),
                        ],
                        defaultOption: "approve",
                        relatedCallKey: "shell:symlink-escape:42"
                    ),
                ],
                artifacts: [],
                resourceBudget: budget,
                resourceUsage: ResourceUsage(
                    modelTokens: 7_260,
                    modelSpendUSDMicros: 5_100,
                    toolCalls: 8,
                    sandboxSeconds: 91
                ),
                cursor: 31
            ),
            IntentSnapshot(
                spec: IntentSpec(
                    id: IntentID("08134A17-EC4C-4D1C-9364-9BC3D4C53A0C"),
                    revision: 1,
                    outcome: "整理本周所有客户反馈",
                    doneWhen: [Criterion(description: "形成按主题归并的反馈摘要")],
                    constraints: [],
                    authority: Authority(
                        maxAutonomousEffect: .readOnly,
                        forbidden: []
                    ),
                    attentionPolicy: AttentionPolicy(
                        allowInterrupt: false,
                        batchIntervalMinutes: 60
                    ),
                    project: "research"
                ),
                state: .completed,
                decisions: [],
                artifacts: [
                    Artifact(
                        id: ArtifactID("EEA41F10"),
                        kind: .document,
                        locator: "store/feedback-report.md",
                        manifest: nil,
                        producedBy: RunID("RUN-40DF"),
                        note: "聚合后的反馈摘要"
                    ),
                ],
                resourceBudget: budget,
                resourceUsage: ResourceUsage(
                    modelTokens: 33_810,
                    modelSpendUSDMicros: 22_600,
                    toolCalls: 21,
                    sandboxSeconds: 604
                ),
                cursor: 76
            ),
        ]
    }()

    private static let defaultBudget = ResourceBudget(
        maxModelTokens: 1_000_000,
        maxModelSpendUSDMicros: 1_000_000,
        maxToolCalls: 10_000,
        maxSandboxSeconds: 86_400
    )
}

private extension ResourceUsage {
    static let zero = ResourceUsage(
        modelTokens: 0,
        modelSpendUSDMicros: 0,
        toolCalls: 0,
        sandboxSeconds: 0
    )
}

private extension IntentSnapshot {
    func replacing(
        state: IntentState? = nil,
        decisions: [AttentionRequest]? = nil,
        artifacts: [Artifact]? = nil
    ) -> IntentSnapshot {
        IntentSnapshot(
            spec: spec,
            state: state ?? self.state,
            decisions: decisions ?? self.decisions,
            artifacts: artifacts ?? self.artifacts,
            resourceBudget: resourceBudget,
            resourceUsage: resourceUsage,
            cursor: cursor + 1
        )
    }
}
