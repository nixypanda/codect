@available(iOS 15, *)
struct Host {
    #if DEBUG
    #if os(iOS)
    @Suite
    struct MathTests {
        @Test func add() {}
    }
    #endif
    #elseif RELEASE
    #else
    struct Fallback {}
    #endif
}
