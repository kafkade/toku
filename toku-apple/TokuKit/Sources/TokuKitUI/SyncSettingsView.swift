import SwiftUI
import TokuKit

/// Shared sync settings screen used by both the macOS and iOS apps.
///
/// When sync is not yet configured it shows a setup form (server URL + optional
/// encryption passphrase). Once configured it shows live status, the registered
/// device list, and push/pull/"sync now" actions. All work is delegated to
/// `SyncViewModel`, which runs FFI calls off the main thread.
public struct SyncSettingsView: View {
    @ObservedObject private var viewModel: SyncViewModel

    @State private var server = ""
    @State private var deviceName = ""
    @State private var passphrase = ""

    public init(viewModel: SyncViewModel) {
        self.viewModel = viewModel
    }

    public var body: some View {
        Form {
            if viewModel.isConfigured, let status = viewModel.status {
                configuredSections(status)
            } else {
                setupSection
            }

            if let result = viewModel.lastResult {
                Section {
                    Label(result, systemImage: "checkmark.circle")
                        .foregroundStyle(.green)
                }
            }

            if let error = viewModel.errorMessage {
                Section {
                    Label(error, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.red)
                }
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Sync")
        .disabled(viewModel.isBusy)
        .overlay {
            if viewModel.isBusy {
                ProgressView()
            }
        }
        .onAppear { viewModel.refresh() }
    }

    // MARK: - Setup (not yet configured)

    private var setupSection: some View {
        Section {
            TextField("Server URL", text: $server, prompt: Text("https://sync.example.com"))
                .textContentType(.URL)
                #if os(iOS)
                .textInputAutocapitalization(.never)
                .keyboardType(.URL)
                #endif
                .autocorrectionDisabled()

            TextField("Device name (optional)", text: $deviceName)
                .autocorrectionDisabled()

            SecureField("Encryption passphrase (optional)", text: $passphrase)

            Button("Enable Sync") {
                viewModel.initialize(
                    server: server.trimmingCharacters(in: .whitespacesAndNewlines),
                    deviceName: deviceName.isEmpty ? nil : deviceName,
                    passphrase: passphrase.isEmpty ? nil : passphrase
                )
            }
            .disabled(server.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        } header: {
            Text("Set up sync")
        } footer: {
            Text("Sync keeps your library, reading sessions, and notes consistent across your devices. Providing a passphrase enables end-to-end encryption; the server never sees your passphrase.")
        }
    }

    // MARK: - Configured

    @ViewBuilder
    private func configuredSections(_ status: SyncStatus) -> some View {
        Section("Status") {
            LabeledContent("Server", value: status.server)
            LabeledContent("Device", value: status.deviceName)
            LabeledContent("Pending changes", value: "\(status.pendingOps)")
            LabeledContent("Encryption", value: status.encryption ? "Enabled" : "Off")
        }

        Section {
            Button {
                viewModel.syncNow()
            } label: {
                Label("Sync Now", systemImage: "arrow.triangle.2.circlepath")
            }

            Button {
                viewModel.push()
            } label: {
                Label("Push Changes", systemImage: "arrow.up.circle")
            }

            Button {
                viewModel.pull()
            } label: {
                Label("Pull Changes", systemImage: "arrow.down.circle")
            }
        }

        if !viewModel.devices.isEmpty {
            Section("Devices") {
                ForEach(viewModel.devices) { device in
                    HStack {
                        Image(systemName: "laptopcomputer.and.iphone")
                            .foregroundStyle(.secondary)
                        VStack(alignment: .leading) {
                            Text(device.deviceName)
                            if let lastSeen = device.lastSeen {
                                Text("Last seen \(lastSeen)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
        }
    }
}
