use std::collections::{HashMap, BTreeMap, VecDeque};
use std::env;
use std::fs::File;
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep, Duration};
use tonic::{transport::Server, Request, Response, Status};
use log::{info, error};

use csv::Writer;
use chrono::Utc;
use ordered_float::OrderedFloat;
use uuid::Uuid;

use rust_exchange::models::model::models::orderbook::{
    order_book_server::{OrderBook, OrderBookServer},
    OrderBookRequest, OrderBookResponse, OrderRequest, OrderResponse, TradeBookRequest, TradeBookResponse,
};
use rust_exchange::utils::config::load_config;
use rust_exchange::error::ExchangeError;
use rust_exchange::validation::validate_order_request;

use futures::future::join_all;
use serde_json::Value;
use csv::ReaderBuilder;

use rust_exchange::models::model::models::{Config, Order, Trade};

type OrderMap = BTreeMap<OrderedFloat<f64>, VecDeque<Order>>;
type OrderBooks = HashMap<String, OrderMap>;
type TradeBooks = HashMap<String, Vec<Trade>>;

#[derive(Debug, Clone)]
pub struct OrderBookService {
    order_books: Arc<Mutex<OrderBooks>>,
    order_tx: mpsc::Sender<OrderRequest>,
    trade_books: Arc<Mutex<TradeBooks>>,
}

// Implement the OrderBook trait for OrderBookService to handle gRPC requests (core)
#[tonic::async_trait]
impl OrderBook for OrderBookService {
    async fn get_order_book(
        &self,
        request: Request<OrderBookRequest>,
    ) -> std::result::Result<Response<OrderBookResponse>, Status> {
        let pair: String = request.into_inner().pair;
        
        if let Err(e) = rust_exchange::validation::validate_trading_pair(&pair) {
            return Err(Status::invalid_argument(format!("Invalid trading pair: {}", e)));
        }
        
        let order_books = self.order_books.lock().await;
        if let Some(price_levels) = order_books.get(&pair) {
            let mut aggregated_orders = Vec::new();
            
            for (price, orders_at_price) in price_levels.iter() {
                let total_volume: f64 = orders_at_price.iter().map(|o| o.volume).sum();
                if total_volume > 0.0 {
                    let _side = if let Some(first_order) = orders_at_price.front() {
                        first_order.side.clone()
                    } else {
                        continue;
                    };
                    
                    aggregated_orders.push(rust_exchange::models::model::models::orderbook::Order {
                        price: price.into_inner(),
                        volume: total_volume,
                    });
                }
            }
            
            Ok(Response::new(OrderBookResponse {
                orders: aggregated_orders,
            }))
        } else {
            Err(Status::not_found("Order book not found"))
        }
    }

    async fn place_market_order(
        &self,
        request: Request<OrderRequest>,
    ) -> std::result::Result<Response<OrderResponse>, Status> {
        let order_request: OrderRequest = request.into_inner();
        
        if let Err(e) = validate_order_request(&order_request) {
            return Err(Status::invalid_argument(format!("Invalid order: {}", e)));
        }
        
        if self.order_tx.send(order_request).await.is_err() {
            error!("Failed to send order to processing channel");
            return Err(Status::internal("Failed to process order"));
        }
        
        info!("Order registered and queued for processing");
        Ok(
            Response::new(OrderResponse {
                status: "new".to_string(),
                message: "order registered and is being processed".to_string(),
            })
        )
    }

    async fn get_trade_book(
        &self,
        request: Request<TradeBookRequest>,
    ) -> std::result::Result<Response<TradeBookResponse>, Status> {
        let trader: String = request.into_inner().trader;
        
        if let Err(e) = rust_exchange::validation::validate_trader_id(&trader) {
            return Err(Status::invalid_argument(format!("Invalid trader ID: {}", e)));
        }
        
        let trade_books = self.trade_books.lock().await;
        if let Some(trades) = trade_books.get(&trader) {
            Ok(
                Response::new(TradeBookResponse {
                    trades: trades
                        .iter()
                        .map(|trade| rust_exchange::models::model::models::orderbook::Trade {
                            id: trade.id.to_string(),
                            pair: trade.pair.clone(),
                            price: trade.price.into_inner(),
                            volume: trade.volume,
                            side: trade.side.clone(),
                            timestamp: trade.timestamp.clone(),
                            trader: trade.trader.clone(),
                            order_type: trade.order_type.clone(),
                            status: trade.status.clone(),
                        })
                        .collect(),
                })
            )
        } else {
            Ok(
                Response::new(TradeBookResponse {
                    trades: vec![],
                })
            )
        }
    }
}

