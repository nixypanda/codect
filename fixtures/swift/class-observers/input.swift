final class Counter: Base {
    private(set) var value: Int = 0 { willSet { print(newValue) } didSet { print(oldValue) } }
    var doubled: Int { get { value * 2 } set { value = newValue / 2 } }
    var written: Int { mutating get { 0 } nonmutating set { } }
    deinit { print("bye") }
}
