pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn subtract(a: i32, b: i32) -> i32 {
    a - b
}

pub fn divide(a: i32, b: i32) -> f64 {
    if b == 0 {
        0.0
    } else {
        f64::from(a) / f64::from(b)
    }
}

pub fn multiply(a: i32, b: i32) -> i32 {
    a * b
}

fn main() {
    println!("add(3, 4) = {}", add(3, 4));
    println!("subtract(10, 3) = {}", subtract(10, 3));
    println!("divide(10, 3) = {}", divide(10, 3));
    println!("multiply(3, 4) = {}", multiply(3, 4));
}
