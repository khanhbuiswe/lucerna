// A small Rust sample.
use std::fmt;

const LIMIT: u32 = 10;

struct Point {
    x: i32,
    y: i32,
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

fn main() {
    let p = Point { x: 1, y: 2 };
    if LIMIT > 5 {
        println!("{p}");
    }
}
