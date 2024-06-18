use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::fmt;
use std::sync::Arc;
use std::error::Error;
use tokio::sync::{ Mutex, mpsc };
use tokio::time::{ sleep, Duration };
use tonic::{ transport::Server, Request, Response, Status };
use futures::future::join_all;
use csv::{ ReaderBuilder, Writer };
use chrono::Utc;
use serde::{ Serialize, Serializer, ser::SerializeStruct, Deserialize, Deserializer };
use serde_json::Value;
use ordered_float::OrderedFloat;
use colored::*;

extern crate env_logger;
extern crate log;
use log::info;

pub mod orderbook {
    tonic::include_proto!("orderbook");
}

use orderbook::order_book_server::{ OrderBook, OrderBookServer };
use orderbook::{ OrderBookRequest, OrderBookResponse, OrderRequest, OrderResponse };

#[derive(Debug)]
pub struct OrderBookService {
    order_books: Arc<Mutex<HashMap<String, Vec<Order>>>>,
    order_tx: mpsc::Sender<OrderRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Order {
    price: OrderedFloat<f64>,
    volume: OrderedFloat<f64>,
    side: String,
    timestamp: String,
    order_type: String, // "market" or "limit"
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
        Ok(Order {
            price: OrderedFloat(helper.price),
            volume: OrderedFloat(helper.volume),
            side: helper.side,
            timestamp: helper.timestamp,
            order_type: helper.order_type,
        })
    }
}

// Implement the Display trait for the Order struct
impl fmt::Display for Order {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let output = format!(
            "Price: {:.1}, Volume: {:.3}, Side: {}",
            self.price,
            self.volume,
            self.side
        );
        if self.side == "ask" {
            write!(f, "{}", output.red())
        } else {
            write!(f, "{}", output.green())
        }
    }
}

// Custom serialization for Order (to avoid serialization of OrderedFloat for csv crate)
impl Serialize for Order {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: Serializer {
        let mut state = serializer.serialize_struct("Order", 5)?;
        state.serialize_field("price", &self.price.into_inner())?;
        state.serialize_field("volume", &self.volume.into_inner())?;
        state.serialize_field("side", &self.side)?;
        state.serialize_field("timestamp", &self.timestamp)?;
        state.serialize_field("order_type", &self.order_type)?;
        state.end()
    }
}

