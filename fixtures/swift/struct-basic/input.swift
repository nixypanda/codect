struct Point: Equatable {
    var x: Double
    var y: Double = 0
    static let origin: Point = Point(x: 0, y: 0)
    func shifted(by dx: Double) -> Point { Point(x: x + dx, y: y) }
}
