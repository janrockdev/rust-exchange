use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::fmt;
use std::sync::Arc;
use std::error::Error;
use models::model::models::Config;
use tokio::sync::{ Mutex, mpsc };
use tokio::time::{ sleep, Duration };
use tonic::{ transport::Server, Request, Response, Status };
use futures::future::join_all;
use csv::{ ReaderBuilder, Writer };
use chrono::Utc;
use serde::{ Serialize, Serializer, ser::SerializeStruct, Deserialize, Deserializer };
use serde_json::Value;
use ordered_float::OrderedFloat;
use uuid::Uuid;
use crate::utils::config::load_config;

use colored::*;
extern crate env_logger;
extern crate log;
use log::info;

use orderbook::order_book_server::{ OrderBook, OrderBookServer };
use orderbook::{
    OrderBookRequest,
    OrderBookResponse,
    OrderRequest,
    OrderResponse,
    TradeBookRequest,
    TradeBookResponse,
};

pub mod utils;
pub mod models;
pub mod orderbook {
    tonic::include_proto!("orderbook");
}

#[derive(Debug)]
pub struct OrderBookService {
    order_books: Arc<Mutex<HashMap<String, Vec<Order>>>>,
    order_tx: mpsc::Sender<OrderRequest>,
    trade_books: Arc<Mutex<HashMap<String, Vec<Trade>>>>,
}

// For Orderbook
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Order {
    id: Uuid,
    price: OrderedFloat<f64>,
    volume: OrderedFloat<f64>,
    side: String,
    timestamp: String,
    order_type: String,
}

//For Tradebook
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trade {
    id: Uuid,
    trader: String,
    pair: String,
    price: OrderedFloat<f64>,
    volume: OrderedFloat<f64>,
    side: String,
    timestamp: String,
    order_type: String,
    status: String,
}

// Custom deserialization for Order
impl<'de> Deserialize<'de> for Order {
    fn deserialize<D>(deserializer: D) -> Result<Order, D::Error> where D: Deserializer<'de> {
        #[derive(Deserialize)]
        struct OrderData {
            price: f64,
            volume: f64,
            side: String,
            timestamp: String,
            order_type: String,
        }

        let helper = OrderData::deserialize(deserializer)?;
        //let id = Uuid::parse_str(&helper.id).map_err(de::Error::custom)?;

        Ok(Order {
            id: Uuid::new_v4(),
            price: OrderedFloat(helper.price),
            volume: OrderedFloat(helper.volume),
            side: helper.side,
            timestamp: helper.timestamp,
            order_type: helper.order_type,
        })
    }
}

// Custom serialization for Order (to avoid serialization of OrderedFloat for csv crate)
// TODO: find a better way to handle serialization of OrderedFloat
impl Serialize for Order {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: Serializer {
        let mut state: <S as Serializer>::SerializeStruct = serializer.serialize_struct(
            "Order",
            5
        )?;
        state.serialize_field("price", &self.price.into_inner())?;
        state.serialize_field("volume", &self.volume.into_inner())?;
        state.serialize_field("side", &self.side)?;
        state.serialize_field("timestamp", &self.timestamp)?;
        state.serialize_field("order_type", &self.order_type)?;
        //state.serialize_field("id", &self.id.to_string())?;
        state.end()
    }
}

// Implement the Display trait for the Order struct (more readable output with colors)
impl fmt::Display for Order {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let output: String = format!(
            "Price: {:.5}, Volume: {:.3}, Side: {}, ID: {}, Timestamp: {}",
            self.price,
            self.volume,
            self.side,
            self.id,
            self.timestamp
        );
        if self.side == "ask" {
            write!(f, "{}", output.red())
        } else {
            write!(f, "{}", output.green())
        }
    }
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
            HashMap<String, Vec<Order>>
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
            HashMap<String, Vec<Trade>>
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

