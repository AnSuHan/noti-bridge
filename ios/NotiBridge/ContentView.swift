import SwiftUI
import UIKit

struct ContentView: View {
    @EnvironmentObject private var model: BridgeModel

    var body: some View {
        TabView {
            NavigationStack { NotificationsView() }
                .tabItem { Label("알림", systemImage: "bell") }
            NavigationStack { DevicesView() }
                .tabItem { Label("기기", systemImage: "laptopcomputer") }
            NavigationStack { SettingsView() }
                .tabItem { Label("설정", systemImage: "gear") }
        }
        .alert(
            "확인 코드",
            isPresented: Binding(get: { model.pendingCode != nil }, set: { _ in }),
            presenting: model.pendingCode
        ) { _ in
            Button("일치") { model.confirm(true) }
            Button("다름", role: .cancel) { model.confirm(false) }
        } message: { pending in
            Text("\(pending.deviceName) 화면의 코드와 같은지 확인하세요.\n\n\(pending.code)")
        }
        .alert(
            model.message ?? "",
            isPresented: Binding(get: { model.message != nil }, set: { if !$0 { model.message = nil } })
        ) {
            Button("확인", role: .cancel) {}
        }
    }
}

private struct NotificationsView: View {
    @EnvironmentObject private var model: BridgeModel

    var body: some View {
        List(model.notifications, id: \.nid) { n in
            NotificationRow(notification: n)
                .swipeActions {
                    Button("지우기", role: .destructive) { model.dismiss(nid: n.nid) }
                }
        }
        .overlay {
            if model.notifications.isEmpty {
                Text("표시할 알림이 없습니다.").foregroundStyle(.secondary)
            }
        }
        .refreshable { model.didBecomeActive() }
        .navigationTitle("알림")
    }
}

private struct NotificationRow: View {
    @EnvironmentObject private var model: BridgeModel
    let notification: NotificationRecord
    @State private var replyAction: ActionInfo?
    @State private var replyText = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                if let data = model.iconData(hash: notification.iconHash), let image = UIImage(data: data) {
                    Image(uiImage: image).resizable().frame(width: 20, height: 20).cornerRadius(4)
                }
                Text(notification.appName).font(.caption).foregroundStyle(.secondary)
            }
            Text(notification.title).font(.headline)
            Text(notification.bodyHidden ? "내용 숨겨짐" : notification.body)
                .foregroundStyle(notification.bodyHidden ? .secondary : .primary)
            HStack {
                ForEach(notification.actions, id: \.id) { action in
                    Button(action.title) {
                        if action.acceptsReply {
                            replyAction = action
                        } else {
                            model.invoke(nid: notification.nid, actionId: action.id)
                        }
                    }
                    .buttonStyle(.bordered)
                }
            }
        }
        .alert("답장", isPresented: Binding(get: { replyAction != nil }, set: { if !$0 { replyAction = nil } })) {
            TextField("답장 입력", text: $replyText)
            Button("보내기") {
                if let action = replyAction {
                    model.reply(nid: notification.nid, actionId: action.id, text: replyText)
                }
                replyText = ""
            }
            Button("취소", role: .cancel) {}
        }
    }
}

private struct DevicesView: View {
    @EnvironmentObject private var model: BridgeModel
    @State private var scanning = false

    private func label(_ path: ConnectionPath) -> String {
        switch path {
        case .direct: return "직접 연결"
        case .relay: return "중계 연결"
        case .disconnected: return "연결 끊김"
        }
    }

    var body: some View {
        List {
            ForEach(model.peers, id: \.deviceId) { peer in
                VStack(alignment: .leading) {
                    Text(peer.name).font(.headline)
                    Text("\(peer.platform) · \(label(peer.path))").foregroundStyle(.secondary)
                }
                .swipeActions {
                    Button("해제", role: .destructive) { model.unpair(deviceId: peer.deviceId) }
                }
            }
            Button("QR 스캔으로 추가") { scanning = true }
        }
        .navigationTitle("기기")
        .sheet(isPresented: $scanning) {
            QRScannerView { code in
                scanning = false
                model.join(uri: code)
            }
            .ignoresSafeArea()
        }
    }
}

private struct SettingsView: View {
    @EnvironmentObject private var model: BridgeModel

    var body: some View {
        Form {
            Section("릴레이") {
                TextField("비우면 iroh 공개 릴레이", text: $model.relayUrl)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Button("저장") { model.saveRelay() }
            }
            Section {
                Text("iOS 는 다른 앱의 알림을 읽을 수 없어 수신 전용입니다. 앱이 백그라운드로 가면 시스템이 연결을 끊을 수 있으며, 다시 열면 그동안의 변경을 동기화합니다.")
                    .foregroundStyle(.secondary)
            }
        }
        .navigationTitle("설정")
    }
}
