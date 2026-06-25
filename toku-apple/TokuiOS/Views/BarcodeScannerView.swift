import SwiftUI
import AVFoundation
import TokuKit

/// Camera-based ISBN barcode scanner using AVFoundation.
///
/// Scans EAN-13 barcodes (ISBN-13 format) and presents the result.
/// Includes manual ISBN entry as a fallback for denied camera permissions
/// or simulator use.
struct BarcodeScannerView: View {
    let ffi: TokuFFI?
    @State private var scannedISBN: String?
    @State private var manualISBN = ""
    @State private var showManualEntry = false
    @State private var addedMessage: String?
    @State private var errorMessage: String?
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            if let isbn = scannedISBN {
                scannedResultView(isbn: isbn)
            } else if showManualEntry {
                manualEntryView
            } else {
                scannerView
            }
        }
        .navigationTitle("Scan ISBN")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(showManualEntry ? "Camera" : "Manual") {
                    showManualEntry.toggle()
                    scannedISBN = nil
                }
            }
        }
    }

    // MARK: - Scanner

    @ViewBuilder
    private var scannerView: some View {
        ZStack {
            CameraPreview(onBarcodeScanned: { code in
                if isValidISBN13(code) {
                    scannedISBN = code
                }
            })
            .ignoresSafeArea()

            // Viewfinder overlay
            VStack {
                Spacer()
                RoundedRectangle(cornerRadius: 12)
                    .stroke(.white, lineWidth: 2)
                    .frame(width: 280, height: 100)
                    .background(.black.opacity(0.1))

                Text("Point camera at ISBN barcode")
                    .font(.callout)
                    .foregroundStyle(.white)
                    .padding(8)
                    .background(.black.opacity(0.6), in: Capsule())
                    .padding(.top, 12)
                Spacer()
            }
        }
    }

    // MARK: - Manual entry

    @ViewBuilder
    private var manualEntryView: some View {
        VStack(spacing: 24) {
            Spacer()

            Image(systemName: "barcode")
                .font(.system(size: 64))
                .foregroundStyle(.secondary)

            Text("Enter ISBN Manually")
                .font(.title2)
                .fontWeight(.semibold)

            TextField("ISBN-13 (e.g. 978-0-13-468599-1)", text: $manualISBN)
                .keyboardType(.numberPad)
                .textFieldStyle(.roundedBorder)
                .padding(.horizontal, 40)

            Button("Look Up") {
                let cleaned = manualISBN.replacingOccurrences(of: "-", with: "")
                    .replacingOccurrences(of: " ", with: "")
                if isValidISBN13(cleaned) {
                    scannedISBN = cleaned
                } else {
                    errorMessage = "Invalid ISBN-13. Must be 13 digits starting with 978 or 979."
                }
            }
            .buttonStyle(.borderedProminent)
            .disabled(manualISBN.isEmpty)

            if let error = errorMessage {
                Text(error)
                    .font(.callout)
                    .foregroundStyle(.red)
            }

            Spacer()
        }
    }

    // MARK: - Scanned result

    @ViewBuilder
    private func scannedResultView(isbn: String) -> some View {
        VStack(spacing: 24) {
            Spacer()

            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 64))
                .foregroundStyle(.green)

            Text("ISBN Scanned")
                .font(.title2)
                .fontWeight(.semibold)

            Text(isbn)
                .font(.title3)
                .monospacedDigit()
                .foregroundStyle(.secondary)

            if let message = addedMessage {
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.green)
            } else {
                VStack(spacing: 12) {
                    // TODO: Metadata fetch from Open Library when toku-meta FFI is available
                    Button {
                        addBookWithISBN(isbn)
                    } label: {
                        Label("Add Book with ISBN", systemImage: "plus.circle")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)

                    Button("Scan Another") {
                        scannedISBN = nil
                        manualISBN = ""
                        showManualEntry = false
                    }
                    .buttonStyle(.bordered)
                }
                .padding(.horizontal, 40)
            }

            Spacer()
        }
    }

    // MARK: - Helpers

    private func isValidISBN13(_ code: String) -> Bool {
        let digits = code.filter(\.isNumber)
        guard digits.count == 13 else { return false }
        guard digits.hasPrefix("978") || digits.hasPrefix("979") else { return false }

        // Validate check digit
        let values = digits.compactMap { Int(String($0)) }
        let sum = values.enumerated().reduce(0) { acc, pair in
            acc + pair.element * (pair.offset.isMultiple(of: 2) ? 1 : 3)
        }
        return sum % 10 == 0
    }

    private func addBookWithISBN(_ isbn: String) {
        guard let ffi = ffi else {
            errorMessage = "Database not available"
            return
        }

        do {
            let title = "ISBN: \(isbn)"
            let _ = try ffi.addBook(title: title)
            addedMessage = "Book added to library. Edit details to complete metadata."
        } catch {
            errorMessage = "Failed to add book: \(error.localizedDescription)"
        }
    }
}

