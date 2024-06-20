#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use orderbook::order_book_client::OrderBookClient;

    //Test the fetch_order_book function by fetching the order book for a trading pair
    #[tokio::test]
    async fn test_fetch_order_book() {
        let pair: &str = "XXBTZUSD";
        let orders = fetch_order_book(pair).await.unwrap();
        println!("\ntest_fetch_order_book(): collected records from API: {:?}\n", orders.len());
        assert!(orders.len() == 200);
    }
}
