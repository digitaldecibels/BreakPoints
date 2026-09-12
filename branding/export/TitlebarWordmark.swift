// BreakPoints titlebar wordmark (5g)
// It's system type, so drawing it as text beats shipping a bitmap:
// stays crisp at any scale and picks up the user's Dark/Light appearance.

import SwiftUI

struct TitlebarWordmark: View {
    /// Wordmark cap height. Titlebar accessory: 15–16. Welcome/about: 30+.
    var size: CGFloat = 15
    var ink: Color = Color(red: 0.91, green: 0.91, blue: 0.92)   // #E8E8EA
    var accent: Color = Color(red: 0.298, green: 0.553, blue: 1)  // #4C8DFF

    var body: some View {
        HStack(spacing: 1) {
            Text("[")
                .font(.system(size: size * 1.27, weight: .regular, design: .monospaced))
                .foregroundColor(accent)
            HStack(spacing: 0) {
                Text("Break").fontWeight(.regular)
                Text("/").fontWeight(.light).foregroundColor(accent)
                Text("Points").fontWeight(.semibold)
            }
            .font(.system(size: size))
            .tracking(-size * 0.02)
            .foregroundColor(ink)
            .padding(.horizontal, size * 0.2)
            Text("]")
                .font(.system(size: size * 1.27, weight: .regular, design: .monospaced))
                .foregroundColor(accent)
        }
        .fixedSize()
        .accessibilityLabel("BreakPoints")
    }
}