// MARK: - Camera Preview (AVFoundation)

/// UIViewControllerRepresentable wrapping AVFoundation for barcode scanning.
struct CameraPreview: UIViewControllerRepresentable {
    let onBarcodeScanned: (String) -> Void

    func makeUIViewController(context: Context) -> CameraScannerViewController {
        let vc = CameraScannerViewController()
        vc.onBarcodeScanned = onBarcodeScanned
        return vc
    }

    func updateUIViewController(_ uiViewController: CameraScannerViewController, context: Context) {}
}

/// View controller that manages the AVCaptureSession for barcode scanning.
final class CameraScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    var onBarcodeScanned: ((String) -> Void)?
    private var captureSession: AVCaptureSession?
    private var previewLayer: AVCaptureVideoPreviewLayer?
    private var hasScanned = false

    override func viewDidLoad() {
        super.viewDidLoad()
        setupCamera()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        previewLayer?.frame = view.bounds
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        if let session = captureSession, !session.isRunning {
            DispatchQueue.global(qos: .userInitiated).async {
                session.startRunning()
            }
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        if let session = captureSession, session.isRunning {
            DispatchQueue.global(qos: .userInitiated).async {
                session.stopRunning()
            }
        }
    }

    private func setupCamera() {
        let session = AVCaptureSession()
        self.captureSession = session

        guard let device = AVCaptureDevice.default(for: .video),
              let input = try? AVCaptureDeviceInput(device: device) else {
            showPermissionDenied()
            return
        }

        if session.canAddInput(input) {
            session.addInput(input)
        }

        let output = AVCaptureMetadataOutput()
        if session.canAddOutput(output) {
            session.addOutput(output)
            output.setMetadataObjectsDelegate(self, queue: .main)
            output.metadataObjectTypes = [.ean13]
        }

        let layer = AVCaptureVideoPreviewLayer(session: session)
        layer.videoGravity = .resizeAspectFill
        layer.frame = view.bounds
        view.layer.addSublayer(layer)
        self.previewLayer = layer

        DispatchQueue.global(qos: .userInitiated).async {
            session.startRunning()
        }
    }

    private func showPermissionDenied() {
        let label = UILabel()
        label.text = "Camera access is required to scan barcodes.\nGo to Settings → Toku → Camera."
        label.numberOfLines = 0
        label.textAlignment = .center
        label.textColor = .secondaryLabel
        label.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(label)
        NSLayoutConstraint.activate([
            label.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            label.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            label.leadingAnchor.constraint(greaterThanOrEqualTo: view.leadingAnchor, constant: 32),
        ])
    }

    func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput metadataObjects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        guard !hasScanned,
              let object = metadataObjects.first as? AVMetadataMachineReadableCodeObject,
              let code = object.stringValue else { return }

        hasScanned = true

        // Haptic feedback
        let generator = UIImpactFeedbackGenerator(style: .medium)
        generator.impactOccurred()

        captureSession?.stopRunning()
        onBarcodeScanned?(code)
    }
}