// Function to persist the order book to a CSV file (for testing and development purposes)
async fn persist_order_book(
    order_books: &HashMap<String, Vec<Order>>,
    pair: &str,
    include_timestamp: bool,
    sort_orders: bool
) -> Result<(), Box<dyn Error>> {
    let config: Config = load_config().unwrap();

    let timestamp: String = if include_timestamp {
        format!("_{}", Utc::now().format("%Y%m%d%H%M%S%6f").to_string())
    } else {
        String::new()
    };

    let file_path: String = format!(
        "{}/{}_order_book{}.csv",
        config.kraken.persist,
        pair,
        timestamp
    );
    let mut wtr: Writer<File> = Writer::from_writer(File::create(&file_path)?);

    if let Some(orders) = order_books.get(pair) {
        let mut orders_to_write: Vec<Order> = orders.clone();

        if sort_orders {
            let mut asks: Vec<Order> = orders
                .iter()
                .filter(|o| o.side == "ask")
                .cloned()
                .collect::<Vec<Order>>();
            let mut bids = orders
                .iter()
                .filter(|o| o.side == "bid")
                .cloned()
                .collect::<Vec<Order>>();
            asks.sort_by(|a, b| b.price.cmp(&a.price));
            bids.sort_by(|a, b| b.price.cmp(&a.price));
            orders_to_write = Vec::new();
            orders_to_write.extend(asks);
            orders_to_write.extend(bids);
        }

        for order in orders_to_write {
            wtr.serialize(order)?;
        }
        wtr.flush()?;
    }
    Ok(())
}

// Function to update order books in a loop (TODO: add error handling when not able to fetch order books, move sleep duration to config)
async fn update_order_books(service: Arc<OrderBookService>, pairs: Vec<&str>, offline_mode: bool) {
    if offline_mode {
        println!("Offline mode: Skipping API fetch.\n");
        return;
    }

    loop {
        let fetches = pairs.iter().map(|pair| {
            let pair: String = pair.to_string();
            async move {
                let orders = fetch_order_book(&pair).await.unwrap_or_else(|_| vec![]);
                (pair, orders)
            }
        });
        let results: Vec<(String, Vec<Order>)> = join_all(fetches).await;

        // Update order_books outside the loop to minimize lock time
        let mut new_order_books: HashMap<String, Vec<Order>> = HashMap::new();
        for (pair, orders) in results {
            new_order_books.insert(pair, orders);
        }

        {
            let mut order_books: tokio::sync::MutexGuard<
                HashMap<String, Vec<Order>>
            > = service.order_books.lock().await;
            *order_books = new_order_books;
        }

        // Persist the order book after updating
        for pair in &pairs {
            let new_order_books: tokio::sync::MutexGuard<
                HashMap<String, Vec<Order>>
            > = service.order_books.lock().await;
            if let Err(e) = persist_order_book(&new_order_books, pair, false, false).await {
                eprintln!("Failed to persist order book: {}", e);
            }
        }

        // Sleep for 10 seconds before fetching order books again (maybe use Tokio timer instead of sleep)
        sleep(Duration::from_secs(10)).await;
    }
}

// Helper function to parse orders from JSON array
fn parse_orders(data: &Value, side: &str, timestamp: &str) -> Vec<Order> {
    data.as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|order| {
            let price = order[0].as_str().unwrap().parse::<f64>().unwrap();
            let volume = order[1].as_str().unwrap().parse::<f64>().unwrap();
            Order {
                id: Uuid::new_v4(),
                price: OrderedFloat(price),
                volume: OrderedFloat(volume),
                side: side.to_string(),
                timestamp: timestamp.to_string(),
                order_type: "limit".to_string(),
            }
        })
        .collect()
}

// Fetch the order book for a given trading pair from Kraken API and return a vector of Order structs
async fn fetch_order_book(pair: &str) -> Result<Vec<Order>, reqwest::Error> {
    let url: String = format!("{}/?pair={}", "https://api.kraken.com/0/public/Depth", pair);
    let response: Value = reqwest::get(&url).await?.json::<Value>().await?;
    let timestamp: String = Utc::now().to_rfc3339();

    let asks: Vec<Order> = parse_orders(&response["result"][pair]["asks"], "ask", &timestamp);
    let bids: Vec<Order> = parse_orders(&response["result"][pair]["bids"], "bid", &timestamp);

    // Combine asks and bids into a single vector of orders sorted by price
    let mut orders: Vec<Order> = Vec::new();
    orders.extend(asks);
    orders.extend(bids);

    // Sort the orders by price
    orders.sort_by(|a, b| b.price.cmp(&a.price));

    Ok(orders)
}

