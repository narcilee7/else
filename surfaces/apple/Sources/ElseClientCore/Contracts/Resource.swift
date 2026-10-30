import Foundation

public struct ResourceBudget: Codable, Equatable, Sendable {
    public let maxModelTokens: UInt64
    public let maxModelSpendUSDMicros: UInt64
    public let maxToolCalls: UInt64
    public let maxSandboxSeconds: UInt64

    public init(
        maxModelTokens: UInt64,
        maxModelSpendUSDMicros: UInt64,
        maxToolCalls: UInt64,
        maxSandboxSeconds: UInt64
    ) {
        self.maxModelTokens = maxModelTokens
        self.maxModelSpendUSDMicros = maxModelSpendUSDMicros
        self.maxToolCalls = maxToolCalls
        self.maxSandboxSeconds = maxSandboxSeconds
    }

    private enum CodingKeys: String, CodingKey {
        case maxModelTokens = "max_model_tokens"
        case maxModelSpendUSDMicros = "max_model_spend_usd_micros"
        case maxToolCalls = "max_tool_calls"
        case maxSandboxSeconds = "max_sandbox_seconds"
    }
}

public struct ResourceUsage: Codable, Equatable, Sendable {
    public let modelTokens: UInt64
    public let modelSpendUSDMicros: UInt64
    public let toolCalls: UInt64
    public let sandboxSeconds: UInt64

    public init(
        modelTokens: UInt64,
        modelSpendUSDMicros: UInt64,
        toolCalls: UInt64,
        sandboxSeconds: UInt64
    ) {
        self.modelTokens = modelTokens
        self.modelSpendUSDMicros = modelSpendUSDMicros
        self.toolCalls = toolCalls
        self.sandboxSeconds = sandboxSeconds
    }

    private enum CodingKeys: String, CodingKey {
        case modelTokens = "model_tokens"
        case modelSpendUSDMicros = "model_spend_usd_micros"
        case toolCalls = "tool_calls"
        case sandboxSeconds = "sandbox_seconds"
    }
}
