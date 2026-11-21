typealias Handler<T> = (T) -> Void
let globalConst: Int = 1
var globalVar: String
infix operator ??? : AdditionPrecedence
@attached(member, names: named(init)) macro Member() = #externalMacro(module: "M", type: "T")
macro M(_ x: Int) -> Int