// Fetch initial order books for the given trading pairs in parallel
async fn fetch_order_books(pairs: Vec<&str>) -> HashMap<String, Vec<Order>> {
    let fetches = pairs.iter().map(|pair| {
        let pair: String = pair.to_string();
        async move {
            let orders = fetch_order_book(&pair).await.unwrap_or_else(|_| vec![]);
            (pair, orders)
        }
    });
    let results: Vec<(String, Vec<Order>)> = join_all(fetches).await;

    results.into_iter().collect()
}

// Function to load order book from CSV files
async fn load_order_book_from_csv(
    file_paths: Vec<&str>
) -> Result<HashMap<String, Vec<Order>>, Box<dyn Error>> {
    let mut order_books: HashMap<String, Vec<Order>> = HashMap::new();

    for file_path in file_paths {
        let mut rdr: csv::Reader<File> = ReaderBuilder::new().from_path(file_path)?;
        for result in rdr.deserialize::<Order>() {
            match result {
                Ok(order) => {
                    let pair: String = file_path
                        .split('_')
                        .next()
                        .unwrap_or("unknown")
                        .to_string()
                        .replace("data/offline/", "");
                    info!("Loaded order for {}: {}", pair, order);
                    order_books.entry(pair.clone()).or_insert_with(Vec::new).push(order);
                }
                Err(e) => {
                    println!("Error deserializing order: {}", e);
                }
            }
        }
    }

    // Sort the orders within each order book
    for orders in order_books.values_mut() {
        orders.sort_by(|a, b| {
            match (a.side.as_str(), b.side.as_str()) {
                ("ask", "ask") => a.price.cmp(&b.price),
                ("bid", "bid") => b.price.cmp(&a.price),
                _ => std::cmp::Ordering::Equal,
            }
        });
    }

    Ok(order_books)
}

