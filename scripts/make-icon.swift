// Draws the app icon: `swift scripts/make-icon.swift app/Resources/AppIcon.icns`.
// A macOS-grid squircle with a cool gradient and a fan.

import AppKit

let out = CommandLine.arguments.dropFirst().first ?? "AppIcon.icns"
let size: CGFloat = 1024
let top = NSColor(srgbRed: 0.25, green: 0.80, blue: 0.98, alpha: 1)
let bottom = NSColor(srgbRed: 0.10, green: 0.30, blue: 0.78, alpha: 1)
let symbol = "fan.fill"

func render(_ px: Int) -> Data {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8,
                               samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
                               bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let ctx = NSGraphicsContext.current!.cgContext
    ctx.scaleBy(x: CGFloat(px) / size, y: CGFloat(px) / size)

    // Big Sur grid: 824 pt body, 100 pt margin, ~185 pt corner radius, soft shadow.
    let body = CGRect(x: 100, y: 100, width: 824, height: 824)
    let shape = CGPath(roundedRect: body, cornerWidth: 185, cornerHeight: 185, transform: nil)
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -12), blur: 28, color: NSColor.black.withAlphaComponent(0.3).cgColor)
    ctx.addPath(shape)
    ctx.setFillColor(NSColor.black.cgColor)
    ctx.fillPath()
    ctx.restoreGState()

    ctx.saveGState()
    ctx.addPath(shape)
    ctx.clip()
    let gradient = CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB),
                              colors: [top.cgColor, bottom.cgColor] as CFArray, locations: [0, 1])!
    ctx.drawLinearGradient(gradient, start: CGPoint(x: 150, y: 924), end: CGPoint(x: 874, y: 100), options: [])
    // A faint highlight on the top half.
    ctx.setFillColor(NSColor.white.withAlphaComponent(0.07).cgColor)
    ctx.fillEllipse(in: CGRect(x: -200, y: 560, width: 1424, height: 760))
    ctx.restoreGState()

    // The glyph: a white SF Symbol, centered in a 520 pt box.
    let config = NSImage.SymbolConfiguration(pointSize: 420, weight: .semibold)
        .applying(NSImage.SymbolConfiguration(paletteColors: [.white]))
    let glyph = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)!.withSymbolConfiguration(config)!
    let scale = min(520 / glyph.size.width, 520 / glyph.size.height)
    let w = glyph.size.width * scale, h = glyph.size.height * scale
    glyph.draw(in: CGRect(x: 512 - w / 2, y: 512 - h / 2, width: w, height: h))

    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

let iconset = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("AppIcon-\(getpid()).iconset")
try? FileManager.default.removeItem(at: iconset)
try FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)
for base in [16, 32, 128, 256, 512] {
    try render(base).write(to: iconset.appendingPathComponent("icon_\(base)x\(base).png"))
    try render(base * 2).write(to: iconset.appendingPathComponent("icon_\(base)x\(base)@2x.png"))
}
let p = Process()
p.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
p.arguments = ["-c", "icns", iconset.path, "-o", out]
try p.run()
p.waitUntilExit()
try? render(1024).write(to: URL(fileURLWithPath: out).deletingPathExtension().appendingPathExtension("png"))
try? FileManager.default.removeItem(at: iconset)
exit(p.terminationStatus)