// Implement the OrderBook trait for OrderBookService to handle gRPC requests
#[tonic::async_trait]
impl OrderBook for Arc<OrderBookService> {
    async fn get_order_book(
        &self,
        request: Request<OrderBookRequest>
    ) -> Result<Response<OrderBookResponse>, Status> {
        let pair = request.into_inner().pair;
        let order_books = self.order_books.lock().await;
        if let Some(orders) = order_books.get(&pair) {
            Ok(
                Response::new(OrderBookResponse {
                    orders: orders
                        .iter()
                        .map(|o| orderbook::Order {
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
        let market_order = request.into_inner();
        if let Err(_) = self.order_tx.send(market_order).await {
            return Err(Status::internal("Failed to process order"));
        }
        Ok(
            Response::new(OrderResponse {
                status: "success".into(),
                message: "Order received and is being processed".into(),
            })
        )
    }
}

async fn persist_order_book(
    order_books: &HashMap<String, Vec<Order>>,
    pair: &str,
    include_timestamp: bool,
    sort_orders: bool
) -> Result<(), Box<dyn Error>> {
    let timestamp = if include_timestamp {
        format!("_{}", Utc::now().format("%Y%m%d%H%M%S%6f").to_string())
    } else {
        String::new()
    };

    let file_path = format!(
        "/home/toor/projects/rust-exchange/data/{}_order_book{}.csv",
        pair,
        timestamp
    );
    let mut wtr = Writer::from_writer(File::create(&file_path)?);

    if let Some(orders) = order_books.get(pair) {
        let mut orders_to_write = orders.clone();

        if sort_orders {
            let mut asks = orders
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

async fn update_order_books(service: Arc<OrderBookService>, pairs: Vec<&str>, offline_mode: bool) {
    if offline_mode {
        println!("Offline mode: Skipping API fetch.\n");
        return;
    }

    loop {
        let fetches = pairs.iter().map(|pair| {
            let pair = pair.to_string();
            async move {
                let orders = fetch_order_book(&pair).await.unwrap_or_else(|_| vec![]);
                (pair, orders)
            }
        });
        let results: Vec<(String, Vec<Order>)> = join_all(fetches).await;

        // Update order_books outside the loop to minimize lock time
        let mut new_order_books = HashMap::new();
        for (pair, orders) in results {
            new_order_books.insert(pair, orders);
        }

        {
            let mut order_books = service.order_books.lock().await;
            *order_books = new_order_books;
        }

        // Persist the order book after updating
        for pair in &pairs {
            let new_order_books = service.order_books.lock().await;
            if let Err(e) = persist_order_book(&new_order_books, pair, false, false).await {
                eprintln!("Failed to persist order book: {}", e);
            }
        }

        // Sleep for 2 hours before fetching order books again
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
    let url = format!("{}/?pair={}", "https://api.kraken.com/0/public/Depth", pair);
    let response = reqwest::get(&url).await?.json::<Value>().await?;
    let timestamp = Utc::now().to_rfc3339();

    let asks = parse_orders(&response["result"][pair]["asks"], "ask", &timestamp);
    let bids = parse_orders(&response["result"][pair]["bids"], "bid", &timestamp);

    // Combine asks and bids into a single vector of orders sorted by price
    let mut orders = Vec::new();
    orders.extend(asks);
    orders.extend(bids);

    // Sort the orders by price
    orders.sort_by(|a, b| b.price.cmp(&a.price));

    Ok(orders)
}

// Fetch initial order books for the given trading pairs in parallel
async fn fetch_order_books(pairs: Vec<&str>) -> HashMap<String, Vec<Order>> {
    let fetches = pairs.iter().map(|pair| {
        let pair = pair.to_string();
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
        let mut rdr = ReaderBuilder::new().from_path(file_path)?;
        for result in rdr.deserialize::<Order>() {
            match result {
                Ok(order) => {
                    let pair = file_path
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

async fn process_orders(
    service: Arc<OrderBookService>,
    mut rx: mpsc::Receiver<OrderRequest>
) {
    while let Some(market_order) = rx.recv().await {
        let pair = market_order.pair.clone();
        let mut order_books = service.order_books.lock().await;

        if let Some(orders) = order_books.get_mut(&pair) {
            let mut orders = orders.clone(); // Work with a local copy of the orders
            let mut matched_orders = vec![];
            let mut remaining_volume = OrderedFloat(market_order.volume);
            let mut orders_to_remove = vec![];

            println!("Processing order for trader: {}", market_order.trader);

            // Ensure the orders are sorted correctly for matching
            orders.sort_by(|a, b| match (a.side.as_str(), b.side.as_str()) {
                ("ask", "ask") => a.price.cmp(&b.price),
                ("bid", "bid") => b.price.cmp(&a.price),
                _ => std::cmp::Ordering::Equal,
            });

            for order in orders.iter_mut() {
                if market_order.order_type == "market" {
                    if (market_order.side == "buy" && order.side == "ask") || // Match buy order with ask order
                       (market_order.side == "sell" && order.side == "bid") { // Match sell order with bid order
                        let matched_volume = order.volume.min(remaining_volume);
                        println!(
                            "Matched order: price: {}, volume: {}, side: {}, timestamp: {}",
                            order.price,
                            order.volume,
                            order.side,
                            order.timestamp
                        );
                        matched_orders.push(Order {
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
                        } else {
                            println!("Order partially matched, remaining volume updated: {:?}", order);
                        }

                        if remaining_volume <= OrderedFloat(0.0) {
                            break;
                        }
                    }
                } else if market_order.order_type == "limit" {
                    // Handle limit order logic
                    if (market_order.side == "buy" && order.side == "ask" && market_order.price >= order.price.into_inner()) ||
                       (market_order.side == "sell" && order.side == "bid" && market_order.price <= order.price.into_inner()) {
                        let matched_volume = order.volume.min(remaining_volume);
                        println!(
                            "Matched order: price: {}, volume: {}, side: {}, timestamp: {}",
                            order.price,
                            order.volume,
                            order.side,
                            order.timestamp
                        );
                        matched_orders.push(Order {
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
                        } else {
                            println!("Order partially matched, remaining volume updated: {:?}", order);
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

                    // Re-sort the orders after inserting the new limit order
                    orders.sort_by(|a, b| match (a.side.as_str(), b.side.as_str()) {
                        ("ask", "ask") => a.price.cmp(&b.price),
                        ("bid", "bid") => b.price.cmp(&a.price),
                        _ => std::cmp::Ordering::Equal,
                    });
                }
            }

            println!("Matched orders: {:?}", matched_orders);

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
    let addr = "[::1]:50051".parse().unwrap();

    // Create a channel for sending market orders
    let (order_tx, order_rx) = mpsc::channel(100);

    // Define the trading pairs
    let pairs = vec!["XETHZUSD", "SUIUSD", "XXBTZUSD"];

    // Determine if the application should run in offline mode
    let args: Vec<String> = env::args().collect();
    let offline_mode = args.contains(&"--offline".to_string());

    // Fetch initial order books when the server starts
    let initial_order_books = if offline_mode {
        println!("Offline mode enabled: Loading order books from CSV files.");
        let csv_file_paths = vec![
            "data/offline/XXBTZUSD_order_book.csv",
            "data/offline/XETHZUSD_order_book.csv",
            "data/offline/SUIUSD_order_book.csv"
        ];
        let order_books = load_order_book_from_csv(csv_file_paths).await.unwrap_or_default();
        order_books
    } else {
        fetch_order_books(pairs.clone()).await
    };

    // Create the OrderBookService
    let order_book_service = Arc::new(OrderBookService {
        order_books: Arc::new(Mutex::new(initial_order_books)),
        order_tx,
    });

    // Clone the service for use in the spawned tasks
    let service_clone = Arc::clone(&order_book_service);
    tokio::spawn(async move {
        update_order_books(service_clone, pairs.clone(), offline_mode).await;
    });

    // Clone the service for use in the spawned tasks
    let service_clone = Arc::clone(&order_book_service);
    tokio::spawn(async move {
        process_orders(service_clone, order_rx).await;
    });

    info!("Exchange is listening on {}\n", addr);

    // Start the server
    Server::builder().add_service(OrderBookServer::new(order_book_service)).serve(addr).await?;

    Ok(())
}