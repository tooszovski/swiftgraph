import SwiftUI

struct ToggleRow: View {
    @State private var isSelected = false

    var body: some View {
        Toggle("On", isOn: $isSelected)
    }
}

@State private var previewFlag = false

func previewHost() -> some View {
    Toggle("Preview", isOn: $previewFlag)
}

#Preview {
    @Previewable @State var macroFlag = false
    Toggle("Macro", isOn: $macroFlag)
}
