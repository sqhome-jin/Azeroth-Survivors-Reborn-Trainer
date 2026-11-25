import SwiftUI

@main
struct TrainerApp: App {
    @StateObject private var store = TrainerStore()

    var body: some Scene {
        WindowGroup {
            RootView(store: store)
                .frame(minWidth: 960, minHeight: 640)
        }
        .defaultSize(width: 1040, height: 720)
        .commands {
            CommandGroup(replacing: .newItem) {}
        }
    }
}
