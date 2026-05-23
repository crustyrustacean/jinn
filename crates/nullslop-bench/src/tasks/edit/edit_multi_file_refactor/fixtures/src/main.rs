use discount::calculate_discount;
use discount::calculate_total;

fn main() {
    let prices = [99.99, 49.50, 150.0, 25.0];
    let rate = 0.15;

    for price in prices {
        let discount = calculate_discount(price, rate);
        let total = calculate_total(price, rate);
        println!("Price: ${:.2}, Discount: ${:.2}, Total: ${:.2}", price, discount, total);
    }

    // Test edge case
    let no_discount = calculate_discount(100.0, 0.0);
    println!("No discount: ${:.2}", no_discount);
}
