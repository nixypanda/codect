enum Result<T, E: Error> {
    case success(T)
    case failure(label: String, E)
    case blank
    case code = "x"
    var isOK: Bool { true }
}
