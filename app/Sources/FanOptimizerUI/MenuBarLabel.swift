import FanOptimizerKit
import SwiftUI

public struct MenuBarLabel: View {
    let reading: MenuBarReading

    public init(reading: MenuBarReading) {
        self.reading = reading
    }

    public var body: some View {
        HStack(spacing: 3) {
            Image(systemName: reading.icon)
            if let text = reading.text {
                Text(text).monospacedDigit()
            }
        }
    }
}
