@Test func basic() { #expect(true) }
@Test(arguments: [1, 2]) func parameterized(value: Int) { #expect(value > 0) }
@Testing.Test func qualified() { }
func notATest() { }
