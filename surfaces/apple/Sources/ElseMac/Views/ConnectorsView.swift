import ElseClientCore
import SwiftUI

struct ConnectorsView: View {
    @EnvironmentObject private var store: AppStore
    @State private var selectedIntentID: IntentID?
    @State private var isRegistering = false

    private var availableIntents: [IntentSnapshot] {
        store.today.isEmpty ? store.snapshots : store.today
    }

    private var connectors: [ConnectorRegistration] {
        guard let selectedIntentID else { return [] }
        return store.connectorsByIntent[selectedIntentID] ?? []
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()

            if availableIntents.isEmpty {
                QuietEmptyState(
                    title: "先创建一个 Intent",
                    description: "连接器属于一个 Intent 授权范围，创建目标后再接入外部来源。"
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if connectors.isEmpty {
                QuietEmptyState(
                    title: "还没有连接器",
                    description: "从已注册的 Adapter 接入日历、Linear 或 Notion。"
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(connectors) { connector in
                            ConnectorRow(
                                intentID: selectedIntentID,
                                connector: connector,
                                health: store.connectorHealth[connector.id]
                            )
                        }
                    }
                }
                .background(ElseColor.surface)
            }
        }
        .navigationTitle("连接")
        .onAppear(perform: selectDefaultIntent)
        .onChange(of: availableIntents.map(\.id)) {
            selectDefaultIntent()
        }
        .sheet(isPresented: $isRegistering) {
            if let selectedIntentID {
                RegisterConnectorSheet(intentID: selectedIntentID)
                    .environmentObject(store)
            }
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: ElseSpacing.x3) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text("连接")
                        .font(ElseType.pageTitle)
                        .foregroundStyle(ElseColor.textPrimary)
                    Text("\(connectors.count) 个连接器 · \(store.connectorManifests.count) 个 Adapter")
                        .font(ElseType.caption)
                        .foregroundStyle(ElseColor.textSecondary)
                }
                Spacer()
                Button {
                    isRegistering = true
                } label: {
                    Image(systemName: "plus")
                }
                .buttonStyle(.borderless)
                .disabled(selectedIntentID == nil)
                .help("接入外部来源")
                Button {
                    Task { await store.refresh() }
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .buttonStyle(.borderless)
                .foregroundStyle(ElseColor.textSecondary)
                .disabled(store.isRefreshing)
                .help("刷新")
            }

            if availableIntents.count > 1 {
                Picker("Intent", selection: $selectedIntentID) {
                    ForEach(availableIntents) { snapshot in
                        Text(snapshot.spec.outcome)
                            .lineLimit(1)
                            .tag(Optional(snapshot.id))
                    }
                }
                .labelsHidden()
                .controlSize(.small)
            }
        }
        .padding(.horizontal, ElseSpacing.x4)
        .padding(.vertical, ElseSpacing.x3)
    }

    private func selectDefaultIntent() {
        guard selectedIntentID == nil || !availableIntents.contains(where: {
            $0.id == selectedIntentID
        }) else {
            return
        }
        selectedIntentID = store.selectedIntentID.flatMap { selected in
            availableIntents.contains(where: { $0.id == selected }) ? selected : nil
        } ?? availableIntents.first?.id
    }
}

private struct ConnectorRow: View {
    @EnvironmentObject private var store: AppStore
    let intentID: IntentID?
    let connector: ConnectorRegistration
    let health: ConnectorHealthReport?

    var body: some View {
        HStack(alignment: .top, spacing: ElseSpacing.x3) {
            Rectangle()
                .fill(health?.health.tint ?? ElseColor.textTertiary)
                .frame(width: 2)
                .padding(.vertical, 2)

            VStack(alignment: .leading, spacing: 5) {
                HStack {
                    Text(connector.name)
                        .font(ElseType.callout)
                        .foregroundStyle(ElseColor.textPrimary)
                    Text(connector.adapter)
                        .font(ElseType.mono)
                        .foregroundStyle(ElseColor.textTertiary)
                }
                Text(connector.source)
                    .font(ElseType.micro)
                    .foregroundStyle(ElseColor.textSecondary)
                    .lineLimit(1)
                    .textSelection(.enabled)

                HStack(spacing: ElseSpacing.x3) {
                    Label(health?.health.label ?? "未知", systemImage: "circle.fill")
                        .foregroundStyle(health?.health.tint ?? ElseColor.textTertiary)
                    if let health {
                        Text("\(health.pagesSynced) 页")
                        Text(
                            health.lastSuccessAt?.formatted(
                                date: .omitted,
                                time: .shortened
                            ) ?? "从未同步"
                        )
                    }
                    if connector.credential != nil {
                        Label("凭据", systemImage: "key")
                    }
                }
                .font(ElseType.micro)
                .foregroundStyle(ElseColor.textSecondary)

                if let error = health?.lastError {
                    Text(error)
                        .font(ElseType.micro)
                        .foregroundStyle(ElseColor.danger)
                        .lineLimit(2)
                }
            }

            Spacer()

            if let intentID {
                Button {
                    Task {
                        await store.syncConnector(
                            intentID: intentID,
                            connectorID: connector.id
                        )
                    }
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .buttonStyle(.borderless)
                .foregroundStyle(ElseColor.textSecondary)
                .help("立即同步")
            }
        }
        .padding(.horizontal, ElseSpacing.x4)
        .padding(.vertical, ElseSpacing.x3)
        .overlay(alignment: .bottom) {
            Rectangle()
                .fill(ElseColor.lineSubtle)
                .frame(height: 1)
        }
    }
}

private struct RegisterConnectorSheet: View {
    @EnvironmentObject private var store: AppStore
    @Environment(\.dismiss) private var dismiss
    let intentID: IntentID