// Function to process market orders and update the order book (core)
async fn process_orders(service: Arc<OrderBookService>, mut rx: mpsc::Receiver<OrderRequest>) {
    while let Some(market_order) = rx.recv().await {
        let pair = market_order.pair.clone();

        let mut order_books: tokio::sync::MutexGuard<
            HashMap<String, Vec<Order>>
        > = service.order_books.lock().await;
        let mut trade_books: tokio::sync::MutexGuard<
            HashMap<String, Vec<Trade>>
        > = service.trade_books.lock().await;

        // Record trader in tradebook before processing the trade

        let trade: Trade = Trade {
            id: Uuid::new_v4(),
            trader: market_order.trader.clone(),
            pair: market_order.pair,
            side: market_order.side.clone(),
            price: market_order.price.into(),
            volume: market_order.volume.into(),
            timestamp: Utc::now().to_rfc3339(),
            order_type: market_order.order_type.clone(),
            status: "new".to_string(), // First status of the trade
        };
        trade_books.entry(market_order.trader.clone()).or_insert_with(Vec::new).push(trade.clone());

        if let Some(orders) = order_books.get_mut(&pair) {
            let mut orders: Vec<Order> = orders.clone(); // Work with a local copy of the orders
            let mut matched_orders: Vec<Order> = vec![];
            let mut remaining_volume: OrderedFloat<f64> = OrderedFloat(market_order.volume);
            let mut orders_to_remove: Vec<Order> = vec![];

            println!("Processing order for trader: {}", market_order.trader);

            // Sorting for printing
            orders.sort_by(|a, b| {
                match (a.side.as_str(), b.side.as_str()) {
                    ("ask", "ask") => b.price.cmp(&a.price), //descending order
                    ("bid", "bid") => b.price.cmp(&a.price), //descending order
                    _ => std::cmp::Ordering::Equal,
                }
            });

            println!("Orderbook status before processing trade: ----");
            for order in orders.iter() {
                println!("{}", order);
            }
            println!("----------------------------------------------\n");

            // Sorting for matching
            orders.sort_by(|a, b| {
                match (a.side.as_str(), b.side.as_str()) {
                    ("ask", "ask") => a.price.cmp(&b.price),
                    ("bid", "bid") => b.price.cmp(&a.price),
                    _ => std::cmp::Ordering::Equal,
                }
            });

            for order in orders.iter_mut() {
                let order_log: Order = order.clone();
                if market_order.order_type == "market" {
                    if
                        (market_order.side == "buy" && order.side == "ask") || // Match buy order with ask order
                        (market_order.side == "sell" && order.side == "bid")
                    {
                        // Match sell order with bid order
                        let matched_volume: OrderedFloat<f64> = order.volume.min(remaining_volume);
                        println!(
                            "Matched order: price: {}, volume: {}, side: {}, timestamp: {}, order_type: {}, id: {}",
                            order.price,
                            order.volume,
                            order.side,
                            order.timestamp,
                            order.order_type,
                            order.id
                        );
                        matched_orders.push(Order {
                            id: order.id,
                            price: order.price,
                            volume: matched_volume,
                            side: order.side.clone(),
                            timestamp: order.timestamp.clone(),
                            order_type: order.order_type.clone(),
                        });
                        order.volume -= matched_volume;
                        remaining_volume -= matched_volume;

                        if order.volume <= OrderedFloat(0.0) {
                            orders_to_remove.push(order.clone());
                            println!("Order fully matched and removed: {:?}", order);

                            //insert to tradebook
                            let trade = Trade {
                                id: order.id,
                                trader: market_order.trader.clone(),
                                pair: pair.clone(),
                                side: order.side.clone(),
                                price: order.price.into(),
                                volume: order_log.volume.into(),
                                timestamp: Utc::now().to_rfc3339(),
                                order_type: order.order_type.clone(),
                                status: "filled".to_string(),
                            };
                            trade_books
                                .entry(market_order.trader.clone())
                                .or_insert_with(Vec::new)
                                .push(trade.clone());
                        } else {
                            println!("Order partially matched, remaining volume updated: {:?}",order);

                            //insert to tradebook
                            let trade = Trade {
                                id: order.id,
                                trader: market_order.trader.clone(),
                                pair: pair.clone(),
                                side: order.side.clone(),
                                price: order.price.into(),
                                volume: matched_volume, //order.volume.into(),
                                timestamp: Utc::now().to_rfc3339(),
                                order_type: order.order_type.clone(),
                                status: "partially_filled".to_string(),
                            };
                            trade_books
                                .entry(market_order.trader.clone())
                                .or_insert_with(Vec::new)
                                .push(trade.clone());
                        }

                        if remaining_volume <= OrderedFloat(0.0) {
                            break;
                        }
                    }
                } else if market_order.order_type == "limit" {
                    // Handle limit order logic
                    if
                        (market_order.side == "buy" &&
                            order.side == "ask" &&
                            market_order.price >= order.price.into_inner()) ||
                        (market_order.side == "sell" &&
                            order.side == "bid" &&
                            market_order.price <= order.price.into_inner())
                    {
                        let matched_volume: OrderedFloat<f64> = order.volume.min(remaining_volume);
                        println!(
                            "Matched order: price: {}, volume: {}, side: {}, timestamp: {}",
                            order.price,
                            order.volume,
                            order.side,
                            order.timestamp
                        );
                        matched_orders.push(Order {
                            id: order.id,
                            price: order.price,
                            volume: matched_volume,
                            side: order.side.clone(),
                            timestamp: order.timestamp.clone(),
                            order_type: order.order_type.clone(),
                        });
                        order.volume -= matched_volume;
                        remaining_volume -= matched_volume;

                        if order.volume <= OrderedFloat(0.0) {
                            orders_to_remove.push(order.clone());
                            println!("Order fully matched and removed: {:?}", order);

                            //insert to tradebook
                            let trade = Trade {
                                id: order.id,
                                trader: market_order.trader.clone(),
                                pair: pair.clone(),
                                side: order.side.clone(),
                                price: order.price.into(),
                                volume: order_log.volume.into(),
                                timestamp: Utc::now().to_rfc3339(),
                                order_type: order.order_type.clone(),
                                status: "filled".to_string(),
                            };
                            trade_books
                                .entry(market_order.trader.clone())
                                .or_insert_with(Vec::new)
                                .push(trade.clone());
                        } else {
                            println!("Order partially matched, remaining volume updated: {:?}", order);

                            //insert to tradebook
                            let trade = Trade {
                                id: order.id,
                                trader: market_order.trader.clone(),
                                pair: pair.clone(),
                                side: order.side.clone(),
                                price: order.price.into(),
                                volume: matched_volume, //order.volume.into(),
                                timestamp: Utc::now().to_rfc3339(),
                                order_type: order.order_type.clone(),
                                status: "partially_filled".to_string(),
                            };
                            trade_books
                                .entry(market_order.trader.clone())
                                .or_insert_with(Vec::new)
                                .push(trade.clone());
                        }

                        if remaining_volume <= OrderedFloat(0.0) {
                            break;
                        }
                    }
                }
            }

            for order in orders_to_remove {
                if let Some(pos) = orders.iter().position(|x| *x == order) {
                    orders.remove(pos);
                }
            }

            if remaining_volume > OrderedFloat(0.0) {
                if market_order.order_type == "market" {
                    println!("Market order could not be fully matched, remaining volume: {}", remaining_volume);
                } else if market_order.order_type == "limit" {
                    let new_order = Order {
                        id: trade.id,
                        price: OrderedFloat(market_order.price), // Limit order retains the specified price
                        volume: remaining_volume,
                        side: if market_order.side == "buy" {
                            "bid".to_string()
                        } else {
                            "ask".to_string()
                        },
                        timestamp: Utc::now().to_rfc3339(),
                        order_type: "limit".to_string(),
                    };
                    orders.push(new_order.clone());
                    println!("Limit order added to order book: {:?}", new_order);

                    // JRO: TODO: aggregate order book by side and price
                }
            }

            orders.sort_by(|a, b| {
                match a.side.as_str().cmp(&b.side.as_str()) {
                    std::cmp::Ordering::Equal =>
                        match a.side.as_str() {
                            "ask" => b.price.cmp(&a.price), //descending order
                            "bid" => b.price.cmp(&a.price), //descending order
                            _ => std::cmp::Ordering::Equal,
                        }
                    other => other,
                }
            });

            println!("\nOrderbook status after processing trade: -----");
            for order in orders.iter() {
                println!("{}", order);
            }
            println!("----------------------------------------------\n");

            // Update the order book with the local copy
            order_books.insert(pair.clone(), orders);

            // Persist the order book after processing the trade
            if let Err(e) = persist_order_book(&order_books, &pair, true, true).await {
                eprintln!("Failed to persist order book with timestamp: {}", e);
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let config: Config = load_config().unwrap();

    // TODO: move to config file
    let addr: std::net::SocketAddr = "[::1]:50051".parse().unwrap();

    // Create a channel for sending market orders
    let (order_tx, order_rx) = mpsc::channel(100);

    // Define the trading pairs (TODO: move to config file)
    let symbols: Vec<String> = config.kraken.symbols.clone();

    // Determine if the application should run in offline mode
    let args: Vec<String> = env::args().collect();
    let offline_mode: bool = args.contains(&"--offline".to_string());

    // Fetch initial order books when the server starts in offline mode
    let initial_order_books: HashMap<String, Vec<Order>> = if offline_mode {
        println!("Offline mode enabled: Loading order books from CSV files.");
        let paths: Vec<String> = config.kraken.offline.clone();
        let csv_file_paths: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
        let order_books: HashMap<String, Vec<Order>> = load_order_book_from_csv(
            csv_file_paths.iter().map(AsRef::as_ref).collect()
        ).await.unwrap_or_default();
        order_books
    } else {
        fetch_order_books(symbols.iter().map(AsRef::as_ref).collect()).await
    };

    // Create the OrderBookService
    let order_book_service: Arc<OrderBookService> = Arc::new(OrderBookService {
        order_books: Arc::new(Mutex::new(initial_order_books)),
        order_tx,
        trade_books: Arc::new(Mutex::new(HashMap::new())), //TODO: load from CSV (recovery/optional)
    });

    // Clone the service for use in the spawned tasks
    let service_clone: Arc<OrderBookService> = Arc::clone(&order_book_service);
    tokio::spawn(async move {
        update_order_books(service_clone, symbols.iter().map(AsRef::as_ref).collect(), offline_mode).await;
    });

    // Clone the service for use in the spawned tasks
    let service_clone: Arc<OrderBookService> = Arc::clone(&order_book_service);
    tokio::spawn(async move {
        process_orders(service_clone, order_rx).await;
    });

    info!("Exchange is listening on {}\n", addr);

    // Start the server
    Server::builder().add_service(OrderBookServer::new(order_book_service)).serve(addr).await?;

    Ok(())
}

// Test module
#[cfg(test)]
mod tests {
    mod integration_tests;
}