import Foundation
import Security
import UIKit
import UserNotifications

/// 기기 키를 키체인에 보관한다.
enum Keychain {
    private static let service = "com.notibridge.ios"

    static func load(_ name: String) throws -> Data {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: name,
            kSecReturnData as String: true,
        ]
        var item: CFTypeRef?
        if SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess, let data = item as? Data {
            return data
        }
        let key = try generateKey()
        query.removeValue(forKey: kSecReturnData as String)
        query[kSecValueData as String] = key
        query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        SecItemAdd(query as CFDictionary, nil)
        return key
    }
}

private final class ListenerBridge: EventListener {
    func onEvent(event: NodeEvent) {
        Task { @MainActor in BridgeModel.shared.handle(event) }
    }
}

struct PendingCode: Identifiable {
    let id = UUID()
    let deviceName: String
    let code: String
}

@MainActor
final class BridgeModel: ObservableObject {
    static let shared = BridgeModel()
    static let mirrorCategory = "mirror"
    private static let retentionMs: Int64 = 24 * 60 * 60 * 1000

    @Published var notifications: [NotificationRecord] = []
    @Published var peers: [PeerInfo] = []
    @Published var pendingCode: PendingCode?
    @Published var message: String?
    @Published var relayUrl = UserDefaults.standard.string(forKey: "relay_url") ?? ""

    private var node: NotiBridgeNode?

    func start() async {
        guard node == nil else { return }
        _ = try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound])
        do {
            let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
                .appendingPathComponent("core")
            let config = NodeConfig(
                dataDir: dir.path,
                secretKey: try Keychain.load("device_key"),
                storageKey: try Keychain.load("storage_key"),
                deviceName: UIDevice.current.name,
                platform: "ios",
                relayUrl: relayUrl.isEmpty ? nil : relayUrl,
                relayDisabled: false,
                mdnsEnabled: true,
                retentionMs: Self.retentionMs
            )
            node = try await Task.detached {
                try NotiBridgeNode.start(config: config, listener: ListenerBridge())
            }.value
            refresh()
        } catch {
            message = error.localizedDescription
        }
    }

    func didBecomeActive() {
        guard let node else { return }
        Task.detached { node.networkChanged() }
        refresh()
    }

    func refresh() {
        guard let node else { return }
        notifications = (try? node.activeNotifications()) ?? []
        peers = (try? node.peers()) ?? []
    }

    func handle(_ event: NodeEvent) {
        switch event {
        case .notificationChanged(let notification):
            present(notification)
        case .notificationRemoved(let nid, _):
            UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [nid])
        case .pairingCode(_, let deviceName, let code):
            pendingCode = PendingCode(deviceName: deviceName, code: code)
        case .pairingFinished(_, let deviceName, let accepted):
            pendingCode = nil
            message = accepted ? "\(deviceName) 연결 완료" : "페어링이 취소되었습니다."
        case .pairingFailed(let reason):
            pendingCode = nil
            message = reason
        default:
            break
        }
        refresh()
    }

    private func present(_ n: NotificationRecord) {
        guard n.state == .active else { return }
        let content = UNMutableNotificationContent()
        content.title = n.title.isEmpty ? n.appName : n.title
        content.subtitle = n.appName
        content.body = n.bodyHidden ? "내용 숨겨짐" : n.body
        content.categoryIdentifier = Self.mirrorCategory
        let request = UNNotificationRequest(identifier: n.nid, content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request)
    }

    func join(uri: String) {
        do {
            try node?.join(uri: uri)
        } catch {
            message = error.localizedDescription
        }
    }

    func confirm(_ accepted: Bool) {
        node?.confirmPairing(accepted: accepted)
    }

    func dismiss(nid: String) {
        try? node?.dismiss(nid: nid)
        refresh()
    }

    func invoke(nid: String, actionId: String) {
        try? node?.invokeAction(nid: nid, actionId: actionId)
    }

    func reply(nid: String, actionId: String, text: String) {
        try? node?.sendReply(nid: nid, actionId: actionId, text: text)
    }

    func unpair(deviceId: String) {
        try? node?.unpair(deviceId: deviceId)
        refresh()
    }

    func iconData(hash: String) -> Data? {
        guard !hash.isEmpty else { return nil }
        return try? node?.icon(hash: hash)
    }

    /// 릴레이 주소는 앱을 다시 시작할 때 적용된다.
    func saveRelay() {
        let trimmed = relayUrl.trimmingCharacters(in: .whitespaces)
        UserDefaults.standard.set(trimmed.isEmpty ? nil : trimmed, forKey: "relay_url")
        message = "앱을 다시 시작하면 적용됩니다."
    }
}