    @State private var adapter = ""
    @State private var name = ""
    @State private var source = ""
    @State private var accessToken = ""
    @State private var refreshToken = ""
    @State private var tokenURL = ""
    @State private var clientID = ""
    @State private var clientSecret = ""
    @State private var syncInterval = 300.0
    @State private var isSaving = false

    private var manifest: ConnectorManifest? {
        store.connectorManifests.first { $0.adapter == adapter }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: ElseSpacing.x4) {
            Text("接入外部来源")
                .font(ElseType.pageTitle)
                .foregroundStyle(ElseColor.textPrimary)

            Form {
                Picker("Adapter", selection: $adapter) {
                    ForEach(store.connectorManifests) { manifest in
                        Text("\(manifest.provider) · \(manifest.adapter)")
                            .tag(manifest.adapter)
                    }
                }

                TextField("名称", text: $name)
                TextField("Source 或 HTTPS URL", text: $source)

                if let manifest, manifest.authMode != .none {
                    SecureField("Access Token", text: $accessToken)
                    SecureField("Refresh Token（可选）", text: $refreshToken)
                    TextField("OAuth Token URL（可选）", text: $tokenURL)
                    TextField("OAuth Client ID（可选）", text: $clientID)
                    SecureField("OAuth Client Secret（可选）", text: $clientSecret)
                }

                Picker("同步间隔", selection: $syncInterval) {
                    Text("1 分钟").tag(60.0)
                    Text("5 分钟").tag(300.0)
                    Text("15 分钟").tag(900.0)
                    Text("1 小时").tag(3_600.0)
                }
            }
            .formStyle(.grouped)

            HStack {
                Text("凭据只发送到本地 Runtime，并写入 Keychain 或私有凭据区。")
                    .font(ElseType.micro)
                    .foregroundStyle(ElseColor.textTertiary)
                Spacer()
                Button("取消") {
                    dismiss()
                }
                Button(isSaving ? "接入中" : "连接并同步") {
                    Task { await save() }
                }
                .buttonStyle(.borderedProminent)
                .tint(ElseColor.action)
                .disabled(!isValid || isSaving)
            }
        }
        .padding(ElseSpacing.x6)
        .frame(width: 560, height: 520)
        .background(ElseColor.canvas)
        .onAppear {
            guard adapter.isEmpty else { return }
            adapter = store.connectorManifests.first?.adapter ?? ""
            name = store.connectorManifests.first?.provider ?? "connector"
        }
    }

    private var isValid: Bool {
        guard let manifest else { return false }
        if source.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return false
        }
        if manifest.authMode != .none
            && accessToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        {
            return false
        }
        return true
    }

    private func save() async {
        guard let manifest else { return }
        isSaving = true
        defer { isSaving = false }
        let hasRefreshConfig = !tokenURL.isEmpty && !clientID.isEmpty
        let credential: ConnectorCredentialInput? =
            manifest.authMode == .none
            ? nil
            : ConnectorCredentialInput(
                accessToken: accessToken,
                refreshToken: refreshToken.nilIfEmpty,
                scopes: manifest.scopes,
                refresh: hasRefreshConfig
                    ? OAuthRefreshConfigInput(
                        tokenURL: tokenURL,
                        clientID: clientID,
                        clientSecret: clientSecret.nilIfEmpty
                    )
                    : nil
            )
        let result = await store.registerConnector(
            intentID: intentID,
            request: RegisterConnectorRequest(
                name: name.trimmingCharacters(in: .whitespacesAndNewlines),
                source: source.trimmingCharacters(in: .whitespacesAndNewlines),
                adapter: adapter,
                credential: credential,
                syncIntervalSeconds: UInt64(syncInterval)
            )
        )
        if result != nil {
            dismiss()
        }
    }
}

private extension ConnectorHealth {
    var label: String {
        switch self {
        case .healthy: "正常"
        case .degraded: "降级"
        case .error: "错误"
        }
    }

    var tint: Color {
        switch self {
        case .healthy: ElseColor.success
        case .degraded: ElseColor.signal
        case .error: ElseColor.danger
        }
    }
}

private extension String {
    var nilIfEmpty: String? {
        self.isEmpty ? nil : self
    }
}
