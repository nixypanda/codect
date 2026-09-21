//! Methods with every receiver form.

struct Widget;

impl Widget {
    fn by_value(self) {}
    fn by_ref(&self) {}
    fn by_mut_ref(&mut self) {}
    fn by_lifetime(&'a self) {}
    fn by_lifetime_mut(&'a mut self) {}
    fn by_mut(mut self) {}
    fn boxed(self: Box<Self>) {}
    fn pinned(self: Pin<&mut Self>) {}
}
