pub mod models {
    use std::collections::BTreeMap;
    use crate::OrderedFloat;
    use std::fmt;
    use colored::Colorize;
    use uuid::Uuid;
    use tonic::{ Request, Response, Status };
    use serde::Deserialize;

    pub mod orderbook {
        tonic::include_proto!("orderbook");
    }

    use std::sync::Arc;
    use tokio::sync::{ Mutex, mpsc };

    use orderbook::order_book_server::OrderBook;
    use orderbook::{
        OrderBookRequest,
        OrderBookResponse,
        OrderRequest,
        OrderResponse,
        TradeBookRequest,
        TradeBookResponse,
    };

    #[derive(Debug, Deserialize)]
    pub struct Config {
        pub kraken: KrakenConfig,
    }

    #[derive(Debug, Deserialize)]
    pub struct KrakenConfig {
        pub symbols: Vec<String>,
        pub persist: String,
        pub offline: Vec<String>,
    }

    // For Orderbook
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Order {
        pub id: Uuid,
        pub price: OrderedFloat<f64>,
        pub volume: OrderedFloat<f64>,
        pub side: String,
        pub timestamp: String,
        pub order_type: String,
    }

    //For Tradebook
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Trade {
        pub id: Uuid,
        pub trader: String,
        pub pair: String,
        pub price: OrderedFloat<f64>,
        pub volume: OrderedFloat<f64>,
        pub side: String,
        pub timestamp: String,
        pub order_type: String,
        pub status: String,
    }

    #[derive(Debug)]
    pub struct OrderBookService {
        pub order_books: Arc<Mutex<BTreeMap<String, Vec<Order>>>>,
        pub order_tx: mpsc::Sender<OrderRequest>,
        pub trade_books: Arc<Mutex<BTreeMap<String, Vec<Trade>>>>,
    }

    // Implement the OrderBook trait for OrderBookService to handle gRPC requests (core)
    #[tonic::async_trait]
    impl OrderBook for Arc<OrderBookService> {
        async fn get_order_book(
            &self,
            request: Request<OrderBookRequest>
        ) -> Result<Response<OrderBookResponse>, Status> {
            let pair: String = request.into_inner().pair;
            let order_books: tokio::sync::MutexGuard<
                BTreeMap<String, Vec<Order>>
            > = self.order_books.lock().await;
            if let Some(orders) = order_books.get(&pair) {
                Ok(
                    Response::new(OrderBookResponse {
                        orders: orders
                            .iter()
                            .map(|o: &Order| orderbook::Order {
                                price: o.price.into_inner(),
                                volume: o.volume.into_inner(),
                            })
                            .collect(),
                    })
                )
            } else {
                Err(Status::not_found("Order book not found"))
            }
        }

        async fn place_market_order(
            &self,
            request: Request<OrderRequest>
        ) -> Result<Response<OrderResponse>, Status> {
            let market_order: OrderRequest = request.into_inner();
            if let Err(_) = self.order_tx.send(market_order).await {
                return Err(Status::internal("Failed to process order"));
            }
            Ok(
                Response::new(OrderResponse {
                    status: "new".into(),
                    message: "order registerted and is being processed".into(),
                })
            )
        }

        async fn get_trade_book(
            &self,
            request: Request<TradeBookRequest>
        ) -> Result<Response<TradeBookResponse>, Status> {
            let trader: String = request.into_inner().trader;
            let trade_books: tokio::sync::MutexGuard<
                BTreeMap<String, Vec<Trade>>
            > = self.trade_books.lock().await;
            if let Some(trades) = trade_books.get(&trader) {
                Ok(
                    Response::new(TradeBookResponse {
                        trades: trades
                            .iter()
                            .map(|t: &Trade| orderbook::Trade {
                                id: t.id.to_string(),
                                trader: t.trader.clone(),
                                order_type: t.order_type.clone(),
                                pair: t.pair.clone(),
                                side: t.side.clone(),
                                price: t.price.into_inner(),
                                volume: t.volume.into_inner(),
                                timestamp: t.timestamp.clone(),
                                status: t.status.clone(),
                            })
                            .collect(),
                    })
                )
            } else {
                Err(Status::not_found("Trade book not found"))
            }
        }
    }

    // Implement the Display trait for the Order struct (more readable output with colors)
    impl core::fmt::Display for Order {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let output: String = format!(
                "Price: {:.5}, Volume: {:.3}, Side: {}, ID: {}, Timestamp: {}",
                self.price, self.volume, self.side, self.id, self.timestamp
            );
            if self.side == "ask" {
                write!(f, "{}", output.red())
            } else {
                write!(f, "{}", output.green())
            }
        }
    }
}