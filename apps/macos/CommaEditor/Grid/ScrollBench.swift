#if DEBUG
import AppKit
import QuartzCore

/// Debug builds only: scrolls the table at a steady speed in step with the display and logs
/// frame times, to check that scrolling keeps up with 120 Hz displays.
@MainActor
final class ScrollBench: NSObject {
    private static var running: ScrollBench?

    private let table: NSTableView
    private let speed: CGFloat
    private let duration: CFTimeInterval
    private var link: CADisplayLink?
    private var times: [CFTimeInterval] = []
    private var target: CFTimeInterval = 1 / 60

    /// Scrolls `table` down at `speed` points a second for `duration` seconds.
    static func run(on table: NSTableView, speed: CGFloat, duration: CFTimeInterval) {
        let bench = ScrollBench(table: table, speed: speed, duration: duration)
        running = bench
        bench.link = table.displayLink(target: bench, selector: #selector(step(_:)))
        bench.link?.add(to: .main, forMode: .common)
    }

    private init(table: NSTableView, speed: CGFloat, duration: CFTimeInterval) {
        self.table = table
        self.speed = speed
        self.duration = duration
    }

    @objc private func step(_ link: CADisplayLink) {
        times.append(link.timestamp)
        target = link.targetTimestamp - link.timestamp
        guard let clip = table.enclosingScrollView?.contentView else { return }
        let elapsed = link.timestamp - times[0]
        if elapsed >= duration {
            link.invalidate()
            report()
            Self.running = nil
            return
        }
        let y = min(CGFloat(elapsed) * speed, table.frame.height - clip.bounds.height)
        clip.scroll(to: NSPoint(x: clip.bounds.origin.x, y: y))
        table.enclosingScrollView?.reflectScrolledClipView(clip)
    }

    private func report() {
        let intervals = zip(times.dropFirst(), times).map { $0 - $1 }.sorted()
        guard !intervals.isEmpty else { return }
        let average = intervals.reduce(0, +) / Double(intervals.count)
        let p95 = intervals[min(intervals.count - 1, Int(Double(intervals.count) * 0.95))]
        let slow = intervals.filter { $0 > target * 1.5 }.count
        NSLog(
            "COMMA scroll %.0f pt/s: %d frames, %.1f fps (display %.0f Hz), p95 %.1f ms, worst %.1f ms, slow %d",
            speed, intervals.count, 1 / average, 1 / target, p95 * 1000, intervals.last! * 1000, slow
        )
    }
}
#endif
