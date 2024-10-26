use orderbook::order_book_client::OrderBookClient;
use orderbook::{OrderRequest, TradeBookRequest};
use structopt::StructOpt;

pub mod orderbook {
    tonic::include_proto!("orderbook");
}

#[derive(StructOpt, Debug)]
#[structopt(name = "Trading-CLI", about = "A CLI to submit market and limit trades or retrieve trades from the trade book.")]
struct Cli {
    #[structopt(subcommand)]
    command: Command,
}

#[derive(StructOpt, Debug)]
enum Command {
    /// Submit a market or limit order (example: client market-order XXBTZUSD 0.01 sell limit 65290.1 Rock)
    #[structopt(name = "market-order")]
    MarketOrder {
        /// Trading pair (e.g., XXBTZUSD, XETHZUSD, SUIUSD)
        #[structopt(help = "Trading pair (e.g., XXBTZUSD, XETHZUSD, SUIUSD)")]
        pair: String,
        
        /// Volume of the asset to trade (e.g., 0.01, 0.1, 1.0)
        #[structopt(help = "Volume of the asset to trade")]
        volume: f64,
        
        /// Side of the order (buy or sell)
        #[structopt(help = "Side of the order (buy or sell)")]
        side: String,
        
        /// Type of the order (market or limit)
        #[structopt(help = "Type of the order (market or limit)")]
        order_type: String,
        
        /// Price for the limit order
        #[structopt(help = "Price for the limit order")]
        price: f64,
        
        /// Trader's identifier
        #[structopt(help = "Trader's identifier")]
        trader: String,
    },
    
    /// Retrieve trades for a specific trader (example: client retrieve-trades Rock)
    #[structopt(name = "retrieve-trades")]
    RetrieveTrades {
        /// Trader's identifier
        #[structopt(help = "Trader's identifier")]
        trader: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = OrderBookClient::connect("http://[::1]:50051").await?;
    println!("Connected to Server...");

    let args = Cli::from_args();

    match args.command {
        Command::MarketOrder { pair, volume, side, order_type, price, trader } => {
            let market_order_request = tonic::Request::new(OrderRequest {
                pair,
                volume,
                side,
                order_type,
                price,
                trader,
            });
            let response = client.place_market_order(market_order_request).await?;
            println!("Order Response: {:?}", response.into_inner());
        },
        Command::RetrieveTrades { trader } => {
            let trade_book_request = tonic::Request::new(TradeBookRequest {
                trader: trader.clone(),
            });
            let response = client.get_trade_book(trade_book_request).await?;
            let trade_book_response = response.into_inner();
            println!("Trades for trader {}:", trader.clone());
            for trade in trade_book_response.trades {
                println!(
                    "{}: ID: {}, Pair: {}, Side: {}, Price: {:.5}, Volume: {:.3}, Timestamp: {}",
                    trade.status, trade.id, trade.pair, trade.side, trade.price, trade.volume, trade.timestamp
                );
            }
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_market_order() {
        let args = vec![
            "client",
            "market-order",
            "XXBTZUSD",
            "0.01",
            "sell",
            "limit",
            "65290.1",
            "Rock",
        ];
        let cli = Cli::from_iter_safe(args).unwrap();
        if let Command::MarketOrder { pair, volume, side, order_type, price, trader } = cli.command {
            assert_eq!(pair, "XXBTZUSD");
            assert_eq!(volume, 0.01);
            assert_eq!(side, "sell");
            assert_eq!(order_type, "limit");
            assert_eq!(price, 65290.1);
            assert_eq!(trader, "Rock");
        } else {
            panic!("Expected MarketOrder command");
        }
    }

    #[test]
    fn test_cli_retrieve_trades() {
        let args = vec!["client", "retrieve-trades", "Rock"];
        let cli = Cli::from_iter_safe(args).unwrap();
        if let Command::RetrieveTrades { trader } = cli.command {
            assert_eq!(trader, "Rock");
        } else {
            panic!("Expected RetrieveTrades command");
        }
    }
}