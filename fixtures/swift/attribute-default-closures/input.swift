@foo(bar: { print("leak") })
@deep(bar: outer({ inner({ leak() }) }))
struct S {
    func f(_ cb: () -> Void = { print("leak") }, _ d: () -> Void = outer({ inner({ leak() }) })) {}
}
