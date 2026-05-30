pub fn calculate_discount(price: f64, rate: f64) -> f64 {
    if rate < 0.0 || rate > 1.0 {
        0.0
    } else {
        price * rate
    }
}

pub fn calculate_total(price: f64, discount_rate: f64) -> f64 {
    let discount = calculate_discount(price, discount_rate);
    price - discount
}
