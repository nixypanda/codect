#if DEBUG
struct A {}
#elseif RELEASE
#else
struct B {}
#endif
