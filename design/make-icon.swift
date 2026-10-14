// Draws the CommaEditor app icon: a table on an indigo tile with a large comma, and optionally
// a letter on a badge in the upper right corner (W for CommaEditor W, the Tauri app on macOS).
// Usage: swift design/make-icon.swift <output.png> [letter]   (1024 × 1024, macOS icon grid)
import AppKit
import CoreText

let size: CGFloat = 1024
let arguments = Array(CommandLine.arguments.dropFirst())
let output = arguments.first ?? "icon_1024.png"
let badge = arguments.count > 1 ? arguments[1] : nil

func color(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(
        srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
        green: CGFloat((hex >> 8) & 0xFF) / 255,
        blue: CGFloat(hex & 0xFF) / 255,
        alpha: alpha
    )
}

let space = CGColorSpace(name: CGColorSpace.sRGB)!
let context = CGContext(
    data: nil, width: Int(size), height: Int(size), bitsPerComponent: 8, bytesPerRow: 0,
    space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
)!

// Tile: 824 × 824 centred, as in Apple's macOS icon template, with a soft shadow.
let tile = CGRect(x: 100, y: 100, width: 824, height: 824)
let tilePath = CGPath(roundedRect: tile, cornerWidth: 185, cornerHeight: 185, transform: nil)
context.saveGState()
context.setShadow(offset: CGSize(width: 0, height: -10), blur: 28, color: color(0x000000, 0.35))
context.addPath(tilePath)
context.setFillColor(color(0x3A4BD6))
context.fillPath()
context.restoreGState()

context.saveGState()
context.addPath(tilePath)
context.clip()
let gradient = CGGradient(colorsSpace: space, colors: [color(0x6A7CFF), color(0x2B37B8)] as CFArray, locations: [0, 1])!
context.drawLinearGradient(gradient, start: CGPoint(x: 0, y: tile.maxY), end: CGPoint(x: 0, y: tile.minY), options: [])
context.restoreGState()

// The table: a white sheet with a tinted header row and grid lines.
let sheet = CGRect(x: 214, y: 268, width: 596, height: 520)
let sheetPath = CGPath(roundedRect: sheet, cornerWidth: 44, cornerHeight: 44, transform: nil)
context.saveGState()
context.setShadow(offset: CGSize(width: 0, height: -8), blur: 20, color: color(0x101540, 0.35))
context.addPath(sheetPath)
context.setFillColor(color(0xFFFFFF))
context.fillPath()
context.restoreGState()

context.saveGState()
context.addPath(sheetPath)
context.clip()
let headerHeight: CGFloat = 112
context.setFillColor(color(0xDCE2FF))
context.fill(CGRect(x: sheet.minX, y: sheet.maxY - headerHeight, width: sheet.width, height: headerHeight))
context.setStrokeColor(color(0xC7CEEB))
context.setLineWidth(6)
for i in 1..<3 {
    let x = sheet.minX + sheet.width * CGFloat(i) / 3
    context.move(to: CGPoint(x: x, y: sheet.minY))
    context.addLine(to: CGPoint(x: x, y: sheet.maxY))
}
let rowHeight = (sheet.height - headerHeight) / 4
for i in 0..<4 {
    let y = sheet.maxY - headerHeight - rowHeight * CGFloat(i)
    context.move(to: CGPoint(x: sheet.minX, y: y))
    context.addLine(to: CGPoint(x: sheet.maxX, y: y))
}
context.strokePath()
context.restoreGState()

// The comma, centred on a point near the sheet's lower right corner.
let font = CTFontCreateWithName("SFPro-Black" as CFString, 1180, nil)
var glyph = CGGlyph()
var character: UniChar = 0x2C
CTFontGetGlyphsForCharacters(font, &character, &glyph, 1)
var bounds = CGRect.zero
CTFontGetBoundingRectsForGlyphs(font, .default, &glyph, &bounds, 1)
let glyphPath = CTFontCreatePathForGlyph(font, glyph, nil)!
let center = CGPoint(x: 712, y: 318)
var move = CGAffineTransform(translationX: center.x - bounds.midX, y: center.y - bounds.midY)
let comma = glyphPath.copy(using: &move)!

context.saveGState()
context.setShadow(offset: CGSize(width: 0, height: -6), blur: 16, color: color(0x101540, 0.45))
context.addPath(comma)
context.setFillColor(color(0xFFA31A))
context.fillPath()
context.restoreGState()
context.addPath(comma)
context.setStrokeColor(color(0xFFFFFF))
context.setLineWidth(18)
context.setLineJoin(.round)
context.strokePath()
context.addPath(comma)
context.setFillColor(color(0xFFA31A))
context.fillPath()

// The badge: a white-rimmed circle over the tile's upper right corner with the letter on it.
if let badge {
    let circle = CGRect(x: 664, y: 664, width: 300, height: 300)
    context.saveGState()
    context.setShadow(offset: CGSize(width: 0, height: -8), blur: 20, color: color(0x101540, 0.45))
    context.addEllipse(in: circle)
    context.setFillColor(color(0xFFFFFF))
    context.fillPath()
    context.restoreGState()
    let inner = circle.insetBy(dx: 18, dy: 18)
    context.saveGState()
    context.addEllipse(in: inner)
    context.clip()
    let fill = CGGradient(colorsSpace: space, colors: [color(0xFFB84D), color(0xF08A00)] as CFArray, locations: [0, 1])!
    context.drawLinearGradient(fill, start: CGPoint(x: 0, y: inner.maxY), end: CGPoint(x: 0, y: inner.minY), options: [])
    context.restoreGState()

    let letterFont = NSFont.systemFont(ofSize: 200, weight: .black) as CTFont
    var letters = Array(badge.utf16)
    var glyphs = [CGGlyph](repeating: 0, count: letters.count)
    CTFontGetGlyphsForCharacters(letterFont, &letters, &glyphs, letters.count)
    var letterBounds = CGRect.zero
    CTFontGetBoundingRectsForGlyphs(letterFont, .default, &glyphs, &letterBounds, 1)
    let letterPath = CTFontCreatePathForGlyph(letterFont, glyphs[0], nil)!
    var place = CGAffineTransform(translationX: circle.midX - letterBounds.midX, y: circle.midY - letterBounds.midY)
    context.addPath(letterPath.copy(using: &place)!)
    context.setFillColor(color(0xFFFFFF))
    context.fillPath()
}

let image = context.makeImage()!
let rep = NSBitmapImageRep(cgImage: image)
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: output))
print("wrote \(output)")
