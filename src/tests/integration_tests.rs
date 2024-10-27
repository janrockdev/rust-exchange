#[cfg(test)]
mod tests {
    use crate::*;
    use crate::fetch_order_book;
    use crate::orderbook::{ OrderBookRequest, OrderRequest };

    //Test the fetch_order_book function by fetching the order book for a trading pair
    #[tokio::test]
    async fn test_fetch_order_book() {
        let pair: &str = "XXBTZUSD";
        let orders = fetch_order_book(pair).await.unwrap();
        println!("\ntest_fetch_order_book(): collected records from API: {:?}\n", orders.len());
        assert!(orders.len() == 200);
    }

    #[tokio::test]
    async fn test_get_order_book() {
        let (order_tx, _order_rx) = mpsc::channel(100);
        let order_books = Arc::new(Mutex::new(HashMap::new()));
        let trade_books = Arc::new(Mutex::new(HashMap::new()));
        let service = Arc::new(OrderBookService {
            order_books: order_books.clone(),
            order_tx,
            trade_books,
        });

        let pair = "XXBTZUSD".to_string();
        let order = Order {
            id: Uuid::new_v4(),
            price: OrderedFloat(50000.0),
            volume: OrderedFloat(1.0),
            side: "ask".to_string(),
            timestamp: Utc::now().to_rfc3339(),
            order_type: "limit".to_string(),
        };

        order_books.lock().await.insert(pair.clone(), vec![order.clone()]);

        let request = Request::new(OrderBookRequest { pair: pair.clone() });
        let response = service.get_order_book(request).await.unwrap().into_inner();

        assert_eq!(response.orders.len(), 1);
        assert_eq!(response.orders[0].price, 50000.0);
        assert_eq!(response.orders[0].volume, 1.0);
    }

    #[tokio::test]
    async fn test_place_market_order() {
        let (order_tx, mut order_rx) = mpsc::channel(100);
        let order_books = Arc::new(Mutex::new(HashMap::new()));
        let trade_books = Arc::new(Mutex::new(HashMap::new()));
        let service = Arc::new(OrderBookService {
            order_books,
            order_tx,
            trade_books,
        });

        let market_order = OrderRequest {
            trader: "trader1".to_string(),
            pair: "XXBTZUSD".to_string(),
            price: 50000.0,
            volume: 1.0,
            side: "buy".to_string(),
            order_type: "market".to_string(),
        };

        let request = Request::new(market_order.clone());
        let response = service.place_market_order(request).await.unwrap().into_inner();

        assert_eq!(response.status, "new");
        assert_eq!(response.message, "order registerted and is being processed");

        let received_order = order_rx.recv().await.unwrap();
        assert_eq!(received_order.trader, market_order.trader);
        assert_eq!(received_order.pair, market_order.pair);
        assert_eq!(received_order.price, market_order.price);
        assert_eq!(received_order.volume, market_order.volume);
        assert_eq!(received_order.side, market_order.side);
        assert_eq!(received_order.order_type, market_order.order_type);
    }

    #[tokio::test]
    async fn test_get_trade_book() {
        let (order_tx, _order_rx) = mpsc::channel(100);
        let order_books = Arc::new(Mutex::new(HashMap::new()));
        let trade_books = Arc::new(Mutex::new(HashMap::new()));
        let service = Arc::new(OrderBookService {
            order_books,
            order_tx,
            trade_books: trade_books.clone(),
        });

        let trader = "trader1".to_string();
        let trade = Trade {
            id: Uuid::new_v4(),
            trader: trader.clone(),
            pair: "XXBTZUSD".to_string(),
            price: OrderedFloat(50000.0),
            volume: OrderedFloat(1.0),
            side: "buy".to_string(),
            timestamp: Utc::now().to_rfc3339(),
            order_type: "market".to_string(),
            status: "filled".to_string(),
        };

        trade_books.lock().await.insert(trader.clone(), vec![trade.clone()]);

        let request = Request::new(TradeBookRequest { trader: trader.clone() });
        let response = service.get_trade_book(request).await.unwrap().into_inner();

        assert_eq!(response.trades.len(), 1);
        assert_eq!(response.trades[0].id, trade.id.to_string());
        assert_eq!(response.trades[0].trader, trade.trader);
        assert_eq!(response.trades[0].pair, trade.pair);
        assert_eq!(response.trades[0].price, trade.price.into_inner());
        assert_eq!(response.trades[0].volume, trade.volume.into_inner());
        assert_eq!(response.trades[0].side, trade.side);
        assert_eq!(response.trades[0].timestamp, trade.timestamp);
        assert_eq!(response.trades[0].order_type, trade.order_type);
        assert_eq!(response.trades[0].status, trade.status);
    }

    #[tokio::test]
    async fn test_persist_order_book() {
        let order_books = HashMap::new();
        let pair = "XXBTZUSD";
        let result = persist_order_book(&order_books, pair, false, false).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_load_order_book_from_csv() {
        let file_paths = vec!["data/offline/XXBTZUSD_order_book.csv"];
        let result = load_order_book_from_csv(file_paths).await;
        assert!(result.is_ok());
        let order_books = result.unwrap();
        assert!(order_books.contains_key("XXBTZUSD"));
    }

    #[tokio::test]
    async fn test_fetch_order_books() {
        let pairs = vec!["XXBTZUSD", "XETHZUSD"];
        let result = fetch_order_books(pairs).await;
        assert!(!result.is_empty());
        assert!(result.contains_key("XXBTZUSD"));
        assert!(result.contains_key("XETHZUSD"));
    }
}