// Function to persist the order book to a CSV file (for testing and development purposes)
async fn persist_order_book(
    order_books: &HashMap<String, BTreeMap<OrderedFloat<f64>, VecDeque<Order>>>,
    pair: &str,
    include_timestamp: bool,
    sort_orders: bool
) -> rust_exchange::error::Result<()> {
    let config: Config = load_config().map_err(|e| ExchangeError::ConfigError(e.to_string()))?;

    let timestamp: String = if include_timestamp {
        format!("_{}", Utc::now().format("%Y%m%d%H%M%S%6f"))
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
        let mut orders_to_write: Vec<Order> = Vec::new();
        
        for (_price, order_queue) in orders.iter() {
            for order in order_queue.iter() {
                orders_to_write.push(order.clone());
            }
        }
        
        if sort_orders {
            let mut asks: Vec<Order> = orders_to_write
                .iter()
                .filter(|o| o.side == "ask")
                .cloned()
                .collect::<Vec<Order>>();
            let mut bids: Vec<Order> = orders_to_write
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
        let mut new_order_books: OrderBooks = HashMap::new();
        for (pair, orders) in results {
            let mut order_map: OrderMap = BTreeMap::new();
            for order in orders {
                order_map.entry(order.price).or_default().push_back(order);
            }
            new_order_books.insert(pair, order_map);
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

        // Sleep for 10 seconds before fetching order books again (maybe use Tokio timer instead of sleep)
        sleep(Duration::from_secs(10)).await;
    }
}

// Helper function to parse orders from JSON array
fn parse_orders(data: &Value, side: &str, timestamp: &str) -> rust_exchange::error::Result<Vec<Order>> {
    let mut orders = Vec::new();
    
    if let Some(array) = data.as_array() {
        for order in array {
            let price_str = order[0].as_str()
                .ok_or_else(|| ExchangeError::ParseError("Invalid price format".to_string()))?;
            let volume_str = order[1].as_str()
                .ok_or_else(|| ExchangeError::ParseError("Invalid volume format".to_string()))?;
            
            let price = price_str.parse::<f64>()
                .map_err(|_| ExchangeError::ParseError("Failed to parse price".to_string()))?;
            let volume = volume_str.parse::<f64>()
                .map_err(|_| ExchangeError::ParseError("Failed to parse volume".to_string()))?;
            
            orders.push(Order {
                id: Uuid::new_v4(),
                price: OrderedFloat(price),
                volume,
                side: side.to_string(),
                timestamp: timestamp.to_string(),
                order_type: "limit".to_string(),
            });
        }
    }
    
    Ok(orders)
}

// Fetch the order book for a given trading pair from Kraken API and return a vector of Order structs
async fn fetch_order_book(pair: &str) -> rust_exchange::error::Result<Vec<Order>> {
    let url: String = format!("{}/?pair={}", "https://api.kraken.com/0/public/Depth", pair);
    let response: Value = reqwest::get(&url).await?.json::<Value>().await?;
    let timestamp: String = Utc::now().to_rfc3339();

    let asks: Vec<Order> = parse_orders(&response["result"][pair]["asks"], "ask", &timestamp)?;
    let bids: Vec<Order> = parse_orders(&response["result"][pair]["bids"], "bid", &timestamp)?;

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
) -> rust_exchange::error::Result<HashMap<String, Vec<Order>>> {
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
                    order_books.entry(pair.clone()).or_default().push(order);
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

        let mut order_books = service.order_books.lock().await;
        let mut trade_books = service.trade_books.lock().await;

        // Record trader in tradebook before processing the trade

        let trade: Trade = Trade {
            id: Uuid::new_v4(),
            trader: market_order.trader.clone(),
            pair: market_order.pair,
            side: market_order.side.clone(),
            price: market_order.price.into(),
            volume: market_order.volume,
            timestamp: Utc::now().to_rfc3339(),
            order_type: market_order.order_type.clone(),
            status: "new".to_string(), // First status of the trade
        };
        trade_books.entry(market_order.trader.clone()).or_default().push(trade.clone());

        if let Some(order_map) = order_books.get_mut(&pair) {
            let mut orders: Vec<Order> = Vec::new();
            for (_price, order_queue) in order_map.iter() {
                for order in order_queue.iter() {
                    orders.push(order.clone());
                }
            }
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
                        let matched_volume: f64 = order.volume.min(remaining_volume.into_inner());
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
                        remaining_volume -= OrderedFloat(matched_volume);

                        if order.volume <= 0.0 {
                            orders_to_remove.push(order.clone());
                            println!("Order fully matched and removed: {:?}", order);

                            //insert to tradebook
                            let trade = Trade {
                                id: order.id,
                                trader: market_order.trader.clone(),
                                pair: pair.clone(),
                                side: order.side.clone(),
                                price: order.price,
                                volume: order_log.volume,
                                timestamp: Utc::now().to_rfc3339(),
                                order_type: order.order_type.clone(),
                                status: "filled".to_string(),
                            };
                            trade_books
                                .entry(market_order.trader.clone())
                                .or_default()
                                .push(trade.clone());
                        } else {
                            println!("Order partially matched, remaining volume updated: {:?}",order);

                            //insert to tradebook
                            let trade = Trade {
                                id: order.id,
                                trader: market_order.trader.clone(),
                                pair: pair.clone(),
                                side: order.side.clone(),
                                price: order.price,
                                volume: matched_volume,
                                timestamp: Utc::now().to_rfc3339(),
                                order_type: order.order_type.clone(),
                                status: "partially_filled".to_string(),
                            };
                            trade_books
                                .entry(market_order.trader.clone())
                                .or_default()
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
                        let matched_volume: f64 = order.volume.min(remaining_volume.into_inner());
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
                        remaining_volume -= OrderedFloat(matched_volume);

                        if order.volume <= 0.0 {
                            orders_to_remove.push(order.clone());
                            println!("Order fully matched and removed: {:?}", order);

                            //insert to tradebook
                            let trade = Trade {
                                id: order.id,
                                trader: market_order.trader.clone(),
                                pair: pair.clone(),
                                side: order.side.clone(),
                                price: order.price,
                                volume: order_log.volume,
                                timestamp: Utc::now().to_rfc3339(),
                                order_type: order.order_type.clone(),
                                status: "filled".to_string(),
                            };
                            trade_books
                                .entry(market_order.trader.clone())
                                .or_default()
                                .push(trade.clone());
                        } else {
                            println!("Order partially matched, remaining volume updated: {:?}", order);

                            //insert to tradebook
                            let trade = Trade {
                                id: order.id,
                                trader: market_order.trader.clone(),
                                pair: pair.clone(),
                                side: order.side.clone(),
                                price: order.price,
                                volume: matched_volume,
                                timestamp: Utc::now().to_rfc3339(),
                                order_type: order.order_type.clone(),
                                status: "partially_filled".to_string(),
                            };
                            trade_books
                                .entry(market_order.trader.clone())
                                .or_default()
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
                        id: Uuid::new_v4(),
                        price: OrderedFloat(market_order.price), // Limit order retains the specified price
                        volume: remaining_volume.into_inner(),
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
                match a.side.as_str().cmp(b.side.as_str()) {
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

            // Convert Vec<Order> back to BTreeMap and update order book
            let mut new_order_map: BTreeMap<OrderedFloat<f64>, VecDeque<Order>> = BTreeMap::new();
            for order in orders {
                new_order_map.entry(order.price).or_default().push_back(order);
            }
            order_books.insert(pair.clone(), new_order_map);

            // Persist the order book after processing the trade
            if let Err(e) = persist_order_book(&order_books, &pair, true, true).await {
                eprintln!("Failed to persist order book with timestamp: {}", e);
            }
        }
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let config: Config = load_config()
        .map_err(|e| format!("Failed to load configuration: {}", e))?;

    let addr: std::net::SocketAddr = config.server.address.parse()
        .map_err(|e| format!("Invalid server address in config: {}", e))?;

    let (order_tx, order_rx) = mpsc::channel(config.server.channel_buffer_size);

    let symbols: Vec<String> = config.kraken.symbols.clone();

    let args: Vec<String> = env::args().collect();
    let offline_mode: bool = args.contains(&"--offline".to_string());

    let initial_order_books = if offline_mode {
        info!("Offline mode enabled: Loading order books from CSV files.");
        let paths: Vec<String> = config.kraken.offline.clone();
        let csv_file_paths: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
        let csv_orders = load_order_book_from_csv(
            csv_file_paths.iter().map(AsRef::as_ref).collect()
        ).await.unwrap_or_default();
        
        // Convert HashMap<String, Vec<Order>> to HashMap<String, BTreeMap<OrderedFloat<f64>, VecDeque<Order>>>
        let mut converted_orders: OrderBooks = HashMap::new();
        for (pair, orders) in csv_orders {
            let mut order_map: OrderMap = BTreeMap::new();
            for order in orders {
                order_map.entry(order.price).or_default().push_back(order);
            }
            converted_orders.insert(pair, order_map);
        }
        converted_orders
    } else {
        let fetched_orders = fetch_order_books(symbols.iter().map(AsRef::as_ref).collect()).await;
        // Convert HashMap<String, Vec<Order>> to HashMap<String, BTreeMap<OrderedFloat<f64>, VecDeque<Order>>>
        let mut converted_orders: OrderBooks = HashMap::new();
        for (pair, orders) in fetched_orders {
            let mut order_map: OrderMap = BTreeMap::new();
            for order in orders {
                order_map.entry(order.price).or_default().push_back(order);
            }
            converted_orders.insert(pair, order_map);
        }
        converted_orders
    };

    let order_book_service = OrderBookService {
        order_books: Arc::new(Mutex::new(initial_order_books)),
        order_tx,
        trade_books: Arc::new(Mutex::new(HashMap::new())),
    };
    let order_book_service_arc: Arc<OrderBookService> = Arc::new(order_book_service.clone());

    let service_clone: Arc<OrderBookService> = Arc::clone(&order_book_service_arc);
    let _config_clone = config.clone();
    tokio::spawn(async move {
        update_order_books(service_clone, symbols.iter().map(AsRef::as_ref).collect(), offline_mode).await;
    });

    let service_clone: Arc<OrderBookService> = Arc::clone(&order_book_service_arc);
    tokio::spawn(async move {
        process_orders(service_clone, order_rx).await;
    });

    info!("Exchange is listening on {}", addr);

    Server::builder().add_service(OrderBookServer::new(order_book_service)).serve(addr).await?;

    Ok(())
}

// Test module
#[cfg(test)]
mod tests {
    mod integration_tests;
}
