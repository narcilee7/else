import AppKit
import SwiftUI

enum ElseColor {
    static let canvas = dynamic(light: 0xF8F8F6, dark: 0x0D0E0D)
    static let surface = dynamic(light: 0xF1F2EF, dark: 0x141614)
    static let elevated = dynamic(light: 0xFFFFFF, dark: 0x1A1C1A)
    static let textPrimary = dynamic(light: 0x171816, dark: 0xF3F4F1)
    static let textSecondary = dynamic(light: 0x5E625C, dark: 0xA4A8A1)
    static let textTertiary = dynamic(light: 0x8A8E88, dark: 0x747970)
    static let lineSubtle = dynamic(light: 0xE4E6E1, dark: 0x292D28)
    static let lineStrong = dynamic(light: 0xC9CCC6, dark: 0x41463F)
    static let selection = dynamic(light: 0xECEEE9, dark: 0x1D201C)
    static let signal = dynamic(light: 0xE85D32, dark: 0xF17A52)
    static let signalSoft = dynamic(light: 0xFDECE6, dark: 0x2B1A14)
    static let success = dynamic(light: 0x25835B, dark: 0x54B887)
    static let danger = dynamic(light: 0xC43D3D, dark: 0xE26C6C)
    static let info = dynamic(light: 0x3667C8, dark: 0x7EA3E8)
    static let action = dynamic(light: 0x1B1D1A, dark: 0xEDEFEA)

    private static func dynamic(light: UInt32, dark: UInt32) -> Color {
        Color(
            nsColor: NSColor(name: nil) { appearance in
                let match = appearance.bestMatch(from: [.darkAqua, .aqua])
                return NSColor(
                    rgb: match == .darkAqua ? dark : light
                )
            }
        )
    }
}

enum ElseSpacing {
    static let x1: CGFloat = 4
    static let x2: CGFloat = 8
    static let x3: CGFloat = 12
    static let x4: CGFloat = 16
    static let x6: CGFloat = 24
    static let x8: CGFloat = 32
    static let x12: CGFloat = 48
}

enum ElseRadius {
    static let small: CGFloat = 4
    static let floating: CGFloat = 8
}

enum ElseType {
    static let display = Font.system(size: 32, weight: .semibold)
    static let pageTitle = Font.system(size: 22, weight: .semibold)
    static let section = Font.system(size: 15, weight: .semibold)
    static let body = Font.system(size: 14, weight: .regular)
    static let callout = Font.system(size: 13, weight: .medium)
    static let caption = Font.system(size: 12, weight: .regular)
    static let micro = Font.system(size: 11, weight: .medium)
    static let mono = Font.system(size: 12, weight: .regular, design: .monospaced)
}

enum ElseMotion {
    static let instant = Animation.easeOut(duration: 0.09)
    static let quick = Animation.easeOut(duration: 0.14)
    static let standard = Animation.timingCurve(0.2, 0, 0, 1, duration: 0.18)
    static let emphasis = Animation.timingCurve(0.2, 0, 0, 1, duration: 0.22)
    static let completion = Animation.easeOut(duration: 0.28)
}

private extension NSColor {
    convenience init(rgb: UInt32) {
        self.init(
            calibratedRed: CGFloat((rgb >> 16) & 0xFF) / 255,
            green: CGFloat((rgb >> 8) & 0xFF) / 255,
            blue: CGFloat(rgb & 0xFF) / 255,
            alpha: 1
        )
    }
}
