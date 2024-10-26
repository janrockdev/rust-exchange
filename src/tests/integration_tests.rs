#[cfg(test)]
mod tests {
    use serial_test::serial;
    use crate::fetch_order_book;
    use crate::orderbook::{ OrderBookRequest, OrderRequest };
    use crate::orderbook::order_book_client::OrderBookClient;

    //Test the fetch_order_book function by fetching the order book for a trading pair
    #[tokio::test]
    async fn test_fetch_order_book() {
        let pair: &str = "XXBTZUSD";
        let orders = fetch_order_book(pair).await.unwrap();
        println!("\ntest_fetch_order_book(): collected records from API: {:?}\n", orders.len());
        assert!(orders.len() == 200);
    }
    
    #[tokio::test]
    #[serial]
    async fn test_get_order_book() {
        let mut client = OrderBookClient::connect("http://[::1]:50051").await.unwrap();

        // Test get_order_book
        let request = tonic::Request::new(OrderBookRequest {
            pair: "XXBTZUSD".to_string(),
        });
        let response = client.get_order_book(request).await.unwrap();
        let order_book_response = response.into_inner();
        assert!(!order_book_response.orders.is_empty());
    }
    #[tokio::test]
    #[serial]
    async fn test_place_limit_buy_order() {
        let mut client = OrderBookClient::connect("http://[::1]:50051").await.unwrap();

        // Test place_market_buy_order
        let request = tonic::Request::new(OrderRequest {
            pair: "XXBTZUSD".to_string(),
            volume: 0.96,
            side: "buy".to_string(),
            price: 65290.0,
            order_type: "limit".to_string(),
            trader: "test_trader".to_string(),
        });
        let response = client.place_market_order(request).await.unwrap();
        let market_order_response = response.into_inner();
        assert_eq!(market_order_response.status, "new");
    }

    #[tokio::test]
    #[serial]
    async fn test_place_limit_sell_order() {
        let mut client = OrderBookClient::connect("http://[::1]:50051").await.unwrap();
        // Test place_market_buy_order
        let request = tonic::Request::new(OrderRequest {
            pair: "XXBTZUSD".to_string(),
            volume: 0.367,
            side: "sell".to_string(),
            price: 65290.1,
            order_type: "limit".to_string(),
            trader: "test_trader".to_string(),
        });
        let response = client.place_market_order(request).await.unwrap();
        let market_order_response = response.into_inner();
        assert_eq!(market_order_response.status, "new");
    }
    #[serial]
    #[tokio::test]
    async fn test_place_market_buy_order() {
        let mut client = OrderBookClient::connect("http://[::1]:50051").await.unwrap();
        // Test place_market_buy_order
        let request = tonic::Request::new(OrderRequest {
            pair: "XXBTZUSD".to_string(),
            volume: 16.0, //15.633,
            side: "buy".to_string(),
            price: 0.0, //market order ignores price parameter
            order_type: "market".to_string(),
            trader: "test_trader".to_string(),
        });
        let response = client.place_market_order(request).await.unwrap();
        let market_order_response = response.into_inner();
        assert_eq!(market_order_response.status, "new");
    }
    #[serial]
    #[tokio::test]
    async fn test_place_market_sell_order() {
        let mut client = OrderBookClient::connect("http://[::1]:50051").await.unwrap();
        // Test place_market_sell_order
        let request = tonic::Request::new(OrderRequest {
            pair: "XXBTZUSD".to_string(),
            volume: 1.0, //0.04,
            side: "sell".to_string(),
            price: 0.0, //market order ignores price parameter
            order_type: "market".to_string(),
            trader: "test_trader".to_string(),
        });
        let response = client.place_market_order(request).await.unwrap();
        let market_order_response = response.into_inner();
        assert_eq!(market_order_response.status, "new");
    }
}
